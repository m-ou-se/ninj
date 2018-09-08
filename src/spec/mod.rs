//! Everything related to the `build.ninja` file format.

mod eat;
mod path;
mod read;

pub mod error;
pub mod expand;
pub mod parse;
pub mod scope;

pub use self::read::read;

use raw_string::RawString;

/// The result of reading a `build.ninja` file, the specification of how to build what.
#[derive(Debug)]
pub struct Spec {
	pub build_rules: Vec<BuildRule>,
	pub default_targets: Vec<RawString>,
}

/// How to build a set of outputs from a set of inputs.
///
/// The direct result of a single `build` definition in the ninja file.
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

/// The command to run for a `BuildRule`.
#[derive(Debug)]
pub enum BuildRuleCommand {
	/// Don't run anything: This rule just serves as an alias.
	Phony,

	/// The command to generate the outputs from the inputs.
	Command {
		/// The (shell-escaped) command to be executed.
		command: RawString,
		/// The description to be shown to the user.
		description: RawString,
		depfile: RawString,
		deps: Option<DepStyle>,
		generator: bool,
		restat: bool,
		rspfile: RawString,
		rspfile_content: RawString,
	},
}

impl Spec {
	/// Create an empty specification.
	pub fn new() -> Self {
		Spec {
			build_rules: Vec::new(),
			default_targets: Vec::new(),
		}
	}
}
