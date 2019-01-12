use crate::status::{BuildStatus, WorkerStatus};
use log::{debug, error};
use ninj::depfile::read_deps_file;
use ninj::deplog::DepLogMut;
use ninj::mtime::Timestamp;
use ninj::queue::AsyncBuildQueue;
use ninj::spec::{DepStyle, Spec};
use raw_string::unix::RawStrExt;
use std::io::Error;
use std::process::exit;
use std::sync::Mutex;

pub struct Worker<'a> {
	pub id: usize,
	pub queue: &'a AsyncBuildQueue,
	pub spec: &'a Spec,
	pub status: &'a BuildStatus,
	pub sleep: bool,
	pub dep_log: &'a Mutex<DepLogMut>,
}

impl<'a> Worker<'a> {
	pub fn run(self) -> Result<(), Error> {
		let Worker { id, queue, spec, status, sleep, dep_log } = self;
		let log = format!("ninj::worker-{}", id);
		let mut lock = queue.lock();
		loop {
			let mut next = lock.next();
			drop(lock);
			if next.is_none() {
				status.set_status(id, WorkerStatus::Idle);
				next = queue.lock().wait();
			}
			let task = if let Some(task) = next {
				task
			} else {
				// There are no remaining jobs
				break;
			};
			status.set_status(id, WorkerStatus::Running { task });
			let restat;
			let mut restat_fn;
			if sleep {
				std::thread::sleep(std::time::Duration::from_millis(
					2500 + id as u64 * 5123 % 2000,
				));
				restat = None;
			} else {
				let rule = &spec.build_rules[task]
					.command
					.as_ref()
					.expect("Got phony task.");
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
						dep_log
							.lock()
							.unwrap()
							.insert_deps(target, Some(mtime), deps)
							.unwrap_or_else(|e| {
								error!("Unable to update dependency log: {}", e);
								exit(1);
							});
						Ok(())
					})
					.unwrap_or_else(|e| {
						error!("Unable to read dependency file {:?}: {}", rule.depfile, e);
						exit(1);
					});
				}
				if rule.restat {
					debug!(
						target: &log,
						"I should now re-stat {:?}", spec.build_rules[task].outputs
					);
					restat_fn = |task: usize| {
						// TODO
						debug!(
							target: &log,
							"I should check if {:?} is now outdated.", spec.build_rules[task].outputs
						);
						false
					};
					restat = Some::<&mut dyn FnMut(usize) -> bool>(&mut restat_fn);
				} else {
					restat = None;
				}
			}
			lock = queue.lock();
			lock.complete_task(task, restat);
		}
		status.set_status(id, WorkerStatus::Done);
		Ok(())
	}
}
