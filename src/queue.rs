use std::collections::VecDeque;
use std::mem::replace;
use std::sync::{Condvar, Mutex, MutexGuard};

#[derive(Clone, Debug)]
struct Deps {
	/// Build rules which depend on this build rule.
	next: Vec<usize>,
	/// Number of unfinished build rules which have this rule in their `next` list.
	n_deps_left: usize,
}

pub struct BuildQueue {
	/// Dependencies of build rules.
	///
	/// The index in this vector is their ID.
	deps: Vec<Deps>,
	/// The tasks which are ready to run.
	ready: Vec<usize>,
	/// Number of tasks which still need to be started.
	n_left: usize,
	/// Number of tasks which still need to be started, which are only phony tasks.
	n_phony_left: usize,
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
	/// The (potiential) tasks are numbered 0 to `max_task_num`.
	///
	/// `targets` are the tasks thad need to be executed.
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

		let mut deps = vec![
			Deps {
				next: vec![],
				n_deps_left: 0,
			};
			max_task_num
		];

		let mut ready = Vec::new();

		let mut n_tasks = 0;
		let mut n_phony = 0;

		#[derive(Copy, Clone, PartialEq, Eq)]
		enum State {
			Unvisited,
			Queued,
			Visited,
		}

		let mut visited = vec![State::Unvisited; max_task_num];
		let mut to_visit = VecDeque::<usize>::new();

		for task in targets.into_iter() {
			if visited[task] == State::Unvisited {
				to_visit.push_back(task);
				visited[task] = State::Queued;
			}
		}

		// Build dependency graph
		while let Some(task) = to_visit.pop_front() {
			visited[task] = State::Visited;
			let (task_deps, phony) = get_task(task);
			n_tasks += 1;
			if phony {
				n_phony += 1;
			}
			let mut n_deps = 0;
			for dep in task_deps.into_iter() {
				if visited[dep] == State::Unvisited {
					to_visit.push_back(dep);
					visited[dep] = State::Queued;
				}
				n_deps += 1;
				deps[dep].next.push(task);
			}
			deps[task].n_deps_left = n_deps;
			if n_deps == 0 {
				ready.push(task);
			}
		}

		// TODO: Check for cycles.

		BuildQueue {
			deps,
			ready,
			n_left: n_tasks,
			n_phony_left: n_phony,
		}
	}

	pub fn make_async(self) -> AsyncBuildQueue {
		AsyncBuildQueue {
			queue: Mutex::new(self),
			condvar: Condvar::new(),
		}
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

impl BuildQueue {
	/// Check if there is something to do right now.
	pub fn next(&mut self) -> Option<usize> {
		let next = self.ready.pop();
		if next.is_some() {
			self.n_left -= 1;
		}
		next
	}

	/// Mark the task as ready, possibly queueing dependent tasks.
	///
	/// Returns the number of newly ready tasks that were unblocked by the
	/// completion of this one.
	pub fn complete_task(&mut self, task: usize) -> usize {
		let mut newly_ready = 0;
		for next in replace(&mut self.deps[task].next, Vec::new()) {
			self.deps[next].n_deps_left -= 1;
			if self.deps[next].n_deps_left == 0 {
				self.ready.push(next);
				newly_ready += 1;
			}
		}
		newly_ready
	}
}

impl<'a> LockedAsyncBuildQueue<'a> {
	/// Check if there is something to do right now.
	pub fn next(&mut self) -> Option<usize> {
		let next = self.queue.next();
		if next.is_some() {
			if self.queue.n_left <= self.queue.n_phony_left {
				self.condvar.notify_all();
			}
		}
		next
	}

	/// Wait for something to do.
	///
	/// Returns None when all tasks are finished.
	pub fn wait(mut self) -> Option<usize> {
		while self.queue.ready.is_empty() && self.queue.n_left > self.queue.n_phony_left {
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
}
