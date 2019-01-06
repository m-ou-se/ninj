use std::time::{Instant, Duration};

pub struct MinSec(u64);

impl std::fmt::Display for MinSec {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{m}:{s:02}", m=self.0 / 60, s=self.0 % 60)
	}
}

impl MinSec {
	pub fn from_duration(d: Duration) -> Self {
		MinSec(d.as_secs())
	}

	pub fn since(i: Instant) -> Self {
		MinSec::from_duration(i.elapsed())
	}
}
