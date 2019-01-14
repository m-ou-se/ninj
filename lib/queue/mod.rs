//! Tracking of which tasks need to be executed in what order.
//!
//! A [`BuildQueue`][self::queue::BuildQueue] tracks which tasks need to be
//! executed, with very minimal information about those tasks. It barely knows
//! anything about the tasks, and only refers to them by 'task number', which is
//! simply an index into a vector.

use std::mem::replace;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Knows which tasks should be executed, and in what order.
///
/// The `BuildQueue` is de-coupled from any details of what the tasks actually
/// are. It only knows about task numbers, and tracks only very minimal
/// information of each task:
///
///  - The state (waiting, running, finished, etc.),
///  - whether it is a 'phony' task,
///  - whether it was marked as outdated, and
///  - the task numbers of the tasks it depends on.
///
/// The [`next`][Self::next] method gives the next task to be run. After the
/// task is done, [`complete_task`][Self::complete_task] should be called to
/// update the queue.
///
/// [`make_async`][Self::make_async] turns this into a concurrent
/// data-structure on which threads can [wait][LockedAsyncBuildQueue::wait].
#[derive(Clone)]
pub struct BuildQueue {
	/// Information related to build rules.
	///
	/// The index in this vector is their ID.
	tasks: Vec<Task>,
	/// The tasks which are ready to run, will never contain phony tasks.
	ready: Vec<usize>,
	/// Number of non-phony tasks which still need to be started.
	///
	/// Includes tasks which are not oudated, but might turn out to be outdated
	/// later.
	n_left: usize,
}

/// The tasks tracked by a [`BuildQueue`].
#[derive(Clone, Debug)]
pub struct Task {
	/// Status of this task.
	status: TaskStatus,
	/// Build rules which depend on this build rule.
	next: Vec<DepInfo>,
	/// Number of unfinished build rules which have this rule in their `next`
	/// list.
	n_deps_left: usize,
}

/// The status of a [`Task`] inside a [`BuildQueue`].
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TaskStatus {
	/// The task does not appear in the dependency tree of the targets we want.
	NotNeeded,
	/// The task does appear in the dependency tree, but we don't know anything
	/// about the task yet.
	///
	/// Only exists while building up the dependency tree (e.g. inside
	/// [`BuildQueue::new`], but never inside a [`BuildQueue`]).
	WillBeNeeded,
	/// The task appears in the dependency tree.
	///
	/// If [`Task::n_deps_left`] is zero, it is ready to be run.
	///
	/// If it is not outdated, it does not need to run.
	/// It might be marked as outdated later.
	Needed { phony: bool, outdated: bool },
	/// The task is running.
	Running {
		/// The time since when it has been running.
		start_time: Instant,
	},
	/// The task is finished.
	Finished {
		/// The time it took to run this task.
		running_time: Duration,
	},
	/// The task was not outdated, so did not need to be run.
	NotRun,
	/// The task is phony and was outdated, and all dependencies have been
	/// finished.
	PhonyFinished,
}

/// Wraps a [`BuildQueue`] to allow multiple threads to use it and wait for it.
pub struct AsyncBuildQueue {
	queue: Mutex<BuildQueue>,
	condvar: Condvar,
}

/// A lock on a [`AsyncBuildQueue`], which prevents other threads from
/// accessing the queue.
pub struct LockedAsyncBuildQueue<'a> {
	queue: MutexGuard<'a, BuildQueue>,
	condvar: &'a Condvar,
}

/// The information the [`BuildQueue`] needs for each task.
///
/// [`BuildQueue::new`] requires [`dependencies`][Self::dependencies] to be
/// <code>IntoIter&lt;Item =
/// <a href="struct.DepInfo.html">DepInfo</a>&gt;</code>.
#[derive(Debug, Clone, Copy)]
pub struct TaskInfo<T> {
	pub phony: bool,
	pub dependencies: T,
	pub outdated: bool,
}

/// The information the [`BuildQueue`] needs for each task dependency.
#[derive(Debug, Clone, Copy)]
pub struct DepInfo {
	pub task: usize,
	pub order_only: bool,
}

