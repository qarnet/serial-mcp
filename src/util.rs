//! Small shared byte-slice utilities.

/// Find the first occurrence of `needle` in `haystack`. Returns the byte
/// index of the match, or `None` if the needle is empty, the haystack is
/// shorter than the needle, or no match exists.
pub(crate) fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::find_subsequence;

    #[test]
    fn find_subsequence_locates_pattern() {
        assert_eq!(find_subsequence(b"hello OK> world", b"OK>"), Some(6));
        assert_eq!(find_subsequence(b"OK>at-start", b"OK>"), Some(0));
        assert_eq!(find_subsequence(b"trailing OK>", b"OK>"), Some(9));
    }

    #[test]
    fn find_subsequence_missing_returns_none() {
        assert_eq!(find_subsequence(b"hello world", b"OK>"), None);
        assert_eq!(find_subsequence(b"", b"x"), None);
    }

    #[test]
    fn find_subsequence_empty_needle_returns_none() {
        assert_eq!(find_subsequence(b"hello", b""), None);
    }

    #[test]
    fn find_subsequence_needle_longer_than_haystack() {
        assert_eq!(find_subsequence(b"hi", b"hello"), None);
    }
}
