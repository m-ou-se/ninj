use std::fmt::Write;
use super::eat::eat_identifier_str;
use super::scope::{VarScope, FoundVar};

pub fn expand_var<S: VarScope>(var_name: &str, scope: &S) -> String {
	let mut s = String::new();
	expand_var_to(var_name, scope, &mut s, None);
	s
}

pub fn expand_str<S: VarScope>(value: &str, scope: &S) -> String {
	let mut s = String::new();
	expand_str_to(value, scope, &mut s, None);
	s
}

pub fn expand_strs<'a, S: VarScope>(value: &'a [&'a str], scope: &'a S) -> impl Iterator<Item=String> + 'a {
	value.iter().map(move |s| expand_str(s, scope))
}

fn expand_var_to<S: VarScope>(var_name: &str, scope: &S, result: &mut String, prot: Option<&RecursionProtection>) {
	match scope.lookup_var(var_name) {
		Some(FoundVar::Expanded(e)) => result.push_str(e),
		Some(FoundVar::Paths(paths)) => {
			for p in paths {
				write!(result, "{:?}", p).unwrap();
			}
		}
		Some(FoundVar::Unexpanded(e)) => {
			if check_recursion(var_name, prot) {
				panic!("Infinite recursion while expanding `{}'", var_name);
			}
			expand_str_to(e, scope, result, Some(&RecursionProtection {
				parent: prot,
				var_name,
			}));
		}
		None => {}
	}
}

fn expand_str_to<S: VarScope>(mut value: &str, scope: &S, result: &mut String, prot: Option<&RecursionProtection>) {
	while let Some(i) = value.find('$') {
		result.push_str(&value[..i]);
		value = &value[i + 1 ..];
		let mut chars = value.chars();
		match chars.next() {
			Some(':') => {
				result.push_str(":");
			}
			Some(' ') => {
				result.push_str(" ");
			}
			Some('\n') => {
				result.push_str("\n");
				while match chars.clone().next() {
					Some(' ') | Some('\t') => true,
					_ => false,
				} {
					chars.next();
				}
			}
			Some('$') => {
				result.push_str("$");
			}
			Some('{') => {
				value = chars.as_str();
				let var = eat_identifier_str(&mut value).unwrap(); // TODO: error handling
				chars = value.chars();
				if chars.next() != Some('}') {
					panic!("Expected `}'.");
				}
				expand_var_to(var, scope, result, prot);
			}
			_ => {
				let var = eat_identifier_str(&mut value).unwrap(); // TODO: error handling
				chars = value.chars();
				expand_var_to(var, scope, result, prot);
			}
		}
		value = chars.as_str();
	}
	result.push_str(value);
}

struct RecursionProtection<'a> {
	parent: Option<&'a RecursionProtection<'a>>,
	var_name: &'a str,
}

fn check_recursion(var_name: &str, mut prot: Option<&RecursionProtection>) -> bool {
	while let Some(p) = prot {
		if p.var_name == var_name {
			return true;
		}
		prot = p.parent;
	}
	return false;
}
