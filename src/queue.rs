use std::mem::{swap, replace};
use std::sync::{Mutex, MutexGuard, Condvar};
use std::collections::BTreeMap;
use raw_string::RawStr;
use ninj::spec::Spec;

#[derive(Clone, Debug)]
struct Deps {
	next: Vec<usize>, // Build rules which depend on this build rule.
	n_prev: usize, // Number of unfinished build rules which have this rule in their `next` list.
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
	pub fn new(spec: &Spec, target_to_rule: &BTreeMap<&RawStr, usize>, mut targets: Vec<usize>) -> BuildQueue {
		let mut deps = vec![Deps {
			next: vec![],
			n_prev: 0,
		}; spec.build_rules.len()];

		let mut need = vec![false; spec.build_rules.len()];
		let mut num_tasks = 0;

		let mut ready: Vec<usize> = Vec::new();
		let mut next: Vec<usize> = Vec::new();

		while !targets.is_empty() {
			for target in targets.drain(..) {
				let rule = &spec.build_rules[target];
				if !need[target] {
					num_tasks += 1;
					need[target] = true;
					for input in &rule.inputs {
						if let Some(&input) = target_to_rule.get(&input[..]) {
							if !need[input] {
								next.push(input);
							}
							deps[target].n_prev += 1;
							deps[input].next.push(target);
						} else {
							// TODO println!("Need file: {:?}", input);
						}
					}
					if deps[target].n_prev == 0 {
						ready.push(target);
					}
				}
			}
			swap(&mut targets, &mut next);
		}

		BuildQueue {
			inner: Mutex::new(BuildQueueInner {
				deps,
				ready,
				n_left: num_tasks,
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

