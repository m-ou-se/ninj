//! The parser.

use super::eat::{eat_identifier, eat_path, eat_paths, eat_whitespace};
use super::error::{ErrorWithLocation, Location, ParseError};
use super::expand::check_escapes;
use raw_string::RawStr;
use std::path::Path;

/// A `ninja.build` file parser.
pub struct Parser<'a, 'b> {
	file_name: &'b Path,
	source: &'a RawStr,
	line_num: i32,
	escaped_lines: i32,
}

/// A variable definition, with a name and an (unexpanded) definition.
#[derive(Debug)]
pub struct Variable<'a> {
	pub name: &'a str,
	pub value: &'a RawStr,
}

/// A statement in a `build.ninja` file.
#[derive(Debug)]
pub enum Statement<'a> {
	/// A file-level variable definition.
	Variable { name: &'a str, value: &'a RawStr },

	/// A rule definition.
	Rule { name: &'a str },

	/// A build definition.
	Build {
		rule_name: &'a str,
		explicit_outputs: Vec<&'a RawStr>,
		implicit_outputs: Vec<&'a RawStr>,
		explicit_deps: Vec<&'a RawStr>,
		implicit_deps: Vec<&'a RawStr>,
		order_deps: Vec<&'a RawStr>,
	},

	/// A pool definition.
	Pool { name: &'a str },

	/// A default target declaration.
	Default { paths: Vec<&'a RawStr> },

	/// An include statement.
	Include { path: &'a RawStr },

	/// A subninja statement.
	SubNinja { path: &'a RawStr },
}

impl<'a, 'b> Parser<'a, 'b> {
	/// Create a new parser, to parse `source`.
	///
	/// The file name is only used in errors.
	pub fn new(file_name: &'b Path, source: &'a RawStr) -> Self {
		Parser {
			file_name: file_name,
			source: source,
			line_num: 0,
			escaped_lines: 0,
		}
	}

	/// The location of the last read line, statement, or variable.
	///
	/// Used for error reporting.
	pub fn location(&self) -> Location<'b> {
		Location {
			file: self.file_name,
			line: self.line_num,
		}
	}

	/// Moves to the beginning of the next non-comment line, returning the
	/// amount of indentation it has.
	///
	/// Calling `next_line` will then give the line without the indentation.
	fn next_indent(&mut self) -> i32 {
		loop {
			let indent = eat_whitespace(&mut self.source);
			if self.source.starts_with("#") {
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
	fn next_line(&mut self) -> Option<&'a RawStr> {
		self.line_num += self.escaped_lines;
		self.escaped_lines = 0;

		if self.source.is_empty() {
			return None;
		}

		let mut escape = false;
		let (line_end, newline) = match self.source.iter().position(|&c| {
			if escape {
				if c == b'\n' {
					self.escaped_lines += 1;
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

	/// Read an (indented) variable definition.
	///
	/// To be used (repeatedly) right after a `build` or `rule` statement.
	/// Returns `None` when done.
	pub fn next_variable(&mut self) -> Result<Option<Variable<'a>>, ErrorWithLocation<ParseError>> {
		if self.next_indent() > 0 {
			if let Some(mut line) = self.next_line() {
				let name = eat_identifier(&mut line)
					.ok_or_else(|| self.location().make_error(ParseError::ExpectedVarDef))?;
				eat_whitespace(&mut line);
				if let Some((b'=', mut value)) = line.split_first() {
					eat_whitespace(&mut value);
					check_escapes(value).map_err(|e| self.location().make_error(e))?;
					return Ok(Some(Variable { name, value }));
				} else {
					return Err(self.location().make_error(ParseError::ExpectedVarDef));
				}
			}
		}
		Ok(None)
	}

	/// Read the next statement in the file.
	///
	/// Does *not* read the variables underneath a `build` or `rule` statement.
	/// That is a separate step, for which `next_variable` needs to be called
	/// in a loop right after such a statement is read.
	pub fn next_statement(
		&mut self,
	) -> Result<Option<Statement<'a>>, ErrorWithLocation<ParseError>> {
		let mut line = loop {
			if self.next_indent() != 0 {
				return Err(self.location().make_error(ParseError::UnexpectedIndent));
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
			.ok_or_else(|| self.location().make_error(ParseError::ExpectedStatement))?;

		eat_whitespace(&mut line);

		let loc = self.location();

		Ok(Some(match ident {
			"build" => {
				let (explicit_outputs, x) = loc.map_error(eat_paths(&mut line, b"|:"))?;
				let (implicit_outputs, x) = if x == Some(b'|') {
					eat_whitespace(&mut line);
					loc.map_error(eat_paths(&mut line, b":"))?
				} else {
					(Vec::new(), x)
				};

				if x != Some(b':') {
					return Err(loc.make_error(ParseError::ExpectedColon));
				}

				eat_whitespace(&mut line);
				let rule_name = eat_identifier(&mut line)
					.ok_or_else(|| loc.make_error(ParseError::ExpectedRuleName))?;

				eat_whitespace(&mut line);
				let (explicit_deps, x) = loc.map_error(eat_paths(&mut line, b"|"))?;
				let (implicit_deps, x) = if x == Some(b'|') && !line.starts_with("|") {
					eat_whitespace(&mut line);
					loc.map_error(eat_paths(&mut line, b"|"))?
				} else {
					(Vec::new(), x)
				};
				let mut order_deps = if x == Some(b'|') && line.starts_with("|") {
					line = &line[1..];
					eat_whitespace(&mut line);
					loc.map_error(eat_paths(&mut line, b""))?.0
				} else {
					Vec::new()
				};

				if !line.is_empty() {
					return Err(loc.make_error(ParseError::ExpectedEndOfLine));
				}

				Statement::Build {
					rule_name,
					explicit_outputs,
					implicit_outputs,
					explicit_deps,
					implicit_deps,
					order_deps,
				}
			}
			"rule" => {
				let name = eat_identifier(&mut line)
					.ok_or_else(|| loc.make_error(ParseError::ExpectedName))?;
				if !line.is_empty() {
					return Err(loc.make_error(ParseError::ExpectedEndOfLine));
				}
				Statement::Rule { name }
			}
			"pool" => {
				let name = eat_identifier(&mut line)
					.ok_or_else(|| loc.make_error(ParseError::ExpectedName))?;
				if !line.is_empty() {
					return Err(loc.make_error(ParseError::ExpectedEndOfLine));
				}
				Statement::Pool { name }
			}
			"include" | "subninja" => {
				let path = loc.map_error(eat_path(&mut line))?;
				if !line.is_empty() {
					return Err(loc.make_error(ParseError::ExpectedEndOfLine));
				}
				if ident == "include" {
					Statement::Include { path }
				} else {
					Statement::SubNinja { path }
				}
			}
			"default" => {
				let paths = loc.map_error(eat_paths(&mut line, b""))?.0;
				if !line.is_empty() {
					return Err(loc.make_error(ParseError::ExpectedEndOfLine));
				}
				Statement::Default { paths }
			}
			var_name => {
				if let Some((b'=', mut value)) = line.split_first() {
					eat_whitespace(&mut value);
					Statement::Variable {
						name: var_name,
						value,
					}
				} else {
					return Err(loc.make_error(ParseError::ExpectedStatement));
				}
			}
		}))
	}
}
