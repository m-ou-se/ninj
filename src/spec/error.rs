use std;

#[derive(Debug)]
pub struct Location<'a> {
	pub file: &'a std::path::Path,
	pub line: i32,
}

#[derive(Debug)]
pub struct ErrorWithLocation<T> {
	pub file: String,
	pub line: i32,
	pub error: T,
}

impl<'a> Location<'a> {
	pub fn make_error<E>(&self, error: E) -> ErrorWithLocation<E> {
		ErrorWithLocation {
			file: self.file.to_string_lossy().into_owned(),
			line: self.line,
			error,
		}
	}
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
	pub fn convert<B: From<A>>(self) -> ErrorWithLocation<B> {
		ErrorWithLocation{
			file: self.file,
			line: self.line,
			error: From::from(self.error),
		}
	}
}

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
		write!(f, "{}", match self {
			ExpectedStatement => "Expected `build', `rule', `default', `include', `subninja', or `var = value'",
			ExpectedVarDef => "Expected `var = value'",
			UnexpectedIndent => "Unexpected indent",
			InvalidUtf8 => "Invalid UTF-8 sequence",
			ExpectedPath => "Missing path",
			ExpectedColon => "Missing `:'",
			ExpectedRuleName => "Missing rule name",
			ExpectedEndOfLine => "Garbage at end of line",
			InvalidEscape => "Invalid $-escape (literal `$' is written as `$$')",
		})
	}
}

impl std::error::Error for ParseError {}

impl From<std::str::Utf8Error> for ParseError {
	fn from(_: std::str::Utf8Error) -> ParseError {
		ParseError::InvalidUtf8
	}
}

#[derive(Debug)]
pub struct ExpansionError {
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

#[derive(Debug)]
pub enum ReadError {
	ParseError(ParseError),
	UndefinedRule(String),
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

impl From<ExpansionError> for ReadError {
	fn from(src: ExpansionError) -> ReadError {
		ReadError::ExpansionError(src)
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
