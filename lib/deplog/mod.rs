//! Reading and writing dependency logs (i.e. `.ninja_deps` files).

use crate::mtime::Timestamp;
use byteorder::{ReadBytesExt, WriteBytesExt, LE};
use indexmap::map::Entry as IndexMapEntry;
use indexmap::map::IndexMap;
use raw_string::{RawStr, RawString};
use std::fs::File;
use std::io::{BufReader, BufWriter, Error, ErrorKind, Read, Write};
use std::mem::replace;
use std::path::Path;

/// Represents the contents of a `.ninja_deps` file.
#[derive(Clone, Debug)]
pub struct DepLog {
	records: IndexMap<RawString, Option<Record>>,
}

/// Represents a `.ninja_deps` file, and allows making additions to it.
#[derive(Debug)]
pub struct DepLogMut {
	deps: DepLog,
	file: BufWriter<File>,
}

/// The information you get out of a `DepLog` for a specific target.
#[derive(Clone, Copy, Debug)]
pub struct TargetInfo<'a> {
	record: &'a Record,
	log: &'a DepLog,
}

#[derive(Clone, Debug)]
struct Record {
	deps: Vec<u32>,
	mtime: Option<Timestamp>,
}

impl DepLog {
	/// Create a new empty log.
	pub fn new() -> Self {
		DepLog {
			records: IndexMap::new(),
		}
	}

	fn path_by_id(&self, id: u32) -> Option<&RawStr> {
		self.records.get_index(id as usize).map(|(k, _)| &k[..])
	}

	/// Look up a target in the log.
	pub fn get(&self, path: &RawStr) -> Option<TargetInfo> {
		self.records.get(path).and_then(|v| {
			v.as_ref().map(|r| TargetInfo {
				record: r,
				log: self,
			})
		})
	}

	/// Iterate over all targets in the log.
	pub fn iter(&self) -> impl Iterator<Item = (&RawStr, TargetInfo)> {
		let log = self;
		self.records.iter().flat_map(move |(k, v)| {
			v.as_ref()
				.map(move |v| (&k[..], TargetInfo { record: v, log }))
		})
	}

	/// Read a log from a file.
	pub fn read(file: impl AsRef<Path>) -> Result<DepLog, Error> {
		let mut file = File::open(file.as_ref()).map_err(|e| {
			Error::new(
				e.kind(),
				format!("Unable to read {:?}: {}", file.as_ref(), e),
			)
		})?;
		DepLog::read_from(&mut file)
	}

