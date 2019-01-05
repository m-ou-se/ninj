mod graph;
mod queue;

use std::collections::BTreeMap;
use self::graph::generate_graph;
use ninj::spec::{read, BuildRuleCommand};
use raw_string::{RawStr, RawString};
use std::convert::AsRef;
use std::process::exit;
use structopt::StructOpt;
use self::queue::BuildQueue;

#[derive(StructOpt)]
struct Options {
	/// The targets to build. Empty to build the default targets.
	#[structopt(parse(from_str))]
	targets: Vec<RawString>,

	// /// Dry run: Don't actually any run commands, but pretend they succeed.
	// #[structopt(short = "n")]
	// dry_run: bool,

	/// Run a subtool. Use -t list to list subtools.
	#[structopt(short = "t")]
	tool: Option<String>,
}

fn main() {
	let opt = Options::from_args();

	let spec = read("build.ninja".as_ref()).unwrap_or_else(|e| {
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

	let targets: Vec<usize> = targets.into_iter().map(|target| {
		*target_to_rule.get(&target[..]).unwrap_or_else(|| {
			eprintln!("Unknown target {:?}", target);
			exit(1);
		})
	}).collect();

	let queue = BuildQueue::new(&spec, &target_to_rule, targets);

	let n_threads = 8;

	eprintln!("Building:");
	for _ in 0..n_threads {
		println!("=> ");
	}
	let print_status = |i, args: std::fmt::Arguments| {
		if i == n_threads - 1 {
			eprint!("\x1b[A\x1b[3C{}\x1b[K\x1b[m\n", args);
		} else {
			eprint!("\x1b[{}A\x1b[3C{}\x1b[K\x1b[m\n\x1b[{}B", n_threads - i, args, n_threads - i - 1);
		}
	};
	crossbeam::thread::scope(|scope| {
		for i in 0..n_threads {
			let queue = &queue;
			let spec = &spec;
			scope.spawn(move |_| {
				let mut lock = queue.lock();
				while let Some(task) = lock.next().or_else(move || {
					print_status(i, format_args!("\x1b[34mIdle"));
					lock.wait()
				}) {
					match &spec.build_rules[task].command {
						BuildRuleCommand::Phony => {}
						BuildRuleCommand::Command {
							description, ..
						} => {
							print_status(i, format_args!("\x1b[33m{} ...", description));
							std::thread::sleep(std::time::Duration::from_millis(2500 + i * 5123 % 2000));
						}
					}
					lock = queue.lock();
					lock.complete_task(task);
				}
				print_status(i, format_args!("\x1b[32mDone"));
			});
		}
	}).unwrap();
	eprintln!("\x1b[32;1mFinished.\x1b[m");
}
