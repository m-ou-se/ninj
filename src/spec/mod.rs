mod check;
mod eat;
mod path;
mod read;

pub mod error;
pub mod expand;
pub mod parse;
pub mod scope;

pub use self::read::read;

use raw_string::RawString;

#[derive(Debug)]
pub struct Spec {
	pub build_rules: Vec<BuildRule>,
	pub default_targets: Vec<RawString>,
}

#[derive(Debug)]
pub struct BuildRule {
	pub outputs: Vec<RawString>,
	pub inputs: Vec<RawString>,
	pub order_deps: Vec<RawString>,
	pub command: BuildRuleCommand,
}

#[derive(Debug)]
pub enum DepStyle {
	Gcc,
	Msvc,
}

#[derive(Debug)]
pub enum BuildRuleCommand {
	Phony,
	Command {
		command: RawString,
		description: RawString,
		/* TODO:
		depfile: String,
		deps: DepStyle
		generator: bool,
		restat: bool,
		rspfile: String,
		rspfile_content: String,
		*/
	},
}

impl Spec {
	pub fn new() -> Self {
		Spec {
			build_rules: Vec::new(),
			default_targets: Vec::new(),
		}
	}
}
