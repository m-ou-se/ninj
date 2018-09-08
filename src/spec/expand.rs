//! `$`-expansion.

use super::eat::{eat_identifier, is_identifier_char};
use super::error::{InvalidEscape, ExpansionError};
use super::scope::{FoundVar, VarScope};
use raw_string::{RawStr, RawString};

/// Check if the given string contains only valid escape sequences.
pub fn check_escapes(src: &RawStr) -> Result<(), InvalidEscape> {
	let mut iter = src.iter();
	while let Some(&c) = iter.next() {
		if c == b'$' {
			match iter.next() {
				Some(b'\n') => (),
				Some(b' ') => (),
				Some(b':') => (),
				Some(b'$') => (),
				Some(&x) if is_identifier_char(x) => (),
				Some(b'{') => {
					while match iter.next() {
						Some(&x) if is_identifier_char(x) => true,
						Some(b'}') => false,
						_ => return Err(InvalidEscape),
					} {}
				}
				_ => return Err(InvalidEscape),
			}
		}
	}
	Ok(())
}

/// Expand a variable, such as `"description"`.
///
/// Note: Takes the variable name without the `$`.
///
/// Note: Does *not* check if the escape sequences (in any unexpanded
/// variables) are valid. Invalid ones are ignored.
///
/// The parser uses `check_escapes` on all variable definitons it reads,
/// so anything from the parser can be assumed to contain only valid escape sequences.
pub fn expand_var<S: VarScope>(var_name: &str, scope: &S) -> Result<RawString, ExpansionError> {
	let mut s = RawString::new();
	expand_var_to(var_name, scope, &mut s, None)?;
	Ok(s)
}

/// Expand a string containing variables and `$`-escapes.
///
/// Note: Does *not* check if the escape sequences (in both the given string,
/// and in any unexpanded variables in scope) are valid. Invalid ones are
/// ignored.
///
/// Use `check_escapes` to validate the escape sequences.
///
/// The parser uses `check_escapes` on all variable definitons it reads,
/// so anything from the parser can be assumed to contain only valid escape sequences.
pub fn expand_str<T: AsRef<RawStr>, S: VarScope>(value: T, scope: &S) -> Result<RawString, ExpansionError> {
	let mut s = RawString::new();
	expand_str_to(value.as_ref(), scope, &mut s, None)?;
	Ok(s)
}

pub(super) fn expand_strs<S: VarScope>(
	values: &[&RawStr],
	scope: &S,
) -> Result<Vec<RawString>, ExpansionError> {
	let mut vec = Vec::new();
	expand_strs_into(values, scope, &mut vec)?;
	Ok(vec)
}

pub(super) fn expand_strs_into<S: VarScope>(
	values: &[&RawStr],
	scope: &S,
	vec: &mut Vec<RawString>,
) -> Result<(), ExpansionError> {
	vec.reserve(values.len());
	for value in values {
		vec.push(expand_str(value, scope)?);
	}
	Ok(())
}

fn is_shell_safe(c: &u8) -> bool {
	match c {
		b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => true,
		b'_' | b'-' | b'+' | b'/' | b'.' => true,
		_ => false,
	}
}

fn write_shell_escaped_to(value: &RawStr, output: &mut RawString) {
	for (i, part) in value.as_bytes().split(|&b| b == b'\'').enumerate() {
		if i > 0 {
			output.push_str("\\\'");
		}
		if part.iter().all(is_shell_safe) {
			output.push_str(part);
		} else {
			output.push(b'\'');
			output.push_str(part);
			output.push(b'\'');
		}
	}
}

fn expand_var_to<S: VarScope>(
	var_name: &str,
	scope: &S,
	result: &mut RawString,
	prot: Option<&RecursionProtection>,
) -> Result<(), ExpansionError> {
	Ok(match scope.lookup_var(var_name) {
		Some(FoundVar::Expanded(e)) => result.push_str(e),
		Some(FoundVar::Paths(paths)) => {
			for (i, p) in paths.iter().enumerate() {
				if i > 0 {
					result.push(b' ');
				}
				write_shell_escaped_to(p, result);
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
	mut value: &RawStr,
	scope: &S,
	result: &mut RawString,
	prot: Option<&RecursionProtection>,
) -> Result<(), ExpansionError> {
	while let Some(i) = value.as_bytes().iter().position(|&b| b == b'$') {
		result.push_str(&value[..i]);
		value = &value[i + 1..];
		if let Some(var) = eat_identifier(&mut value) {
			expand_var_to(var, scope, result, prot)?;
		} else {
			let mut bytes = value.iter();
			match bytes.next() {
				Some(b'\n') => {
					while match bytes.clone().next() {
						Some(b' ') | Some(b'\t') => true,
						_ => false,
					} {
						bytes.next();
					}
				}
				Some(b'{') => {
					let mut x = RawStr::from_bytes(bytes.as_slice());
					bytes = value.iter();
					if let Some(var) = eat_identifier(&mut x) {
						let mut x = x.iter();
						if x.next() == Some(&b'}') {
							expand_var_to(var, scope, result, prot)?;
							bytes = x;
						}
					}
				}
				Some(&x) => result.push(x),
				None => (),
			}
			value = RawStr::from_bytes(bytes.as_slice());
		}
	}
	result.push_str(value);
	Ok(())
}

struct RecursionProtection<'a> {
	parent: Option<&'a RecursionProtection<'a>>,
	var_name: &'a str,
}

fn check_recursion(
	var_name: &str,
	mut prot: Option<&RecursionProtection>,
) -> Result<(), ExpansionError> {
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
					return Err(ExpansionError {
						cycle: cycle.into_boxed_slice(),
					});
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
