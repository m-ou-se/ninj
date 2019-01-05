use ninj::spec::Spec;
use raw_string::RawStr;
use std::collections::{BTreeMap, VecDeque};
use std::mem::replace;
use std::sync::{Condvar, Mutex, MutexGuard};

#[derive(Clone, Debug)]
struct Deps {
	next: Vec<usize>, // Build rules which depend on this build rule.
	n_prev: usize,    // Number of unfinished build rules which have this rule in their `next` list.
}

struct BuildQueueInner {
	deps: Vec<Deps>,
	ready: Vec<usize>,
	n_left: usize,
}

pub struct BuildQueue {
	inner: Mutex<BuildQueueInner>,
	ready: Condvar,
}

pub struct BuildQueueLock<'a> {
	guard: MutexGuard<'a, BuildQueueInner>,
	ready: &'a Condvar,
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
		let mut tasks = Vec::new();

		let mut ready: Vec<usize> = Vec::new();

		let mut to_visit: VecDeque<usize> = targets.into();

		// Build dependency graph
		while let Some(task) = to_visit.pop_front() {
			let rule = &spec.build_rules[task];
			if !replace(&mut visited[task], true) {
				if !rule.is_phony() {
					tasks.push(task);
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

		// Remove phony nodes, and connect their inputs/outputs directly.
		for &task in &tasks {
			if deps[task]
				.next
				.iter()
				.any(|&i| spec.build_rules[i].is_phony())
			{
				let mut new_next = vec![];
				unphony(spec, &deps, &mut new_next, &deps[task].next);
				deps[task].next = new_next;
			}
		}

		BuildQueue {
			inner: Mutex::new(BuildQueueInner {
				deps,
				ready,
				n_left: tasks.len(),
			}),
			ready: Condvar::new(),
		}
	}

	pub fn lock(&self) -> BuildQueueLock {
		BuildQueueLock {
			guard: self.inner.lock().unwrap(),
			ready: &self.ready,
		}
	}
}

impl<'a> BuildQueueLock<'a> {
	/// Check if there is something to do right now.
	pub fn next(&mut self) -> Option<usize> {
		let next = self.guard.ready.pop();
		if next.is_some() {
			self.guard.n_left -= 1;
			if self.guard.n_left == 0 {
				self.ready.notify_all();
			}
		}
		next
	}

	/// Wait for something to do.
	///
	/// Returns None when all tasks are finished.
	pub fn wait(mut self) -> Option<usize> {
		while self.guard.ready.is_empty() && self.guard.n_left > 0 {
			self.guard = self.ready.wait(self.guard).unwrap();
		}
		self.next()
	}

	/// Mark the task as ready, unblocking dependent tasks.
	pub fn complete_task(&mut self, task: usize) {
		for next in replace(&mut self.guard.deps[task].next, Vec::new()) {
			self.guard.deps[next].n_prev -= 1;
			if self.guard.deps[next].n_prev == 0 {
				self.guard.ready.push(next);
				self.ready.notify_one();
			}
		}
	}
}

// Copies `next` into `new_next`, replacing all phony tasks by their
// (recursively 'unphonied') next tasks.
fn unphony(spec: &Spec, deps: &[Deps], new_next: &mut Vec<usize>, next: &[usize]) {
	for &next in next {
		if spec.build_rules[next].is_phony() {
			unphony(spec, deps, new_next, &deps[next].next);
		} else {
			new_next.push(next);
		}
	}
}
