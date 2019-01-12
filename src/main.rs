mod graph;
mod logger;
mod status;
mod timeformat;
mod worker;

use self::logger::Logger;
use self::graph::generate_graph;
use self::status::{BuildStatus, show_build_status};
use self::worker::Worker;
use ninj::outdated::is_outdated;
use ninj::queue::{BuildQueue, DepInfo, TaskInfo};
use ninj::mtime::{Timestamp, StatCache};
use ninj::buildlog::BuildLog;
use ninj::deplog::DepLogMut;
use ninj::spec::read;
use raw_string::unix::RawStrExt;
use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;
use std::path::{PathBuf, Path};
use std::process::exit;
use std::sync::Mutex;
use std::time::Instant;
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
			queue.complete_task(task, None);
		}
		exit(0);
	}

	let n_threads = opt.n_threads;
	let queue = queue.make_async();
	let dep_log = Mutex::new(dep_log);
	let status = BuildStatus::new(n_threads);
	let start_time = Instant::now();

	crossbeam::thread::scope(|scope| {
		for i in 0..n_threads {
			let worker = Worker {
				id: i,
				queue: &queue,
				spec: &spec,
				status: &status,
				sleep: opt.sleep_run,
				dep_log: &dep_log,
			};
			scope.spawn(move |_| worker.run().unwrap());
		}
		if opt.debug {
			debug!("Regular output disabled because debug messages are enabled.");
		} else {
			show_build_status(start_time, &status, &queue, &spec, opt.sleep_run);
		}
	}).unwrap();
}
