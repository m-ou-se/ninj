use log::{debug, error};
use ninj::buildlog::BuildLog;
use ninj::depfile::read_deps_file;
use ninj::deplog::DepLogMut;
use ninj::mtime::Timestamp;
use ninj::queue::AsyncBuildQueue;
use ninj::spec::{DepStyle, Spec};
use raw_string::unix::RawStrExt;
use std::io::Error;
use std::process::exit;
use std::sync::Mutex;
use std::time::Instant;

/// A worker that executes tasks of a [`Spec`] according to a [`BuildQueue`].
pub struct Worker<'a> {
	pub id: usize,
	pub spec: &'a Spec,
	pub queue: &'a AsyncBuildQueue,
	pub status_updater: &'a (dyn StatusUpdater + Sync),
	pub sleep: bool,
	pub dep_log: &'a Mutex<DepLogMut>,
	pub build_log: &'a Mutex<BuildLog>,
	pub start_time: Instant,
}

/// Something that a [`Worker`] can report its status to.
pub trait StatusUpdater {
	fn idle(&self, worker: usize);
	fn running(&self, worker: usize, task: usize);
	fn done(&self, worker: usize);
	fn failed(&self, worker: usize);
}

impl<'a> Worker<'a> {
	pub fn run(self) -> Result<(), Error> {
		// This function wraps _run, to make sure we always call done() or
		// failed() on the StatusUpdater.
		let id = self.id;
		let s = self.status_updater;
		let result = self.run_();
		if result.is_err() {
			s.failed(id);
		} else {
			s.done(id);
		}
		result
	}

	pub fn run_(self) -> Result<(), Error> {
		let Worker {
			id,
			queue,
			spec,
			status_updater,
			sleep,
			dep_log,
			build_log,
			start_time
		} = self;
		let log = format!("ninj::worker-{}", id);
		let mut lock = queue.lock();
		loop {
			let mut next = lock.next();
			drop(lock);
			if next.is_none() {
				status_updater.idle(id);
				next = queue.lock().wait();
			}
			let task = if let Some(task) = next {
				task
			} else {
				// There are no remaining jobs
				break;
			};
			let worker_start_time = Instant::now();
			status_updater.running(id, task);
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
							"I should check if {:?} is now outdated.",
							spec.build_rules[task].outputs
						);
						false
					};
					restat = Some::<&mut dyn FnMut(usize) -> bool>(&mut restat_fn);
				} else {
					restat = None;
				}
			}
			build_log.lock().unwrap().add_entry(&spec.build_rules[task], start_time, worker_start_time, Instant::now());
			lock = queue.lock();
			lock.complete_task(task, restat);
		}
		Ok(())
	}
}
