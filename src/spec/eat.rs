use super::error::ParseError;
use super::expand::check_escapes;
use raw_string::RawStr;
use std::str::from_utf8_unchecked;

// Eats whitespace. Returns the amount of space eaten (tabs count for 8).
pub fn eat_whitespace(src: &mut &RawStr) -> i32 {
	let mut n = 0;
	let whitespace_end = src
		.bytes()
		.position(|c| match c {
			b' ' => {
				n += 1;
				false
			}
			b'\t' => {
				n += 8;
				false
			}
			_ => true,
		}).unwrap_or(src.len());
	*src = &src[whitespace_end..];
	n
}

pub fn is_identifier_char(c: u8) -> bool {
	c.is_ascii_alphanumeric() || c == b'_' || c == b'-'
}

pub fn eat_identifier<'a>(src: &mut &'a RawStr) -> Option<&'a str> {
	let ident_end = src
		.bytes()
		.position(|c| !is_identifier_char(c))
		.unwrap_or(src.len());
	let (ident, rest) = src.split_at(ident_end);
	*src = rest;
	if ident.is_empty() {
		None
	} else {
		Some(unsafe { from_utf8_unchecked(ident.as_bytes()) })
	}
}

pub fn eat_path<'a>(src: &mut &'a RawStr) -> Result<&'a RawStr, ParseError> {
	let mut escape = false;
	let mut newline = false;
	let ident_end = src
		.bytes()
		.position(|c| {
			if newline {
				match c {
					b' ' | b'\t' => return false,
					_ => newline = false,
				}
			}
			if escape {
				if c == b'\n' {
					newline = true;
				}
				escape = false;
			} else if b" :|".contains(&c) {
				return true;
			} else if c == b'$' {
				escape = true;
			}
			false
		}).unwrap_or(src.len());
	let (path, rest) = src.split_at(ident_end);
	*src = rest;
	if path.is_empty() {
		Err(ParseError::ExpectedPath)
	} else {
		check_escapes(path)?;
		Ok(path)
	}
}

pub fn eat_paths<'a>(
	src: &mut &'a RawStr,
	endings: &[u8],
) -> Result<(Vec<&'a RawStr>, Option<u8>), ParseError> {
	let mut paths = Vec::new();
	loop {
		if let Some((first, rest)) = src.split_first() {
			if endings.contains(&first) {
				*src = rest;
				return Ok((paths, Some(first)));
			}
		} else {
			return Ok((paths, None));
		}
		paths.push(eat_path(src)?);
		eat_whitespace(src);
	}
}
