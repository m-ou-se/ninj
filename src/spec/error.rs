//! Errors.

use raw_string::RawString;
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
		if !self.file.is_empty() || self.line != 0 {
			write!(f, "{}", self.file)?;
			if self.line != 0 {
				write!(f, ":{}", self.line)?;
			}
			write!(f, ": ")?;
		}
		write!(f, "{}", self.error)
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
	ExpectedName,
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
					"Expected `build', `rule', `pool', `default', `include', `subninja', or `var = value'"
				}
				ExpectedVarDef => "Expected `var = value'",
				UnexpectedIndent => "Unexpected indent",
				InvalidUtf8 => "Invalid UTF-8 sequence",
				ExpectedPath => "Missing path",
				ExpectedColon => "Missing `:'",
				ExpectedName => "Missing name of definition",
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
	/// A `build` definition refers to a `pool` which doesn't exist.
	UndefinedPool(RawString),
	/// A pool with this name was already defined.
	DuplicatePool(String),
	/// The depth value of a `pool` is not a valid value.
	InvalidPoolDepth,
	/// Missing the `depth =` variable in a pool definition.
	ExpectedPoolDepth,
	/// Got a definition of a variable which is not recognized in this (`pool`) definition.
	UnknownVariable(String),
	/// Variable expansion encountered a cycle.
	ExpansionError(ExpansionError),
	/// A problem while trying to open or read a file.
	IoError {
		file_name: std::path::PathBuf,
		error: std::io::Error,
	},
}

impl std::fmt::Display for ReadError {
	fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
		match self {
			ReadError::ParseError(e) => write!(f, "{}", e),
			ReadError::UndefinedRule(n) => write!(f, "Undefined rule name: {}", n),
			ReadError::UndefinedPool(n) => write!(f, "Undefined pool name: {}", n),
			ReadError::DuplicatePool(n) => write!(f, "Duplicate pool: {}", n),
			ReadError::InvalidPoolDepth => write!(f, "Invalid pool depth"),
			ReadError::ExpectedPoolDepth => write!(f, "Missing `depth =' line"),
			ReadError::UnknownVariable(n) => write!(f, "Unexpected variable: {}", n),
			ReadError::ExpansionError(e) => write!(f, "{}", e),
			ReadError::IoError { file_name, error } => {
				write!(f, "Unable to read {:?}: {}", file_name, error)
			}
		}
	}
}

impl std::error::Error for ReadError {
	fn cause(&self) -> Option<&std::error::Error> {
		match self {
			ReadError::IoError { error, .. } => Some(error),
			_ => None,
		}
	}
}

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
