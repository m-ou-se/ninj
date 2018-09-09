//! Everything related to the `build.ninja` file format.

mod eat;
mod read;

pub mod error;
pub mod expand;
pub mod parse;
pub mod scope;

pub use self::read::read;

use raw_string::RawString;
use std::ffi::OsString;
use std::path::PathBuf;

/// The result of reading a `build.ninja` file, the specification of how to build what.
#[derive(Debug)]
pub struct Spec {
	pub build_rules: Vec<BuildRule>,
	pub default_targets: Vec<PathBuf>,
	pub build_dir: RawString,
}

/// How to build a set of outputs from a set of inputs.
///
/// The direct result of a single `build` definition in the ninja file.
#[derive(Debug)]
pub struct BuildRule {
	pub outputs: Vec<PathBuf>,
	pub inputs: Vec<PathBuf>,
	pub order_deps: Vec<PathBuf>,
	pub command: BuildRuleCommand,
}

/// The method of discovering extra dependencies.
#[derive(Debug)]
pub enum DepStyle {
	/// Through a Makefile-formatted file as specified by `depfile`.
	Gcc,
	/// Through specific messages detected on the standard output.
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
		command: OsString,
		/// The description to be shown to the user.
		description: RawString,
		/// The file to read the extra dependencies from.
		depfile: PathBuf,
		/// The way extra dependencies are to be discovered.
		deps: Option<DepStyle>,
		/// The message to watch for on standard output for extra dependencies.
		msvc_deps_prefix: RawString,
		/// Rule is used to re-invoke the generator. See ninja manual.
		generator: bool,
		/// Re-stat the command output to check if they actually changed.
		restat: bool,
		/// A file to write before executing the command.
		rspfile: PathBuf,
		/// The contents of the file to write before executing the command.
		rspfile_content: RawString,
		/// The name of the pool in which the command should run.
		pool: String,
		/// The depth of the pool, i.e. the maximum number of concurrent jobs in the pool.
		pool_depth: Option<u16>,
	},
}

impl Spec {
	/// Create an empty specification.
	pub fn new() -> Self {
		Spec {
			build_rules: Vec::new(),
			default_targets: Vec::new(),
			build_dir: RawString::new(),
		}
	}
}