impl BuildQueue {
	/// Construct a new build dependency graph.
	///
	/// - The (potential) tasks are numbered 0 to `max_task_num`. ([`Task`]s
	///   will be stored in a vector of this size.)
	///
	/// - `targets` are the tasks that need to be executed.
	///
	/// - `get_task` is used to get the information the queue needs of each
	///   (relevant) task: Whether it is phony, on which tasks it depends (and
	///   how), and if the target is outdated. It is called exactly once for
	///   every task in the dependency tree of the targets.
	pub fn new<T, F, D>(max_task_num: usize, targets: T, mut get_task: F) -> BuildQueue
	where
		T: IntoIterator<Item = usize>,
		F: FnMut(usize) -> TaskInfo<D>,
		D: IntoIterator<Item = DepInfo>,
	{
		let mut tasks = vec![
			Task {
				status: TaskStatus::NotNeeded,
				next: vec![],
				n_deps_left: 0,
			};
			max_task_num
		];

		let mut to_visit = Vec::new();

		for task in targets {
			if tasks[task].status == TaskStatus::NotNeeded {
				to_visit.push(task);
				tasks[task].status = TaskStatus::WillBeNeeded;
			}
		}

		let mut n_tasks = 0;
		let mut finished = Vec::new();
		let mut ready = Vec::new();

		// Build dependency graph
		while let Some(task) = to_visit.pop() {
			assert_eq!(tasks[task].status, TaskStatus::WillBeNeeded);
			let info = get_task(task);
			let mut n_deps = 0;
			for dep in info.dependencies {
				if tasks[dep.task].status == TaskStatus::NotNeeded {
					to_visit.push(dep.task);
					tasks[dep.task].status = TaskStatus::WillBeNeeded;
				}
				n_deps += 1;
				tasks[dep.task].next.push(DepInfo {
					task,
					order_only: dep.order_only,
				});
			}
			tasks[task].status = TaskStatus::Needed {
				phony: info.phony,
				outdated: info.outdated,
			};
			if !info.phony {
				n_tasks += 1;
			}
			tasks[task].n_deps_left = n_deps;
			if n_deps == 0 {
				if !info.outdated {
					if !info.phony {
						n_tasks -= 1;
					}
					tasks[task].status = TaskStatus::NotRun;
					finished.push(task);
				} else if info.phony {
					tasks[task].status = TaskStatus::PhonyFinished;
					finished.push(task);
				} else {
					ready.push(task);
				}
			}
		}

		let mut queue = BuildQueue {
			tasks,
			ready,
			n_left: n_tasks,
		};

		// Mark any ready phony tasks as finished, and update the tasks
		// dependent on it.
		while let Some(task) = finished.pop() {
			queue.update_finished_task(task, &mut finished, None);
		}

		// TODO: Check for cycles.

		queue
	}

	/// Turn the [`BuildQueue`] into an [`AsyncBuildQueue`], which can be used
	/// concurrently from multiple threads.
	pub fn make_async(self) -> AsyncBuildQueue {
		AsyncBuildQueue {
			queue: Mutex::new(self),
			condvar: Condvar::new(),
		}
	}

	/// Check if there is something to do right now.
	///
	/// Returns the index of the task. Will never return a phony tasks, as
	/// those don't have any work to do.
	pub fn next(&mut self) -> Option<usize> {
		self.next_at(Instant::now())
	}

	/// Like next(), returns the next thing to do, but notes it as having
	/// started at the given time instead of now.
	pub fn next_at(&mut self, start_time: Instant) -> Option<usize> {
		let next = self.ready.pop();
		if let Some(next) = next {
			assert_eq!(self.tasks[next].n_deps_left, 0);
			assert_eq!(
				self.tasks[next].status,
				TaskStatus::Needed {
					phony: false,
					outdated: true
				}
			);
			self.tasks[next].status = TaskStatus::Running { start_time };
			self.n_left -= 1;
		}
		next
	}

	/// Mark the task as ready, possibly queueing dependent tasks.
	///
	/// `restat` is called for the non-outdated tasks dependent on this task to
	/// check if they're now outdated. If not given, they are all considered
	/// outdated.
	///
	/// Returns the number of newly ready tasks that were unblocked by the
	/// completion of this one.
	pub fn complete_task(
		&mut self,
		task: usize,
		restat: Option<&mut dyn FnMut(usize) -> bool>,
	) -> usize {
		self.complete_task_at(task, restat, Instant::now())
	}

	/// Like complete_task, marks a task as completed, but notes it as having
	/// finished at the given time instead of now.
	pub fn complete_task_at(
		&mut self,
		task: usize,
		restat: Option<&mut dyn FnMut(usize) -> bool>,
		finish_time: Instant,
	) -> usize {
		self.tasks[task].status = match &self.tasks[task].status {
			TaskStatus::Running { start_time } => TaskStatus::Finished {
				running_time: finish_time - *start_time,
			},
			_ => panic!(
				"complete_task({}) on task that isn't Running: {:?}",
				task, self.tasks[task]
			),
		};
		let mut newly_ready = 0;
		let mut newly_finished = Vec::new();
		newly_ready += self.update_finished_task(task, &mut newly_finished, restat);
		while let Some(task) = newly_finished.pop() {
			newly_ready += self.update_finished_task(task, &mut newly_finished, None);
		}
		newly_ready
	}

