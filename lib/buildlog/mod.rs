//! Reading and writing build logs (i.e. `.ninja_log` files).

use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Error, ErrorKind, BufRead};
use std::path::Path;

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
	pub restat_mtime: u64,
	pub command_hash: u64,
}

impl BuildLog {
	/// Create an empty build log.
	pub fn new() -> BuildLog {
		BuildLog { entries: BTreeMap::new() }
	}

	/// Read a build log from a file.
	pub fn read(file: impl AsRef<Path>) -> Result<BuildLog, Error> {
		BuildLog::read_from(File::open(file)?)
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

		let version: u32 =
			parse(&line[13..]).ok_or_else(|| Error::new(ErrorKind::InvalidData, "Version is not an integer"))?;

		if version != 4 && version != 5 {
			return Err(Error::new(
				ErrorKind::InvalidData,
				format!("Unsupported version {} (only version 4 and 5 are supported)", version),
			));
		}

		let missing_field = || Error::new(ErrorKind::InvalidData, "Missing field");
		let not_an_integer = || Error::new(ErrorKind::InvalidData, "Field is not an integer");
		let not_hex = || {
			Error::new(
				ErrorKind::InvalidData,
				"Command hash is not a 64-bit hexadecimal number",
			)
		};

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
				restat_mtime: parse(&line[tab2 + 1..tab3]).ok_or_else(not_an_integer)?,
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
	s.to_str().ok().and_then(|s| u64::from_str_radix(s, 16).ok())
}
