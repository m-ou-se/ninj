//! Errors that can occur while reading or parsing `build.ninja` files.

use crate::error::ErrorWithLocation;
use raw_string::RawString;
use std::error::Error;
use std::fmt;

/// The error when an invalid `$`-escape sequence is found.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct InvalidEscape;

impl fmt::Display for InvalidEscape {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		ParseError::InvalidEscape.fmt(f)
	}
}

impl Error for InvalidEscape {}

/// A parsing error.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParseError {
	ExpectedStatement,
	ExpectedVarDef,
	UnexpectedIndent,
	ExpectedPath,
	ExpectedColon,
	ExpectedName,
	ExpectedRuleName,
	ExpectedEndOfLine,
	InvalidEscape,
}

impl fmt::Display for ParseError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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

impl Error for ParseError {}

impl From<InvalidEscape> for ParseError {
	fn from(_: InvalidEscape) -> ParseError {
		ParseError::InvalidEscape
	}
}

/// An error while expanding variables: Variable definitions make an infinite
/// cycle.
#[derive(Debug)]
pub struct ExpansionError {
	/// The 'stack trace' of the cycle, containing the variable names.
	///
	/// Starts with the name of the variable that was last expanded before
	/// the cycle was found:
	/// So, for `a -> b -> c -> a`, contains: `["c", "b", "a"]`.
	pub cycle: Box<[String]>,
}

impl fmt::Display for ExpansionError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		write!(f, "Cycle in variable expansion: ")?;
		for var in self.cycle.iter().rev() {
			write!(f, "{} -> ", var)?;
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
	DuplicateRule(String),
	/// A pool with this name was already defined.
	DuplicatePool(String),
	/// The depth value of a `pool` is not a valid value.
	InvalidPoolDepth,
	/// Missing the `depth =` variable in a pool definition.
	ExpectedPoolDepth,
	/// Got a definition of a variable which is not recognized in this (`pool`
	/// or `rule`) definition.
	UnknownVariable(String),
	/// Variable expansion encountered a cycle.
	ExpansionError(ExpansionError),
	/// A problem while trying to open or read a file.
	IoError {
		file_name: std::path::PathBuf,
		error: std::io::Error,
	},
	/// Invalid UTF-8 encoding in path.
	///
	/// This error does not occor on Unix. On Unix, the raw bytes are used in
	/// paths, without any assumed encoding.
	///
	/// On Windows, the paths are converted to UTF-16 before giving them to the
	/// operating system.
	InvalidUtf8 {
		/// The name of the variable in which it went wrong.
		/// Not set for inputs, outputs, include paths, and subninja paths.
		var: Option<String>,
	},
}

impl fmt::Display for ReadError {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		match self {
			ReadError::ParseError(e) => write!(f, "{}", e),
			ReadError::UndefinedRule(n) => write!(f, "Undefined rule name: {}", n),
			ReadError::UndefinedPool(n) => write!(f, "Undefined pool name: {}", n),
			ReadError::DuplicateRule(n) => write!(f, "Duplicate rule: {}", n),
			ReadError::DuplicatePool(n) => write!(f, "Duplicate pool: {}", n),
			ReadError::InvalidPoolDepth => write!(f, "Invalid pool depth"),
			ReadError::ExpectedPoolDepth => write!(f, "Missing `depth =' line"),
			ReadError::UnknownVariable(n) => write!(f, "Unexpected variable: {}", n),
			ReadError::ExpansionError(e) => write!(f, "{}", e),
			ReadError::IoError { file_name, error } => {
				write!(f, "Unable to read {:?}: {}", file_name, error)
			}
			ReadError::InvalidUtf8 { var } => {
				write!(f, "Invalid UTF-8 encoding")?;
				if let Some(var) = var {
					write!(f, " in: {}", var)?;
				}
				Ok(())
			}
		}
	}
}

impl Error for ReadError {
	fn source(&self) -> Option<&(dyn Error + 'static)> {
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

impl From<ExpansionError> for ReadError {
	fn from(src: ExpansionError) -> ReadError {
		ReadError::ExpansionError(src)
	}
}

impl From<std::str::Utf8Error> for ReadError {
	fn from(_: std::str::Utf8Error) -> ReadError {
		ReadError::InvalidUtf8 { var: None }
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
