//! Errors at a specific line in a file.

use std::error::Error;
use std::fmt;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};

/// A line in a file: The place where something went srong.
///
/// Both fields are optional, in case they are not known.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Location<'a> {
	pub file: Option<&'a Path>,
	pub line: Option<NonZeroU32>,
}

impl Location<'static> {
	/// A [`Location`] with no location information.
	pub const UNKNOWN: Self = Location {
		file: None,
		line: None,
	};
}

/// An error which happened at a specific line in some file.
///
/// Use [`at`][AddLocationToError::at] on an [`Error`], or
/// [`err_at`][AddLocationToResult::err_at] on a [`Result`] to make one.
#[derive(Debug)]
pub struct ErrorWithLocation<T> {
	pub file: Option<PathBuf>,
	pub line: Option<NonZeroU32>,
	pub error: T,
}

impl<'a> Location<'a> {
	/// Create an error containing location information.
	pub fn error<E>(&self, error: E) -> ErrorWithLocation<E> {
		ErrorWithLocation {
			file: self.file.map(|p| p.to_path_buf()),
			line: self.line,
			error,
		}
	}
}

impl<A> ErrorWithLocation<A> {
	/// Convert one error type to another, while keeping the location
	/// information.
	pub fn convert<B: From<A>>(self) -> ErrorWithLocation<B> {
		ErrorWithLocation {
			file: self.file,
			line: self.line,
			error: From::from(self.error),
		}
	}

	/// Get the location at which the error occured.
	pub fn location(&self) -> Location {
		Location {
			file: self.file.as_ref().map(PathBuf::as_path),
			line: self.line,
		}
	}
}

/// Extension trait: Adds [`err_at()`][Self::err_at] to [`Result`].
pub trait AddLocationToResult {
	type WithLocation;
	/// Add location information to the error.
	fn err_at(self, location: Location) -> Self::WithLocation;
}

/// Extension trait: Adds [`at()`][Self::at] to any [`Error`].
pub trait AddLocationToError {
	type WithLocation;
	/// Add location information to the error.
	fn at(self, location: Location) -> Self::WithLocation;
}

impl<T, E> AddLocationToResult for Result<T, E> {
	type WithLocation = Result<T, ErrorWithLocation<E>>;
	fn err_at(self, location: Location) -> Self::WithLocation {
		self.map_err(|e| location.error(e))
	}
}

impl<E: Error> AddLocationToError for E {
	type WithLocation = ErrorWithLocation<E>;
	fn at(self, location: Location) -> Self::WithLocation {
		location.error(self)
	}
}

impl<'a> fmt::Display for Location<'a> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		if let Some(file) = self.file.as_ref() {
			write!(f, "{}", file.display())?;
		}
		if let Some(line) = self.line {
			write!(f, ":{}", line)?;
		}
		Ok(())
	}
}

impl<T: fmt::Display> fmt::Display for ErrorWithLocation<T> {
	fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
		if self.location() != Location::UNKNOWN {
			write!(f, "{}: ", self.location())?;
		}
		write!(f, "{}", self.error)
	}
}

impl<T: Error> Error for ErrorWithLocation<T> {}

impl<T: Error + Send + Sync + 'static> From<ErrorWithLocation<T>> for std::io::Error {
	fn from(src: ErrorWithLocation<T>) -> std::io::Error {
		std::io::Error::new(std::io::ErrorKind::Other, src)
	}
}
