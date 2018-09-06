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

pub fn expand_strs<S: VarScope>(value: &[&str], scope: &S) -> Vec<String> {
	value.iter().map(|s| expand_str(s, scope)).collect()
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

fn expand_str_to<S: VarScope>(value: &str, scope: &S, result: &mut String, prot: Option<&RecursionProtection>) {
	for (i, mut part) in value.split('$').enumerate() {
		if i > 0 {
			// Right after a '$'-sign.
			// TODO: Other $-escape sequences
			let var = eat_identifier_str(&mut part).unwrap();
			expand_var_to(var, scope, result, prot);
		}
		result.push_str(part);
	}
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
