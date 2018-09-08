use raw_string::{RawStr, RawString};

use super::parse::Variable as Var;

#[derive(Debug)]
pub struct ExpandedVar<'a> {
	pub name: &'a str,
	pub value: RawString,
}

#[derive(Debug)]
pub struct Rule<'a> {
	pub name: &'a str,
	pub vars: Vec<Var<'a>>,
}

#[derive(Debug)]
pub struct Scope<'a: 'p, 'p> {
	pub parent_scope: Option<&'p Scope<'a, 'p>>,
	pub vars: Vec<ExpandedVar<'a>>,
	pub rules: Vec<Rule<'a>>,
}

#[derive(Debug)]
pub struct BuildScope<'a> {
	pub file_scope: &'a Scope<'a, 'a>,
	pub build_vars: &'a [ExpandedVar<'a>],
}

#[derive(Debug)]
pub struct BuildRuleScope<'a> {
	pub build_scope: &'a BuildScope<'a>,
	pub rule_vars: &'a [Var<'a>],
	pub inputs: &'a [RawString],
	pub outputs: &'a [RawString],
}

pub enum FoundVar<'a> {
	Expanded(&'a RawStr),
	Unexpanded(&'a RawStr),
	Paths(&'a [RawString]),
}

pub trait VarScope {
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

impl<'a, 'p> VarScope for Scope<'a, 'p> {
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
		if var_name == "in" {
			Some(FoundVar::Paths(self.inputs))
		} else if var_name == "out" {
			Some(FoundVar::Paths(self.outputs))
		} else {
			self.build_scope
				.build_vars
				.lookup_var(var_name)
				.or_else(|| {
					self.rule_vars
						.lookup_var(var_name)
						.or_else(|| self.build_scope.file_scope.lookup_var(var_name))
				})
		}
	}
}

impl<'a, 'p> Scope<'a, 'p> {
	pub fn new() -> Self {
		Scope {
			parent_scope: None,
			vars: Vec::new(),
			rules: Vec::new(),
		}
	}

	pub fn new_subscope(&'p self) -> Scope<'a, 'p> {
		Scope {
			parent_scope: Some(self),
			vars: Vec::new(),
			rules: Vec::new(),
		}
	}

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
