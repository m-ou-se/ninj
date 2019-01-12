//! Everything related to the `build.ninja` file format.
//!
//! > `ninja.build` file → [`read()`][spec::read()] → [`Spec`][spec::Spec]

mod canonicalizepath;
mod eat;
mod read;

pub mod error;
pub mod expand;
pub mod parse;
pub mod scope;

pub use self::read::read;
pub use self::read::read_from;

use raw_string::{RawStr, RawString};
use std::collections::BTreeMap;

/// The result of reading a `build.ninja` file, the specification of how to
/// build what.
#[derive(Debug)]
pub struct Spec {
	/// All the build rules.
	pub build_rules: Vec<BuildRule>,
	/// The targets to build by default.
	pub default_targets: Vec<RawString>,
	/// The build dir specified by `builddir = ..`, if any.
	pub build_dir: Option<RawString>,
}

/// How to build a set of outputs from a set of inputs.
///
/// The direct result of a single `build` definition in the ninja file.
#[derive(Debug)]
pub struct BuildRule {
	/// The list outputs.
	///
	/// Usually just one.
	///
	/// Never empty, if produced by [`read()`].
	pub outputs: Vec<RawString>,
	/// The list of inputs.
	pub inputs: Vec<RawString>,
	/// The list of order-only dependencies
	pub order_deps: Vec<RawString>,
	/// The details of command to run, or `None` for phony rules.
	pub command: Option<BuildCommand>,
}

impl BuildRule {
	/// Check if the build rule is just a phony rule.
	///
	/// Returns true iff `command` is `None`.
	pub fn is_phony(&self) -> bool {
		self.command.is_none()
	}
}

/// The method of discovering extra dependencies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepStyle {
	/// Through a Makefile-formatted file as specified by `depfile`.
	Gcc,
	/// Through specific messages detected on the standard output.
	Msvc,
}

/// The command to run for a non-phony `BuildRule`.
#[derive(Debug)]
pub struct BuildCommand {
	/// The name of the rule which was used for this build rule.
	pub rule_name: String,
	/// The (shell-escaped) command to be executed.
	pub command: RawString,
	/// The description to be shown to the user.
	pub description: RawString,
	/// The file to read the extra dependencies from.
	pub depfile: RawString,
	/// The way extra dependencies are to be discovered.
	pub deps: Option<DepStyle>,
	/// The message to watch for on standard output for extra dependencies.
	pub msvc_deps_prefix: RawString,
	/// Rule is used to re-invoke the generator. See ninja manual.
	pub generator: bool,
	/// Re-stat the command output to check if they actually changed.
	pub restat: bool,
	/// A file to write before executing the command.
	pub rspfile: RawString,
	/// The contents of the file to write before executing the command.
	pub rspfile_content: RawString,
	/// The name of the pool in which the command should run.
	pub pool: String,
	/// The depth of the pool, i.e. the maximum number of concurrent jobs in the
	/// pool.
	pub pool_depth: Option<u16>,
}

impl Spec {
	/// Create an empty specification.
	pub fn new() -> Self {
		Spec {
			build_rules: Vec::new(),
			default_targets: Vec::new(),
			build_dir: None,
		}
	}

	/// Get the 'builddir'.
	pub fn build_dir(&self) -> &std::path::Path {
		use raw_string::unix::RawStrExt;
		self.build_dir.as_ref().map_or(std::path::Path::new(""), |p| p.as_path())
	}

	/// Generate an index mapping output file names to build rule indexes.
	pub fn make_index(&self) -> BTreeMap<&RawStr, usize> {
		use log::warn;
		let mut index = BTreeMap::<&RawStr, usize>::new();
		for (rule_i, rule) in self.build_rules.iter().enumerate() {
			for output in &rule.outputs {
				if index.insert(&output, rule_i).is_some() {
					warn!(
						"Warning, multiple rules generating {:?}. Ignoring all but last one.",
						output
					);
				}
			}
		}
		index
	}
}
