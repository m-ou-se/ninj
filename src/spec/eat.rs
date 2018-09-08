use std::str::{from_utf8, from_utf8_unchecked};

// Eats whitespace. Returns the amount of space eaten (tabs count for 8).
pub fn eat_whitespace(src: &mut &[u8]) -> i32 {
	let mut n = 0;
	let whitespace_end = src
		.iter()
		.position(|&c| match c {
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

pub fn eat_identifier<'a>(src: &mut &'a [u8]) -> Option<&'a str> {
	let ident_end = src
		.iter()
		.position(|&c| !c.is_ascii_alphanumeric() && c != b'_' && c != b'-')
		.unwrap_or(src.len());
	let (ident, rest) = src.split_at(ident_end);
	*src = rest;
	if ident.is_empty() {
		None
	} else {
		Some(unsafe { from_utf8_unchecked(ident) })
	}
}

pub fn eat_identifier_str<'a>(src: &mut &'a str) -> Option<&'a str> {
	let mut bytes = src.as_bytes();
	let ident = eat_identifier(&mut bytes);
	*src = unsafe { from_utf8_unchecked(bytes) };
	ident
}

pub fn eat_path<'a>(src: &mut &'a [u8]) -> Option<&'a str> {
	let mut escape = false;
	let ident_end = src
		.iter()
		.position(|c| {
			if escape {
				escape = false;
			} else if b" :|".contains(c) {
				return true
			} else if *c == b'$' {
				escape = true;
			}
			false
		}).unwrap_or(src.len());
	let (ident, rest) = src.split_at(ident_end);
	*src = rest;
	if ident.is_empty() {
		None
	} else {
		Some(from_utf8(ident).unwrap())
	}
}

pub fn eat_paths<'a>(src: &mut &'a [u8], endings: &[u8]) -> (Vec<&'a str>, Option<u8>) {
	let mut paths = Vec::new();
	loop {
		if let Some((first, rest)) = src.split_first() {
			if endings.contains(first) {
				*src = rest;
				return (paths, Some(*first));
			}
		} else {
			return (paths, None);
		}
		if let Some(path) = eat_path(src) {
			paths.push(path);
			eat_whitespace(src);
		} else {
			panic!("Expected path.");
		}
	}
}
