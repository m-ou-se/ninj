use ninj::spec::{BuildRuleCommand, Spec};

pub fn generate_graph(spec: &Spec) {
	println!("digraph G {{");
	println!("rankdir = \"LR\";");
	println!("node [fontsize=10, shape=box, height=0.25]");
	println!("edge [fontsize=10]");
	for (i, rule) in spec.build_rules.iter().enumerate() {
		let label = match &rule.command {
			BuildRuleCommand::Command { rule_name, .. } => rule_name,
			BuildRuleCommand::Phony => "phony",
		};
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
}
