use super::error::{ErrorWithLocation, ReadError};
use super::expand::{expand_str, expand_strs, expand_strs_into, expand_var};
use super::parse::{Parser, Statement, Variable};
use super::scope::{BuildRuleScope, BuildScope, ExpandedVar, FileScope, Rule, VarScope};
use super::{BuildRule, BuildRuleCommand, DepStyle, Spec};
use pile::Pile;
use raw_string::{RawStr, RawString};
use std::borrow::ToOwned;
use std::fs::File;
use std::io::Read;
use std::mem::replace;
use std::path::{Path, PathBuf};
use std::str::from_utf8;

fn read_bytes<'a>(file_name: &Path, pile: &'a Pile<Vec<u8>>) -> Result<&'a RawStr, ReadError> {
	let mut bytes = Vec::new();
	File::open(file_name)
		.and_then(|mut f| f.read_to_end(&mut bytes))
		.map_err(|error| ReadError::IoError {
			file_name: file_name.to_owned(),
			error,
		})?;
	Ok(RawStr::from_bytes(pile.add(bytes)))
}

/// Read, parse, and resolve rules and variables in a `ninja.build` file.
///
/// Parses the file, including any included and subninja'd files, and resolves
/// all rules and variables, resulting in a `Spec`.
pub fn read(file_name: &Path) -> Result<Spec, ErrorWithLocation<ReadError>> {
	let pile = Pile::new();
	let source = read_bytes(file_name, &pile).map_err(|error| ErrorWithLocation {
		file: String::new(),
		line: 0,
		error,
	})?;
	let mut spec = Spec::new();
	let mut scope = FileScope::new();
	let mut pools = Vec::new();
	read_into(file_name, &source, &pile, &mut spec, &mut scope, &mut pools)?;
	if let Some(var) = scope
		.vars
		.iter_mut()
		.rfind(|var| var.name.as_bytes() == b"builddir")
	{
		spec.build_dir = replace(&mut var.value, RawString::new());
	}
	Ok(spec)
}

