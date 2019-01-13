use super::error::ReadError;
use super::expand::{expand_str, expand_strs, expand_strs_into, expand_var};
use super::parse::{Parser, Statement, Variable};
use super::scope::{BuildRuleScope, BuildScope, ExpandedVar, FileScope, Rule, VarScope};
use super::{BuildCommand, BuildRule, DepStyle, Spec};
use crate::error::{AddLocationToError, AddLocationToResult, ErrorWithLocation, Location};
use pile::Pile;
use raw_string::{RawStr, RawString};
use std::borrow::ToOwned;
use std::fs::File;
use std::io::{BufReader, Read};
use std::mem::replace;
use std::path::Path;
use std::str::from_utf8;

fn read_bytes<'a>(file_name: &Path) -> Result<Vec<u8>, ReadError> {
	let mut bytes = Vec::new();
	File::open(file_name)
		.and_then(|f| BufReader::with_capacity(0x10000, f).read_to_end(&mut bytes))
		.map_err(|error| ReadError::IoError {
			file_name: file_name.to_owned(),
			error,
		})?;
	Ok(bytes)
}

/// Read, parse, and resolve rules and variables in a `ninja.build` file.
///
/// Parses the file, including any included and subninja'd files, and resolves
/// all rules and variables, resulting in a `Spec`.
pub fn read(file_name: &Path) -> Result<Spec, ErrorWithLocation<ReadError>> {
	let source = read_bytes(file_name).err_at(Location::UNKNOWN)?;
	read_from(file_name, &source)
}

