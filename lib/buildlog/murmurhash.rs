use byteorder::{ByteOrder, LE};

const SEED: u64 = 0xdecafbaddecafbad;
const M: u64 = 0xc6a4a7935bd1e995;
const R: u32 = 47;

/// Calculate the 'Murmur 64A' hash.
///
/// This is used as the hash for the commands in the build log.
pub fn murmur_hash_64a(key: &[u8]) -> u64 {
	let mut h = SEED ^ M.wrapping_mul(key.len() as u64);
	let mut iter = key.chunks_exact(8);
	while let Some(part) = iter.next() {
		let k = M.wrapping_mul(LE::read_u64(part));
		h = M.wrapping_mul(h ^ M.wrapping_mul(k ^ k >> R));
	}
	let part = iter.remainder();
	if !part.is_empty() {
		h = M.wrapping_mul(h ^ LE::read_uint(part, part.len()));
	}
	h = M.wrapping_mul(h ^ h >> R);
	h ^ h >> R
}

#[test]
fn test_murmur_hash_64a() {
	assert_eq!(murmur_hash_64a(b""), 0x87c2bc0beaf1d91d);
	assert_eq!(murmur_hash_64a(b"echo hello world"), 0x651507f607a0c6ae);
	assert_eq!(murmur_hash_64a(b"echo This is a test"), 0xe24483e1ba23b555);
}
