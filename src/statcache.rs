use std::collections::BTreeMap;
use std::path::Path;
use std::process::exit;
use std::time::SystemTime;

pub struct StatCache<'a> {
	cache: BTreeMap<&'a Path, Option<SystemTime>>,
}

impl<'a> StatCache<'a> {
	pub fn new() -> Self {
		StatCache {
			cache: BTreeMap::new(),
		}
	}

	pub fn mtime(&mut self, file: &'a Path) -> Option<SystemTime> {
		*self.cache.entry(file).or_insert_with(|| {
			match std::fs::metadata(file).and_then(|m| m.modified()) {
				Ok(time) => Some(time),
				Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => None,
				Err(e) => {
					eprintln!("Unable to get modification time of {:?}: {}", file, e);
					exit(1);
				}
			}
		})
	}
}
