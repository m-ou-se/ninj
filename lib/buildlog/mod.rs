//! Reading and writing build logs (i.e. `.ninja_log` files).

use crate::mtime::Timestamp;
use crate::spec::BuildRule;
use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Error, ErrorKind, Write};
use std::path::Path;
use std::time::{Instant, Duration};
use std::iter::FromIterator;

mod murmurhash;

pub use self::murmurhash::murmur_hash_64a;

/// The latest entries for all targets in the build log.
#[derive(Clone, Debug)]
pub struct BuildLog {
	pub entries: BTreeMap<RawString, Entry>,
}

/// An entry in the build log for a specific target.
#[derive(Clone, Debug)]
pub struct Entry {
	pub start_time_ms: u32,
	pub end_time_ms: u32,
	pub restat_mtime: Option<Timestamp>,
	pub command_hash: u64,
}

impl BuildLog {
	/// Create an empty build log.
	pub fn new() -> BuildLog {
		BuildLog {
			entries: BTreeMap::new(),
		}
	}

	pub fn estimated_total_task_time(&self, output: RawString, _command: RawString) -> Option<Duration> {
		match self.entries.get(&output) {
			Some(entry) => Some(Duration::from_millis((entry.end_time_ms - entry.start_time_ms).into())),
			None => {
				// TODO: Search entries for command_hash equal to murmur_hash_64a(_command)
				None
			},
		}
	}

	pub fn average_historic_task_time(&self) -> Option<Duration> {
		if self.entries.is_empty() {
			None
		} else {
			let mut sum_ms = 0;
			for (_output, entry) in self.entries.iter() {
				sum_ms += (entry.end_time_ms - entry.start_time_ms) as u64;
			}
			Some(Duration::from_millis(sum_ms / self.entries.len() as u64))
		}
	}

	pub fn add_entry(&mut self, rule: &BuildRule, build_starttime: Instant, starttime: Instant, endtime: Instant) {
		let command = &rule.command.as_ref().expect("Got phony rule").command;
		for output in &rule.outputs {
			self.entries.insert(output.clone(), Entry {
				start_time_ms: (starttime - build_starttime).as_millis() as u32,
				end_time_ms:   (endtime - build_starttime).as_millis() as u32,
				restat_mtime:  None,
				command_hash:  murmur_hash_64a(command.as_bytes()),
			});
		}
	}

	/// Read a build log from a file.
	pub fn read(file: impl AsRef<Path>) -> Result<BuildLog, Error> {
		let file = File::open(file.as_ref()).map_err(|e| {
			Error::new(
				e.kind(),
				format!("Unable to read {:?}: {}", file.as_ref(), e),
			)
		})?;
		BuildLog::read_from(file)
	}

	pub fn write(&self, file: impl AsRef<Path>) -> Result<(), Error> {
		// TODO: should we append to this file instead of truncating it?
		self.write_to(File::create(file)?)
	}

	pub fn write_to(&self, file: File) -> Result<(), Error> {
		let mut file = BufWriter::new(file);

		file.write(b"# ninja log v5\n")?;

		// Write entries in order of finishing time. Note that this removes all
		// 'dead' entries immediately, while ninja would only do that later.

		let mut entries = Vec::from_iter(&self.entries);
		entries.sort_by(|(_, left), (_, right)| right.end_time_ms.cmp(&left.end_time_ms));
		for (output, entry) in entries {
			write!(file, "{}\t{}\t{}\t{}\t{:x}\n",
				entry.start_time_ms,
				entry.end_time_ms,
				entry.restat_mtime.map_or(0, Timestamp::to_nanos),
				output,
				entry.command_hash)?;
		}

		Ok(())
	}

	/// Read a build log.
	pub fn read_from(file: File) -> Result<BuildLog, Error> {
		let mut file = BufReader::new(file);

		let mut line = RawString::new();

		file.read_until(b'\n', &mut line.as_mut_bytes())?;

		if !line.starts_with("# ninja log v") {
			return Err(Error::new(ErrorKind::InvalidData, "Not a ninja log file"));
		}

		if line.last() == Some(b'\n') {
			line.pop();
		}

		let version: u32 = parse(&line[13..])
			.ok_or_else(|| Error::new(ErrorKind::InvalidData, "Version is not an integer"))?;

		if version != 4 && version != 5 {
			return Err(Error::new(
				ErrorKind::InvalidData,
				format!(
					"Unsupported version {} (only version 4 and 5 are supported)",
					version
				),
			));
		}

		let missing_field = || Error::new(ErrorKind::InvalidData, "Missing field");
		let not_an_integer = || Error::new(ErrorKind::InvalidData, "Field is not an integer");
		let not_hex = || Error::new(ErrorKind::InvalidData, "Invalid command hash");

		let mut entries = BTreeMap::new();

		loop {
			line.clear();
			if file.read_until(b'\n', &mut line.as_mut_bytes())? == 0 {
				break;
			}

			if line.last() == Some(b'\n') {
				line.pop();
			}

			let mut tab_iter = memchr::memchr_iter(b'\t', line.as_bytes());

			let tab1 = tab_iter.next().ok_or_else(missing_field)?;
			let tab2 = tab_iter.next().ok_or_else(missing_field)?;
			let tab3 = tab_iter.next().ok_or_else(missing_field)?;
			let tab4 = tab_iter.next().ok_or_else(missing_field)?;

			let key = line[tab3 + 1..tab4].into();
			let value = Entry {
				start_time_ms: parse(&line[0..tab1]).ok_or_else(not_an_integer)?,
				end_time_ms: parse(&line[tab1 + 1..tab2]).ok_or_else(not_an_integer)?,
				restat_mtime: Timestamp::from_nanos(
					parse(&line[tab2 + 1..tab3]).ok_or_else(not_an_integer)?,
				),
				command_hash: if version < 5 {
					murmur_hash_64a(&line[tab4 + 1..].as_bytes())
				} else {
					parse_hex(&line[tab4 + 1..]).ok_or_else(not_hex)?
				},
			};

			entries.insert(key, value);
		}

		Ok(BuildLog { entries })
	}
}

fn parse<T: std::str::FromStr>(s: &RawStr) -> Option<T> {
	s.to_str().ok().and_then(|s| s.parse().ok())
}

fn parse_hex(s: &RawStr) -> Option<u64> {
	s.to_str()
		.ok()
		.and_then(|s| u64::from_str_radix(s, 16).ok())
}
