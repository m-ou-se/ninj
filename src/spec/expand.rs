//! `$`-expansion.

use super::eat::{eat_identifier, is_identifier_char};
use super::error::{ExpansionError, InvalidEscape};
use super::scope::{FoundVar, VarScope};
use raw_string::{RawStr, RawString};

/// Check if the given string contains only valid escape sequences.
pub fn check_escapes(src: &RawStr) -> Result<(), InvalidEscape> {
	let mut i = 0;
	while let Some(n) = memchr::memchr(b'$', &src.as_bytes()[i..]) {
		i += n + 1;
		match src.get(i) {
			Some(b'\n') | Some(b' ') | Some(b':') | Some(b'$') => i += 1,
			Some(x) if is_identifier_char(*x) => i += 1,
			Some(b'{') => {
				loop {
					match src.get(i + 1) {
						Some(x) if is_identifier_char(*x) => i += 1,
						Some(b'}') => break,
						_ => return Err(InvalidEscape),
					}
				}
				i += 1;
			}
			_ => return Err(InvalidEscape),
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
pub fn expand_str<T: AsRef<RawStr>, S: VarScope>(
	source: T,
	scope: &S,
) -> Result<RawString, ExpansionError> {
	let mut s = RawString::new();
	expand_str_to(source.as_ref(), scope, &mut s, None)?;
	Ok(s)
}

pub(super) fn expand_strs<S: VarScope>(
	sources: &[&RawStr],
	scope: &S,
) -> Result<Vec<RawString>, ExpansionError> {
	let mut vec = Vec::new();
	expand_strs_into(sources, scope, &mut vec)?;
	Ok(vec)
}

pub(super) fn expand_strs_into<S: VarScope>(
	sources: &[&RawStr],
	scope: &S,
	vec: &mut Vec<RawString>,
) -> Result<(), ExpansionError> {
	vec.reserve(sources.len());
	for source in sources {
		vec.push(expand_str(source, scope)?);
	}
	Ok(())
}

fn is_shell_safe(c: u8) -> bool {
	match c {
		b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' => true,
		b'_' | b'-' | b'+' | b'/' | b'.' => true,
		_ => false,
	}
}

fn write_shell_escaped_to(source: &RawStr, output: &mut RawString) {
	let mut i = 0;
	loop {
		let next_quote = memchr::memchr(b'\'', &source.as_bytes()[i..]);
		let part = &source[i..i + next_quote.unwrap_or(source.len() - i)];
		if part.bytes().all(is_shell_safe) {
			output.push_str(part);
		} else {
			output.push(b'\'');
			output.push_str(part);
			output.push(b'\'');
		}
		if let Some(next_quote) = next_quote {
			output.push_str("\\\'");
			i += next_quote + 1;
		} else {
			break;
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
		Some(FoundVar::Paths { paths, newlines }) => {
			for (i, p) in paths.iter().enumerate() {
				if !newlines && i > 0 {
					result.push(b' ');
				}
				write_shell_escaped_to(p, result);
				if newlines {
					result.push(b'\n');
				}
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
	mut source: &RawStr,
	scope: &S,
	result: &mut RawString,
	prot: Option<&RecursionProtection>,
) -> Result<(), ExpansionError> {
	while let Some(i) = memchr::memchr(b'$', source.as_bytes()) {
		result.push_str(&source[..i]); // The part before the '$' is used literally
		source = &source[i + 1..]; // Only keep part after the '$' for further processing
		if let Some(var) = eat_identifier(&mut source) {
			// Simple variable: "$var"
			expand_var_to(var, scope, result, prot)?;
		} else if source.starts_with("{") {
			// Braced variable: "${var}"
			let mut s = &source[1..]; // Skip the '{'.
			if let Some(var) = eat_identifier(&mut s) {
				if s.starts_with("}") {
					// Only do the expansion when the matching '}' exists in the right place.
					// (This should already have been checked by `check_escapes`.)
					expand_var_to(var, scope, result, prot)?;
					source = &s[1..]; // Ignore the '}'.
				}
			}
		} else if source.starts_with("\n") {
			// Escaped newline: "$\n"
			source = &source[1..]; // Skip the newline itself first.
			let n = source.bytes().position(|b| b != b' ' && b != b'\t').unwrap_or(source.len());
			source = &source[n..]; // Then skip any the indentation.
		} else if source.starts_with("$") {
			// Escaped dollar sign: "$$"
			source = &source[1..]; // Skip the escaped dollar sign.
			result.push(b'$');
		}
	}
	result.push_str(source);
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

#[test]
pub fn expand_str_test() {
	struct Scope;
	impl VarScope for Scope {
		fn lookup_var(&self, var_name: &str) -> Option<FoundVar> {
			match var_name {
				"world" => Some(FoundVar::Expanded("TEST".as_ref())),
				"WORLD" => Some(FoundVar::Expanded("$TEST".as_ref())),
				"foo" => Some(FoundVar::Unexpanded("blah".as_ref())),
				"bar" => Some(FoundVar::Unexpanded("a $foo b $world $$c".as_ref())),
				"r" => Some(FoundVar::Unexpanded("1 2 3 $r 4 5".as_ref())),
				"r1" => Some(FoundVar::Unexpanded("$r2".as_ref())),
				"r2" => Some(FoundVar::Unexpanded("$r3".as_ref())),
				"r3" => Some(FoundVar::Unexpanded("$r1".as_ref())),
				"in" => Some(FoundVar::Paths {
					paths: Box::leak(Box::new([
						RawString::from("hello"),
						RawString::from("wor ld"),
					])),
					newlines: false,
				}),
				"in_newline" => Some(FoundVar::Paths {
					paths: Box::leak(Box::new([
						RawString::from("he||o"),
						RawString::from("wo'r|d"),
					])),
					newlines: true,
				}),
				_ => None,
			}
		}
	}
	assert_eq!(expand_str("hello $world", &Scope).unwrap(), "hello TEST");
	assert_eq!(expand_str("hello $WORLD", &Scope).unwrap(), "hello $TEST");
	assert_eq!(expand_str("hello $nope", &Scope).unwrap(), "hello ");
	assert_eq!(expand_str("hello ${world} $world$$", &Scope).unwrap(), "hello TEST TEST$");
	assert_eq!(expand_str("$|$|", &Scope).unwrap(), "||");
	assert_eq!(expand_str("foo$\n  bar", &Scope).unwrap(), "foobar");
	assert_eq!(expand_str("$foo$bar", &Scope).unwrap(), "blaha blah b TEST $c");
	assert!(expand_str("$r", &Scope).unwrap_err().cycle.iter().eq(&["r"]));
	assert!(expand_str("$r2", &Scope).unwrap_err().cycle.iter().eq(&["r1", "r3", "r2"]));
	assert_eq!(expand_str("$in", &Scope).unwrap(), "hello 'wor ld'");
	assert_eq!(expand_str("$in_newline", &Scope).unwrap(), "'he||o'\nwo\\\''r|d'\n");
}
