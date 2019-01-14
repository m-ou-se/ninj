use chrono::Local;
use crate::timeformat::MinSec;
use crate::worker::StatusUpdater;
use ninj::buildlog::BuildLog;
use ninj::queue::{AsyncBuildQueue, TaskStatus};
use ninj::spec::Spec;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};
use time::Duration as TimeDuration;

#[derive(Debug, Clone, PartialEq)]
enum WorkerStatus {
	Starting,
	Idle,
	Running { task: usize },
	Done,
}

#[derive(Clone)]
struct BuildStatusInner {
	workers: Vec<WorkerStatus>,
	dirty: bool,
}

pub struct BuildStatus {
	inner: Mutex<BuildStatusInner>,
	condvar: Condvar,
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

	fn set_status(&self, worker: usize, status: WorkerStatus) {
		let mut lock = self.inner.lock().unwrap();
		lock.set_status(worker, status);
		self.condvar.notify_all();
	}
}

impl StatusUpdater for BuildStatus {
	fn idle(&self, worker: usize) {
		self.set_status(worker, WorkerStatus::Idle);
	}
	fn running(&self, worker: usize, task: usize) {
		self.set_status(worker, WorkerStatus::Running { task });
	}
	fn done(&self, worker: usize) {
		self.set_status(worker, WorkerStatus::Done);
	}
	fn failed(&self, worker: usize) {
		self.set_status(worker, WorkerStatus::Done);
	}
}

impl BuildStatusInner {
	fn set_status(&mut self, worker: usize, status: WorkerStatus) {
		self.workers[worker] = status;
		self.dirty = true;
	}

	fn all_workers_done(&self) -> bool {
		self.workers.iter().all(|worker| *worker == WorkerStatus::Done)
	}

	fn all_workers_done_or_idle(&self) -> bool {
		self.workers.iter().all(|worker| *worker == WorkerStatus::Done || *worker == WorkerStatus::Idle)
	}
}

fn estimated_total_task_time(spec: &Spec, task: usize, build_log: &std::sync::MutexGuard<BuildLog>) -> Option<Duration> {
	let rule = &spec.build_rules[task];
	let command = &rule.command.as_ref().expect("Got phony task").command;

	if rule.outputs.len() == 0 {
		build_log.average_historic_task_time()
	} else {
		let output = &rule.outputs[0];
		build_log.estimated_total_task_time(output.clone(), command.clone()).or_else(||{ build_log.average_historic_task_time() })
	}
}

pub fn show_build_status(
	start_time: Instant,
	status: &BuildStatus,
	queue: &AsyncBuildQueue,
	spec: &Spec,
	build_log: &Mutex<BuildLog>,
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
		let mut queuestate = queuelock.clone_queue();
		drop(queuelock);
		let mut buildstate = lock.clone();
		lock.dirty = false;
		drop(lock);
		for worker in &buildstate.workers {
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

		let build_is_done = buildstate.all_workers_done();

		// Compute remaining time for this build, by simulating a build.
		let mut simulated_time = Instant::now();
		let mut estimation_impossible = false;
		while !buildstate.all_workers_done_or_idle() {
			// Give all simulated workers something to do
			for i in 0..buildstate.workers.len() {
				let worker = &buildstate.workers[i];
				match worker {
					WorkerStatus::Starting | WorkerStatus::Idle => {
						let next = queuestate.next_at(simulated_time);
						buildstate.set_status(i, match next {
							Some(task) => WorkerStatus::Running{task},
							None => WorkerStatus::Idle,
						});
					}
					_ => {}
				}
			}

			// Find the job with the lowest remaining time
			let earliest_remaining_job = match buildstate.workers.iter()
				.enumerate()
				.flat_map(|(i, worker)| {
					match worker {
						WorkerStatus::Running{task, ..} => {
							let taskstatus = queuestate.get_task_status(*task);
							match taskstatus {
								TaskStatus::Running{start_time, ..} => {
									let runtime = if simulated_time > start_time { simulated_time - start_time } else { Duration::from_millis(0) };
									match estimated_total_task_time(spec, *task, &build_log.lock().unwrap()) {
										Some(time) => Some((i, task, time.checked_sub(runtime).unwrap_or(Duration::from_millis(0)))),
										None => None,
									}
								},
								TaskStatus::Finished{..} => Some((i, task, Duration::from_millis(0))),
								_ => unreachable!(),
							}
						},
						_ => None,
					}
				})
				.min_by_key(|&(_, _, time)| time)
			{
				Some(earliest_remaining_job) => earliest_remaining_job,
				None => {
					estimation_impossible = true;
					break;
				}
			};

			// Pass that time
			simulated_time += earliest_remaining_job.2;

			// Complete that task, if it isn't already Finished
			match queuestate.get_task_status(*earliest_remaining_job.1) {
				TaskStatus::Running{ .. } => {queuestate.complete_task_at(*earliest_remaining_job.1, None, simulated_time);},
				TaskStatus::Finished{ .. } => {},
				_ => unreachable!(),
			};

			// Mark that worker as finished so it gets a new job next round
			let i = earliest_remaining_job.0;
			drop(earliest_remaining_job);
			buildstate.workers[i] = WorkerStatus::Idle;
		}

		let now = Instant::now();

		if estimation_impossible || simulated_time <= now {
			println!("Building for {}, estimating remaining time...\x1b[K\x1b[m",
				MinSec::since(start_time));
		} else {
			let remaining_duration = simulated_time - now;
			let eta = TimeDuration::from_std(remaining_duration).map(|duration| Local::now() + duration);
			match eta {
				Ok(eta) => {
					let is_soon = eta.date() == Local::now().date() && remaining_duration < Duration::from_secs(8 * 3600);
					let timeformat = if is_soon { "%H:%M:%S" } else { "%Y-%m-%d %H:%M:%S" };
					println!("Building for {}, remaining time for build is {} (ETA {})\x1b[K\x1b[m",
						MinSec::since(start_time), MinSec::from_duration(remaining_duration),
						eta.format(timeformat));
				},
				_ => println!("Building for {}, remaining time is infinite...\x1b[K\x1b[m",
						MinSec::since(start_time)),
			}
		}

		if build_is_done {
			break;
		}

		print!("\x1b[{}A", buildstate.workers.len() + 1);
		lock = status.inner.lock().unwrap();
	}
	println!("\x1b[32;1mFinished.\x1b[m");
}
