use super::Options;
use ninj::deplog::DepLogMut;
use ninj::mtime::Timestamp;
use raw_string::unix::RawStrExt;
use std::io::Error;

pub(super) fn main(opt: &Options) -> Result<(), Error> {
	use ninj::spec::read;
	let spec = read(&opt.file)?;
	let targets = spec.make_index();
	let dep_log = DepLogMut::open(spec.build_dir().join(".ninja_deps"))?;
	for (path, deps) in dep_log.iter() {
		if targets.contains_key(&path[..]) {
			let mtime = || {
				std::fs::metadata(path.as_path())
					.and_then(|m| m.modified())
					.ok()
					.map(Timestamp::from_system_time)
			};
			let nanos = deps.mtime().map_or(0, Timestamp::to_nanos);
			println!(
				"{}: #deps {}, deps mtime {} ({})",
				path,
				deps.deps().len(),
				nanos,
				if deps.mtime().map_or(true, |t| Some(t) < mtime()) {
					"STALE"
				} else {
					"VALID"
				}
			);
			for dep in deps.deps() {
				println!("    {}", dep);
			}
			println!();
		}
	}
	Ok(())
}