	/// Read a log.
	pub fn read_from(file: &mut dyn Read) -> Result<DepLog, Error> {
		let mut file = BufReader::new(file);

		{
			let mut header = [0u8; 12];
			file.read_exact(&mut header)?;
			if &header != b"# ninjadeps\n" {
				return Err(Error::new(ErrorKind::InvalidData, "Not a ninjadeps file"));
			}
		}

		let version = file.read_u32::<LE>()?;
		if version != 3 && version != 4 {
			return Err(Error::new(
				ErrorKind::InvalidData,
				format!(
					"Only version 3 and 4 are supported, but version {} was found",
					version
				),
			));
		}

		let mut records = IndexMap::<RawString, Option<Record>>::new();

		while let Some(record_head) = match file.read_u32::<LE>() {
			Ok(data) => Some(data),
			Err(ref e) if e.kind() == ErrorKind::UnexpectedEof => None,
			Err(e) => return Err(e),
		} {
			if record_head & 0x8000_0000 == 0 {
				// Path record
				let size = record_head;

				if size % 4 != 0 || size < 4 {
					return Err(Error::new(
						ErrorKind::InvalidData,
						format!("Invalid path record size: 0x{:x}", size),
					));
				}

				let id = records.len() as u32;

				let mut name = vec![0u8; size as usize - 4];
				file.read_exact(&mut name)?;
				while name.last() == Some(&0u8) {
					// Remove padding
					name.pop();
				}

				let checksum = file.read_u32::<LE>()?;
				if checksum != !id {
					return Err(Error::new(
						ErrorKind::InvalidData,
						format!(
							"Invalid checksum in file: 0x{:08x} for ID 0x{:08x}",
							checksum, id
						),
					));
				}

				if records.insert(RawString::from_bytes(name), None).is_some() {
					return Err(Error::new(
						ErrorKind::InvalidData,
						format!(
							"Duplicate path in file: {:?}",
							records.get_index(id as usize).unwrap().0
						),
					));
				}
			} else {
				// Deps record
				let size = record_head & 0x7FFF_FFFF;

				if size % 4 != 0 || size < if version < 4 { 8 } else { 12 } {
					return Err(Error::new(
						ErrorKind::InvalidData,
						format!("Invalid dependencies record size: 0x{:x}", size),
					));
				}

				let len = (size / 4 - if version < 4 { 2 } else { 3 }) as usize;

				let id = file.read_u32::<LE>()? as usize;

				let mtime = if version < 4 {
					u64::from(file.read_u32::<LE>()?) * 1_000_000_000 + 999_999_999
				} else {
					file.read_u64::<LE>()?
				};

				let n_records = records.len();

				let record = match records.get_index_mut(id) {
					Some((_, r)) => r,
					None => {
						return Err(Error::new(
							ErrorKind::InvalidData,
							format!("Dependencies record for undefined path ID: 0x{:x}", id),
						));
					}
				};

				let mut record_deps = match record {
					Some(r) => {
						// Re-use the old deps vector.
						let mut d = replace(&mut r.deps, Vec::new());
						d.clear();
						d
					}
					None => Vec::new(),
				};

				record_deps.reserve_exact(len);

				for _ in 0..len {
					let dep = file.read_u32::<LE>()?;
					if dep as usize >= n_records {
						return Err(Error::new(
							ErrorKind::InvalidData,
							format!("Undefined path ID in dependency: 0x{:x}", dep),
						));
					}
					record_deps.push(dep);
				}

				*record = Some(Record {
					deps: record_deps,
					mtime: Timestamp::from_nanos(mtime),
				});
			}
		}

		Ok(DepLog { records })
	}
}

impl<'a> TargetInfo<'a> {
	/// Get the `mtime` that was recorded in the log.
	pub fn mtime(&self) -> Option<Timestamp> {
		self.record.mtime
	}

	/// Get an iterator over the dependencies.
	pub fn deps(&self) -> impl Iterator<Item = &'a RawStr> + ExactSizeIterator {
		let log = self.log;
		self.record
			.deps
			.iter()
			.map(move |&i| log.path_by_id(i).unwrap())
	}
}

impl DepLogMut {
	/// Open and read a dependency log, or start a new one.
	pub fn open(file: impl AsRef<Path>) -> Result<DepLogMut, Error> {
		let mut file = std::fs::OpenOptions::new()
			.read(true)
			.write(true)
			.create(true)
			.open(file)?;
		if file.metadata()?.len() == 0 {
			file.write_all(b"# ninjadeps\n\x04\0\0\0")?;
			Ok(DepLogMut {
				deps: DepLog::new(),
				file: BufWriter::new(file),
			})
		} else {
			Ok(DepLogMut {
				deps: DepLog::read_from(&mut file)?,
				file: BufWriter::new(file),
			})
		}
	}

	/// Writes a path to the file, if it wasn't already in there.
	///
	/// In both cases, it returns the ID of the path.
	fn insert_path(&mut self, path: RawString) -> Result<u32, Error> {
		let entry = self.deps.records.entry(path);
		let id = entry.index() as u32;
		if let IndexMapEntry::Vacant(entry) = entry {
			let padding = (4 - entry.key().len() % 4) % 4;
			let size = entry.key().len() as u32 + padding as u32 + 4;
			self.file.write_u32::<LE>(size)?;
			self.file.write_all(entry.key().as_bytes())?;
			self.file.write_all(&b"\0\0\0"[..padding])?;
			self.file.write_u32::<LE>(!id)?;
			entry.insert(None);
		}
		Ok(id)
	}

