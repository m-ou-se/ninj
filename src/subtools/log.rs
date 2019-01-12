use super::Options;
use ninj::buildlog::BuildLog;
use ninj::spec::read;
use std::io::Error;

pub(super) fn main(opt: &Options) -> Result<(), Error> {
	let spec = read(&opt.file)?;
	let build_log = BuildLog::read(spec.build_dir().join(".ninja_log"))?;
	println!("{:#?}", build_log);
	Ok(())
}
