use crate::timeformat::MinSec;
use ninj::queue::{AsyncBuildQueue, TaskStatus};
use ninj::spec::Spec;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum WorkerStatus {
	Starting,
	Idle,
	Running { task: usize },
	Done,
}

pub struct BuildStatusInner {
	pub workers: Vec<WorkerStatus>,
	pub dirty: bool,
}

pub struct BuildStatus {
	pub inner: Mutex<BuildStatusInner>,
	pub condvar: Condvar,
}

impl BuildStatus {
	pub fn new(n_threads: usize) -> Self {
		BuildStatus {
			inner: Mutex::new(BuildStatusInner {
				workers: vec![WorkerStatus::Starting; n_threads],
				dirty: true,
			}),
			condvar: Condvar::new(),
		}
	}

	pub fn set_status(&self, worker: usize, status: WorkerStatus) {
		let mut lock = self.inner.lock().unwrap();
		lock.workers[worker] = status;
		lock.dirty = true;
		self.condvar.notify_all();
	}
}

pub fn show_build_status(
	start_time: Instant,
	status: &BuildStatus,
	queue: &AsyncBuildQueue,
	spec: &Spec,
	sleep: bool,
) {
	let mut lock = status.inner.lock().unwrap();
	println!("{}:", if sleep { "Sleeping" } else { "Building" });
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
					let command = spec.build_rules[*task]
						.command
						.as_ref()
						.expect("Got phony task");
					let statustext = match queuestate.get_task_status(*task) {
						TaskStatus::Running { start_time } => {
							format!("{}", MinSec::since(start_time))
						}
						x => format!("{:?}", x),
					};
					println!(
						"=> [{t}] \x1b[33m{d} ...\x1b[K\x1b[m",
						d = command.description,
						t = statustext
					);
				}
			}
		}
		println!("Building for {}...", MinSec::since(start_time));
		if workers.iter().all(|worker| *worker == WorkerStatus::Done) {
			break;
		}
		print!("\x1b[{}A", workers.len() + 1);
		lock = status.inner.lock().unwrap();
	}
	println!("\x1b[32;1mFinished.\x1b[m");
}
