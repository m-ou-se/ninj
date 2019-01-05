use ninj::spec::Spec;
use raw_string::RawStr;
use std::collections::{BTreeMap, VecDeque};
use std::mem::replace;
use std::sync::{Condvar, Mutex, MutexGuard};

#[derive(Clone, Debug)]
struct Deps {
	/// Build rules which depend on this build rule.
	next: Vec<usize>,
	/// Number of unfinished build rules which have this rule in their `next` list.
	n_prev: usize,
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
	pub fn new(
		spec: &Spec,
		target_to_rule: &BTreeMap<&RawStr, usize>,
		targets: Vec<usize>,
	) -> BuildQueue {
		// TODO: Order-only dependencies.

		let mut deps = vec![
			Deps {
				next: vec![],
				n_prev: 0,
			};
			spec.build_rules.len()
		];

		let mut visited = vec![false; spec.build_rules.len()];
		let mut n_tasks = 0;
		let mut n_phony = 0;

		let mut ready: Vec<usize> = Vec::new();

		let mut to_visit: VecDeque<usize> = targets.into();

		// Build dependency graph
		while let Some(task) = to_visit.pop_front() {
			let rule = &spec.build_rules[task];
			if !replace(&mut visited[task], true) {
				n_tasks += 1;
				if rule.is_phony() {
					n_phony += 1;
				}
				for input in &rule.inputs {
					if let Some(&input) = target_to_rule.get(&input[..]) {
						if !visited[input] {
							to_visit.push_back(input);
						}
						deps[task].n_prev += 1;
						deps[input].next.push(task);
					} else {
						// TODO println!("Need file: {:?}", input);
					}
				}
				if deps[task].n_prev == 0 {
					ready.push(task);
				}
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
			self.deps[next].n_prev -= 1;
			if self.deps[next].n_prev == 0 {
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
