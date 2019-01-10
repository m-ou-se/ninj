//! Everything related to the `.ninja_deps` file format.

use byteorder::{ReadBytesExt, WriteBytesExt, LE};
use indexmap::map::IndexMap;
use indexmap::map::Entry as IndexMapEntry;
use raw_string::{RawStr, RawString};
use std::fs::File;
use std::io::{BufReader, BufWriter, Error, ErrorKind, Read, Write};
use std::mem::replace;
use std::path::Path;

/// Represents the contents of a dependency log (from a `.ninja_deps` file).
#[derive(Clone, Debug)]
pub struct DepLog {
	records: IndexMap<RawString, Option<Record>>,
}

/// Represents a `./ninja_deps` file, and allows making additions to it.
#[derive(Debug)]
pub struct DepLogMut {
	deps: DepLog,
	file: BufWriter<File>,
}

#[derive(Clone, Debug)]
pub struct Record {
	pub deps: Vec<u32>,
	pub mtime: u64,
}

impl DepLog {
	pub fn new() -> Self {
		DepLog {
			records: IndexMap::new(),
		}
	}

	pub fn get_by_id(&self, id: u32) -> Option<(&RawStr, Option<&Record>)> {
		self.records.get_index(id as usize).map(|(k, v)| (&k[..], v.as_ref()))
	}

	pub fn path_by_id(&self, id: u32) -> Option<&RawStr> {
		self.records.get_index(id as usize).map(|(k, _)| &k[..])
	}

	pub fn deps_by_id(&self, id: u32) -> Option<&Record> {
		self.records.get_index(id as usize).and_then(|(_, v)| v.as_ref())
	}

	pub fn get_by_path(&self, path: &RawStr) -> Option<(u32, Option<&Record>)> {
		self.records.get_full(path).map(|(i, _, v)| (i as u32, v.as_ref()))
	}

	pub fn deps_by_path(&self, path: &RawStr) -> Option<&Record> {
		self.records.get(path).and_then(|v| v.as_ref())
	}

	pub fn id_by_path(&self, path: &RawStr) -> Option<u32> {
		self.records.get_full(path).map(|(i, _, _)| i as u32)
	}

	pub fn iter(&self) -> impl Iterator<Item=(&RawString, &Option<Record>)> {
		self.records.iter()
	}

	pub fn read(file: impl AsRef<Path>) -> Result<DepLog, Error> {
		DepLog::read_from(&mut File::open(file)?)
	}

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
						format!("Duplicate path in file: {:?}", records.get_index(id as usize).unwrap().0),
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
					file.read_u32::<LE>()? as u64 * 1_000_000_000 + 999_999_999
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
					mtime,
				});
			}
		}

		Ok(DepLog { records })
	}
}

impl DepLogMut {

	pub fn open(file: impl AsRef<Path>) -> Result<DepLogMut, Error> {
		let mut file = std::fs::OpenOptions::new().read(true).write(true).create(true).open(file)?;
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
		match entry {
			IndexMapEntry::Vacant(entry) => {
				let padding = (4 - entry.key().len() % 4) % 4;
				self.file.write_u32::<LE>(entry.key().len() as u32 + padding as u32 + 4)?;
				self.file.write_all(entry.key().as_bytes())?;
				self.file.write_all(&b"\0\0\0"[..padding])?;
				self.file.write_u32::<LE>(!id)?;
				entry.insert(None);
			}
			_ => {}
		}
		Ok(id)
	}

	/// Write a list of dependencies to the file, if it is different than
	/// what's already in the file.
	pub fn insert_deps(&mut self, target: RawString, mtime: u64, deps: Vec<RawString>) -> Result<(), Error> {
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
			self.file.write_u32::<LE>(0x8000_0000 | (dep_ids.len() as u32 * 4 + 12))?;
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
	fn test() -> Result<(), Error> {
		let file_name = "ninj-test-deps-file";
		std::fs::remove_file(file_name).ok();
		for _ in 0..2 {
			{
				let mut dep_log = DepLogMut::open(file_name)?;
				dep_log.insert_deps("output1".into(), 100, vec!["input1".into(), "input2".into()])?;
				dep_log.insert_deps("output2".into(), 200, vec!["input1".into(), "input3".into()])?;
			}
			{
				let dep_log = DepLog::read(file_name)?;
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output1")).unwrap().mtime, 100);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output2")).unwrap().mtime, 200);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output1")).unwrap().deps, vec![1, 2]);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output2")).unwrap().deps, vec![1, 4]);
				assert_eq!(dep_log.path_by_id(1).unwrap(), "input1");
				assert_eq!(dep_log.path_by_id(2).unwrap(), "input2");
				assert_eq!(dep_log.path_by_id(4).unwrap(), "input3");
			}
			{
				let mut dep_log = DepLogMut::open(file_name)?;
				dep_log.insert_deps("output1".into(), 100, vec!["input1".into(), "input2".into()])?;
				dep_log.insert_deps("output2".into(), 200, vec!["input1".into()])?;
				dep_log.insert_deps("output3".into(), 300, vec!["input4".into()])?;
			}
			{
				let dep_log = DepLog::read(file_name)?;
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output1")).unwrap().mtime, 100);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output2")).unwrap().mtime, 200);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output3")).unwrap().mtime, 300);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output1")).unwrap().deps, vec![1, 2]);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output2")).unwrap().deps, vec![1]);
				assert_eq!(dep_log.deps_by_path(RawStr::from_str("output3")).unwrap().deps, vec![6]);
				assert_eq!(dep_log.path_by_id(1).unwrap(), "input1");
				assert_eq!(dep_log.path_by_id(2).unwrap(), "input2");
				assert_eq!(dep_log.path_by_id(6).unwrap(), "input4");
			}
		}
		std::fs::remove_file(file_name)?;
		Ok(())
	}
}
