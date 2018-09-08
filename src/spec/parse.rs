use super::check::check_escapes;
use super::eat::{eat_identifier, eat_path, eat_paths, eat_whitespace};
use super::error::{ErrorWithLocation, ParseError};
use super::types::{Build, Rule, Var};
use std::path::Path;
use std::str::from_utf8;

pub struct Parser<'a, 'b> {
	file_name: &'b Path,
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

impl<'a, 'b> Parser<'a, 'b> {
	pub fn new(file_name: &'b Path, source: &'a [u8]) -> Self {
		Parser {
			file_name,
			source,
			line_num: 0,
		}
	}

	pub fn make_error<E>(&self, error: E) -> ErrorWithLocation<E> {
		ErrorWithLocation {
			file: self.file_name.to_string_lossy().into_owned(),
			line: self.line_num,
			error,
		}
	}

	pub fn map_error<T, E>(&self, result: Result<T, E>) -> Result<T, ErrorWithLocation<E>> {
		result.map_err(|e| self.make_error(e))
	}

	/// Moves to the beginning of the next non-comment line, returning the
	/// amount of indentation it has.
	///
	/// Calling `next_line` will then give the line without the indentation.
	fn next_indent(&mut self) -> i32 {
		loop {
			let indent = eat_whitespace(&mut self.source);
			if self.source.starts_with(b"#") {
				// Ignore comment line.
				let next_line_pos = self
					.source
					.iter()
					.position(|&c| c == b'\n')
					.map_or(self.source.len(), |n| n + 1);
				self.source = &self.source[next_line_pos..];
				self.line_num += 1;
			} else {
				return indent;
			}
		}
	}

	/// Returns the next line, including any $\n escape sequences.
	fn next_line(&mut self) -> Option<&'a [u8]> {
		if self.source.is_empty() {
			return None;
		}

		let mut escape = false;
		let (line_end, newline) = match self.source.iter().position(|&c| {
			if escape {
				if c == b'\n' {
					self.line_num += 1;
				}
				escape = false;
			} else if c == b'\n' {
				return true;
			} else if c == b'$' {
				escape = true;
			}
			false
		}) {
			Some(i) => (i, 1),
			None => (self.source.len(), 0),
		};

		let line = &self.source[..line_end];
		self.source = &self.source[line_end + newline..];
		self.line_num += 1;
		Some(line)
	}

	// TODO: Values as [u8] ?
	fn parse_vars(&mut self) -> Result<Vec<Var<'a>>, ErrorWithLocation<ParseError>> {
		let mut vars = Vec::new();
		while self.next_indent() > 0 {
			if let Some(mut line) = self.next_line() {
				let name = eat_identifier(&mut line)
					.ok_or_else(|| self.make_error(ParseError::ExpectedVarDef))?;
				eat_whitespace(&mut line);
				if let Some((b'=', mut value)) = line.split_first() {
					eat_whitespace(&mut value);
					check_escapes(value).map_err(|e| self.make_error(e))?;
					let value = from_utf8(value).unwrap(); // TODO: error handling
					vars.push(Var { name, value });
				} else {
					return Err(self.make_error(ParseError::ExpectedVarDef));
				}
			}
		}
		Ok(vars)
	}

	pub fn next(&mut self) -> Result<Option<Statement<'a>>, ErrorWithLocation<ParseError>> {
		let mut line = loop {
			if self.next_indent() != 0 {
				return Err(self.make_error(ParseError::UnexpectedIndent));
			}

			let line = match self.next_line() {
				Some(line) => line,
				None => return Ok(None),
			};

			if !line.is_empty() {
				break line;
			}
		};

		let ident = eat_identifier(&mut line)
			.ok_or_else(|| self.make_error(ParseError::ExpectedStatement))?;

		eat_whitespace(&mut line);

		Ok(Some(match ident {
			"build" => {
				let (explicit_outputs, x) = self.map_error(eat_paths(&mut line, b"|:"))?;
				let (implicit_outputs, x) = if x == Some(b'|') {
					eat_whitespace(&mut line);
					self.map_error(eat_paths(&mut line, b":"))?
				} else {
					(Vec::new(), x)
				};

				if x != Some(b':') {
					return Err(self.make_error(ParseError::ExpectedColon));
				}

				eat_whitespace(&mut line);
				let rule_name = eat_identifier(&mut line)
					.ok_or_else(|| self.make_error(ParseError::ExpectedRuleName))?;

				eat_whitespace(&mut line);
				let (explicit_deps, x) = self.map_error(eat_paths(&mut line, b"|"))?;
				let (implicit_deps, x) = if x == Some(b'|') && !line.starts_with(b"|") {
					eat_whitespace(&mut line);
					self.map_error(eat_paths(&mut line, b"|"))?
				} else {
					(Vec::new(), x)
				};
				let mut order_deps = if x == Some(b'|') && line.starts_with(b"|") {
					line = &line[1..];
					eat_whitespace(&mut line);
					self.map_error(eat_paths(&mut line, b""))?.0
				} else {
					Vec::new()
				};

				if !line.is_empty() {
					return Err(self.make_error(ParseError::ExpectedEndOfLine));
				}

				Statement::Build(Build {
					rule_name,
					explicit_outputs,
					implicit_outputs,
					explicit_deps,
					implicit_deps,
					order_deps,
					vars: self.parse_vars()?,
				})
			}
			"rule" => {
				let name = eat_identifier(&mut line)
					.ok_or_else(|| self.make_error(ParseError::ExpectedRuleName))?;
				if !line.is_empty() {
					return Err(self.make_error(ParseError::ExpectedEndOfLine));
				}
				Statement::Rule(Rule {
					name,
					vars: self.parse_vars()?,
				})
			}
			"include" | "subninja" => {
				let path = self.map_error(eat_path(&mut line))?;
				if !line.is_empty() {
					return Err(self.make_error(ParseError::ExpectedEndOfLine));
				}
				if ident == "include" {
					Statement::Include { path }
				} else {
					Statement::SubNinja { path }
				}
			}
			"default" => {
				let paths = self.map_error(eat_paths(&mut line, b""))?.0;
				if !line.is_empty() {
					return Err(self.make_error(ParseError::ExpectedEndOfLine));
				}
				Statement::Default { paths }
			}
			var_name => {
				if let Some((b'=', mut value)) = line.split_first() {
					eat_whitespace(&mut value);
					Statement::Variable(Var {
						name: var_name,
						value: from_utf8(value).unwrap(), // TODO: error handling
					})
				} else {
					return Err(self.make_error(ParseError::ExpectedStatement));
				}
			}
		}))
	}
}
