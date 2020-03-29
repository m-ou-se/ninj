mod deps;
mod graph;
mod log;
mod rules;
mod spec;
mod targets;

use super::Options;
use std::io::{Error, ErrorKind};

static SUBTOOLS: &'static [(&'static str, fn(&Options) -> Result<(), Error>)] = &[
	("deps", deps::main),
	("graph", self::graph::main),
	("log", log::main),
	("rules", rules::main),
	("spec", spec::main),
	("targets", targets::main),
	("list", list),
];

pub(super) fn run_subtool(tool: &str, options: &Options) -> Result<(), Error> {
	if let Some((_, main)) = SUBTOOLS.iter().find(|(name, _)| *name == tool) {
		main(options)
	} else {
		Err(Error::new(
			ErrorKind::Other,
			format!("Unknown subtool {:?}", tool),
		))
	}
}

fn list(_: &Options) -> Result<(), Error> {
	println!("Subtools:");
	for (name, _) in SUBTOOLS {
		println!("\t{}", name);
	}
	Ok(())
}
