use super::Options;
use ninj::spec::read;
use std::io::Error;

pub(super) fn main(opt: &Options) -> Result<(), Error> {
	let spec = read(&opt.file)?;
	println!("digraph BuildGraph {{");
	println!("rankdir = \"LR\";");
	println!("node [fontsize=10, shape=box, height=0.25]");
	println!("edge [fontsize=10]");
	for (i, rule) in spec.build_rules.iter().enumerate() {
		let label = rule.command.as_ref().map_or("phony", |c| &c.rule_name);
		println!("rule{} [label={:?}, shape=ellipse]", i, label);
		for input in &rule.inputs {
			println!("{:?} -> rule{} [arrowhead=none]", input, i);
		}
		for order_dep in &rule.order_deps {
			println!("{:?} -> rule{} [arrowhead=none style=dotted]", order_dep, i);
		}
		for output in &rule.outputs {
			println!("rule{} -> {:?}", i, output);
		}
	}
	println!("}}");
	Ok(())
}
