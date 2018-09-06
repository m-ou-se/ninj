mod eat;
mod expand;
mod parse;
mod read;
mod scope;
mod types;

pub use self::parse::{Parser, Statement};
pub use self::read::{read, read_into};
pub use self::scope::{Scope, BuildScope, BuildRuleScope, ExpandedVar, VarScope};
pub use self::types::{Build, Rule, Var};

#[derive(Debug)]
pub struct Spec {
	build_rules: Vec<BuildRule>,
	default_targets: Vec<String>,
}

#[derive(Debug)]
pub struct BuildRule {
	command: String,
	description: String,
	explicit_outputs: Vec<String>,
	implicit_outputs: Vec<String>,
	explicit_deps: Vec<String>,
	implicit_deps: Vec<String>,
	order_deps: Vec<String>,
}

impl Spec {
	pub fn new() -> Self {
		Spec {
			build_rules: Vec::new(),
			default_targets: Vec::new(),
		}
	}
}