fn read_into<'a: 'p, 'p>(
	file_name: &Path,
	source: &'a RawStr,
	pile: &'a Pile<Vec<u8>>,
	spec: &mut Spec,
	scope: &mut FileScope<'a, 'p>,
	pools: &mut Vec<(String, u16)>,
) -> Result<(), ErrorWithLocation<ReadError>> {
	let mut parser = Parser::new(file_name, source);

	while let Some(statement) = parser.next_statement()? {
		match statement {
			Statement::Variable { name, value } => {
				let value = parser.location().map_error(expand_str(value, scope))?;
				scope.vars.push(ExpandedVar { name, value })
			}
			Statement::Rule { name } => {
				if scope.rules.iter().any(|rule| rule.name == name) {
					return Err(parser
						.location()
						.make_error(ReadError::DuplicateRule(name.to_string())));
				}
				let mut vars = Vec::new();
				while let Some(var) = parser.next_variable()? {
					if !match var.name {
						"command" | "description" | "depfile" | "deps" | "msvc_deps_prefix" => true,
						"rspfile" | "rspfile_content" | "generator" | "restat" | "pool" => true,
						_ => false,
					} {
						return Err(parser
							.location()
							.make_error(ReadError::UnknownVariable(var.name.to_string())));
					}
					vars.push(var);
				}
				scope.rules.push(Rule { name, vars })
			}
			Statement::Pool { name } => {
				if pools.iter().any(|(n, _)| n == name) {
					return Err(parser
						.location()
						.make_error(ReadError::DuplicatePool(name.to_string())));
				}
				let mut depth = None;
				while let Some(Variable { name, value }) = parser.next_variable()? {
					let loc = parser.location();
					if name != "depth" {
						return Err(loc.make_error(ReadError::UnknownVariable(name.to_string())));
					}
					// Expand the value.
					let value = loc.map_error(expand_str(value, scope))?;
					// Parse the value as an u32.
					depth = Some(
						from_utf8(value.as_bytes())
							.ok()
							.and_then(|s| s.parse().ok())
							.ok_or_else(|| loc.make_error(ReadError::InvalidPoolDepth))?,
					);
				}
				if let Some(depth) = depth {
					pools.push((name.to_owned(), depth));
				} else {
					return Err(parser.location().make_error(ReadError::ExpectedPoolDepth));
				}
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
					let expand_var_os = |name| {
						expand_var(name).map_err(|e| e.convert()).and_then(|val| {
							val.to_osstring().map_err(|_| {
								location.make_error(ReadError::InvalidUtf8 {
									var: Some(name.to_string()),
								})
							})
						})
					};

					// And expand the special variables with it:

					// First the pool, and also look it up:
					let pool = expand_var("pool")?;
					let (pool, pool_depth) = if pool.is_empty() {
						(String::new(), None)
					} else {
						let (n, d) = pools
							.iter()
							.find(|(name, _)| name.as_bytes() == pool.as_bytes())
							.ok_or_else(|| {
								location.make_error(ReadError::UndefinedPool(pool))
							})?;
						(n.clone(), Some(*d))
					};

					// And then the rest:
					BuildRuleCommand::Command {
						command: expand_var_os("command")?,
						description: expand_var("description")?,
						depfile: PathBuf::from(expand_var_os("depfile")?),
						deps: match expand_var("deps")?.as_bytes() {
							b"gcc" => Some(DepStyle::Gcc),
							b"msvc" => Some(DepStyle::Msvc),
							_ => None,
						},
						msvc_deps_prefix: expand_var("msvc_deps_prefix")?,
						generator: build_rule_scope.lookup_var("generator").is_some(),
						restat: build_rule_scope.lookup_var("restat").is_some(),
						rspfile: PathBuf::from(expand_var_os("rspfile")?),
						rspfile_content: expand_var("rspfile")?,
						pool,
						pool_depth,
					}
				};

				expand_strs_into(&implicit_outputs, &build_scope, &mut outputs)
					.map_err(make_error)?;
				expand_strs_into(&implicit_deps, &build_scope, &mut inputs).map_err(make_error)?;

				let order_deps = expand_strs(&order_deps, &build_scope).map_err(make_error)?;

				let to_paths = |strs: Vec<RawString>| {
					strs.into_iter()
						.map(|s| s.to_pathbuf())
						.collect::<Result<Vec<PathBuf>, _>>()
						.map_err(|e| location.make_error(ReadError::from(e)))
				};

				spec.build_rules.push(BuildRule {
					outputs: to_paths(outputs)?,
					inputs: to_paths(inputs)?,
					order_deps: to_paths(order_deps)?,
					command,
				});
			}
			Statement::Default { paths } => {
				let loc = parser.location();
				spec.default_targets.reserve(paths.len());
				for p in paths {
					spec.default_targets.push(
						loc.map_error(
							expand_str(p, scope)
								.map_err(ReadError::from)
								.and_then(|s| s.to_pathbuf().map_err(ReadError::from)),
						)?,
					);
				}
			}
			Statement::Include { path } => {
				let loc = parser.location();
				let path = loc.map_error(expand_str(path, scope))?;
				let path = loc.map_error(path.to_pathbuf())?;
				let source = loc.map_error(read_bytes(&path, &pile))?;
				read_into(
					&file_name.with_file_name(path),
					&source,
					&pile,
					spec,
					scope,
					pools,
				)?;
			}
			Statement::SubNinja { path } => {
				let loc = parser.location();
				let path = loc.map_error(expand_str(path, scope))?;
				let path = loc.map_error(path.to_pathbuf())?;
				let subpile = Pile::new();
				let source = loc.map_error(read_bytes(&path, &subpile))?;
				let mut subscope = scope.new_subscope();
				read_into(
					&file_name.with_file_name(path),
					&source,
					&subpile,
					spec,
					&mut subscope,
					pools,
				)?;
			}
		}
	}

	Ok(())
}
