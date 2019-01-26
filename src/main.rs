mod logger;
mod status;
mod subtools;
mod timeformat;
mod worker;

use self::logger::Logger;
use self::status::{show_build_status, BuildStatus, ProgressFormat};
use self::worker::status::WorkerStatusUpdater;
use self::worker::Worker;
use log::{debug, error};
use ninj::buildlog::BuildLog;
use ninj::deplog::DepLogMut;
use ninj::mtime::StatCache;
use ninj::outdated::is_outdated;
use ninj::queue::{BuildQueue, DepInfo, TaskInfo};
use ninj::spec::read;
use raw_string::{RawStr, RawString};
use std::path::PathBuf;
use std::process::exit;
use std::sync::Mutex;
use std::time::Instant;
use structopt::StructOpt;

#[derive(StructOpt)]
struct Options {
	/// The targets to build. Empty to build the default targets.
	#[structopt(parse(from_str))]
	targets: Vec<RawString>,

	/// Change directory before doing anything else.
	#[structopt(short = "C", parse(from_os_str))]
	directory: Option<PathBuf>,

	/// Dry run: Don't actually any run commands, but instead list what commands
	/// would be run.
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

	/// Set format of progress indication (none/text/ascii/highres).
	#[structopt(short = "P", long = "progress", default_value = "highres")]
	progress: ProgressFormat,
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

	if let Some(tool) = opt.tool.as_ref() {
		subtools::run_subtool(tool, &opt).unwrap_or_else(|e| {
			error!("{}", e);
			exit(1);
		});
		exit(0);
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

	let target_to_rule = spec.make_index();

	let build_log = Mutex::new(
		BuildLog::read(spec.build_dir().join(".ninja_log")).unwrap_or_else(|e| {
			error!("Error while reading .ninja_log: {}", e);
			error!("Not using .ninja_log.");
			BuildLog::new()
		}),
	);

	let dep_log = DepLogMut::open(spec.build_dir().join(".ninja_deps")).unwrap_or_else(|e| {
		error!("Error while reading .ninja_deps: {}", e);
		// TODO: Delete and start a new file.
		exit(1);
	});

	let targets = targets.iter().map(|target| {
		*target_to_rule.get(&target[..]).unwrap_or_else(|| {
			error!("Unknown target {:?}", target);
			exit(1);
		})
	});

	let mut stat_cache = StatCache::new();
	let mut dep_stat_cache = StatCache::new();

	let mut queue = BuildQueue::new(spec.build_rules.len(), targets, |task: usize| {
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
		)
		.unwrap();
		TaskInfo {
			dependencies,
			phony: rule.is_phony(),
			outdated,
		}
	});

	drop(dep_stat_cache);

	if opt.dry_run {
		let n_tasks = queue.n_left();
		while let Some(task) = queue.next() {
			let c = spec.build_rules[task]
				.command
				.as_ref()
				.expect("Got phony task.");
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
				spec: &spec,
				queue: &queue,
				status_updater: WorkerStatusUpdater {
					status_listener: &status,
					worker_id: i,
				},
				sleep: opt.sleep_run,
				dep_log: &dep_log,
				build_log: &build_log,
				start_time,
			};
			scope.spawn(move |_| worker.run());
		}
		if opt.debug {
			debug!("Regular output disabled because debug messages are enabled.");
		} else {
			show_build_status(
				start_time,
				&status,
				&queue,
				&spec,
				&build_log,
				opt.progress,
			);
		}
	})
	.unwrap();

	build_log
		.into_inner()
		.unwrap()
		.write(spec.build_dir().join(".ninja_log"))
		.unwrap_or_else(|e| {
			eprintln!("Unable to store logfile: {}", e);
			exit(1);
		});
}
