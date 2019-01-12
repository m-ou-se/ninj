use super::Options;
use ninj::spec::read;
use std::io::Error;

pub(super) fn main(opt: &Options) -> Result<(), Error> {
	let spec = read(&opt.file)?;
	for target in &spec.build_rules {
		for output in &target.outputs {
			println!(
				"{}: {}",
				output,
				target.command.as_ref().map_or("phony", |c| &c.rule_name)
			);
		}
	}
	Ok(())
}
