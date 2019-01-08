use std::time::{Duration, Instant};

pub struct MinSec(Duration);

impl std::fmt::Display for MinSec {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		let secs = self.0.as_secs();
		let msecs = self.0.subsec_millis();
		write!(f, "{}:{:02}.{}", secs / 60, secs % 60, msecs / 100)
	}
}

impl MinSec {
	pub fn from_duration(d: Duration) -> Self {
		MinSec(d)
	}

	pub fn since(i: Instant) -> Self {
		MinSec::from_duration(i.elapsed())
	}
}
