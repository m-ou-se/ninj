mod progressbar;

use crate::timeformat::MinSec;
use crate::worker::StatusUpdater;
use ninj::buildlog::BuildLog;
use ninj::queue::{AsyncBuildQueue, TaskStatus};
use ninj::spec::Spec;
use std::fmt;
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
	HighResBar,
}

#[derive(Debug)]
pub struct ParseProgressFormatError {
	value: String,
}

impl core::str::FromStr for ProgressFormat {
	type Err = ParseProgressFormatError;
	fn from_str(s: &str) -> Result<Self, ParseProgressFormatError> {
		let s = s.to_lowercase();
		if s == "none" {
			Ok(ProgressFormat::None)
		} else if s == "text" {
			Ok(ProgressFormat::Text)
		} else if s == "ascii" {
			Ok(ProgressFormat::ASCIIBar)
		} else if s == "highres" {
			Ok(ProgressFormat::HighResBar)
		} else {
			Err(ParseProgressFormatError { value: s })
		}
	}
}

impl fmt::Display for ParseProgressFormatError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "{}", self.value)
	}
}

fn render_progress_bar(width: usize, percentage: f32, text: &String, highres: bool) -> String {
	while text.len() > width {
		return String::from(text.split_at(width).0);
	}

	let textwidth = text.len();
	let spaceleft = (width - textwidth) / 2;
	let spaceright = width - textwidth - spaceleft;

	let arrow_width = percentage * width as f32 / 100.;
	let arrow_full_char = if highres { "▉" } else { "=" };
	let arrow_cut_char = if highres { "▋" } else { arrow_full_char };

	let arrow_rest = arrow_width - arrow_width.floor();
	let arrow_rest_char = if arrow_rest < 0.01 {
		""
	} else if !highres {
		">"
	} else if arrow_rest > 0.84 {
		"▊"
	} else if arrow_rest > 0.68 {
		"▋"
	} else if arrow_rest > 0.52 {
		"▌"
	} else if arrow_rest > 0.36 {
		"▍"
	} else if arrow_rest > 0.20 {
		"▎"
	} else {
		"▏"
	};

	// If highres, the arrow characters are all 3 bytes. This is assumed below.
	let arrow_charlen = if highres { 3 } else { 1 };
	assert!(arrow_full_char.len() == arrow_charlen);
	assert!(arrow_rest_char.is_empty() || arrow_rest_char.len() == arrow_charlen);

	let arrow = arrow_full_char.repeat(arrow_width.floor() as usize) + arrow_rest_char;
	let arrowlen = arrow.chars().count();

	let left = if arrowlen < spaceleft {
		format!("{}{}", arrow, " ".repeat(spaceleft - arrowlen))
	} else {
		format!(
			"{}{}",
			arrow.split_at((spaceleft - 1) * arrow_charlen).0,
			arrow_cut_char
		)
	};
	let lefttextlen = spaceleft + textwidth;
	let right = if arrowlen < lefttextlen {
		" ".repeat(spaceright)
	} else if arrowlen == width {
		format!(
			"{}{}",
			arrow
				.split_at(lefttextlen * arrow_charlen)
				.1
				.split_at((spaceright - 1) * arrow_charlen)
				.0,
			arrow_cut_char
		)
	} else {
		format!(
			"{}{}",
			arrow.split_at(lefttextlen * arrow_charlen).1,
			" ".repeat(width - arrowlen)
		)
	};

	assert!(left.chars().count() == spaceleft);
	assert!(right.chars().count() == spaceright);
	assert!(spaceleft + text.chars().count() + spaceright == width);

	format!("\x1b[32m{}\x1b[m{}\x1b[32m{}\x1b[m", left, text, right)
}

pub fn show_build_status(
	start_time: Instant,
	status: &BuildStatus,
	queue: &AsyncBuildQueue,
	spec: &Spec,
	build_log: &Mutex<BuildLog>,
	sleep: bool,
	progress_format: ProgressFormat,
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

		let (percentage, text) = match remaining_duration {
			None => (0., format!("??% (estimating time)")),
			Some(remaining_duration) => {
				let percentage = (100 * as_millis(current_duration)) as f32
					/ as_millis(current_duration + remaining_duration) as f32;

				// Every 5 seconds, switch between showing ETA and remaining duration
				let show_eta = (current_duration.as_secs() % 10) > 5;

				(
					percentage,
					if show_eta {
						let eta = TimeDuration::from_std(remaining_duration)
							.map(|duration| chrono::Local::now() + duration)
							.unwrap();
						let is_soon = eta.date() == chrono::Local::now().date()
							&& remaining_duration < Duration::from_secs(8 * 3600);
						let timeformat = if is_soon {
							"%H:%M:%S"
						} else {
							"%Y-%m-%d %H:%M:%S"
						};
						format!(
							"{}% (ETA {})",
							percentage.ceil() as u8,
							eta.format(timeformat)
						)
					} else {
						format!(
							"{}% ({} remaining)",
							percentage.ceil() as u8,
							MinSec::from_duration(remaining_duration)
						)
					},
				)
			}
		};

		let progress = match progress_format {
			ProgressFormat::None => "".to_owned(),
			ProgressFormat::Text => format!(
				"[Building for {}, {}]\x1b[K\x1b[m\n",
				MinSec::since(start_time),
				text
			),
			ProgressFormat::ASCIIBar | ProgressFormat::HighResBar => format!(
				"[{}]\x1b[K\x1b[m\n",
				render_progress_bar(
					terminal_width() - 3,
					percentage,
					&text,
					progress_format == ProgressFormat::HighResBar
				)
			),
		};

		print!("{}", progress);

		if build_is_done {
			break;
		}

		print!(
			"\x1b[{}A",
			buildstate.workers.len() + progress.lines().count()
		);
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
