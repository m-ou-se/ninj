use super::eat::{eat_identifier, eat_whitespace, eat_path, eat_paths};
use super::types::{Build, Rule, Var};
use std::str::from_utf8;

pub struct Parser<'a> {
	source: &'a [u8],
	line_num: i32,
}

// TODO: Use OsStr for paths and values?
#[derive(Debug)]
pub enum Statement<'a> {
	Variable(Var<'a>),
	Rule(Rule<'a>),
	Build(Build<'a>),
	Default { paths: Vec<&'a str> },
	Include { path: &'a str },
	SubNinja { path: &'a str },
}

impl<'a> Parser<'a> {
	pub fn new(source: &'a [u8]) -> Self {
		Parser {
			source,
			line_num: 0,
		}
	}

	pub fn line_num(&self) -> i32 {
		self.line_num
	}

	fn next_line(&mut self) -> Option<&'a [u8]> {
		if self.source.is_empty() {
			return None;
		}

		let line = {
			let line_len = self
				.source
				.iter()
				.position(|&c| c == b'\n' || c == b'#')
				.unwrap_or(self.source.len());
			let (line, rest) = self.source.split_at(line_len);
			self.source = rest;
			line
		};

		{
			let line_end = self
				.source
				.iter()
				.position(|&c| c == b'\n')
				.map_or(self.source.len(), |n| n + 1);
			self.source = &self.source[line_end..];
		}

		self.line_num += 1;
		Some(line)
	}

	// TODO: Values as [u8] ?
	fn parse_vars(&mut self) -> Vec<Var<'a>> {
		let mut vars = Vec::new();
		while eat_whitespace(&mut self.source) > 0 {
			if let Some(mut line) = self.next_line() {
				let name = eat_identifier(&mut line).unwrap();
				eat_whitespace(&mut line);
				if let Some((b'=', mut value)) = line.split_first() {
					eat_whitespace(&mut value);
					let value = from_utf8(value).unwrap();
					vars.push(Var { name, value });
				} else {
					panic!("Expected `=' on line {}.", self.line_num);
				}
			}
		}
		vars
	}

	pub fn next(&mut self) -> Option<Statement<'a>> {
		loop {
			if let Some(mut line) = self.next_line() {
				if line.is_empty() {
					continue;
				}

				let ident = eat_identifier(&mut line).unwrap_or_else(|| {
					panic!("Expected identifier on line {}", self.line_num);
				});

				eat_whitespace(&mut line);

				return Some(match ident {
					"build" => {
						let (explicit_outputs, x) = eat_paths(&mut line, b"|:");
						let (implicit_outputs, x) = if x == Some(b'|') {
							eat_whitespace(&mut line);
							eat_paths(&mut line, b":")
						} else {
							(Vec::new(), x)
						};
						if x != Some(b':') {
							panic!("Missing ':' on line {}", self.line_num)
						}

						eat_whitespace(&mut line);
						let rule_name = eat_identifier(&mut line).unwrap_or_else(|| {
							panic!("Missing rule name on line {}", self.line_num);
						});

						eat_whitespace(&mut line);
						let (explicit_deps, x) = eat_paths(&mut line, b"|");
						let (implicit_deps, x) = if x == Some(b'|') && !line.starts_with(b"|") {
							eat_whitespace(&mut line);
							eat_paths(&mut line, b"|")
						} else {
							(Vec::new(), x)
						};
						let mut order_deps = if x == Some(b'|') && line.starts_with(b"|") {
							line = &line[1..];
							eat_whitespace(&mut line);
							eat_paths(&mut line, b"").0
						} else {
							Vec::new()
						};

						if !line.is_empty() {
							panic!("Unexpected garbage after 'build ...' on line {}", self.line_num);
						}

						Statement::Build(Build {
							rule_name,
							explicit_outputs,
							implicit_outputs,
							explicit_deps,
							implicit_deps,
							order_deps,
							vars: self.parse_vars(),
						})
					}
					"rule" => {
						let name = eat_identifier(&mut line).unwrap();
						if !line.is_empty() {
							panic!(
								"Unexpected garbage after 'rule {}' on line {}",
								name, self.line_num
							);
						}
						Statement::Rule(Rule {
							name,
							vars: self.parse_vars(),
						})
					}
					"include" | "subninja" => {
						let path = eat_path(&mut line).unwrap();
						if !line.is_empty() {
							panic!(
								"Unexpected garbage after '{} {}' on line {}",
								ident, path, self.line_num
							);
						}
						if ident == "include" {
							Statement::Include { path }
						} else {
							Statement::SubNinja { path }
						}
					}
					"default" => Statement::Default {
						// TODO: error handling
						paths: from_utf8(line)
							.unwrap()
							.split(' ')
							.filter(|s| !s.is_empty())
							.collect(),
					},
					var_name => {
						if let Some((b'=', mut value)) = line.split_first() {
							eat_whitespace(&mut value);
							Statement::Variable(Var {
								name: var_name,
								value: from_utf8(value).unwrap(),
							})
						} else {
							panic!("Expected `=' on line {}.", self.line_num);
						}
					}
				});
			} else {
				return None;
			}
		}
	}
}
