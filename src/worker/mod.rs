pub mod status;
mod subprocess;

use self::status::{TaskStatusUpdater, WorkerStatusUpdater};
use self::subprocess::listen_to_child;
use log::{debug, error};
use ninj::buildlog::BuildLog;
use ninj::depfile::read_deps_file;
use ninj::deplog::DepLogMut;
use ninj::mtime;
use ninj::mtime::Timestamp;
use ninj::queue::AsyncBuildQueue;
use ninj::spec::{BuildCommand, BuildRule, DepStyle, Spec};
use raw_string::unix::RawStrExt;
use raw_string::RawStr;
use std::os::unix::process::ExitStatusExt;
use std::process::exit;
use std::process::ExitStatus;
use std::sync::Mutex;
use std::time::Instant;

/// A worker that executes tasks of a [`Spec`] according to a [`BuildQueue`].
pub struct Worker<'a> {
	pub spec: &'a Spec,
	pub queue: &'a AsyncBuildQueue,
	pub status_updater: WorkerStatusUpdater<'a>,
	pub sleep: bool,
	pub dep_log: &'a Mutex<DepLogMut>,
	pub build_log: &'a Mutex<BuildLog>,
	pub start_time: Instant,
}

impl<'a> Worker<'a> {
	/// Run the worker.
	pub fn run(self) {
		let log = format!("ninj::worker-{}", self.status_updater.worker_id);

		let mut queue = self.queue.lock();

		loop {
			// Get the next task from the queue.
			let mut next = queue.next();
			drop(queue);

			// If nothing is avialable now, block until there is.
			if next.is_none() {
				self.status_updater.idle();
				next = self.queue.lock().wait();
			}

			// If there's still nothing available, stop.
			let task = if let Some(task) = next {
				task
			} else {
				break;
			};

			// Look up the command for this task.
			let rule = &self.spec.build_rules[task];
			let command = rule.command.as_ref().expect("Got phony task");

			// Tell the world we're starting this task.
			let task_status_updater = self.status_updater.start_task(task);

			// Run the task.
			debug!(target: &log, "Running: {:?}", command.command);
			self.run_task(rule, task_status_updater);

			// Check if we need to re-stat anything.
			let mut restat_fn;
			let restat = if !self.sleep && command.restat {
				self.restat(task);
				restat_fn = |task: usize| self.recheck_outdated(task);
				Some::<&mut dyn FnMut(usize) -> bool>(&mut restat_fn)
			} else {
				None
			};

			// Update the queue now that another task is complete.
			queue = self.queue.lock();
			queue.complete_task(task, restat);
		}
	}

	fn restat(&self, task: usize) {
		// TODO
		debug!(
			"I should now re-stat {:?}",
			self.spec.build_rules[task].outputs
		);
	}

	fn recheck_outdated(&self, task: usize) -> bool {
		// TODO
		debug!(
			"I should check if {:?} is now outdated.",
			self.spec.build_rules[task].outputs
		);
		false
	}

	fn run_task(&self, rule: &BuildRule, status_updater: TaskStatusUpdater) {
		let command = rule.command.as_ref().expect("Got phony rule");

		if self.sleep {
			// Just sleep. Zzz.
			std::thread::sleep(std::time::Duration::from_millis(
				2500 + command.command.len() as u64 * 5123 % 2000,
			));

			// Pretend success.
			status_updater.finished(ExitStatus::from_raw(0));

			return;
		}

		// Start the clock!
		let start_time = Instant::now();

		// Run the command, capturing its output.
		let child = std::process::Command::new("sh")
			.arg("-c")
			.arg(command.command.as_osstr())
			.stdin(std::process::Stdio::null())
			.stdout(std::process::Stdio::piped())
			.stderr(std::process::Stdio::piped())
			.spawn()
			.unwrap_or_else(|e| {
				error!("Unable to spawn sh process: {}", e);
				exit(1);
			});

		// Listen for output.
		let status = listen_to_child(child, 100, &|output| {
			status_updater.output(RawStr::from(output));
		})
		.unwrap_or_else(|e| {
			error!("Unable to read from subprocess: {}", e);
			exit(1);
		});

		// Report the status.
		status_updater.finished(status);

		// Handle a failed task.
		if !status.success() {
			error!("Command exited with {}: {}", status, command.command);
			exit(1);
		}

		// Check for any extra dependencies.
		match command.deps {
			Some(DepStyle::Gcc) => self.check_gcc_deps(command),
			Some(DepStyle::Msvc) => unimplemented!("MSVC-style dependencies"),
			None => {}
		}

		// Stop the clock!
		let end_time = Instant::now();

		let mut mtime = None;
		// TODO: If this is a restat rule and any output's mtime is unchanged,
		// use the mtime of the newest input instead.
		for output in &rule.outputs {
			mtime = mtime.max(mtime::mtime(output.as_path()).unwrap_or_else(|e| {
				error!("Unable to get mtime of {:?}: {}", output, e);
				exit(1);
			}));
		}

		// Record the success to the build log.
		self.build_log
			.lock()
			.unwrap()
			.add_entry(rule, self.start_time, start_time, end_time, mtime);
	}

	fn check_gcc_deps(&self, command: &BuildCommand) {
		// TODO: Don't use now().
		let mtime = Timestamp::from_system_time(std::time::SystemTime::now());
		read_deps_file(command.depfile.as_path(), |target, deps| {
			self.dep_log
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
			error!(
				"Unable to read dependency file {:?}: {}",
				command.depfile, e
			);
			exit(1);
		});
		std::fs::remove_file(command.depfile.as_path()).unwrap_or_else(|e| {
			error!(
				"Unable to remove dependency file {:?}: {}",
				command.depfile, e
			);
			exit(1);
		});
	}
}
