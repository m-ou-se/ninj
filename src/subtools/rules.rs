use super::Options;
use ninj::spec::read;
use std::collections::BTreeSet;
use std::io::Error;

/// Output the list of rule names.
///
/// Unlike the original ninja, this only outputs the names of the rules that
/// are actually used.
pub(super) fn main(opt: &Options) -> Result<(), Error> {
	let spec = read(&opt.file)?;

	let mut rule_names = BTreeSet::new();
	let mut phony = false;

	for rule in spec.build_rules {
		if let Some(command) = rule.command {
			rule_names.insert(command.rule_name);
		} else {
			phony = true;
		}
	}

	if phony {
		rule_names.insert("phony".into());
	}

	for name in rule_names {
		println!("{}", name);
	}

	Ok(())
}
