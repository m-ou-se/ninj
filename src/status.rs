use crate::timeformat::MinSec;
use crate::worker::status::{StatusEvent, StatusListener};
use memchr::memchr_iter;
use ninj::buildlog::BuildLog;
use ninj::queue::{AsyncBuildQueue, TaskStatus};
use ninj::spec::Spec;
use raw_string::RawString;
use std::mem::replace;
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

struct BuildStatusInner {
	workers: Vec<WorkerStatus>,
	output: Vec<BufferedOutput>,
	dirty: bool,
}

struct BufferedOutput {
	task: usize,
	output: RawString,
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
				output: Vec::new(),
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

	fn buffer_output(&self, task: usize, output: RawString) {
		let mut lock = self.inner.lock().unwrap();
		lock.buffer_output(task, output);
		self.condvar.notify_all();
	}
}

impl StatusListener for BuildStatus {
	fn status_update(&self, worker: usize, event: StatusEvent) {
		match event {
			StatusEvent::Idle => self.set_status(worker, WorkerStatus::Idle),
			StatusEvent::Running { task } => {
				self.set_status(worker, WorkerStatus::Running { task })
			}
			StatusEvent::Done | StatusEvent::Failed => self.set_status(worker, WorkerStatus::Done),
			StatusEvent::Output { task, data } => {
				self.buffer_output(task, RawString::from(data));
			}
		}
	}
}

impl BuildStatusInner {
	fn set_status(&mut self, worker: usize, status: WorkerStatus) {
		self.workers[worker] = status;
		self.dirty = true;
	}

	fn buffer_output(&mut self, task: usize, output: RawString) {
		self.output.push(BufferedOutput { task, output });
	}

	fn are_all_workers_done(&self) -> bool {
		self.workers
			.iter()
			.all(|worker| *worker == WorkerStatus::Done)
	}
}

fn estimated_total_task_time(
	spec: &Spec,
	task: usize,
	build_log: &std::sync::MutexGuard<BuildLog>,
) -> Option<Duration> {
	if let Some(output) = &spec.build_rules[task].outputs.first() {
		if let Some(estimate_ms) = build_log
			.entries
			.get(*output)
			.and_then(|entry| entry.end_time_ms.checked_sub(entry.start_time_ms))
		{
			return Some(Duration::from_millis(estimate_ms.into()));
		}
	}

	// Failing to find an estimation for this particular job, we return
	// the average job time.
	if build_log.entries.is_empty() {
		None
	} else {
		let sum_ms: u64 = build_log
			.entries
			.iter()
			.map(|(_, entry)| u64::from(entry.end_time_ms.saturating_sub(entry.start_time_ms)))
			.sum();
		Some(Duration::from_millis(
			sum_ms / build_log.entries.len() as u64,
		))
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
	let mut last_output_task = usize::max_value();
	let mut lock = status.inner.lock().unwrap();
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
		let mut buildstate = BuildStatusInner {
			workers: lock.workers.clone(),
			output: replace(&mut lock.output, Vec::new()),
			dirty: replace(&mut lock.dirty, false),
		};
		drop(lock);
		for BufferedOutput { task, output } in &buildstate.output {
			if *task != last_output_task {
				let command = spec.build_rules[*task]
					.command
					.as_ref()
					.expect("Got output for phony task");
				println!("\x1b[30;44m {}:\x1b[K\x1b[m", command.description);
				last_output_task = *task;
			}
			let mut n_written = 0;
			for newline in memchr_iter(b'\n', output.as_bytes()) {
				println!("{}\x1b[K", &output[n_written..newline]);
				n_written = newline + 1;
			}
			println!("{}\x1b[K\x1b[m", &output[n_written..]);
		}
		println!(
			"\x1b[30;47m {}:\x1b[K\x1b[m",
			if sleep { "Sleeping" } else { "Building" }
		);
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

		let build_is_done = buildstate.are_all_workers_done();

		// Compute remaining time for this build, by simulating a build.
		let mut simulated_time = Instant::now();
		let mut estimation_impossible = false;
		loop {
			// Give all simulated workers something to do
			for i in 0..buildstate.workers.len() {
				let worker = &buildstate.workers[i];
				match worker {
					WorkerStatus::Starting | WorkerStatus::Idle => {
						let next = queuestate.next_at(simulated_time);
						buildstate.set_status(
							i,
							match next {
								Some(task) => WorkerStatus::Running { task },
								None => WorkerStatus::Idle,
							},
						);
					}
					_ => {}
				}
			}

			// All workers still idle? Nothing else to do, stop simulating
			if buildstate
				.workers
				.iter()
				.find(|&w| *w != WorkerStatus::Idle)
				.is_none()
			{
				break;
			}

			// Find the job with the lowest remaining time
			let (worker, task, remainingtime) = match buildstate
				.workers
				.iter()
				.enumerate()
				.flat_map(|(i, worker)| match worker {
					WorkerStatus::Running { task, .. } => match queuestate.get_task_status(*task) {
						TaskStatus::Running { start_time, .. } => {
							let runtime = if simulated_time > start_time {
								simulated_time - start_time
							} else {
								Duration::from_millis(0)
							};
							match estimated_total_task_time(spec, *task, &build_log.lock().unwrap())
							{
								Some(time) => Some((
									i,
									task,
									time.checked_sub(runtime)
										.unwrap_or(Duration::from_millis(0)),
								)),
								None => None,
							}
						}
						TaskStatus::Finished { .. } => Some((i, task, Duration::from_millis(0))),
						_ => unreachable!(),
					},
					_ => None,
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
			simulated_time += remainingtime;

			// Complete that task, if it isn't already Finished
			match queuestate.get_task_status(*task) {
				TaskStatus::Running { .. } => {
					queuestate.complete_task_at(*task, None, simulated_time);
				}
				TaskStatus::Finished { .. } => {}
				_ => unreachable!(),
			};

			// Mark that worker as finished so it gets a new job next round
			buildstate.workers[worker] = WorkerStatus::Idle;
		}

		let now = Instant::now();

		if estimation_impossible || simulated_time <= now {
			println!(
				"Building for {}, estimating remaining time...\x1b[K\x1b[m",
				MinSec::since(start_time)
			);
		} else {
			let remaining_duration = simulated_time - now;
			let eta = TimeDuration::from_std(remaining_duration)
				.map(|duration| chrono::Local::now() + duration);
			match eta {
				Ok(eta) => {
					let is_soon = eta.date() == chrono::Local::now().date()
						&& remaining_duration < Duration::from_secs(8 * 3600);
					let timeformat = if is_soon {
						"%H:%M:%S"
					} else {
						"%Y-%m-%d %H:%M:%S"
					};
					println!(
						"Building for {}, remaining time for build is {} (ETA {})\x1b[K\x1b[m",
						MinSec::since(start_time),
						MinSec::from_duration(remaining_duration),
						eta.format(timeformat)
					);
				}
				_ => println!(
					"Building for {}, remaining time is infinite...\x1b[K\x1b[m",
					MinSec::since(start_time)
				),
			}
		}

		if build_is_done {
			break;
		}

		print!("\x1b[{}A", buildstate.workers.len() + 2);
		lock = status.inner.lock().unwrap();
	}
	println!("\x1b[32;1mFinished.\x1b[m");
}
