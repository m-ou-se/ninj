use raw_string::RawString;

pub fn canonicalize_path_in_place(path: &mut RawString) {
	if path.is_empty() {
		return;
	}

	if cfg!(windows) {
		for b in path.as_mut_bytes() {
			if *b == b'\\' {
				*b = b'/';
			}
		}
	}

	let mut src = 0;
	let mut dst = 0;

	// Invariants:
	// - We still need to process path[src..].
	// - The output is path[..dst].
	// - dst <= src, so the two substrings above don't overlap.
	// - Anything in path[..fixed] is not going to change anymore (only for "/" and "../" prefixes).
	// - fixed <= dst

	if path[0] == b'/' {
		src += 1;
		dst += 1;
		if cfg!(windows) && path.len() > 1 && path[1] == b'/' {
			src += 1;
			dst += 1;
		}
	}

	let mut fixed = dst;

	while src < path.len() {
		if path[src] == b'/' {
			// Skip duplicate path separators.
			src += 1;
			continue;
		} else if path[src] == b'.' {
			if src + 1 >= path.len() || path[src + 1] == b'/' {
				// Remove './' component.
				src += 2;
				continue;
			} else if path[src + 1] == b'.' && (src + 2 >= path.len() || path[src + 2] == b'/') {
				// Remove '../' together with previous component.
				if dst > fixed {
					dst = path[..dst - 1].bytes().rposition(|c| c == b'/').map_or(0, |n| n + 1);
					src += 3;
					continue;
				}
				// No previous component. Keep the '../'.
				path[dst] = path[src];
				dst += 1;
				src += 1;
				path[dst] = path[src];
				dst += 1;
				src += 1;
				if src == path.len() {
					path.truncate(dst);
					return;
				}
				path[dst] = path[src];
				dst += 1;
				src += 1;
				fixed = dst;
				continue;
			}
		}
		path[dst] = path[src];
		dst += 1;
		src += 1;
		loop {
			if src >= path.len() {
				path.truncate(dst);
				return;
			}
			path[dst] = path[src];
			dst += 1;
			src += 1;
			if path[src - 1] == b'/' {
				break;
			}
		}
	}

	if dst == 0 {
		path.clear();
		path.push(b'.');
	} else if dst == 1 && path[0] == b'/' {
		path.truncate(1);
	} else {
		path.truncate(dst - 1);
	}
}

#[cfg(test)]
mod test {
	use super::*;

	fn canonicalize_path_str(path: String) -> String {
		let mut path = RawString::from_bytes(path.into_bytes());
		canonicalize_path_in_place(&mut path);
		unsafe {
			// Canonicalize_path_in_place only removes '.' and '/' characters (or
			// replace the entire string by "."), so if valid UTF-8 goes in, valid
			// UTF-8 comes out.
			String::from_utf8_unchecked(path.into_bytes())
		}
	}

	#[test]
	fn test_canonicalize_path() {
		assert_eq!(canonicalize_path_str("".to_string()), "");
		assert_eq!(canonicalize_path_str("hello".to_string()), "hello");
		assert_eq!(canonicalize_path_str("./hello".to_string()), "hello");
		assert_eq!(canonicalize_path_str("./a".to_string()), "a");
		assert_eq!(canonicalize_path_str("foo/bar/baz".to_string()), "foo/bar/baz");
		assert_eq!(canonicalize_path_str("foo/./bar/baz".to_string()), "foo/bar/baz");
		assert_eq!(canonicalize_path_str("foo/bar/baz/.".to_string()), "foo/bar/baz");
		assert_eq!(canonicalize_path_str("foo/bar/baz/./.".to_string()), "foo/bar/baz");
		assert_eq!(canonicalize_path_str("./foo/bar/baz".to_string()), "foo/bar/baz");
		assert_eq!(canonicalize_path_str("/foo/bar/baz".to_string()), "/foo/bar/baz");
		assert_eq!(canonicalize_path_str("/foo/./bar/baz".to_string()), "/foo/bar/baz");
		assert_eq!(canonicalize_path_str("/foo/bar/baz/.".to_string()), "/foo/bar/baz");
		assert_eq!(canonicalize_path_str("/./foo/bar/baz".to_string()), "/foo/bar/baz");
		assert_eq!(canonicalize_path_str("foo/../baz".to_string()), "baz");
		assert_eq!(canonicalize_path_str("foo/.ok".to_string()), "foo/.ok");
		assert_eq!(canonicalize_path_str("./foo/bar/../baz/blah.x".to_string()), "foo/baz/blah.x");
		assert_eq!(canonicalize_path_str(".//foo///bar////..//baz////blah.x".to_string()), "foo/baz/blah.x");
		assert_eq!(canonicalize_path_str("./.".to_string()), ".");
		assert_eq!(canonicalize_path_str("/.".to_string()), "/");
		assert_eq!(canonicalize_path_str("foo/..".to_string()), ".");
		assert_eq!(canonicalize_path_str("/foo/..".to_string()), "/");
		assert_eq!(canonicalize_path_str("/foo/../".to_string()), "/");
		assert_eq!(canonicalize_path_str("../foo/../".to_string()), "..");
		assert_eq!(canonicalize_path_str("../foo/../test".to_string()), "../test");
		assert_eq!(canonicalize_path_str("../test".to_string()), "../test");
		assert_eq!(canonicalize_path_str("../../test".to_string()), "../../test");
		assert_eq!(canonicalize_path_str("./../test".to_string()), "../test");
		assert_eq!(canonicalize_path_str("foo/../../test".to_string()), "../test");
		assert_eq!(canonicalize_path_str("../foo/../..".to_string()), "../..");
		assert_eq!(canonicalize_path_str("../x/a/b/../c/../..".to_string()), "../x");
	}
}
