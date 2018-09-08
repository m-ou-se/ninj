use super::error::{ErrorWithLocation, ReadError};
use super::expand::{expand_str, expand_strs, expand_strs_into, expand_var};
use super::parse::{Parser, Statement, Variable};
use super::path::to_path;
use super::scope::{BuildRuleScope, BuildScope, ExpandedVar, Rule, FileScope};
use super::{BuildRule, BuildRuleCommand, Spec};
use pile::Pile;
use raw_string::RawStr;
use std::fs::File;
use std::io::Read;
use std::path::Path;

fn read_bytes<'a>(file_name: &Path, pile: &'a Pile<Vec<u8>>) -> &'a RawStr {
	let mut bytes = Vec::new();
	File::open(file_name)
		.unwrap()
		.read_to_end(&mut bytes)
		.unwrap(); // TODO: error handling
	RawStr::from_bytes(pile.add(bytes))
}

/// Read, parse, and resolve rules and variables in a `ninja.build` file.
///
/// Parses the file, including any included and subninja'd files, and resolves
/// all rules and variables, resulting in a `Spec`.
pub fn read(file_name: &Path) -> Result<Spec, ErrorWithLocation<ReadError>> {
	let mut spec = Spec::new();
	let pile = Pile::new();
	let mut scope = FileScope::new();
	read_into(file_name, &pile, &mut spec, &mut scope)?;
	Ok(spec)
}

fn read_into<'a: 'p, 'p>(
	file_name: &Path,
	pile: &'a Pile<Vec<u8>>,
	spec: &mut Spec,
	scope: &mut FileScope<'a, 'p>,
) -> Result<(), ErrorWithLocation<ReadError>> {
	let source = read_bytes(file_name, &pile);

	let mut parser = Parser::new(file_name, source);

	while let Some(statement) = parser.next_statement()? {
		match statement {
			Statement::Variable { name, value } => {
				let value = parser.location().map_error(expand_str(value, scope))?;
				scope.vars.push(ExpandedVar { name, value })
			}
			Statement::Rule { name } => {
				let mut vars = Vec::new();
				while let Some(var) = parser.next_variable()? {
					vars.push(var);
				}
				scope.rules.push(Rule { name, vars })
			}
			Statement::Build {
				rule_name,
				explicit_outputs,
				implicit_outputs,
				explicit_deps,
				implicit_deps,
				order_deps,
			} => {
				let location = parser.location();

				let mut vars = Vec::new();
				while let Some(Variable { name, value }) = parser.next_variable()? {
					vars.push(ExpandedVar {
						name,
						value: parser.location().map_error(expand_str(value, scope))?,
					});
				}

				// Bring the build variables into scope.
				let build_scope = BuildScope {
					file_scope: &scope,
					build_vars: &vars,
				};

				let make_error = |e| location.make_error(e);

				// And expand the input and output paths with it.
				let mut outputs =
					Vec::with_capacity(explicit_outputs.len() + implicit_outputs.len());
				let mut inputs = Vec::with_capacity(explicit_deps.len() + implicit_deps.len());
				expand_strs_into(&explicit_outputs, &build_scope, &mut outputs)
					.map_err(make_error)?;
				expand_strs_into(&explicit_deps, &build_scope, &mut inputs).map_err(make_error)?;

				let command = if rule_name == "phony" {
					BuildRuleCommand::Phony
				} else {
					// Look up the rule in the current scope.
					let rule = scope.lookup_rule(rule_name).ok_or_else(|| {
						location.make_error(ReadError::UndefinedRule(rule_name.to_string()))
					})?;

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
						description: expand_var("description", &build_rule_scope)
							.map_err(make_error)?,
					}
				};

				expand_strs_into(&implicit_outputs, &build_scope, &mut outputs)
					.map_err(make_error)?;
				expand_strs_into(&implicit_deps, &build_scope, &mut inputs).map_err(make_error)?;

				let order_deps = expand_strs(&order_deps, &build_scope).map_err(make_error)?;

				spec.build_rules.push(BuildRule {
					outputs,
					inputs,
					order_deps,
					command,
				});
			}
			Statement::Default { paths } => {
				parser.location().map_error(expand_strs_into(
					&paths,
					scope,
					&mut spec.default_targets,
				))?;
			}
			Statement::Include { path } => {
				let path = parser.location().map_error(expand_str(path, scope))?;
				let path = parser.location().map_error(to_path(&path))?;
				read_into(&file_name.with_file_name(path), &pile, spec, scope)?;
			}
			Statement::SubNinja { path } => {
				let path = parser.location().map_error(expand_str(path, scope))?;
				let path = parser.location().map_error(to_path(&path))?;
				let subpile = Pile::new();
				let mut subscope = scope.new_subscope();
				read_into(
					&file_name.with_file_name(path),
					&subpile,
					spec,
					&mut subscope,
				)?;
			}
		}
	}

	Ok(())
}
