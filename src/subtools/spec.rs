use super::Options;
use ninj::spec::read;
use std::io::Error;

pub(super) fn main(opt: &Options) -> Result<(), Error> {
	let spec = read(&opt.file)?;
	println!("{:#?}", spec);
	Ok(())
}
