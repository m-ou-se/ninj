use super::expand::{expand_str, expand_strs, expand_var};
use super::{BuildRule, BuildScope, ExpandedVar, Parser, Scope, Spec, Statement, BuildRuleScope};
use pile::Pile;
use std::fs::File;
use std::io::Read;
use std::path::Path;

fn read_bytes<'a>(file_name: &Path, pile: &'a Pile<Vec<u8>>) -> &'a [u8] {
	let mut bytes = Vec::new();
	File::open(file_name)
		.unwrap()
		.read_to_end(&mut bytes)
		.unwrap(); // TODO: error handling
	pile.add(bytes)
}

pub fn read(file_name: &Path) -> Spec {
	let mut spec = Spec::new();
	let pile = Pile::new();
	let mut scope = Scope::new();
	read_into(file_name, &pile, &mut spec, &mut scope);
	spec
}

pub fn read_into<'a: 'p, 'p>(
	file_name: &Path,
	pile: &'a Pile<Vec<u8>>,
	spec: &mut Spec,
	scope: &mut Scope<'a, 'p>,
) {
	let source = read_bytes(file_name, &pile);

	let mut parser = Parser::new(source);

	while let Some(statement) = parser.next() {
		use self::Statement::*;
		match statement {
			Variable(var) => {
				let value = expand_str(var.value, scope);
				scope.vars.push(ExpandedVar {
					name: var.name,
					value,
				})
			}
			Rule(rule) => scope.rules.push(rule),
			Build(build) => {
				if build.rule_name == "phony" {
					//TODO println!("Phony rule ignored!");
				} else {
					let rule = scope.lookup_rule(build.rule_name).unwrap_or_else(|| {
						panic!(
							"Unknown rule {:?} on line {}",
							build.rule_name,
							parser.line_num()
						);
					});

					let scope = BuildScope {
						file_scope: &scope,
						build_vars: &build.vars,
					};

					let outputs = expand_strs(&build.explicit_outputs, &scope);
					let inputs = expand_strs(&build.explicit_deps, &scope);

					let mut build_rule = {
						let scope = BuildRuleScope {
							build_scope: scope,
							rule_vars: &rule.vars,
							inputs: &inputs,
							outputs: &outputs,
						};
						BuildRule {
							explicit_outputs: Vec::new(),
							explicit_deps: Vec::new(),
							implicit_outputs: expand_strs(&build.implicit_outputs, &scope),
							implicit_deps: expand_strs(&build.implicit_deps, &scope),
							order_deps: expand_strs(&build.order_deps, &scope),
							command: expand_var("command", &scope),
							description: expand_var("description", &scope),
						}
					};

					build_rule.explicit_outputs = outputs;
					build_rule.explicit_deps = inputs;

					spec.build_rules.push(build_rule);
				}
			}
			Default { paths } => {
				spec.default_targets
					.extend(paths.iter().map(|s| expand_str(s, scope)));
			}
			Include { path } => {
				let path = file_name.parent().unwrap().join(expand_str(path, scope));
				read_into(&path, &pile, spec, scope);
			}
			SubNinja { path } => {
				let path = file_name.parent().unwrap().join(expand_str(path, scope));
				let subpile = Pile::new();
				let mut subscope = scope.new_subscope();
				read_into(&path, &subpile, spec, &mut subscope);
			}
		}
	}
}
