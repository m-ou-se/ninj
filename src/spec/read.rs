use super::expand::{expand_str, expand_strs, expand_var};
use super::{BuildRule, BuildScope, ExpandedVar, Parser, Scope, Spec, Statement, BuildRuleScope, BuildRuleCommand};
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
				// Bring the build variables into scope.
				let build_scope = BuildScope {
					file_scope: &scope,
					build_vars: &build.vars,
				};

				// And expand the input and output paths with it.
				let mut outputs: Vec<String> = expand_strs(&build.explicit_outputs, &build_scope).collect();
				let mut inputs: Vec<String> = expand_strs(&build.explicit_deps, &build_scope).collect();

				let command = if build.rule_name == "phony" {
					BuildRuleCommand::Phony
				} else {
					// Look up the rule in the current scope.
					let rule = scope.lookup_rule(build.rule_name).unwrap_or_else(|| {
						panic!(
							"Unknown rule {:?} on line {}",
							build.rule_name,
							parser.line_num()
						);
					});

					// Bring $in, $out, and the rule variables into scope.
					let build_rule_scope = BuildRuleScope {
						build_scope: &build_scope,
						rule_vars: &rule.vars,
						inputs: &inputs,
						outputs: &outputs,
					};

					// And expand the command and description with it.
					BuildRuleCommand::Command {
						command: expand_var("command", &build_rule_scope),
						description: expand_var("description", &build_rule_scope),
					}
				};

				outputs.extend(expand_strs(&build.implicit_outputs, &build_scope));
				inputs.extend(expand_strs(&build.implicit_deps, &build_scope));

				spec.build_rules.push(BuildRule {
					outputs: outputs,
					deps: inputs,
					order_deps: expand_strs(&build.order_deps, &build_scope).collect(),
					command,
				});
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
