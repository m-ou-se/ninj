mod progressbar;

use crate::timeformat::MinSec;
use std::process::ExitStatus;
use crate::worker::status::{StatusListener, TaskUpdate, WorkerUpdate};
use memchr::memchr_iter;
use ninj::buildlog::BuildLog;
use ninj::queue::{AsyncBuildQueue, TaskStatus};
use ninj::spec::Spec;
use progressbar::ProgressBar;
use raw_string::RawString;
use std::error::Error;
use std::fmt;
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
	output: Message,
}

enum Message {
	Started,
	Output(RawString),
	Success,
	Failed(Option<ExitStatus>),
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

	fn buffer_output(&self, task: usize, output: Message) {
		let mut lock = self.inner.lock().unwrap();
		lock.buffer_output(task, output);
		self.condvar.notify_all();
	}
}

impl StatusListener for BuildStatus {
	fn update(&self, worker_id: usize, update: WorkerUpdate) {
		match update {
			WorkerUpdate::Idle => self.set_status(worker_id, WorkerStatus::Idle),
			WorkerUpdate::Done => self.set_status(worker_id, WorkerStatus::Done),
			WorkerUpdate::Task {
				task_id,
				update: TaskUpdate::Started,
			} => {
				let mut lock = self.inner.lock().unwrap();
				lock.set_status(worker_id, WorkerStatus::Running { task: task_id });
				lock.buffer_output(task_id, Message::Started);
				self.condvar.notify_all();
			}
			WorkerUpdate::Task {
				task_id,
				update: TaskUpdate::Output { data },
			} => self.buffer_output(task_id, Message::Output(RawString::from(data))),
			WorkerUpdate::Task {
				task_id,
				update: TaskUpdate::Finished { status },
			} if status.success() => self.buffer_output(task_id, Message::Success),
			WorkerUpdate::Task {
				task_id,
				update: TaskUpdate::Finished { status },
			} => self.buffer_output(task_id, Message::Failed(Some(status))),
			WorkerUpdate::Task {
				task_id,
				update: TaskUpdate::Error,
			} => self.buffer_output(task_id, Message::Failed(None))
		}
	}
}

impl BuildStatusInner {
	fn set_status(&mut self, worker: usize, status: WorkerStatus) {
		self.workers[worker] = status;
		self.dirty = true;
	}

	fn buffer_output(&mut self, task: usize, output: Message) {
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

#[derive(Debug, Clone, PartialEq)]
pub enum ProgressFormat {
	None,
	Text,
	ASCIIBar,
	ASCIISplitBar,
	HighResBar,
	HighResSplitBar,
}

#[derive(Debug)]
pub struct ParseProgressFormatError {
	value: String,
}

impl core::str::FromStr for ProgressFormat {
	type Err = ParseProgressFormatError;
	fn from_str(s: &str) -> Result<Self, ParseProgressFormatError> {
		match s.to_lowercase().as_str() {
			"none" => Ok(ProgressFormat::None),
			"text" => Ok(ProgressFormat::Text),
			"ascii" => Ok(ProgressFormat::ASCIIBar),
			"highres" => Ok(ProgressFormat::HighResBar),
			"ascii.split" => Ok(ProgressFormat::ASCIISplitBar),
			"highres.split" => Ok(ProgressFormat::HighResSplitBar),
			value => Err(ParseProgressFormatError {
				value: value.to_string(),
			}),
		}
	}
}

impl fmt::Display for ParseProgressFormatError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.value)
	}
}

impl Error for ParseProgressFormatError {}

