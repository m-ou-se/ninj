mod graph;
mod queue;
mod statcache;
mod timeformat;

use self::graph::generate_graph;
use self::queue::{BuildQueue, DepInfo, TaskStatus, TaskInfo};
use ninj::spec::{read, BuildRuleCommand};
use ninj::deplog::Deps;
use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::exit;
use structopt::StructOpt;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};
use self::timeformat::MinSec;
use raw_string::unix::RawStrExt;
use self::statcache::StatCache;

#[derive(StructOpt)]
struct Options {
	/// The targets to build. Empty to build the default targets.
	#[structopt(parse(from_str))]
	targets: Vec<RawString>,

	/// Change directory before doing anything else.
	#[structopt(short = "C", parse(from_os_str))]
	directory: Option<PathBuf>,

	/// Dry run: Don't actually any run commands, but instead list what commands would be run.
	#[structopt(short = "n")]
	dry_run: bool,

	/// Show command lines instead of descriptions. (Currently only in
	/// combination with -n.)
	#[structopt(short = "v")]
	verbose: bool,

	/// Sleep run: Instead of running commands, sleep a few seconds instead.
	#[structopt(long = "sleep")]
	sleep_run: bool,

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

	let mut target_to_rule = BTreeMap::<&RawStr, usize>::new();
	for (rule_i, rule) in spec.build_rules.iter().enumerate() {
		for output in &rule.outputs {
			if target_to_rule.insert(&output, rule_i).is_some() {
				eprintln!("Warning, multiple rules generating {:?}. Ignoring all but last one.", output);
			}
		}
	}

	let deps_file = Deps::read(spec.build_dir.as_path().join(".ninja_deps")).unwrap_or_else(|e| {
		eprintln!("Error while reading .ninja_deps: {}", e);
		exit(1);
	});

	if let Some(tool) = opt.tool {
		match &tool[..] {
			"graph" => generate_graph(&spec),
			"deps" => {
				for record in &deps_file.records {
					if let Some(deps) = &record.deps {
						if target_to_rule.contains_key(&record.path[..]) {
							let mtime = || std::fs::metadata(record.path.as_path()).and_then(|m| m.modified()).unwrap_or(UNIX_EPOCH);
							let deps_mtime = UNIX_EPOCH + Duration::from_nanos(deps.mtime);
							println!(
								"{}: #deps {}, deps mtime {}.{:09} ({})",
								record.path,
								deps.deps.len(),
								deps.mtime / 1_000_000_000,
								deps.mtime % 1_000_000_000,
								if deps.mtime == 0 || deps_mtime < mtime() { "STALE" } else { "VALID" }
							);
							for &dep in &deps.deps {
								println!("    {}", deps_file.records[dep as usize].path);
							}
							println!();
						}
					}
				}
			}
			"list" => {
				println!("Subtools:\n\tdeps\n\tgraph");
			}
			x => {
				eprintln!("Unknown subtool {:?}.", x);
				exit(1);
			}
		}
		exit(0);
	}

	let targets = targets.into_iter().map(|target| {
		*target_to_rule.get(&target[..]).unwrap_or_else(|| {
			eprintln!("Unknown target {:?}", target);
			exit(1);
		})
	});

	let mut stat_cache = StatCache::new();

	let mut queue = BuildQueue::new(
		spec.build_rules.len(),
		targets,
		|task: usize| {
			let rule = &spec.build_rules[task];

			// Get the time of the oldest output.
			let mut output_time = None;
			let mut outdated = false;
			for output in &rule.outputs {
				if let Some(mtime) = stat_cache.mtime(output.as_path()) {
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
			for (input, order_only) in rule.inputs.iter().map(|p| (p, false))
				.chain(rule.order_deps.iter().map(|p| (p, true)))
			{
				let task = target_to_rule.get(&input[..]);
				if let Some(&task) = task {
					dependencies.push(DepInfo { task: task, order_only });
				}
				if let Some(mtime) = stat_cache.mtime(input.as_path()) {
					if output_time.map_or(false, |m| m < mtime) {
						outdated = true;
					}
				} else if task.is_none() {
					// The file does not exist, and no rule generates it.
					eprintln!("Missing file {:?}", input);
					exit(1);
				}
			}

			TaskInfo { dependencies, phony: rule.is_phony(), outdated }
		}
	);

	if opt.dry_run {
		let n_tasks = queue.n_left();
		while let Some(task) = queue.next() {
			match &spec.build_rules[task].command {
				BuildRuleCommand::Phony => unreachable!("Got phony task."),
				BuildRuleCommand::Command { description, command, .. } => {
					let label = if opt.verbose || description.is_empty() {
						command
					} else {
						description
					};
					println!("[{}/{}] {}", n_tasks - queue.n_left(), n_tasks, label);
				}
			};
			queue.complete_task(task, true);
		}
		exit(0);
	}

	let queue = queue.make_async();

	let n_threads = opt.n_threads;

	#[derive(Debug, Clone, PartialEq)]
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
			let opt = &opt;
			scope.spawn(move |_| {
				let mut lock = queue.lock();
				loop {
					let mut next = lock.next();
					drop(lock);
					if next.is_none() {
						status.set_status(i, WorkerStatus::Idle);
						next = queue.lock().wait();
					}
					let task = if let Some(task) = next {
						task
					} else {
						// There are no remaining jobs
						break;
					};
					status.set_status(i, WorkerStatus::Running{task});
					let command = match &spec.build_rules[task].command {
						BuildRuleCommand::Phony => unreachable!("Got phony task."),
						BuildRuleCommand::Command { command, .. } => {
							command
						}
					};
					if opt.sleep_run {
						std::thread::sleep(std::time::Duration::from_millis(2500 + i as u64 * 5123 % 2000));
					} else {
						let status = std::process::Command::new("sh")
							.arg("-c")
							.arg(command.as_osstr())
							.status()
							.unwrap_or_else(|e| {
								eprintln!("Unable to spawn sh process: {}", e);
								exit(1);
							});
						match status.code() {
							Some(0) => {},
							Some(x) => {
								eprintln!("Exited with status code {}: {}", x, command);
								exit(1);
							}
							None => {
								eprintln!("Exited with signal: {}", command);
								exit(1);
							}
						}
					}
					lock = queue.lock();
					lock.complete_task(task, true);
				}
				status.set_status(i, WorkerStatus::Done);
			});
		}
		println!("{}:", if opt.sleep_run { "Sleeping" } else { "Building" });
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