	/// Write a list of dependencies to the file, if it is different than
	/// what's already in the file.
	pub fn insert_deps(
		&mut self,
		target: RawString,
		mtime: Option<Timestamp>,
		deps: Vec<RawString>,
	) -> Result<(), Error> {
		let target = self.insert_path(target)?;
		let record = self.deps.records.get_index_mut(target as usize).unwrap().1;

		let mut need_write = false;

		let mut dep_ids = if let Some(record) = record.as_mut() {
			if record.mtime != mtime {
				need_write = true;
			}
			replace(&mut record.deps, Vec::new())
		} else {
			need_write = true;
			Vec::new()
		};

		if deps.len() != dep_ids.len() {
			need_write = true;
			dep_ids.resize(deps.len(), !0);
		}

		for (dep, dep_id) in deps.into_iter().zip(dep_ids.iter_mut()) {
			let new_id = self.insert_path(dep)?;
			if *dep_id != new_id {
				need_write = true;
				*dep_id = new_id;
			}
		}

		if need_write {
			let size = dep_ids.len() as u32 * 4 + 12;
			let mtime = mtime.map_or(0, Timestamp::to_nanos);
			self.file.write_u32::<LE>(0x8000_0000 | size)?;
			self.file.write_u32::<LE>(target)?;
			self.file.write_u64::<LE>(mtime)?;
			for &dep in &dep_ids {
				self.file.write_u32::<LE>(dep)?;
			}
		}

		*self.deps.records.get_index_mut(target as usize).unwrap().1 = Some(Record {
			deps: dep_ids,
			mtime,
		});

		Ok(())
	}
}

impl std::ops::Deref for DepLogMut {
	type Target = DepLog;
	fn deref(&self) -> &Self::Target {
		&self.deps
	}
}

#[cfg(test)]
mod test {
	use super::*;

	#[test]
	#[rustfmt::skip]
	fn test() -> Result<(), Error> {
		let file_name = "ninj-test-deps-file";
		std::fs::remove_file(file_name).ok();
		for _ in 0..2 {
			{
				let mut dep_log = DepLogMut::open(file_name)?;
				dep_log.insert_deps("output1".into(), Timestamp::from_nanos(100), vec!["input1".into(), "input2".into()])?;
				dep_log.insert_deps("output2".into(), Timestamp::from_nanos(200), vec!["input1".into(), "input3".into()])?;
			}
			{
				let dep_log = DepLog::read(file_name)?;
				assert_eq!(dep_log.get(RawStr::from_str("output1")).unwrap().mtime(), Timestamp::from_nanos(100));
				assert_eq!(dep_log.get(RawStr::from_str("output2")).unwrap().mtime(), Timestamp::from_nanos(200));
				assert!(dep_log.get(RawStr::from_str("output1")).unwrap().deps().eq(&["input1", "input2"]));
				assert!(dep_log.get(RawStr::from_str("output2")).unwrap().deps().eq(&["input1", "input3"]));
			}
			{
				let mut dep_log = DepLogMut::open(file_name)?;
				dep_log.insert_deps("output1".into(), Timestamp::from_nanos(100), vec!["input1".into(), "input2".into()])?;
				dep_log.insert_deps("output2".into(), Timestamp::from_nanos(200), vec!["input1".into()])?;
				dep_log.insert_deps("output3".into(), Timestamp::from_nanos(300), vec!["input4".into()])?;
			}
			{
				let dep_log = DepLog::read(file_name)?;
				assert_eq!(dep_log.get(RawStr::from_str("output1")).unwrap().mtime(), Timestamp::from_nanos(100));
				assert_eq!(dep_log.get(RawStr::from_str("output2")).unwrap().mtime(), Timestamp::from_nanos(200));
				assert_eq!(dep_log.get(RawStr::from_str("output3")).unwrap().mtime(), Timestamp::from_nanos(300));
				assert!(dep_log.get(RawStr::from_str("output1")).unwrap().deps().eq(&["input1", "input2"]));
				assert!(dep_log.get(RawStr::from_str("output2")).unwrap().deps().eq(&["input1"]));
				assert!(dep_log.get(RawStr::from_str("output3")).unwrap().deps().eq(&["input4"]));
			}
		}
		std::fs::remove_file(file_name)?;
		Ok(())
	}
}
