//! Shared match configuration and byte-substring matching for RX tools.
//!
//! PLAN 4 introduces a `match` option on `read` and `subscribe` that specifies
//! a byte pattern to detect in the incoming RX stream. Matching always happens
//! on raw bytes; `pattern_encoding` controls how the `pattern` string is decoded
//! into the byte needle.
//!
//! This module provides:
//! - `MatchRequest` — the JSON-serialisable request shape
//! - `MatchMode` — only `literal_substring` for now, extensible later
//! - `PatternEncoding` — alias for the encoding used to decode the pattern
//! - `Matcher` — stateful pattern matcher supporting literal, regex, and glob

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::codec;
use crate::tools::helpers::find_subslice;

// ---- Request shape --------------------------------------------------------

/// Match configuration supplied alongside a `read` or `subscribe` request.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MatchRequest {
    /// Pattern string, interpreted according to `config.pattern_encoding`.
    pub pattern: String,
    /// Configuration controlling how the pattern is decoded and matched.
    #[serde(default)]
    pub config: MatchConfig,
}

/// Configuration for how a match pattern is decoded and matched.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MatchConfig {
    /// Matching mode. Only `literal_substring` is supported in this phase.
    #[serde(default = "default_match_mode")]
    pub mode: MatchMode,
    /// Encoding used to decode `pattern` into raw bytes before matching.
    #[serde(default = "default_pattern_encoding")]
    pub pattern_encoding: PatternEncoding,
    /// When set, returned payload includes up to N bytes before the matched
    /// bytes, plus the matched bytes themselves. `match_index` in the result
    /// reflects the byte offset within the returned payload where the matched
    /// bytes start (which equals the number of pre-match context bytes returned).
    /// If fewer than N bytes exist before the match, whatever exists is returned.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub context_amount_of_matched_bytes: Option<usize>,
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            mode: default_match_mode(),
            pattern_encoding: default_pattern_encoding(),
            context_amount_of_matched_bytes: None,
        }
    }
}

/// Supported match modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Literal byte-substring match on raw RX bytes.
    LiteralSubstring,
    /// Regular expression match using the `regex` crate's bytes API.
    /// The pattern is compiled as `regex::bytes::Regex` and matches on
    /// raw bytes (no UTF-8 requirement). Use the standard regex syntax.
    Regex,
    /// Glob pattern match. Lines are split on `\n` and each line is
    /// tested against the glob pattern via `glob::Pattern::matches`.
    /// This is a per-line whole-match: the glob must describe the
    /// entire line. Use `*` and `?` wildcards.
    Glob,
}

fn default_match_mode() -> MatchMode {
    MatchMode::LiteralSubstring
}

/// Pattern encoding — just an alias for the codec `Encoding` type with a
/// different JSON schema name so the MCP tool description is clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PatternEncoding {
    Utf8,
    Hex,
    #[serde(rename = "base64")]
    Base64,
}

fn default_pattern_encoding() -> PatternEncoding {
    PatternEncoding::Utf8
}

impl std::fmt::Display for PatternEncoding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PatternEncoding::Utf8 => f.write_str("utf8"),
            PatternEncoding::Hex => f.write_str("hex"),
            PatternEncoding::Base64 => f.write_str("base64"),
        }
    }
}

impl From<PatternEncoding> for codec::Encoding {
    fn from(pe: PatternEncoding) -> Self {
        match pe {
            PatternEncoding::Utf8 => codec::Encoding::Utf8,
            PatternEncoding::Hex => codec::Encoding::Hex,
            PatternEncoding::Base64 => codec::Encoding::Base64,
        }
    }
}

// ---- Byte matcher ----------------------------------------------------------

/// Stateful byte pattern matcher supporting literal, regex, and glob modes.
///
/// Call [`Matcher::push`] with each incoming chunk. When the concatenated
/// bytes contain a match, returns [`MatchResult::Found`] with the byte offset.
/// Callers are responsible for truncating buffered data that can no longer
/// participate in a future match.
pub enum Matcher {
    /// Literal byte-substring match.
    Literal {
        needle: Vec<u8>,
        /// Rolling buffer mirroring the caller's accumulation, used for
        /// substring search.
        window: Vec<u8>,
        /// Pre-match context byte count for payload shaping.
        context_amount: Option<usize>,
    },
    /// Regular expression match on raw bytes.
    Regex {
        re: regex::bytes::Regex,
        window: Vec<u8>,
        context_amount: Option<usize>,
    },
    /// Glob pattern per-line match. Lines are split on `\n` and each
    /// line is tested against the glob pattern.
    Glob {
        pat: glob::Pattern,
        window: Vec<u8>,
        context_amount: Option<usize>,
    },
}

