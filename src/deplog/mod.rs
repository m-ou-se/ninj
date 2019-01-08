//! Everything related to the `.ninja_deps` file format.

use byteorder::{ReadBytesExt, LE};
use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufReader, Error, ErrorKind, Read};
use std::mem::replace;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Deps {
	pub records: Vec<Record>,
}

#[derive(Clone, Debug)]
pub struct Record {
	pub path: RawString,
	pub deps: Option<RecordDeps>,
}

#[derive(Clone, Debug)]
pub struct RecordDeps {
	pub deps: Vec<u32>,
	pub mtime: u64,
}

impl Deps {
	pub fn new() -> Self {
		Deps { records: Vec::new() }
	}

	pub fn read(file: impl AsRef<Path>) -> Result<Deps, std::io::Error> {
		Deps::read_from(File::open(file)?)
	}

	pub fn read_from(file: File) -> Result<Deps, std::io::Error> {
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
				format!("Only version 3 and 4 are supported, but version {} was found", version),
			));
		}

		let mut records = Vec::new();

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
						format!("Invalid checksum in file: 0x{:08x} for ID 0x{:08x}", checksum, id),
					));
				}

				records.push(Record {
					path: RawString::from_bytes(name),
					deps: None,
				});
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

				let record = match records.get_mut(id) {
					Some(r) => r,
					None => {
						return Err(Error::new(
							ErrorKind::InvalidData,
							format!("Dependencies record for undefined path ID: 0x{:x}", id),
						));
					}
				};

				let mut record_deps = match &mut record.deps {
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

				record.deps = Some(RecordDeps {
					deps: record_deps,
					mtime,
				});
			}
		}

		Ok(Deps { records })
	}

	/// Generate a map of the paths to their index in the `records` vector.
	pub fn index_paths(&self) -> BTreeMap<&RawStr, usize> {
		self.records
			.iter()
			.enumerate()
			.map(|(i, record)| (&record.path[..], i))
			.collect()
	}

	/// Generate a map of the paths to their index in the `records` vector, but
	/// only for the paths that have dependencies recorded.
	pub fn index_targets(&self) -> BTreeMap<&RawStr, usize> {
		self.records
			.iter()
			.enumerate()
			.filter(|(_, record)| record.deps.is_some())
			.map(|(i, record)| (&record.path[..], i))
			.collect()
	}
}
