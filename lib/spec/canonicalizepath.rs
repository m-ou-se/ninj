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

	let mut fixed = 0;

	if path[0] == b'/' {
		fixed += 1;
		if cfg!(windows) && path[1..].first() == Some(b'/') {
			// Paths on Windows may start with '//'.
			fixed += 1;
		}
	}

	let mut src = fixed;
	let mut dst = fixed;

	// Invariants:
	// - We still need to process path[src..].
	// - The output is path[..dst].
	// - dst <= src, so the two substrings above don't overlap.
	// - Anything in path[..fixed] is not going to change anymore (only for "/" and
	//   "../" prefixes).
	// - fixed <= dst

	// Copies N bytes from path[src..] to path[..dst], and advances src and dst.
	#[rustfmt::skip]
	macro_rules! copy {
		($n:expr) => {
			let n = $n;
			if src != dst {
				debug_assert!(src + n <= path.len());
				debug_assert!(dst + n <= path.len());
				unsafe {
					std::ptr::copy(path.get_unchecked(src), path.get_unchecked_mut(dst), n);
				}
			}
			src += n;
			dst += n;
		};
	}

	while src < path.len() {
		if path[src] == b'/' {
			// Skip duplicate path separators.
			src += 1;
		} else if path[src..].starts_with("./") || path[src..] == "." {
			// Skip './' components.
			src += 2;
		} else if path[src..].starts_with("../") || path[src..] == ".." {
			if dst > fixed {
				// Remove '../' together with previous component.
				dst = path[..dst - 1]
					.bytes()
					.rposition(|c| c == b'/')
					.map_or(0, |n| n + 1);
				src += 3;
			} else {
				// No previous component. Keep the '../'.
				copy!(if path.len() - src == 2 { 2 } else { 3 });
				if src == path.len() {
					path.truncate(dst);
					return;
				}
				fixed = dst;
			}
		} else {
			copy!(memchr::memchr(b'/', path[src..].as_bytes()).map_or(path.len() - src, |n| n + 1));
			if src == path.len() {
				path.truncate(dst);
				return;
			}
		}
	}

	if dst == 0 {
		path.clear();
		path.push(b'.');
	} else if path[..dst] == "/" {
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
	#[rustfmt::skip]
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
