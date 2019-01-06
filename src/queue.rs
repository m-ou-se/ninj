use std::mem::replace;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Instant, Duration};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TaskStatus {
	/// The task does not appear in the dependency tree of the targets we want.
	NotNeeded,
	/// The task does appear in the dependency tree, but we don't know anything
	/// about the task yet.
	///
	/// Only happens while we're building the dependency tree.
	WillBeNeeded,
	/// The task appears in the dependency tree.
	///
	/// If `n_deps_left` is zero, it is ready to be run.
	///
	/// If it is not outdated, it does not need to run.
	/// It might be marked as outdated later.
	Needed {
		phony: bool,
		outdated: bool,
	},
	/// The task is running.
	Running {
		start_time: Instant,
	},
	/// The task is finished.
	Finished {
		running_time: Duration,
		was_outdated: bool,
	},
	/// The task was not run but can be considered 'finished', as it was not outdated.
	NotRun,
	/// The task is phony, and is fulfilled.
	PhonyFinished,
}

#[derive(Clone, Debug)]
pub struct Task {
	/// Status of this task.
	status: TaskStatus,
	/// Build rules which depend on this build rule.
	next: Vec<DepInfo>,
	/// Number of unfinished build rules which have this rule in their `next` list.
	n_deps_left: usize,
}

/// A BuildQueue which knows in which order tasks may execute.
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
	/// Includes tasks which are not oudated, but might turn out to be outdated later.
	n_left: usize,
}

pub struct AsyncBuildQueue {
	queue: Mutex<BuildQueue>,
	condvar: Condvar,
}

pub struct LockedAsyncBuildQueue<'a> {
	queue: MutexGuard<'a, BuildQueue>,
	condvar: &'a Condvar,
}

#[derive(Debug, Clone, Copy)]
pub struct DepInfo {
	pub task: usize,
	pub order_only: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct TaskInfo<T: IntoIterator<Item=DepInfo>> {
	pub phony: bool,
	pub dependencies: T,
	pub outdated: bool,
}

impl BuildQueue {

	/// Construct a new build dependency graph.
	///
	/// The (potential) tasks are numbered 0 to `max_task_num`.
	///
	/// `targets` are the tasks that need to be executed.
	///
	/// `get_task` is used to get the dependencies of a task and to see if a
	/// task is phony. It is called exactly once for every task in the
	/// dependency tree of the targets.
	pub fn new<F, D>(
		max_task_num: usize,
		targets: impl IntoIterator<Item=usize>,
		mut get_task: F,
	) -> BuildQueue
		where
			F: FnMut(usize) -> TaskInfo<D>,
			D: IntoIterator<Item=DepInfo>,
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
			queue.update_next_tasks_for_finished_task(task, &mut finished);
		}

		// TODO: Check for cycles.

		queue
	}

	/// Turn the BuildQueue into an AsyncBuildQueue, which can be used from
	/// multiple threads at once.
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
		let next = self.ready.pop();
		if let Some(next) = next {
			assert_eq!(self.tasks[next].n_deps_left, 0);
			assert_eq!(self.tasks[next].status, TaskStatus::Needed { phony: false, outdated: true });
			self.tasks[next].status = TaskStatus::Running {
				start_time: Instant::now()
			};
			self.n_left -= 1;
		}
		next
	}

	/// Mark the task as ready, possibly queueing dependent tasks.
	///
	/// Returns the number of newly ready tasks that were unblocked by the
	/// completion of this one.
	pub fn complete_task(&mut self, task: usize, was_outdated: bool) -> usize {
		self.tasks[task].status = match &self.tasks[task].status {
			TaskStatus::Running { start_time } => TaskStatus::Finished {
				running_time: start_time.elapsed(),
				was_outdated,
			},
			_ => panic!("complete_task({}) on task that isn't Running or PhonyQueued: {:?}", task, self.tasks[task]),
		};
		let mut newly_ready = 0;
		let mut newly_finished = Vec::new();
		newly_ready += self.update_next_tasks_for_finished_task(task, &mut newly_finished);
		while let Some(task) = newly_finished.pop() {
			newly_ready += self.update_next_tasks_for_finished_task(task, &mut newly_finished);
		}
		newly_ready
	}

	/// Decrement the `n_deps_left` of all the tasks depending on this task,
	/// and mark any newly ready tasks as ready.
	///
	/// Returns the amount of newly ready tasks.
	///
	/// Adds any now finished (phony and up-to-date) tasks to `newly_finished`.
	fn update_next_tasks_for_finished_task(&mut self, task: usize, newly_finished: &mut Vec<usize>) -> usize {
		let was_outdated = match &self.tasks[task].status {
			TaskStatus::NotRun => false,
			TaskStatus::PhonyFinished => true,
			TaskStatus::Finished{ was_outdated, .. } => *was_outdated,
			_ => unreachable!("Task {} was not finished: {:?}", task, self.tasks[task]),
		};
		let mut newly_ready = 0;
		for DepInfo { task: next, order_only } in replace(&mut self.tasks[task].next, Vec::new()) {
			let next_phony;
			let next_outdated;
			match &mut self.tasks[next].status {
				TaskStatus::Needed { phony, outdated } => {
					if was_outdated && !order_only {
						*outdated = true;
					}
					next_phony = *phony;
					next_outdated = *outdated;
				}
				_ => unreachable!("Task {} in `next' list was not `Needed': {:?}", next, self.tasks[next]),
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

	pub fn get_task_status(&self, task: usize) -> TaskStatus {
		self.tasks[task].status
	}
}

impl AsyncBuildQueue {
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
	/// Returns None when all tasks are finished.
	pub fn wait(mut self) -> Option<usize> {
		while self.queue.ready.is_empty() && self.queue.n_left > 0 {
			self.queue = self.condvar.wait(self.queue).unwrap();
		}
		self.next()
	}

	/// Mark the task as ready, unblocking dependent tasks.
	pub fn complete_task(&mut self, task: usize, was_outdated: bool) {
		let n = self.queue.complete_task(task, was_outdated);
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

	pub fn clone_queue(&self) -> BuildQueue {
		self.queue.clone()
	}
}
