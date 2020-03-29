use super::error::ParseError;
use super::expand::check_escapes;
use raw_string::RawStr;
use std::str::from_utf8_unchecked;

// Eats whitespace. Returns the amount of space eaten.
pub fn eat_whitespace(src: &mut &RawStr) -> usize {
	let n = src.bytes().position(|c| c != b' ').unwrap_or(src.len());
	*src = &src[n..];
	n
}

pub fn is_identifier_char(c: u8) -> bool {
	c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b'.'
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
	let mut len = 0;
	loop {
		match memchr::memchr3(b' ', b':', b'|', &src.as_bytes()[len..]) {
			Some(n) if n > 0 && src[len + n] == b' ' && src[len + n - 1] == b'\n' => {
				// Whitespace at the beginning of a line. Skip it.
				len += n + 1;
				match src[len..].bytes().position(|c| c != b' ') {
					Some(n_whitespace) => len += n_whitespace,
					None => break,
				}
			}
			Some(n) if n > 0 && src[len + n - 1] == b'$' => {
				// Escaped character. Continue.
				len += n + 1;
			}
			Some(n) => {
				len += n;
				break;
			}
			None => {
				len = src.len();
				break;
			}
		}
	}
	let (path, rest) = src.split_at(len);
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
