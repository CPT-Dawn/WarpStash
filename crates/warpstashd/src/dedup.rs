// ═══════════════════════════════════════════════════════════════════════════
// WarpStash Daemon — Content Deduplication
// ═══════════════════════════════════════════════════════════════════════════
//
// Uses BLAKE3 to hash clipboard content. BLAKE3 is:
//   • Faster than SHA-256 (SIMD-accelerated, single-threaded ~1 GiB/s)
//   • Cryptographically secure (256-bit output)
//   • Perfect for dedup: same content → same hash, always
//
// Dedup rules:
//   1. Hash every incoming clipboard payload.
//   2. Compare against the most recent entry's hash in the DB.
//   3. If they match, skip the insert (consecutive duplicate).
//   4. If they differ, proceed with insertion (the DB's UNIQUE constraint
//      on content_hash handles non-consecutive duplicates by replacing).

/// Compute the BLAKE3 hash of arbitrary content, returning a 64-char hex string.
pub fn content_hash(data: &[u8]) -> String {
    blake3::hash(data).to_hex().to_string()
}

/// Returns `true` if the given hash matches the most recent entry — meaning
/// this is a consecutive duplicate and should be skipped.
pub fn is_consecutive_duplicate(hash: &str, most_recent: Option<&str>) -> bool {
    most_recent.is_some_and(|recent| recent == hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_content_same_hash() {
        let a = content_hash(b"hello world");
        let b = content_hash(b"hello world");
        assert_eq!(a, b);
    }

    #[test]
    fn different_content_different_hash() {
        let a = content_hash(b"hello");
        let b = content_hash(b"world");
        assert_ne!(a, b);
    }

    #[test]
    fn dedup_detection() {
        let h = content_hash(b"test");
        assert!(is_consecutive_duplicate(&h, Some(&h)));
        assert!(!is_consecutive_duplicate(&h, None));
        assert!(!is_consecutive_duplicate(&h, Some("other_hash")));
    }

    #[test]
    fn hash_is_hex_64_chars() {
        let h = content_hash(b"data");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
