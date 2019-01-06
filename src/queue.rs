use std::mem::replace;
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Instant, Duration};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum TaskStatus {
	NotQueued,
	Queued,
	Ready,
	Running(Instant),
	Finished(Duration),
	PhonyQueued,
	PhonyFinished,
}

#[derive(Clone, Debug)]
pub struct Task {
	/// Status of this task.
	status: TaskStatus,
	/// Build rules which depend on this build rule.
	next: Vec<usize>,
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
	/// Number of (non-phony) tasks which still need to be started.
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
			F: FnMut(usize) -> (D, bool),
			D: IntoIterator<Item=usize>,
	{

		let mut tasks = vec![
			Task {
				status: TaskStatus::NotQueued,
				next: vec![],
				n_deps_left: 0,
			};
			max_task_num
		];

		let mut ready = Vec::new();
		let mut phony_finished = Vec::new();

		let mut n_tasks = 0;

		let mut to_visit = Vec::<usize>::new();

		for task in targets {
			if tasks[task].status == TaskStatus::NotQueued {
				to_visit.push(task);
				tasks[task].status = TaskStatus::Queued;
			}
		}

		// Build dependency graph
		while let Some(task) = to_visit.pop() {
			let (task_deps, phony) = get_task(task);
			if phony {
				tasks[task].status = TaskStatus::PhonyQueued;
			} else {
				n_tasks += 1;
			}
			let mut n_deps = 0;
			for dep in task_deps {
				if tasks[dep].status == TaskStatus::NotQueued {
					to_visit.push(dep);
					tasks[dep].status = TaskStatus::Queued;
				}
				n_deps += 1;
				tasks[dep].next.push(task);
			}
			tasks[task].n_deps_left = n_deps;
			if n_deps == 0 {
				if phony {
					phony_finished.push(task);
					tasks[task].status = TaskStatus::PhonyFinished;
				} else {
					ready.push(task);
					tasks[task].status = TaskStatus::Ready;
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
		while let Some(task) = phony_finished.pop() {
			queue.update_next_tasks_for_finished_task(task, &mut phony_finished);
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
			self.tasks[next].status = match &self.tasks[next].status {
				TaskStatus::Ready => TaskStatus::Running(Instant::now()),
				_ => panic!("Task {} was in the ready list, but was not ready: {:?}", next, self.tasks[next]),
			};
			self.n_left -= 1;
		}
		next
	}

	/// Mark the task as ready, possibly queueing dependent tasks.
	///
	/// Returns the number of newly ready tasks that were unblocked by the
	/// completion of this one.
	pub fn complete_task(&mut self, task: usize) -> usize {
		self.tasks[task].status = match &self.tasks[task].status {
			TaskStatus::Running(starttime) => TaskStatus::Finished(starttime.elapsed()),
			_ => panic!("complete_task({}) on task that isn't Running or PhonyQueued: {:?}", task, self.tasks[task]),
		};
		let mut newly_ready = 0;
		let mut phony_finished = Vec::new();
		newly_ready += self.update_next_tasks_for_finished_task(task, &mut phony_finished);
		while let Some(task) = phony_finished.pop() {
			newly_ready += self.update_next_tasks_for_finished_task(task, &mut phony_finished);
		}
		newly_ready
	}

	/// Decrement the `n_deps_left` of all the tasks depending on this task,
	/// and mark any newly ready tasks as ready.
	///
	/// Returns the amount of newly ready tasks.
	///
	/// Adds any now finished phony tasks to `phony_finished`.
	fn update_next_tasks_for_finished_task(&mut self, task: usize, phony_finished: &mut Vec<usize>) -> usize {
		let mut newly_ready = 0;
		for next in replace(&mut self.tasks[task].next, Vec::new()) {
			self.tasks[next].n_deps_left -= 1;
			if self.tasks[next].n_deps_left == 0 {
				match self.tasks[next].status {
					TaskStatus::Queued => {
						self.tasks[next].status = TaskStatus::Ready;
						self.ready.push(next);
						newly_ready += 1;
					}
					TaskStatus::PhonyQueued => {
						// Phony tasks are instantly finished, as they have no work to do.
						self.tasks[next].status = TaskStatus::PhonyFinished;
						phony_finished.push(next);
					}
					_ => panic!("By finishing task {}, task {} got ready even though it was not in queued state: {:?}", task, next, self.tasks[next]),
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
	pub fn complete_task(&mut self, task: usize) {
		let n = self.queue.complete_task(task);
		// TODO: In most cases we'll want to notify one time less, because this
		// thread itself will also continue executing tasks.
		for _ in 0..n {
			self.condvar.notify_one();
		}
	}

	pub fn clone_queue(&self) -> BuildQueue {
		self.queue.clone()
	}
}
