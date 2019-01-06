mod graph;
mod queue;
mod timeformat;

use self::graph::generate_graph;
use self::queue::{BuildQueue, TaskStatus, TaskInfo};
use ninj::spec::{read, BuildRuleCommand};
use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::exit;
use structopt::StructOpt;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use self::timeformat::MinSec;
use raw_string::unix::RawStrExt;
use std::os::unix::fs::MetadataExt;

#[derive(StructOpt)]
struct Options {
	/// The targets to build. Empty to build the default targets.
	#[structopt(parse(from_str))]
	targets: Vec<RawString>,

	/// Change directory before doing anything else.
	#[structopt(short = "C", parse(from_os_str))]
	directory: Option<PathBuf>,

	// /// Dry run: Don't actually any run commands, but pretend they succeed.
	// #[structopt(short = "n")]
	// dry_run: bool,

	/// Run a subtool. Use -t list to list subtools.
	#[structopt(short = "t")]
	tool: Option<String>,

	/// The build specification.
	#[structopt(short = "f", default_value = "build.ninja", parse(from_os_str))]
	file: PathBuf,

	/// Number of concurrent jobs.
	#[structopt(short = "j", default_value = "8")]
	n_threads: usize,
}

fn main() {
	let opt = Options::from_args();

	if let Some(dir) = opt.directory.as_ref() {
		std::env::set_current_dir(dir).unwrap_or_else(|e| {
			eprintln!("Unable to change directory to {:?}: {}", dir, e);
			exit(1);
		});
	}

	let spec = read(&opt.file).unwrap_or_else(|e| {
		eprintln!("{}", e);
		exit(1);
	});

	let targets: &[RawString] = if opt.targets.is_empty() {
		&spec.default_targets
	} else {
		&opt.targets
	};

	if let Some(tool) = opt.tool {
		match &tool[..] {
			"graph" => generate_graph(&spec),
			"list" => {
				println!("Subtools:\n\tgraph");
			}
			x => {
				eprintln!("Unknown subtool {:?}.", x);
				exit(1);
			}
		}
		exit(0);
	}

	let mut target_to_rule = BTreeMap::<&RawStr, usize>::new();

	for (rule_i, rule) in spec.build_rules.iter().enumerate() {
		for output in &rule.outputs {
			if target_to_rule.insert(&output, rule_i).is_some() {
				eprintln!("Warning, multiple rules generating {:?}. Ignoring all but last one.", output);
			}
		}
	}

	let targets = targets.into_iter().map(|target| {
		*target_to_rule.get(&target[..]).unwrap_or_else(|| {
			eprintln!("Unknown target {:?}", target);
			exit(1);
		})
	});

	let queue = BuildQueue::new(
		spec.build_rules.len(),
		targets,
		|task: usize| {
			let rule = &spec.build_rules[task];

			// Get the time of the oldest output.
			let mut output_time = None;
			let mut outdated = false;
			for output in &rule.outputs {
				if let Some(mtime) = mtime(output) {
					if output_time.map_or(true, |m| m > mtime) {
						output_time = Some(mtime);
					}
				} else {
					// This output doesn't even exist, so the task is definitely out of date.
					output_time = None;
					outdated = true;
					break;
				}
			}

			// Check all the inputs, and resolve dependencies.
			let mut dependencies = Vec::new();
			for input in &rule.inputs {
				let dep = target_to_rule.get(&input[..]);
				if let Some(&dep) = dep {
					dependencies.push(dep);
				}
				if let Some(mtime) = mtime(input) {
					if output_time.map_or(false, |m| m < mtime) {
						outdated = true;
					}
				} else if dep.is_none() {
					// The file does not exist, and no rule generates it.
					eprintln!("Missing file {:?}", input);
					exit(1);
				}
			}

			TaskInfo { dependencies, phony: rule.is_phony(), outdated }
		}
	).make_async();

	let n_threads = opt.n_threads;

	#[derive(Debug,Clone,PartialEq)]
	enum WorkerStatus {
		Starting,
		Idle,
		Running{task: usize},
		Done
	}

	struct BuildStatusInner {
		workers: Vec<WorkerStatus>,
		dirty: bool,
	}

	struct BuildStatus {
		inner: Mutex<BuildStatusInner>,
		condvar: Condvar,
	}

	let status = BuildStatus{
		inner: Mutex::new(BuildStatusInner{workers: vec![WorkerStatus::Starting; n_threads], dirty: true}),
		condvar: Condvar::new(),
	};

	impl BuildStatus {
		fn set_status(&self, worker: usize, status: WorkerStatus) {
			let mut lock = self.inner.lock().unwrap();
			lock.workers[worker] = status;
			lock.dirty = true;
			self.condvar.notify_all();
		}
	}

	let starttime = Instant::now();

	crossbeam::thread::scope(|scope| {
		let mut lock = status.inner.lock().unwrap();
		for i in 0..n_threads {
			let queue = &queue;
			let spec = &spec;
			let status = &status;
			scope.spawn(move |_| {
				let mut lock = queue.lock();
				while let Some(task) = lock.next().or_else(move || {
					drop(lock);
					status.set_status(i, WorkerStatus::Idle);
					queue.lock().wait()
				}) {
					status.set_status(i, WorkerStatus::Running{task});

					match &spec.build_rules[task].command {
						BuildRuleCommand::Phony => {}
						BuildRuleCommand::Command { .. } => {
							std::thread::sleep(std::time::Duration::from_millis(2500 + i as u64 * 5123 % 2000));
						}
					}
					lock = queue.lock();
					lock.complete_task(task, true);
				}

				status.set_status(i, WorkerStatus::Done);
			});
		}
		println!("Building:");
		loop {
			let mut now = Instant::now();
			let waittime = now + Duration::from_millis(100);
			while !lock.dirty && now < waittime {
				lock = status.condvar.wait_timeout(lock, waittime - now).unwrap().0;
				now = Instant::now();
			}
			let queuelock = queue.lock();
			let queuestate = queuelock.clone_queue();
			drop(queuelock);
			let workers = lock.workers.clone();
			lock.dirty = false;
			drop(lock);
			for worker in &workers {
				match worker {
					WorkerStatus::Starting => {
						println!("=> \x1b[34mStarting...\x1b[K\x1b[m");
					}
					WorkerStatus::Idle => {
						println!("=> \x1b[34mIdle\x1b[K\x1b[m");
					}
					WorkerStatus::Done => {
						println!("=> \x1b[32mDone\x1b[K\x1b[m");
					}
					WorkerStatus::Running { task } => {
						match &spec.build_rules[*task].command {
							BuildRuleCommand::Phony => {}
							BuildRuleCommand::Command { description, .. } => {
								let statustext = match queuestate.get_task_status(*task) {
									TaskStatus::Running { start_time } => {
										format!("{}", MinSec::since(start_time))
									},
									x => {
										format!("{:?}", x)
									}
								};
								println!("=> [{t}] \x1b[33m{d} ...\x1b[K\x1b[m", d=description, t=statustext);
							}
						}
					}
				}
			}
			println!("Building for {}...", MinSec::since(starttime));
			if workers.iter().all(|worker| *worker == WorkerStatus::Done ) {
				break;
			}
			print!("\x1b[{}A", workers.len() + 1);
			lock = status.inner.lock().unwrap();
		}
	}).unwrap();
	println!("\x1b[32;1mFinished.\x1b[m");
}

fn mtime(file: &RawStr) -> Option<i64> {
	match std::fs::metadata(file.as_path()) {
		Ok(meta) => Some(meta.mtime()),
		Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => None,
		Err(e) => {
			eprintln!("Unable to stat {:?}: {}", file, e);
			exit(1);
		}
	}
}
