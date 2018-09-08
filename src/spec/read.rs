use super::error::{ErrorWithLocation, ReadError};
use std::borrow::ToOwned;
use super::expand::{expand_str, expand_strs, expand_strs_into, expand_var};
use super::parse::{Parser, Statement, Variable};
use super::path::to_path;
use super::scope::{BuildRuleScope, BuildScope, ExpandedVar, Rule, FileScope, VarScope};
use super::{BuildRule, BuildRuleCommand, Spec, DepStyle};
use pile::Pile;
use raw_string::RawStr;
use std::fs::File;
use std::io::Read;
use std::path::Path;

fn read_bytes<'a>(file_name: &Path, pile: &'a Pile<Vec<u8>>) -> Result<&'a RawStr, ReadError> {
	let mut bytes = Vec::new();
	File::open(file_name).and_then(|mut f| f.read_to_end(&mut bytes)).map_err(|error| {
		ReadError::IoError { file_name: file_name.to_owned(), error }
	})?;
	Ok(RawStr::from_bytes(pile.add(bytes)))
}

/// Read, parse, and resolve rules and variables in a `ninja.build` file.
///
/// Parses the file, including any included and subninja'd files, and resolves
/// all rules and variables, resulting in a `Spec`.
pub fn read(file_name: &Path) -> Result<Spec, ErrorWithLocation<ReadError>> {
	let pile = Pile::new();
	let source = read_bytes(file_name, &pile).map_err(|error| {
		ErrorWithLocation {
			file: String::new(),
			line: 0,
			error,
		}
	})?;
	let mut spec = Spec::new();
	let mut scope = FileScope::new();
	read_into(file_name, &source, &pile, &mut spec, &mut scope)?;
	Ok(spec)
}

fn read_into<'a: 'p, 'p>(
	file_name: &Path,
	source: &'a RawStr,
	pile: &'a Pile<Vec<u8>>,
	spec: &mut Spec,
	scope: &mut FileScope<'a, 'p>,
) -> Result<(), ErrorWithLocation<ReadError>> {
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

					let expand_var = |name| expand_var(name, &build_rule_scope).map_err(make_error);

					// And expand the special variables with it:
					BuildRuleCommand::Command {
						command: expand_var("command")?,
						description: expand_var("description")?,
						depfile: expand_var("depfile")?,
						deps: match expand_var("deps")?.as_bytes() {
							b"gcc" => Some(DepStyle::Gcc),
							b"msvc" => Some(DepStyle::Msvc),
							_ => None,
						},
						msvc_deps_prefix: expand_var("msvc_deps_prefix")?,
						generator: build_rule_scope.lookup_var("generator").is_some(),
						restat: build_rule_scope.lookup_var("restat").is_some(),
						rspfile: expand_var("rspfile")?,
						rspfile_content: expand_var("rspfile")?,
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
				let loc = parser.location();
				let path = loc.map_error(expand_str(path, scope))?;
				let path = loc.map_error(to_path(&path))?;
				let source = loc.map_error(read_bytes(path, &pile))?;
				read_into(&file_name.with_file_name(path), &source, &pile, spec, scope)?;
			}
			Statement::SubNinja { path } => {
				let loc = parser.location();
				let path = loc.map_error(expand_str(path, scope))?;
				let path = loc.map_error(to_path(&path))?;
				let subpile = Pile::new();
				let source = loc.map_error(read_bytes(path, &subpile))?;
				let mut subscope = scope.new_subscope();
				read_into(
					&file_name.with_file_name(path),
					&source,
					&subpile,
					spec,
					&mut subscope,
				)?;
			}
		}
	}

	Ok(())
}
