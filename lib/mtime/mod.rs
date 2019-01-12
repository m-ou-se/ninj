//! Getting the `mtime` of files to check if they're outdated.

use std::cmp::max;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::io::Error;
use std::num::NonZeroU64;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A timestamp of a file.
///
/// `Option<Timestamp>` is the same size as `Timestamp`, as a timestamp is
/// never 0.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Timestamp(NonZeroU64);

impl Timestamp {
	/// Convert a `mtime` in nanoseconds (as used by the log files) to a
	/// [`Timestamp`].
	///
	/// A value of `0` is used for files that do not exist, and results in
	/// [`None`].
	pub fn from_nanos(mtime: u64) -> Option<Self> {
		NonZeroU64::new(mtime).map(Timestamp)
	}

	/// Convert [`Timestamp`] to a timestamp in nanoseconds (as used in the log
	/// files).
	pub fn to_nanos(self) -> u64 {
		self.0.get()
	}

	/// Convert a [`SystemTime`] to a [`Timestamp`].
	pub fn from_system_time(time: SystemTime) -> Self {
		let ns = time.duration_since(UNIX_EPOCH).ok().map_or(1, |d| {
			max(
				1,
				d.as_secs()
					.saturating_mul(1_000_000_000)
					.saturating_add(d.subsec_nanos().into()),
			)
		});
		debug_assert!(ns > 0);
		Timestamp(unsafe { NonZeroU64::new_unchecked(ns) })
	}

	/// Convert a [`Timestamp`] to a [`SystemTime`].
	pub fn to_system_time(self) -> SystemTime {
		UNIX_EPOCH + Duration::from_nanos(self.to_nanos())
	}
}

/// Looks up the `mtime` of a file. Returns `None` if the file does not exist.
///
/// Each call to this function corresponds to a syscall. To save on syscalls,
/// consider using [`StatCache`] if you're going to check the same path
/// multiple times.
pub fn mtime(file: &Path) -> Result<Option<Timestamp>, Error> {
	match std::fs::metadata(file).and_then(|m| m.modified()) {
		Ok(time) => Ok(Some(Timestamp::from_system_time(time))),
		Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(e) => Err(e),
	}
}

/// A cache that remembers the `mtime`s of files.
pub struct StatCache<'a> {
	// `None` means the file does not exist.
	cache: BTreeMap<&'a Path, Option<Timestamp>>,
}

impl<'a> StatCache<'a> {
	/// Create an empty StatCache.
	pub fn new() -> Self {
		StatCache {
			cache: BTreeMap::new(),
		}
	}

	/// Looks up the `mtime` of a file, returns the cached value if it exists.
	pub fn mtime(&mut self, file: &'a Path) -> Result<Option<Timestamp>, Error> {
		match self.cache.entry(file) {
			Entry::Vacant(v) => Ok(*v.insert(mtime(file)?)),
			Entry::Occupied(v) => Ok(*v.get()),
		}
	}

	/// Looks up the current `mtime` of a file without consulting the cache.
	///
	/// It does, however, store the result in the cache.
	pub fn fresh_mtime(&mut self, file: &'a Path) -> Result<Option<Timestamp>, Error> {
		let mtime = mtime(file)?;
		self.cache.insert(file, mtime);
		Ok(mtime)
	}

	/// Looks up the `mtime` of a file in the cache.
	///
	/// *Only* checks the cache. Will not check the file system.
	///
	/// If the cache does not contain an entry for this file, returns `None`.
	///
	/// If the file does not exist according to the cache, returns `Some(None)`.
	pub fn cached_mtime(&mut self, file: &Path) -> Option<Option<Timestamp>> {
		self.cache.get(file).cloned()
	}
}
