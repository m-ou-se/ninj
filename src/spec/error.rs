//! Errors.

use std;

/// A line in a file: The place where something went srong.
#[derive(Debug)]
pub struct Location<'a> {
	pub file: &'a std::path::Path,
	pub line: i32,
}

/// An error which happened at a specific line in some file.
#[derive(Debug)]
pub struct ErrorWithLocation<T> {
	pub file: String,
	pub line: i32,
	pub error: T,
}

impl<'a> Location<'a> {
	/// Create an error containing location information.
	pub fn make_error<E>(&self, error: E) -> ErrorWithLocation<E> {
		ErrorWithLocation {
			file: self.file.to_string_lossy().into_owned(),
			line: self.line,
			error,
		}
	}
	/// Add location information to a `Result`, if it contains an error.
	pub fn map_error<T, E>(&self, result: Result<T, E>) -> Result<T, ErrorWithLocation<E>> {
		result.map_err(|e| self.make_error(e))
	}
}

impl<T: std::fmt::Display> std::fmt::Display for ErrorWithLocation<T> {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "{}:{}: {}", self.file, self.line, self.error)
	}
}

impl<T: std::fmt::Display + std::fmt::Debug> std::error::Error for ErrorWithLocation<T> {}

impl<A> ErrorWithLocation<A> {
	/// Convert one error type to another, while keeping the location information.
	pub fn convert<B: From<A>>(self) -> ErrorWithLocation<B> {
		ErrorWithLocation {
			file: self.file,
			line: self.line,
			error: From::from(self.error),
		}
	}
}

/// The error when an invalid `$`-escape sequence is found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidEscape;

impl std::fmt::Display for InvalidEscape {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "Invalid $-escape (literal `$' is written as `$$')")
	}
}

impl std::error::Error for InvalidEscape {}

/// A parsing error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
	ExpectedStatement,
	ExpectedVarDef,
	UnexpectedIndent,
	InvalidUtf8,
	ExpectedPath,
	ExpectedColon,
	ExpectedRuleName,
	ExpectedEndOfLine,
	InvalidEscape,
}

impl std::fmt::Display for ParseError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		use self::ParseError::*;
		write!(
			f,
			"{}",
			match self {
				ExpectedStatement => {
					"Expected `build', `rule', `default', `include', `subninja', or `var = value'"
				}
				ExpectedVarDef => "Expected `var = value'",
				UnexpectedIndent => "Unexpected indent",
				InvalidUtf8 => "Invalid UTF-8 sequence",
				ExpectedPath => "Missing path",
				ExpectedColon => "Missing `:'",
				ExpectedRuleName => "Missing rule name",
				ExpectedEndOfLine => "Garbage at end of line",
				InvalidEscape => "Invalid $-escape (literal `$' is written as `$$')",
			}
		)
	}
}

impl std::error::Error for ParseError {}

impl From<InvalidEscape> for ParseError {
	fn from(_: InvalidEscape) -> ParseError {
		ParseError::InvalidEscape
	}
}

impl From<std::str::Utf8Error> for ParseError {
	fn from(_: std::str::Utf8Error) -> ParseError {
		ParseError::InvalidUtf8
	}
}

/// An error while expanding variables: Variable definitions make an infinite cycle.
#[derive(Debug)]
pub struct ExpansionError {
	/// The 'stack trace' of the cycle, containing the variable names.
	///
	/// Starts with the name of the variable that was last expanded before
	/// the cycle was found:
	/// So, for `a -> b -> c -> a`, contains: `["c", "b", "a"]`.
	pub cycle: Box<[String]>,
}

impl std::fmt::Display for ExpansionError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		write!(f, "Cycle in variable expansion: ")?;
		for var in self.cycle.iter().rev() {
			write!(f, "{} -> ", var)?
		}
		write!(f, "{}", self.cycle[self.cycle.len() - 1])
	}
}

/// An error while reading a `build.ninja` file.
#[derive(Debug)]
pub enum ReadError {
	/// Some syntax error.
	ParseError(ParseError),
	/// A `build` definition refers to a `rule` which doesn't exist.
	UndefinedRule(String),
	/// Variable expansion encountered a cycle.
	ExpansionError(ExpansionError),
}

impl std::fmt::Display for ReadError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			ReadError::ParseError(e) => write!(f, "{}", e),
			ReadError::UndefinedRule(n) => write!(f, "Undefined rule name: {}", n),
			ReadError::ExpansionError(e) => write!(f, "{}", e),
		}
	}
}

impl std::error::Error for ReadError {}

impl From<ParseError> for ReadError {
	fn from(src: ParseError) -> ReadError {
		ReadError::ParseError(src)
	}
}

impl From<std::str::Utf8Error> for ReadError {
	fn from(src: std::str::Utf8Error) -> ReadError {
		ReadError::ParseError(src.into())
	}
}

impl From<ExpansionError> for ReadError {
	fn from(src: ExpansionError) -> ReadError {
		ReadError::ExpansionError(src)
	}
}

impl From<ErrorWithLocation<InvalidEscape>> for ErrorWithLocation<ParseError> {
	fn from(src: ErrorWithLocation<InvalidEscape>) -> Self {
		src.convert()
	}
}

impl From<ErrorWithLocation<ParseError>> for ErrorWithLocation<ReadError> {
	fn from(src: ErrorWithLocation<ParseError>) -> Self {
		src.convert()
	}
}

impl From<ErrorWithLocation<ExpansionError>> for ErrorWithLocation<ReadError> {
	fn from(src: ErrorWithLocation<ExpansionError>) -> Self {
		src.convert()
	}
}

impl From<ErrorWithLocation<std::str::Utf8Error>> for ErrorWithLocation<ReadError> {
	fn from(src: ErrorWithLocation<std::str::Utf8Error>) -> Self {
		src.convert()
	}
}
