use super::error::{ErrorWithLocation, ExpansionError, ReadError};
use super::expand::{expand_str, expand_strs, expand_strs_into, expand_var};
use super::{
	BuildRule, BuildRuleCommand, BuildRuleScope, BuildScope, ExpandedVar, Parser, Scope, Spec,
	Statement,
};
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

pub fn read(file_name: &Path) -> Result<Spec, ErrorWithLocation<ReadError>> {
	let mut spec = Spec::new();
	let pile = Pile::new();
	let mut scope = Scope::new();
	read_into(file_name, &pile, &mut spec, &mut scope)?;
	Ok(spec)
}

pub fn read_into<'a: 'p, 'p>(
	file_name: &Path,
	pile: &'a Pile<Vec<u8>>,
	spec: &mut Spec,
	scope: &mut Scope<'a, 'p>,
) -> Result<(), ErrorWithLocation<ReadError>> {
	let source = read_bytes(file_name, &pile);

	let mut parser = Parser::new(file_name, source);

	while let Some(statement) = parser.next()? {
		let make_error = |e: ExpansionError| parser.make_error(e).convert::<ReadError>();
		use self::Statement::*;
		match statement {
			Variable(var) => {
				let value = expand_str(var.value, scope).map_err(make_error)?;
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
				let mut outputs = expand_strs(&build.explicit_outputs, &build_scope).map_err(make_error)?;
				let mut inputs = expand_strs(&build.explicit_deps, &build_scope).map_err(make_error)?;

				let command = if build.rule_name == "phony" {
					BuildRuleCommand::Phony
				} else {
					// Look up the rule in the current scope.
					let rule = scope
						.lookup_rule(build.rule_name)
						.ok_or_else(|| parser.make_error(ReadError::UndefinedRule(build.rule_name.to_string())))?;

					// Bring $in, $out, and the rule variables into scope.
					let build_rule_scope = BuildRuleScope {
						build_scope: &build_scope,
						rule_vars: &rule.vars,
						inputs: &inputs,
						outputs: &outputs,
					};

					// And expand the command and description with it.
					BuildRuleCommand::Command {
						command: expand_var("command", &build_rule_scope).map_err(make_error)?,
						description: expand_var("description", &build_rule_scope).map_err(make_error)?,
					}
				};

				expand_strs_into(&build.implicit_outputs, &build_scope, &mut outputs).map_err(make_error)?;
				expand_strs_into(&build.implicit_deps, &build_scope, &mut inputs).map_err(make_error)?;

				spec.build_rules.push(BuildRule {
					outputs: outputs,
					deps: inputs,
					order_deps: expand_strs(&build.order_deps, &build_scope).map_err(make_error)?,
					command,
				});
			}
			Default { paths } => {
				expand_strs_into(&paths, scope, &mut spec.default_targets).map_err(make_error)?;
			}
			Include { path } => {
				let path = file_name
					.parent()
					.unwrap_or("".as_ref())
					.join(expand_str(path, scope).map_err(make_error)?);
				read_into(&path, &pile, spec, scope)?;
			}
			SubNinja { path } => {
				let path = file_name
					.parent()
					.unwrap_or("".as_ref())
					.join(expand_str(path, scope).map_err(make_error)?);
				let subpile = Pile::new();
				let mut subscope = scope.new_subscope();
				read_into(&path, &subpile, spec, &mut subscope)?;
			}
		}
	}

	Ok(())
}