/// Result of checking for a match after pushing a chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchResult {
    /// No match found yet after processing the latest chunk.
    NoMatch,
    /// Match found at the given byte offset within the total accumulated data.
    Found(usize),
}

impl Matcher {
    /// Create a new literal matcher for the given needle bytes.
    ///
    /// Returns `None` if the needle is empty (empty patterns never match).
    pub fn new_literal(needle: Vec<u8>) -> Option<Self> {
        if needle.is_empty() {
            return None;
        }
        Some(Self::Literal {
            needle,
            window: Vec::new(),
            context_amount: None,
        })
    }

    /// Create a new matcher with a context amount from an existing builder.
    /// Used by `validate_match_request` to set context after construction.
    pub fn with_context(self, context_amount: Option<usize>) -> Self {
        match self {
            Self::Literal { needle, window, .. } => Self::Literal {
                needle,
                window,
                context_amount,
            },
            Self::Regex { re, window, .. } => Self::Regex {
                re,
                window,
                context_amount,
            },
            Self::Glob { pat, window, .. } => Self::Glob {
                pat,
                window,
                context_amount,
            },
        }
    }

    /// Return the configured context amount.
    pub fn context_amount(&self) -> Option<usize> {
        match self {
            Self::Literal { context_amount, .. }
            | Self::Regex { context_amount, .. }
            | Self::Glob { context_amount, .. } => *context_amount,
        }
    }

    /// Return the matched needle length for the last found match.
    /// For literal mode: matches `needle.len()`.
    /// For regex/glob: returns `None` (caller uses match length from stop outcome).
    pub fn needle_len(&self) -> Option<usize> {
        match self {
            Self::Literal { needle, .. } => Some(needle.len()),
            _ => None,
        }
    }

    /// Scan the current window for a match. Does not push new data.
    pub fn check(&self) -> MatchResult {
        match self {
            Self::Literal { needle, window, .. } => {
                find_subslice(window, needle).map_or(MatchResult::NoMatch, MatchResult::Found)
            }
            Self::Regex { re, window, .. } => re
                .find(window)
                .map_or(MatchResult::NoMatch, |m| MatchResult::Found(m.start())),
            Self::Glob { pat, window, .. } => {
                let decoded = String::from_utf8_lossy(window);
                let mut byte_offset: usize = 0;
                for line in decoded.lines() {
                    if pat.matches(line) {
                        return MatchResult::Found(byte_offset);
                    }
                    // +1 for the `\n` separator
                    byte_offset += line.len() + 1;
                }
                MatchResult::NoMatch
            }
        }
    }

    /// Append a chunk to the internal window and check for a match in the
    /// combined data. Returns the byte offset within the total accumulated
    /// buffer where the match starts, or `NoMatch`.
    pub fn push(&mut self, chunk: &[u8]) -> MatchResult {
        match self {
            Self::Literal { window, .. }
            | Self::Regex { window, .. }
            | Self::Glob { window, .. } => {
                window.extend_from_slice(chunk);
            }
        }
        self.check()
    }

    /// Truncate the internal window to keep at most `keep` bytes from the
    /// back. Call after consuming match data to prevent unbounded growth.
    #[cfg_attr(mutants, mutants::skip)]
    pub fn truncate_front(&mut self, keep: usize) {
        let window = match self {
            Self::Literal { window, .. }
            | Self::Regex { window, .. }
            | Self::Glob { window, .. } => window,
        };
        let drop = window.len().saturating_sub(keep);
        if drop > 0 {
            window.drain(..drop);
        }
    }

    /// Current window length.
    pub fn len(&self) -> usize {
        match self {
            Self::Literal { window, .. }
            | Self::Regex { window, .. }
            | Self::Glob { window, .. } => window.len(),
        }
    }