/// [`read()`], but with the source given directly instead of read from a file.
///
/// Useful for testing and fuzzing.
///
/// `file_name` is used in errors, and to know where to look for `include` and
/// `subninja` files.
pub fn read_from(file_name: &Path, source: &[u8]) -> Result<Spec, ErrorWithLocation<ReadError>> {
	let pile = Pile::new();
	let mut spec = Spec::new();
	let mut scope = FileScope::new();
	let mut pools = vec![("console".to_string(), 1)];
	read_into(
		file_name,
		RawStr::from_bytes(source),
		&pile,
		&mut spec,
		&mut scope,
		&mut pools,
	)?;
	if let Some(var) = scope
		.vars
		.iter_mut()
		.rfind(|var| var.name.as_bytes() == b"builddir")
	{
		spec.build_dir = Some(replace(&mut var.value, RawString::new()));
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
		let loc = parser.location();
		match statement {
			Statement::Variable { name, value } => {
				let value = expand_str(value, scope).err_at(loc)?;
				scope.vars.push(ExpandedVar { name, value })
			}
			Statement::Rule { name } => {
				if scope.rules.iter().any(|rule| rule.name == name) {
					return Err(ReadError::DuplicateRule(name.to_string()).at(loc));
				}
				let mut vars = Vec::new();
				while let Some(var) = parser.next_variable()? {
					if !match var.name {
						"command" | "description" | "depfile" | "deps" | "msvc_deps_prefix" => true,
						"rspfile" | "rspfile_content" | "generator" | "restat" | "pool" => true,
						_ => false,
					} {
						return Err(
							ReadError::UnknownVariable(var.name.to_string()).at(parser.location())
						);
					}
					vars.push(var);
				}
				scope.rules.push(Rule { name, vars })
			}
			Statement::Pool { name } => {
				if pools.iter().any(|(n, _)| n == name) {
					return Err(ReadError::DuplicatePool(name.to_string()).at(loc));
				}
				let mut depth = None;
				while let Some(Variable { name, value }) = parser.next_variable()? {
					let loc = parser.location();
					if name != "depth" {
						return Err(ReadError::UnknownVariable(name.to_string()).at(loc));
					}
					// Expand the value.
					let value = expand_str(value, scope).err_at(loc)?;
					// Parse the value as an u32.
					depth = Some(
						from_utf8(value.as_bytes())
							.ok()
							.and_then(|s| s.parse().ok())
							.ok_or_else(|| ReadError::InvalidPoolDepth.at(loc))?,
					);
				}
				if let Some(depth) = depth {
					pools.push((name.to_owned(), depth));
				} else {
					return Err(ReadError::ExpectedPoolDepth.at(parser.location()));
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
				let mut vars = Vec::new();
				while let Some(Variable { name, value }) = parser.next_variable()? {
					vars.push(ExpandedVar {
						name,
						value: expand_str(value, scope).err_at(parser.location())?,
					});
				}

				// Bring the build variables into scope.
				let build_scope = BuildScope {
					file_scope: &scope,
					build_vars: &vars,
				};

				// And expand the input and output paths with it.
				let mut outputs =
					Vec::with_capacity(explicit_outputs.len() + implicit_outputs.len());
				let mut inputs = Vec::with_capacity(explicit_deps.len() + implicit_deps.len());
				expand_strs_into(&explicit_outputs, &build_scope, &mut outputs).err_at(loc)?;
				expand_strs_into(&explicit_deps, &build_scope, &mut inputs).err_at(loc)?;

				let command = if rule_name == "phony" {
					None
				} else {
					// Look up the rule in the current scope.
					let rule = scope
						.lookup_rule(rule_name)
						.ok_or_else(|| ReadError::UndefinedRule(rule_name.to_string()).at(loc))?;

					// Bring $in, $out, and the rule variables into scope.
					let build_rule_scope = BuildRuleScope {
						build_scope: &build_scope,
						rule_vars: &rule.vars,
						inputs: &inputs,
						outputs: &outputs,
					};

					let expand_var = |name| expand_var(name, &build_rule_scope).err_at(loc);

					// And expand the special variables with it:

					// First the pool, and also look it up:
					let pool = expand_var("pool")?;
					let (pool, pool_depth) = if pool.is_empty() {
						(String::new(), None)
					} else {
						let (n, d) = pools
							.iter()
							.find(|(name, _)| name.as_bytes() == pool.as_bytes())
							.ok_or_else(|| ReadError::UndefinedPool(pool).at(loc))?;
						(n.clone(), Some(*d))
					};

					// And then the rest:
					Some(BuildCommand {
						rule_name: rule_name.to_string(),
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
						pool,
						pool_depth,
					})
				};

				expand_strs_into(&implicit_outputs, &build_scope, &mut outputs).err_at(loc)?;
				expand_strs_into(&implicit_deps, &build_scope, &mut inputs).err_at(loc)?;

				let mut order_deps = expand_strs(&order_deps, &build_scope).err_at(loc)?;

				for path in outputs
					.iter_mut()
					.chain(inputs.iter_mut())
					.chain(order_deps.iter_mut())
				{
					super::canonicalizepath::canonicalize_path_in_place(path);
				}

				spec.build_rules.push(BuildRule {
					outputs,
					inputs,
					order_deps,
					command,
				});
			}
			Statement::Default { paths } => {
				spec.default_targets.reserve(paths.len());
				for p in paths {
					spec.default_targets.push(expand_str(p, scope).err_at(loc)?);
				}
			}
			Statement::Include { path } => {
				let path = expand_str(path, scope).err_at(loc)?;
				let path = path.to_path().err_at(loc)?;
				let source = RawStr::from_bytes(pile.add(read_bytes(&path).err_at(loc)?));
				read_into(
					&file_name.with_file_name(path),
					source,
					pile,
					spec,
					scope,
					pools,
				)?;
			}
			Statement::SubNinja { path } => {
				let path = expand_str(path, scope).err_at(loc)?;
				let path = path.to_path().err_at(loc)?;
				let source = read_bytes(&path).err_at(loc)?;
				read_into(
					&file_name.with_file_name(path),
					RawStr::from_bytes(&source),
					&Pile::new(),
					spec,
					&mut scope.new_subscope(),
					pools,
				)?;
			}
		}
	}

	Ok(())
}