pub fn show_build_status(
	start_time: Instant,
	status: &BuildStatus,
	queue: &AsyncBuildQueue,
	spec: &Spec,
	build_log: &Mutex<BuildLog>,
	progress_format: ProgressFormat,
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
		let mut i = buildstate.output.iter();
		while let Some(BufferedOutput { task, output }) = i.next() {
			let command = spec.build_rules[*task]
				.command
				.as_ref()
				.expect("Got output for phony task");
			match output {
				Message::Output(data) => {
					if *task != last_output_task {
						let mut failed = false;
						if let Some(BufferedOutput { task: next_task, output: Message::Failed(status) }) = i.clone().next() {
							if task == next_task {
								i.next();
								failed = true;
								if let Some(status) = status {
									println!("\x1b[30;41m  \x1b[m\x1b[31;1m Failed with {}: {}:\x1b[K\x1b[m", status, command.description);
								} else {
									println!("\x1b[30;41m  \x1b[m\x1b[31;1m Failed: {}:\x1b[K\x1b[m", command.description);
								}
							}
						}
						if !failed {
							println!("\x1b[30;43m  \x1b[m\x1b[33m {}:\x1b[K\x1b[m", command.description);
						}
					}
					let mut n_written = 0;
					for newline in memchr_iter(b'\n', data.as_bytes()) {
						println!("{}\x1b[K", &data[n_written..newline]);
						n_written = newline + 1;
					}
					println!("{}\x1b[K\x1b[m", &data[n_written..]);
					last_output_task = *task;
				}
				Message::Started => {},
				Message::Success => {
					println!("\x1b[30;42m  \x1b[m\x1b[32m Finished {}\x1b[K\x1b[m", command.description);
					last_output_task = *task;
				}
				Message::Failed(Some(status)) => {
					println!("\x1b[30;41m  \x1b[m\x1b[31;1m Failed with {}: {}\x1b[K\x1b[m", status, command.description);
					last_output_task = *task;
				}
				Message::Failed(None) => {
					println!("\x1b[30;41m  \x1b[m\x1b[31;1m Failed: {}\x1b[K\x1b[m", command.description);
					last_output_task = *task;
				}
			}
		}

		let mut worker_status_lines = 0;
		for worker in &buildstate.workers {
			if let WorkerStatus::Running { task } = worker {
				let command = spec.build_rules[*task]
					.command
					.as_ref()
					.expect("Got phony task");
				let statustext = match queuestate.get_task_status(*task) {
					TaskStatus::Running { start_time } => {
						format!("[{}] ", MinSec::since(start_time))
					}
					_ => String::new()
				};
				println!(
					"\x1b[30;44m  \x1b[m\x1b[34m {}{} ...\x1b[K\x1b[m",
					statustext,
					command.description,
				);
				worker_status_lines += 1;
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

			// All workers still idle or done? Nothing else to do, stop simulating
			if buildstate
				.workers
				.iter()
				.find(|&w| *w != WorkerStatus::Idle && *w != WorkerStatus::Done)
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
					// This happens when workers are working on a job for which we cannot
					// guess how long it will take.
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

		let current_duration = start_time.elapsed();
		let remaining_duration = if estimation_impossible {
			None
		} else if simulated_time < now {
			// Should have been done already, estimate it will be done immediately
			Some(Duration::from_millis(0))
		} else {
			Some(simulated_time - now)
		};

		let (progress, percentagetext, remainingtext, etatext) = match remaining_duration {
			None => (
				0.,
				"??".to_owned(),
				"estimating time".to_owned(),
				"estimating time".to_owned(),
			),
			Some(remaining_duration) => {
				let progress = as_millis(current_duration) as f64
					/ as_millis(current_duration + remaining_duration) as f64;
				(
					progress,
					format!("{:02}%", (progress * 100.) as u8),
					format!("{}", MinSec::from_duration(remaining_duration)),
					{
						let eta = chrono::Local::now()
							+ TimeDuration::from_std(remaining_duration).unwrap();
						let is_soon = eta.date() == chrono::Local::now().date()
							&& remaining_duration < Duration::from_secs(8 * 3600);
						let timeformat = if is_soon {
							"%H:%M:%S"
						} else {
							"%Y-%m-%d %H:%M:%S"
						};
						eta.format(timeformat).to_string()
					},
				)
			}
		};

		let progress = match progress_format {
			ProgressFormat::None => "".to_owned(),
			ProgressFormat::Text => format!(
				"[Building for {}, {}, {} remaining, ETA {}]\x1b[K\x1b[m\n",
				MinSec::since(start_time),
				percentagetext,
				remainingtext,
				etatext
			),
			ProgressFormat::ASCIIBar | ProgressFormat::HighResBar => {
				// Every 5 seconds, switch between showing ETA and remaining duration
				let show_eta = (current_duration.as_secs() % 10) > 5;
				let text = format!(
					"{} ({})",
					percentagetext,
					if show_eta {
						format!("ETA {}", etatext)
					} else {
						format!("{} remaining", remainingtext)
					}
				);

				format!(
					"[{}]\x1b[K\x1b[m\n",
					ProgressBar {
						progress,
						width: terminal_width() - 3,
						ascii: progress_format == ProgressFormat::ASCIIBar,
						label: &text,
					}
				)
			}
			ProgressFormat::ASCIISplitBar | ProgressFormat::HighResSplitBar => {
				let text = format!("{} remaining", remainingtext);
				format!(
					"{} [{}] ETA {}\x1b[K\x1b[m\n",
					percentagetext,
					ProgressBar {
						progress,
						width: terminal_width()
							.saturating_sub(etatext.len() + percentagetext.len() + 9),
						ascii: progress_format == ProgressFormat::ASCIISplitBar,
						label: &text,
					},
					etatext
				)
			}
		};

		print!("{}", progress);

		if build_is_done {
			break;
		}

		print!("\x1b[{}A", progress.lines().count() + worker_status_lines);
		lock = status.inner.lock().unwrap();
	}
	println!("\x1b[32;1mFinished.\x1b[m");
}

fn terminal_width() -> usize {
	if let Some((w, _)) = term_size::dimensions() {
		w
	} else {
		80 /* an educated guess */
	}
}

fn as_millis(d: Duration) -> u64 {
	d.as_secs() * 1000 + u64::from(d.subsec_millis())
}
