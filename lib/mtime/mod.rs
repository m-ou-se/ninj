//! Getting the `mtime` of files to check if they're outdated.

use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::io::Error;
use std::path::Path;
use std::time::SystemTime;

/// Looks up the `mtime` of a file. Returns `None` if the file does not exist.
///
/// Each call to this function corresponds to a syscall. To save on syscalls,
/// consider using [`StatCache`] if you're going to check the same path
/// multiple times.
pub fn mtime(file: &Path) -> Result<Option<SystemTime>, Error> {
	match std::fs::metadata(file).and_then(|m| m.modified()) {
		Ok(time) => Ok(Some(time)),
		Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(e),
	}
}

/// A cache that remembers the `mtime`s of files.
pub struct StatCache<'a> {
	// `None` means the file does not exist.
	cache: BTreeMap<&'a Path, Option<SystemTime>>,
}

impl<'a> StatCache<'a> {
	/// Create an empty StatCache.
	pub fn new() -> Self {
		StatCache { cache: BTreeMap::new() }
	}

	/// Looks up the `mtime` of a file, returns the cached value if it exists.
	pub fn mtime(&mut self, file: &'a Path) -> Result<Option<SystemTime>, Error> {
		match self.cache.entry(file) {
			Entry::Vacant(v) => Ok(*v.insert(mtime(file)?)),
			Entry::Occupied(v) => Ok(*v.get()),
		}
	}

	/// Looks up the current `mtime` of a file without consulting the cache.
	///
	/// It does, however, store the result in the cache.
	pub fn fresh_mtime(&mut self, file: &'a Path) -> Result<Option<SystemTime>, Error> {
		let mtime = mtime(file)?;
		self.cache.insert(file, mtime);
		Ok(mtime)
	}
}
