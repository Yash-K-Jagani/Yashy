//! Content-addressing and memoisation.
//!
//! Everything in the database is keyed by a 64-bit FNV-1a content hash, the
//! same algorithm `yashy-cache` uses for program-level caching, so a hash
//! computed here matches a hash computed in the driver. The hash is a
//! deterministic digest of the data, not a stable identifier across compiler
//! versions: a version change means all cached values are stale and
//! re-computation is the default.

/// FNV-1a 64-bit non-rotating mix. The same algorithm as
/// `crates/yashy-cache` and `crates/yashy-driver/pipeline.rs`: the query engine
/// is allowed to match the driver's cache keys, not to invent a family of its
/// own.
#[inline]
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut state: u64 = 0xcbf29ce484222325;
    for b in bytes {
        state ^= *b as u64;
        state = state.wrapping_mul(0x100000001b3);
    }
    state
}

/// A stable hash of a string.
#[inline]
pub fn hash_str(s: &str) -> u64 {
    let mut state = fnv1a(s.as_bytes());
    state = state.wrapping_mul(0x9e3779b97f4a7c15);
    state
}

/// A stable hash of two hashes, so a list of inputs is a single key. Both
/// operands are folded in order, so `combine(a, b)` and `combine(b, a)` are
/// different keys: a query that read `(a, b)` is not the same query as one
/// that read `(b, a)`.
#[inline]
pub fn combine(left: u64, right: u64) -> u64 {
    let mut state: u64 = 0xcbf29ce484222325;
    for b in left.to_le_bytes() {
        state ^= b as u64;
        state = state.wrapping_mul(0x100000001b3);
    }
    for b in right.to_le_bytes() {
        state ^= b as u64;
        state = state.wrapping_mul(0x100000001b3);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv1a_is_the_canonical_64_bit_algorithm() {
        // Published FNV-1a 64-bit test vectors: pinning them catches an
        // offset-basis or prime typo that would otherwise silently change
        // every cache key in the compiler.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv1a(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn combine_is_order_sensitive() {
        assert_ne!(combine(1, 2), combine(2, 1));
    }

    #[test]
    fn hash_str_is_deterministic() {
        assert_eq!(hash_str("fn main() {}"), hash_str("fn main() {}"));
        assert_ne!(hash_str("a"), hash_str("b"));
    }
}
