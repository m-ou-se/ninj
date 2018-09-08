use super::eat::is_identifier_char;
use super::error::ParseError;

pub fn check_escapes(src: &[u8]) -> Result<(), ParseError> {
	let mut iter = src.iter();
	while let Some(&c) = iter.next() {
		if c == b'$' {
			match iter.next() {
				Some(b'\n') => (),
				Some(b' ') => (),
				Some(b':') => (),
				Some(b'$') => (),
				Some(&x) if is_identifier_char(x) => (),
				Some(b'{') => {
					while match iter.next() {
						Some(&x) if is_identifier_char(x) => true,
						Some(b'}') => false,
						_ => return Err(ParseError::InvalidEscape),
					} {}
				}
				_ => return Err(ParseError::InvalidEscape),
			}
		}
	}
	Ok(())
}
