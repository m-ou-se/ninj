use raw_string::RawStr;
use std::path::Path;
use std::str::Utf8Error;

#[cfg(unix)]
pub fn to_path(path: &RawStr) -> Result<&Path, Utf8Error> {
	use std::ffi::OsStr;
	use std::os::unix::ffi::OsStrExt;
	Ok(Path::new(OsStr::from_bytes(path.as_bytes())))
}

#[cfg(not(unix))]
pub fn to_path(path: &RawStr) -> Result<&Path, Utf8Error> {
	use std::str::from_utf8;
	Ok(Path::new(from_utf8(path.as_bytes())?))
}
