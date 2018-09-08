use super::eat::eat_identifier_str;
use super::error::ExpansionError;
use super::scope::{FoundVar, VarScope};
use std::fmt::Write;

pub fn expand_var<S: VarScope>(var_name: &str, scope: &S) -> Result<String, ExpansionError> {
	let mut s = String::new();
	expand_var_to(var_name, scope, &mut s, None)?;
	Ok(s)
}

pub fn expand_str<S: VarScope>(value: &str, scope: &S) -> Result<String, ExpansionError> {
	let mut s = String::new();
	expand_str_to(value, scope, &mut s, None)?;
	Ok(s)
}

pub fn expand_strs<S: VarScope>(
	values: &[&str],
	scope: &S,
) -> Result<Vec<String>, ExpansionError> {
	let mut vec = Vec::new();
	expand_strs_into(values, scope, &mut vec)?;
	Ok(vec)
}

pub fn expand_strs_into<S: VarScope>(
	values: &[&str],
	scope: &S,
	vec: &mut Vec<String>,
) -> Result<(), ExpansionError> {
	vec.reserve(values.len());
	for value in values {
		vec.push(expand_str(value, scope)?);
	}
	Ok(())
}

fn expand_var_to<S: VarScope>(
	var_name: &str,
	scope: &S,
	result: &mut String,
	prot: Option<&RecursionProtection>,
) -> Result<(), ExpansionError> {
	Ok(match scope.lookup_var(var_name) {
		Some(FoundVar::Expanded(e)) => result.push_str(e),
		Some(FoundVar::Paths(paths)) => {
			for p in paths {
				// TODO: Use proper shell escaping.
				write!(result, "{:?}", p).unwrap();
			}
		}
		Some(FoundVar::Unexpanded(e)) => {
			check_recursion(var_name, prot)?;
			expand_str_to(
				e,
				scope,
				result,
				Some(&RecursionProtection {
					parent: prot,
					var_name,
				}),
			)?;
		}
		None => {}
	})
}

fn expand_str_to<S: VarScope>(
	mut value: &str,
	scope: &S,
	result: &mut String,
	prot: Option<&RecursionProtection>,
) -> Result<(), ExpansionError> {
	while let Some(i) = value.find('$') {
		result.push_str(&value[..i]);
		value = &value[i + 1..];
		if let Some(var) = eat_identifier_str(&mut value) {
			expand_var_to(var, scope, result, prot)?;
		} else {
			let mut chars = value.chars();
			match chars.next() {
				Some('\n') => {
					result.push_str("\n");
					while match chars.clone().next() {
						Some(' ') | Some('\t') => true,
						_ => false,
					} {
						chars.next();
					}
				}
				Some('{') => {
					value = chars.as_str();
					let var = eat_identifier_str(&mut value).unwrap_or("");
					chars = value.chars();
					if chars.next() == Some('}') {
						expand_var_to(var, scope, result, prot)?;
					} else {
						unreachable!("Expanding '${' without '}', but `check_escapes` should have prevented this");
					}
				}
				Some(x) => result.push(x),
				None => (),
			}
			value = chars.as_str();
		}
	}
	result.push_str(value);
	Ok(())
}

struct RecursionProtection<'a> {
	parent: Option<&'a RecursionProtection<'a>>,
	var_name: &'a str,
}

fn check_recursion(var_name: &str, mut prot: Option<&RecursionProtection>) -> Result<(), ExpansionError> {
	let start = prot;
	let mut n = 1;
	while let Some(p) = prot {
		if p.var_name == var_name {
			// Found cycle.
			// Iterate over it again to build up the expansion chain for the error message.
			let mut cycle = Vec::with_capacity(n);
			prot = start;
			while let Some(p) = prot {
				cycle.push(p.var_name.to_string());
				if p.var_name == var_name {
					return Err(ExpansionError { cycle: cycle.into_boxed_slice() });
				}
				prot = p.parent;
			}
			unreachable!();
		}
		prot = p.parent;
		n += 1;
	}
	Ok(())
}
