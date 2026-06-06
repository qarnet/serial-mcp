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
//! - `ByteMatcher` — stateful byte-substring matcher (finds first occurrence)

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
}

impl Default for MatchConfig {
    fn default() -> Self {
        Self {
            mode: default_match_mode(),
            pattern_encoding: default_pattern_encoding(),
        }
    }
}

/// Supported match modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MatchMode {
    /// Literal byte-substring match on raw RX bytes.
    LiteralSubstring,
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

/// Stateful literal byte-substring matcher.
///
/// Call [`ByteMatcher::push`] with each incoming chunk. When the concatenated
/// bytes contain a match, returns [`MatchResult::Found`] with the byte offset.
/// Callers are responsible for truncating buffered data that can no longer
/// participate in a future match (everything at and before the match start
/// plus the needle length).
pub struct ByteMatcher {
    needle: Vec<u8>,
    /// Rolling buffer that mirrors whatever the caller accumulates, used only
    /// for substring search. The caller owns the authoritative accumulation.
    window: Vec<u8>,
}

/// Result of checking for a match after pushing a chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatchResult {
    /// No match found yet after processing the latest chunk.
    NoMatch,
    /// Match found at the given byte offset within the total accumulated data.
    Found(usize),
}

impl ByteMatcher {
    /// Create a new matcher for the given needle bytes.
    ///
    /// Returns `None` if the needle is empty (empty patterns never match).
    pub fn new(needle: Vec<u8>) -> Option<Self> {
        if needle.is_empty() {
            return None;
        }
        Some(Self {
            needle,
            window: Vec::new(),
        })
    }

    /// Return a reference to the needle bytes.
    pub fn needle(&self) -> &[u8] {
        &self.needle
    }

    /// Scan the current window for the needle. Does not push new data.
    pub fn check(&self) -> MatchResult {
        find_subslice(&self.window, &self.needle).map_or(MatchResult::NoMatch, MatchResult::Found)
    }

    /// Append a chunk to the internal window and check for a match in the
    /// combined data. Returns the byte offset within the total accumulated
    /// buffer where the needle starts, or `NoMatch`.
    pub fn push(&mut self, chunk: &[u8]) -> MatchResult {
        self.window.extend_from_slice(chunk);
        self.check()
    }

    /// Truncate the internal window to `len` bytes from the front.  Call this
    /// after consuming match data or after `max_buffered_bytes` is reached,
    /// keeping only the tail that could still be part of a subsequent match.
    pub fn truncate_front(&mut self, keep: usize) {
        let drop = self.window.len().saturating_sub(keep);
        if drop > 0 {
            self.window.drain(..drop);
        }
    }

    /// Current window length.
    pub fn len(&self) -> usize {
        self.window.len()
    }

    /// Whether the window is empty.
    pub fn is_empty(&self) -> bool {
        self.window.is_empty()
    }
}

// ---- Validation helper ------------------------------------------------------

/// Validate a `MatchRequest`, decode the pattern into raw bytes, and return
/// an owned `ByteMatcher` ready to use.
pub fn validate_match_request(req: &MatchRequest) -> Result<ByteMatcher, String> {
    match req.config.mode {
        MatchMode::LiteralSubstring => {}
    }
    let encoding: codec::Encoding = req.config.pattern_encoding.into();
    let needle = codec::decode(encoding, &req.pattern)
        .map_err(|e| format!("Pattern decoding failed - {e}"))?;
    if needle.is_empty() {
        return Err("Pattern must not be empty after decoding".into());
    }
    ByteMatcher::new(needle).ok_or_else(|| "Pattern must not be empty after decoding".into())
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_matcher_finds_immediate_match() {
        let mut m = ByteMatcher::new(b"OK>".to_vec()).unwrap();
        assert_eq!(m.push(b"OK>"), MatchResult::Found(0));
    }

    #[test]
    fn byte_matcher_finds_offset_match() {
        let mut m = ByteMatcher::new(b"OK>".to_vec()).unwrap();
        assert_eq!(m.push(b"hell"), MatchResult::NoMatch);
        assert_eq!(m.push(b"O"), MatchResult::NoMatch);
        assert_eq!(m.push(b"K>!"), MatchResult::Found(4));
    }

    #[test]
    fn byte_matcher_rejects_empty_needle() {
        assert!(ByteMatcher::new(Vec::new()).is_none());
    }

    #[test]
    fn byte_matcher_truncate_front_works() {
        let mut m = ByteMatcher::new(b"OK>".to_vec()).unwrap();
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
            },
        };
        let matcher = validate_match_request(&req).unwrap();
        assert_eq!(matcher.needle(), b"OK>");
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
        let mut m = ByteMatcher::new(b"XYZ".to_vec()).unwrap();
        assert_eq!(m.push(b"ABCDEF"), MatchResult::NoMatch);
        assert_eq!(m.check(), MatchResult::NoMatch);
    }
}
