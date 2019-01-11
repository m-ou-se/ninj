//! Variable and rule definition scoping and lookup.

use raw_string::{RawStr, RawString};

use super::parse::Variable as Var;

/// A variable with a name and an (already expanded) definition.
#[derive(Debug)]
pub struct ExpandedVar<'a> {
	pub name: &'a str,
	pub value: RawString,
}

/// A rule definition with a name and a set of (unexpanded) variables.
#[derive(Debug)]
pub struct Rule<'a> {
	pub name: &'a str,
	pub vars: Vec<Var<'a>>,
}

/// A file-level scope, containing variables and rules.
#[derive(Debug)]
pub struct FileScope<'a: 'p, 'p> {
	/// The scope of the file that subninja'd this file, if any.
	pub parent_scope: Option<&'p FileScope<'a, 'p>>,

	/// The variables defined in this file (and included files).
	///
	/// Can contain duplicates. All definitions are added in order, so lookup
	/// starts at the end.
	pub vars: Vec<ExpandedVar<'a>>,

	/// The rules defined in this file (and included files).
	pub rules: Vec<Rule<'a>>,
}

/// The scope which includes the `build` variables, but not the `rule`
/// variables.
///
/// The input and output paths are expanded using this scope.
#[derive(Debug)]
pub struct BuildScope<'a> {
	/// The file scope.
	pub file_scope: &'a FileScope<'a, 'a>,

	/// The variables of the current `build` definition.
	pub build_vars: &'a [ExpandedVar<'a>],
}

/// The scope which includes both the `build` and the `rule` variables, and
/// `$in`, `$in_newline` and `$out`.
///
/// The built-in variables (`$command`, `$description`, etc.) are looked up in
/// this scope.
#[derive(Debug)]
pub struct BuildRuleScope<'a> {
	/// The file and `build` definition scope.
	pub build_scope: &'a BuildScope<'a>,

	/// The variables of the `rule`.
	pub rule_vars: &'a [Var<'a>],

	/// The list of inputs used for `$in` and `$in_newline`.
	pub inputs: &'a [RawString],

	/// The list of outputs used for `$out`.
	pub outputs: &'a [RawString],
}

/// The result of looking a variale up in a `VarScope`.
pub enum FoundVar<'a> {
	/// The variable is found, and the value was already expanded.
	Expanded(&'a RawStr),

	/// The variable is found, and the value needs to be expanded.
	///
	/// This is the case for variables defined in a `rule` definition.
	Unexpanded(&'a RawStr),

	/// The variable is a special variable (`$in`, `$out`, or `$in_newline`)
	/// containing paths which need to be escaped and separated by either
	/// spaces or newlines.
	Paths {
		paths: &'a [RawString],
		newlines: bool,
	},
}

/// A scope containing variable definitions.
pub trait VarScope {
	/// Look up a variable definition.
	fn lookup_var(&self, var_name: &str) -> Option<FoundVar>;
}

impl<'a> VarScope for [Var<'a>] {
	fn lookup_var(&self, var_name: &str) -> Option<FoundVar> {
		self.iter()
			.rfind(|Var { name, .. }| *name == var_name)
			.map(|var| FoundVar::Unexpanded(var.value))
	}
}

impl<'a> VarScope for [ExpandedVar<'a>] {
	fn lookup_var(&self, var_name: &str) -> Option<FoundVar> {
		self.iter()
			.rfind(|ExpandedVar { name, .. }| *name == var_name)
			.map(|var| FoundVar::Expanded(&*var.value))
	}
}

impl<'a, 'p> VarScope for FileScope<'a, 'p> {
	fn lookup_var(&self, var_name: &str) -> Option<FoundVar> {
		self.vars.lookup_var(var_name).or_else(|| {
			self.parent_scope
				.and_then(|parent| parent.lookup_var(var_name))
		})
	}
}

impl<'a> VarScope for BuildScope<'a> {
	fn lookup_var(&self, var_name: &str) -> Option<FoundVar> {
		self.build_vars
			.lookup_var(var_name)
			.or_else(|| self.file_scope.lookup_var(var_name))
	}
}

impl<'a> VarScope for BuildRuleScope<'a> {
	fn lookup_var(&self, var_name: &str) -> Option<FoundVar> {
		match var_name {
			"in" => Some(FoundVar::Paths {
				paths: self.inputs,
				newlines: false,
			}),
			"out" => Some(FoundVar::Paths {
				paths: self.outputs,
				newlines: false,
			}),
			"in_newline" => Some(FoundVar::Paths {
				paths: self.inputs,
				newlines: true,
			}),
			_ => self
				.build_scope
				.build_vars
				.lookup_var(var_name)
				.or_else(|| {
					self.rule_vars
						.lookup_var(var_name)
						.or_else(|| self.build_scope.file_scope.lookup_var(var_name))
				}),
		}
	}
}

impl<'a, 'p> FileScope<'a, 'p> {
	/// Create an empty scope containing no definitions.
	pub fn new() -> Self {
		FileScope {
			parent_scope: None,
			vars: Vec::new(),
			rules: Vec::new(),
		}
	}

	/// Create an empty scope which inherits the parents scope's definitions.
	pub fn new_subscope(&'p self) -> FileScope<'a, 'p> {
		FileScope {
			parent_scope: Some(self),
			vars: Vec::new(),
			rules: Vec::new(),
		}
	}

	/// Look up a rule definition.
	pub fn lookup_rule(&self, rule_name: &str) -> Option<&Rule<'a>> {
		self.rules
			.iter()
			.rfind(|Rule { name, .. }| *name == rule_name)
			.or_else(|| {
				self.parent_scope
					.and_then(|parent| parent.lookup_rule(rule_name))
			})
	}
}
