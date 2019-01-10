//! Parsing of Makefile-style dependency files.

use raw_string::{RawStr, RawString};
use std::fs::File;
use std::io::{BufRead, BufReader, Error, ErrorKind, Read};
use std::mem::replace;
use std::path::Path;

/// Read a Makfile-style dependency file.
///
/// `f` is called for every target. The first argument is the target, the
/// second is the list of dependencies.
pub fn read_deps_file(
	file_name: &Path,
	f: impl FnMut(RawString, Vec<RawString>) -> Result<(), Error>,
) -> Result<(), Error> {
	read_deps_file_from(File::open(file_name)?, f)
}

#[derive(Default)]
struct State {
	/// The (incomplete) path we're currently reading.
	path: RawString,
	/// The target, once we've finished reading it.
	target: Option<RawString>,
	/// The rest of the paths we've finished reading.
	deps: Vec<RawString>,
}

impl State {
	fn add_part(&mut self, s: &RawStr) {
		self.path.push_str(s);
	}
	fn finish_path(&mut self) -> Result<(), Error> {
		if !self.path.is_empty() {
			let mut path = replace(&mut self.path, RawString::new());
			if self.target.is_none() && path.last() == Some(b':') {
				path.pop();
				self.target = Some(path);
			} else if self.target.is_none() {
				return Err(Error::new(
					ErrorKind::InvalidData,
					"Rule in dependency file has multiple outputs",
				));
			} else {
				self.deps.push(path);
			}
		}
		Ok(())
	}
	fn finish_deps(
		&mut self,
		f: &mut impl FnMut(RawString, Vec<RawString>) -> Result<(), Error>,
	) -> Result<(), Error> {
		self.finish_path()?;
		if let Some(target) = self.target.take() {
			f(target, replace(&mut self.deps, Vec::new()))?;
		}
		Ok(())
	}
}

fn read_deps_file_from(
	file: impl Read,
	mut f: impl FnMut(RawString, Vec<RawString>) -> Result<(), Error>,
) -> Result<(), Error> {
	let mut file = BufReader::new(file);

	let mut state = State::default();

	let mut line = RawString::new();

	loop {
		line.clear();
		if file.read_until(b'\n', &mut line.as_mut_bytes())? == 0 {
			break;
		}

		if line.last() == Some(b'\n') {
			line.pop();
		}

		if cfg!(windows) && line.last() == Some(b'\r') {
			line.pop();
		}

		let mut write_offset = 0;
		let mut read_offset = 0;

		loop {
			match memchr::memchr2(b' ', b'\\', line[read_offset..].as_bytes())
				.map(|i| i + read_offset)
			{
				Some(i) if line[i] == b'\\' && i + 1 == line.len() => {
					// Backslash at the end of the line
					state.add_part(&line[write_offset..i]);
					state.finish_path()?;
					break;
				}
				Some(i) if line[i] == b'\\' => {
					// Backslash before character.
					let c = line[i + 1];
					match c {
						b' ' | b'\\' | b'#' | b'*' | b'[' | b']' | b'|' => {
							// Escaped character. Drop the '\'.
							state.add_part(&line[write_offset..i]);
							write_offset = i + 1;
						}
						_ => (), // Keep the '\'.
					}
					read_offset = i + 2;
				}
				Some(i) => {
					// A space.
					debug_assert_eq!(line[i], b' ');
					state.add_part(&line[write_offset..i]);
					state.finish_path()?;
					write_offset = i + 1;
					read_offset = i + 1;
				}
				None => {
					// End of the line.
					state.add_part(&line[write_offset..]);
					state.finish_deps(&mut f)?;
					break;
				}
			}
		}
	}

	if state.target.is_none() {
		Ok(())
	} else {
		Err(Error::new(ErrorKind::InvalidData, "Unexpected end of file"))
	}
}

#[cfg(test)]
mod test {
	use super::*;
	use std::io::Cursor;

	fn check(input: &str, mut expected: &[(&str, &[&str])]) {
		let file = Cursor::new(input);
		read_deps_file_from(file, |target, deps| {
			assert_eq!(target, expected[0].0);
			assert!(deps.iter().eq(expected[0].1.iter()));
			expected = &expected[1..];
			Ok(())
		})
		.unwrap();
		assert!(expected.is_empty());
	}

	#[test]
	fn simple() {
		check(
			r#"
output: input input2 input3 \
 input4 input5 \
 input6

output2: input7

output3: input8 \

"#,
			&[
				(
					"output",
					&["input", "input2", "input3", "input4", "input5", "input6"],
				),
				("output2", &["input7"]),
				("output3", &["input8"]),
			],
		);
	}

	#[test]
	fn esacpes() {
		check(
			r#"
bloep\ bloep: a\ b\*c\\d\ab"#,
			&[("bloep bloep", &["a b*c\\d\\ab"])],
		);
	}

	#[test]
	fn colons() {
		check(
			r#"
output: in:put in:put:2:"#,
			&[("output", &["in:put", "in:put:2:"])],
		);
	}

	#[test]
	fn no_deps() {
		check(
			r#"
hello:
world:

test: \

test2:"#,
			&[
				("hello", &[]),
				("world", &[]),
				("test", &[]),
				("test2", &[]),
			],
		);
	}

	#[test]
	fn truncated() {
		let file = Cursor::new(
			r#"
output: input input2 input3 \
 input4 input5 \"#,
		);
		assert!(read_deps_file_from(file, |_, _| Ok(())).is_err());
	}

	#[test]
	fn multiple_outputs() {
		let file = Cursor::new(
			r#"
output output2: input input2 input3 \
 input4 input5 \"#,
		);
		assert!(read_deps_file_from(file, |_, _| Ok(())).is_err());
	}

	#[test]
	fn no_outputs() {
		let file = Cursor::new(
			r#"
a: input input2 input3 \
 input4 input5 \"#,
		);
		assert!(read_deps_file_from(file, |_, _| Ok(())).is_err());
	}
}