	/// Decrement the `n_deps_left` of all the tasks depending on this task,
	/// and mark any newly ready tasks as ready.
	///
	/// Returns the amount of newly ready tasks.
	///
	/// Adds any now finished (phony and up-to-date) tasks to `newly_finished`.
	fn update_finished_task(
		&mut self,
		task: usize,
		newly_finished: &mut Vec<usize>,
		mut restat: Option<&mut dyn FnMut(usize) -> bool>,
	) -> usize {
		let did_run = match &self.tasks[task].status {
			TaskStatus::NotRun => false,
			TaskStatus::PhonyFinished => true,
			TaskStatus::Finished { .. } => true,
			_ => unreachable!("Task {} was not finished: {:?}", task, self.tasks[task]),
		};
		let mut newly_ready = 0;
		for DepInfo {
			task: next,
			order_only,
		} in replace(&mut self.tasks[task].next, Vec::new())
		{
			let next_phony;
			let next_outdated;
			match &mut self.tasks[next].status {
				TaskStatus::Needed { phony, outdated } => {
					if did_run && !order_only && !*outdated {
						*outdated = if let Some(restat) = restat.as_mut() {
							restat(next)
						} else {
							true
						};
					}
					next_phony = *phony;
					next_outdated = *outdated;
				}
				_ => unreachable!(
					"Task {} in `next' list was not `Needed': {:?}",
					next, self.tasks[next]
				),
			}
			self.tasks[next].n_deps_left -= 1;
			if self.tasks[next].n_deps_left == 0 {
				if !next_outdated {
					if !next_phony {
						self.n_left -= 1;
					}
					self.tasks[next].status = TaskStatus::NotRun;
					newly_finished.push(next);
				} else if next_phony {
					// Phony tasks are instantly finished, as they have no work to do.
					self.tasks[next].status = TaskStatus::PhonyFinished;
					newly_finished.push(next);
				} else {
					self.ready.push(next);
					newly_ready += 1;
				}
			}
		}
		newly_ready
	}

	/// Get the status of a task.
	pub fn get_task_status(&self, task: usize) -> TaskStatus {
		self.tasks[task].status
	}

	/// Number of tasks left.
	///
	/// Does not include phony tasks.
	/// Does include tasks which are not marked as outdated, but might be later
	/// because a (indirect) dependencies is outdated.
	pub fn n_left(&self) -> usize {
		self.n_left
	}
}

impl AsyncBuildQueue {
	/// Get exclusive access to the build queue.
	pub fn lock(&self) -> LockedAsyncBuildQueue {
		LockedAsyncBuildQueue {
			queue: self.queue.lock().unwrap(),
			condvar: &self.condvar,
		}
	}
}

impl<'a> LockedAsyncBuildQueue<'a> {
	/// Check if there is something to do right now.
	///
	/// Returns the index of the task. Will never return a phony tasks, as
	/// those don't have any work to do.
	///
	/// Does not block.
	pub fn next(&mut self) -> Option<usize> {
		let next = self.queue.next();
		if next.is_some() {
			if self.queue.n_left == 0 {
				self.condvar.notify_all();
			}
		}
		next
	}

	/// Wait for something to do.
	///
	/// Returns `None` when all tasks are finished.
	pub fn wait(mut self) -> Option<usize> {
		while self.queue.ready.is_empty() && self.queue.n_left > 0 {
			self.queue = self.condvar.wait(self.queue).unwrap();
		}
		self.next()
	}

	/// Mark the task as ready, unblocking dependent tasks.
	///
	/// See [`BuildQueue::complete_task`].
	pub fn complete_task(&mut self, task: usize, restat: Option<&mut dyn FnMut(usize) -> bool>) {
		let n = self.queue.complete_task(task, restat);
		// TODO: In most cases we'll want to notify one time less, because this
		// thread itself will also continue executing tasks.
		if self.queue.n_left == 0 {
			self.condvar.notify_all();
		} else {
			for _ in 0..n {
				self.condvar.notify_one();
			}
		}
	}

	/// Get a full copy of the internal state.
	///
	/// This is useful if you want to inspect the full state without blocking
	/// other threads.
	pub fn clone_queue(&self) -> BuildQueue {
		self.queue.clone()
	}
}