    /// Whether the window is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

// ---- Validation helper ------------------------------------------------------

/// Validate a `MatchRequest`, decode the pattern into raw bytes, and return
/// a `Matcher` ready to use.
pub fn validate_match_request(req: &MatchRequest) -> Result<Matcher, String> {
    let encoding: codec::Encoding = req.config.pattern_encoding.into();
    let decoded = codec::decode(encoding, &req.pattern)
        .map_err(|e| format!("Pattern decoding failed - {e}"))?;
    if decoded.is_empty() {
        return Err("Pattern must not be empty after decoding".into());
    }

    let context = req.config.context_amount_of_matched_bytes;

    match req.config.mode {
        MatchMode::LiteralSubstring => Matcher::new_literal(decoded)
            .map(|m| m.with_context(context))
            .ok_or_else(|| "Pattern must not be empty after decoding".into()),
        MatchMode::Regex => {
            let re = regex::bytes::Regex::new(&String::from_utf8_lossy(&decoded))
                .map_err(|e| format!("Invalid regex pattern: {e}"))?;
            Ok(Matcher::Regex {
                re,
                window: Vec::new(),
                context_amount: context,
            })
        }
        MatchMode::Glob => {
            let pat = glob::Pattern::new(&String::from_utf8_lossy(&decoded))
                .map_err(|e| format!("Invalid glob pattern: {e}"))?;
            Ok(Matcher::Glob {
                pat,
                window: Vec::new(),
                context_amount: context,
            })
        }
    }
}

// ---- Context shaping -------------------------------------------------------

/// Result of shaping a matched payload with pre-match context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShapedMatchPayload {
    /// The shaped bytes: up to `context_amount` bytes before the match plus the
    /// matched bytes.
    pub data: Vec<u8>,
    /// Byte offset within `data` where the matched bytes start.
    /// Equals the number of pre-match context bytes actually returned.
    pub match_index: usize,
    /// The needle length (matched bytes length).
    pub needle_len: usize,
}

