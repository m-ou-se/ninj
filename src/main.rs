mod graph;
mod logger;
mod timeformat;

use self::logger::Logger;
use self::graph::generate_graph;
use ninj::outdated::is_outdated;
use ninj::queue::{BuildQueue, DepInfo, TaskInfo, TaskStatus};
use ninj::mtime::{Timestamp, StatCache};
use self::timeformat::MinSec;
use ninj::buildlog::BuildLog;
use ninj::deplog::DepLogMut;
use ninj::depfile::read_deps_file;
use ninj::spec::{read, DepStyle};
use raw_string::unix::RawStrExt;
use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;
use std::path::{PathBuf, Path};
use std::process::exit;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use structopt::StructOpt;
use log::{error, debug};

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

	/// Enable debug messages.
	#[structopt(long)]
	debug: bool,
}

fn main() {
	log::set_logger(&Logger).unwrap();
	log::set_max_level(log::LevelFilter::Warn);

	let opt = Options::from_args();

	if let Some(dir) = opt.directory.as_ref() {
		std::env::set_current_dir(dir).unwrap_or_else(|e| {
			error!("Unable to change directory to {:?}: {}", dir, e);
			exit(1);
		});
	}

	if opt.debug {
		log::set_max_level(log::LevelFilter::Debug);
		debug!("Debug messages enabled.");
	}

	let spec = read(&opt.file).unwrap_or_else(|e| {
		error!("{}", e);
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
				error!(
					"Warning, multiple rules generating {:?}. Ignoring all but last one.",
					output
				);
			}
		}
	}

	let build_dir = spec.build_dir.as_ref().map_or(Path::new(""), |p| p.as_path());

	let build_log = BuildLog::read(build_dir.join(".ninja_log")).unwrap_or_else(|e| {
		error!("Error while reading .ninja_log: {}", e);
		error!("Not using .ninja_log.");
		BuildLog::new()
	});

	let dep_log = DepLogMut::open(build_dir.join(".ninja_deps")).unwrap_or_else(|e| {
		error!("Error while reading .ninja_deps: {}", e);
		// TODO: Delete and start a new file.
		exit(1);
	});

	if let Some(tool) = opt.tool {
		match &tool[..] {
			"graph" => generate_graph(&spec),
			"log" => println!("{:#?}", build_log),
			"deps" => {
				for (path, deps) in dep_log.iter() {
					if target_to_rule.contains_key(&path[..]) {
						let mtime = || {
							std::fs::metadata(path.as_path())
								.and_then(|m| m.modified())
								.ok()
								.map(Timestamp::from_system_time)
						};
						let nanos = deps.mtime().map_or(0, Timestamp::to_nanos);
						println!(
							"{}: #deps {}, deps mtime {}.{:09} ({})",
							path,
							deps.deps().len(),
							nanos / 1_000_000_000,
							nanos % 1_000_000_000,
							if deps.mtime().map_or(true, |t| Some(t) < mtime()) {
								"STALE"
							} else {
								"VALID"
							}
						);
						for dep in deps.deps() {
							println!("    {}", dep);
						}
						println!();
					}
				}
			}
			"targets" => {
				for target in &spec.build_rules {
					for output in &target.outputs {
						println!("{}: {}", output, target.command.as_ref().map_or("phony", |c| &c.rule_name));
					}
				}
			}
			"list" => {
				println!("Subtools:\n\tdeps\n\tgraph\n\tlog\n\ttargets");
			}
			x => {
				error!("Unknown subtool {:?}.", x);
				exit(1);
			}
		}
		exit(0);
	}

	let targets = targets.into_iter().map(|target| {
		*target_to_rule.get(&target[..]).unwrap_or_else(|| {
			error!("Unknown target {:?}", target);
			exit(1);
		})
	});

	let mut stat_cache = StatCache::new();
	let mut dep_stat_cache = StatCache::new();

	let mut queue = BuildQueue::new(
		spec.build_rules.len(),
		targets,
		|task: usize| {
			let rule = &spec.build_rules[task];
			let mut dependencies = Vec::new();
			let outdated = is_outdated(
				rule,
				&dep_log,
				&mut stat_cache,
				&mut dep_stat_cache,
				|dependency: &RawStr, order_only| {
					let task = target_to_rule.get(dependency);
					if let Some(&task) = task {
						dependencies.push(DepInfo { task, order_only });
					}
					task.is_some()
				},
			).unwrap();
			TaskInfo {
				dependencies,
				phony: rule.is_phony(),
				outdated,
			}
		}
	);

	drop(dep_stat_cache);

	if opt.dry_run {
		let n_tasks = queue.n_left();
		while let Some(task) = queue.next() {
			let c = spec.build_rules[task].command.as_ref().expect("Got phony task.");
			let label = if opt.verbose || c.description.is_empty() {
				&c.command
			} else {
				&c.description
			};
			println!("[{}/{}] {}", n_tasks - queue.n_left(), n_tasks, label);
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
		Running { task: usize },
		Done,
	}

	struct BuildStatusInner {
		workers: Vec<WorkerStatus>,
		dirty: bool,
	}

	struct BuildStatus {
		inner: Mutex<BuildStatusInner>,
		condvar: Condvar,
	}

	let status = BuildStatus {
		inner: Mutex::new(BuildStatusInner {
			workers: vec![WorkerStatus::Starting; n_threads],
			dirty: true,
		}),
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

	let dep_log = Mutex::new(dep_log);

	let starttime = Instant::now();

	crossbeam::thread::scope(|scope| {
		let mut lock = status.inner.lock().unwrap();
		for i in 0..n_threads {
			let queue = &queue;
			let spec = &spec;
			let status = &status;
			let opt = &opt;
			let dep_log = &dep_log;
			scope.spawn(move |_| {
				let log = format!("ninj::worker-{}", i);
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
					status.set_status(i, WorkerStatus::Running { task });
					if opt.sleep_run {
						std::thread::sleep(std::time::Duration::from_millis(2500 + i as u64 * 5123 % 2000));
					} else {
						let rule = &spec.build_rules[task].command.as_ref().expect("Got phony task.");
						debug!(target: &log, "Running: {:?}", rule.command);
						let status = std::process::Command::new("sh")
							.arg("-c")
							.arg(rule.command.as_osstr())
							.status()
							.unwrap_or_else(|e| {
								error!("Unable to spawn sh process: {}", e);
								exit(1);
							});
						match status.code() {
							Some(0) => {}
							Some(x) => {
								error!("Exited with status code {}: {}", x, rule.command);
								exit(1);
							}
							None => {
								error!("Exited with signal: {}", rule.command);
								exit(1);
							}
						}
						if rule.deps == Some(DepStyle::Gcc) {
							read_deps_file(rule.depfile.as_path(), |target, deps| {
								// TODO: Don't use now().
								let mtime = Timestamp::from_system_time(std::time::SystemTime::now());
								dep_log.lock().unwrap().insert_deps(target, Some(mtime), deps).unwrap_or_else(|e| {
									error!("Unable to update dependency log: {}", e);
									exit(1);
								});
								Ok(())
							}).unwrap_or_else(|e| {
								error!("Unable to read dependency file {:?}: {}", rule.depfile, e);
								exit(1);
							});
						}
					}
					lock = queue.lock();
					lock.complete_task(task, true);
				}
				status.set_status(i, WorkerStatus::Done);
			});
		}
		if opt.debug {
			debug!("Regular output disabled because debug messages are enabled.");
			return;
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
						let command = spec.build_rules[*task].command.as_ref().expect("Got phony task");
						let statustext = match queuestate.get_task_status(*task) {
							TaskStatus::Running { start_time } => {
								format!("{}", MinSec::since(start_time))
							},
							x => {
								format!("{:?}", x)
							}
						};
						println!("=> [{t}] \x1b[33m{d} ...\x1b[K\x1b[m", d=command.description, t=statustext);
					},
				}
			}
			println!("Building for {}...", MinSec::since(starttime));
			if workers.iter().all(|worker| *worker == WorkerStatus::Done) {
				break;
			}
			print!("\x1b[{}A", workers.len() + 1);
			lock = status.inner.lock().unwrap();
		}
	}).unwrap();
	println!("\x1b[32;1mFinished.\x1b[m");
}
