#[derive(Debug)]
pub struct Var<'a> {
	pub name: &'a str,
	pub value: &'a str,
}

#[derive(Debug)]
pub struct Rule<'a> {
	pub name: &'a str,
	pub vars: Vec<Var<'a>>,
}

#[derive(Debug)]
pub struct Build<'a> {
	pub rule_name: &'a str,
	pub explicit_outputs: Vec<&'a str>,
	pub implicit_outputs: Vec<&'a str>,
	pub explicit_deps: Vec<&'a str>,
	pub implicit_deps: Vec<&'a str>,
	pub order_deps: Vec<&'a str>,
	pub vars: Vec<Var<'a>>,
}