/// Shape the accumulated buffer around a match to include pre-match context.
///
/// Given the full `accumulated` buffer, the `match_index` where the needle
/// was found, `needle_len`, and an optional `context_amount`, returns a
/// payload containing up to `context_amount` bytes before the match plus the
/// matched bytes.
///
/// When `context_amount` is `None`, returns the entire accumulated buffer
/// (preserving existing behavior for non-context-aware callers).
///
/// When `context_amount` is `Some(N)`, returns at most N bytes before the match
/// + `needle_len` matched bytes. If fewer than N bytes exist before the match,
///   whatever exists is returned.
///
/// The returned `match_index` is always relative to the returned `data`.
pub fn shape_match_context(
    accumulated: &[u8],
    match_index: usize,
    needle_len: usize,
    context_amount: Option<usize>,
) -> ShapedMatchPayload {
    let Some(n) = context_amount else {
        return ShapedMatchPayload {
            data: accumulated.to_vec(),
            match_index,
            needle_len,
        };
    };

    let pre_start = match_index.saturating_sub(n);
    let match_end = match_index + needle_len;
    let shaped = accumulated[pre_start..match_end].to_vec();
    let new_match_index = match_index - pre_start;

    ShapedMatchPayload {
        data: shaped,
        match_index: new_match_index,
        needle_len,
    }
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_matcher_finds_immediate_match() {
        let mut m = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        assert_eq!(m.push(b"OK>"), MatchResult::Found(0));
    }

    #[test]
    fn byte_matcher_finds_offset_match() {
        let mut m = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        assert_eq!(m.push(b"hell"), MatchResult::NoMatch);
        assert_eq!(m.push(b"O"), MatchResult::NoMatch);
        assert_eq!(m.push(b"K>!"), MatchResult::Found(4));
    }

    #[test]
    fn byte_matcher_rejects_empty_needle() {
        assert!(Matcher::new_literal(Vec::new()).is_none());
    }

    #[test]
    fn byte_matcher_truncate_front_works() {
        let mut m = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        m.push(b"AAAABBB");
        // truncate_front keeps the last 3 bytes from window: "BBB"
        m.truncate_front(3);
        assert_eq!(m.len(), 3);
        // Push more data; "BBBOK>" contains "OK>" at offset 3
        assert_eq!(m.push(b"OK>"), MatchResult::Found(3));
    }

    #[test]
    fn validate_match_request_literal_hex() {
        let req = MatchRequest {
            pattern: "4f4b3e".into(),
            config: MatchConfig {
                mode: MatchMode::LiteralSubstring,
                pattern_encoding: PatternEncoding::Hex,
                context_amount_of_matched_bytes: None,
            },
        };
        let matcher = validate_match_request(&req).unwrap();
        assert_eq!(matcher.needle_len(), Some(3)); // OK> = 3 bytes
    }

    #[test]
    fn validate_match_request_empty_pattern_rejected() {
        let req = MatchRequest {
            pattern: "".into(),
            config: MatchConfig::default(),
        };
        assert!(validate_match_request(&req).is_err());
    }

    #[test]
    fn match_config_default_is_literal_utf8() {
        let cfg = MatchConfig::default();
        assert_eq!(cfg.mode, MatchMode::LiteralSubstring);
        assert_eq!(cfg.pattern_encoding, PatternEncoding::Utf8);
    }

    #[test]
    fn pattern_encoding_display_roundtrips() {
        assert_eq!(PatternEncoding::Utf8.to_string(), "utf8");
        assert_eq!(PatternEncoding::Hex.to_string(), "hex");
        assert_eq!(PatternEncoding::Base64.to_string(), "base64");
    }

    #[test]
    fn byte_matcher_no_match_returns_no_match() {
        let mut m = Matcher::new_literal(b"XYZ".to_vec()).unwrap();
        assert_eq!(m.push(b"ABCDEF"), MatchResult::NoMatch);
        assert_eq!(m.check(), MatchResult::NoMatch);
    }

    #[test]
    fn shape_context_returns_pre_match_bytes_plus_matched() {
        let accumulated = b"prefix___OK>suffix".to_vec();
        let shaped = shape_match_context(&accumulated, 9, 3, Some(4));
        assert_eq!(shaped.data, b"x___OK>");
        assert_eq!(shaped.match_index, 4);
        assert_eq!(shaped.needle_len, 3);
    }

    #[test]
    fn shape_context_fewer_than_n_pre_match_bytes() {
        let accumulated = b"OK>suffix".to_vec();
        let shaped = shape_match_context(&accumulated, 0, 3, Some(128));
        assert_eq!(shaped.data, b"OK>");
        assert_eq!(shaped.match_index, 0);
    }

    #[test]
    fn shape_context_none_returns_full_buffer() {
        let accumulated = b"prefix___OK>suffix".to_vec();
        let shaped = shape_match_context(&accumulated, 9, 3, None);
        assert_eq!(shaped.data, accumulated);
        assert_eq!(shaped.match_index, 9);
    }

    #[test]
    fn shape_context_zero_amount_returns_only_matched_bytes() {
        let accumulated = b"prefix___OK>suffix".to_vec();
        let shaped = shape_match_context(&accumulated, 9, 3, Some(0));
        assert_eq!(shaped.data, b"OK>");
        assert_eq!(shaped.match_index, 0);
    }

    #[test]
    fn shape_context_match_index_remains_relative_to_returned_payload() {
        let accumulated = b"AAAAAAAAAOK>".to_vec();
        let shaped = shape_match_context(&accumulated, 9, 3, Some(5));
        assert_eq!(shaped.data, b"AAAAAOK>");
        assert_eq!(shaped.match_index, 5);
    }

    #[test]
    fn byte_matcher_with_context_stores_amount() {
        let m = Matcher::new_literal(b"OK>".to_vec())
            .map(|m| m.with_context(Some(64)))
            .unwrap();
        assert_eq!(m.context_amount(), Some(64));
    }

    #[test]
    fn byte_matcher_with_context_none_same_as_new() {
        let m = Matcher::new_literal(b"OK>".to_vec())
            .map(|m| m.with_context(None))
            .unwrap();
        assert_eq!(m.context_amount(), None);
    }

    #[test]
    fn byte_matcher_is_empty_on_fresh_instance() {
        let m = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        assert!(m.is_empty());
    }

    #[test]
    fn byte_matcher_is_not_empty_after_push() {
        let mut m = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        m.push(b"hello");
        assert!(!m.is_empty());
    }

    #[test]
    fn validate_match_request_passes_context_through() {
        let req = MatchRequest {
            pattern: "OK>".into(),
            config: MatchConfig {
                mode: MatchMode::LiteralSubstring,
                pattern_encoding: PatternEncoding::Utf8,
                context_amount_of_matched_bytes: Some(128),
            },
        };
        let matcher = validate_match_request(&req).unwrap();
        assert_eq!(matcher.context_amount(), Some(128));
    }
}
