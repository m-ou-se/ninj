mod check;
mod eat;
mod error;
mod expand;
mod parse;
mod read;
mod scope;

pub use self::parse::{Parser, Statement};
pub use self::read::{read, read_into};
pub use self::scope::{Scope, BuildScope, BuildRuleScope, ExpandedVar, VarScope};

#[derive(Debug)]
pub struct Spec {
	build_rules: Vec<BuildRule>,
	default_targets: Vec<String>,
}

#[derive(Debug)]
pub struct BuildRule {
	outputs: Vec<String>,
	deps: Vec<String>,
	order_deps: Vec<String>,
	command: BuildRuleCommand,
}

#[derive(Debug)]
pub enum BuildRuleCommand {
	Phony,
	Command {
		command: String,
		description: String,
	}
}

impl Spec {
	pub fn new() -> Self {
		Spec {
			build_rules: Vec::new(),
			default_targets: Vec::new(),
		}
	}
}
