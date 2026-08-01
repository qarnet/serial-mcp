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
//!
//! # Bounded-window policy
//!
//! Raw `read` and raw `subscribe` share one matcher-owned retention policy so
//! their windows cannot drift apart. [`Matcher::push_bounded`] appends and
//! checks a chunk, then enforces the retention cap before returning:
//!
//! - retained limit = `max_buffered_bytes + overlap_allowance`, where the
//!   literal allowance is `needle.len().saturating_sub(1)` (so a match
//!   straddling the cap boundary is still detected) and the regex/glob
//!   allowance is the conservative constant [`REGEX_GLOB_OVERLAP_ALLOWANCE`]
//!   (256 bytes; preserves the old subscribe heuristic).
//! - The retained window after every `push_bounded` call never exceeds the
//!   computed limit, including after `NoMatch`.
//! - [`Matcher::check`], [`Matcher::push`], and [`Matcher::push_bounded`]
//!   return `Found(index)` relative to the total bytes fed since the last
//!   [`Matcher::reset_window`], not relative to the truncated window.
//!   Front truncation advances an internal base offset by exactly the bytes
//!   removed, so match indexes stay stream-global for the lifetime of the
//!   window. `reset_window` clears the window and resets the base to zero,
//!   keeping framed (per-frame) matches frame-local.
//! - Literal pre-match context is shaped by the matcher at match time via
//!   [`Matcher::shape_literal_match_context`]: bounded raw paths shape over
//!   the retained window with pre-match context capped at
//!   `min(configured_context, max_buffered_bytes)`, framed paths over the
//!   matching frame's bytes (bounded naturally by the frame). Regex/glob
//!   store no shaped context.
//! - Glob truncation never creates a false whole-line match from a retained
//!   suffix that starts mid-line: truncation marks the first retained line
//!   partial when the byte before the new window start was not `\n`, and
//!   `check` does not treat an incomplete prefix as a complete line.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::codec;
use crate::util::find_subsequence;

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

impl std::fmt::Display for MatchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MatchMode::LiteralSubstring => f.write_str("literal_substring"),
            MatchMode::Regex => f.write_str("regex"),
            MatchMode::Glob => f.write_str("glob"),
        }
    }
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

/// Conservative overlap allowance (bytes) retained beyond
/// `max_buffered_bytes` for regex and glob windows. Regex patterns have no
/// fixed match length and glob lines are arbitrarily long, so a fixed 256
/// byte allowance preserves the pre-Phase-5 subscribe heuristic: a match
/// candidate that spans the boundary of the previous chunk is still seen.
pub(crate) const REGEX_GLOB_OVERLAP_ALLOWANCE: usize = 256;

/// Saved matcher-owned literal context for the most recent push.
///
/// Stored so a later stop-outcome match index can be shaped over the bytes
/// retained at match time — the bounded retained window for raw paths, the
/// frame bytes for framed paths. The `global_index` lets the accessor verify
/// it is answering for the most recent `Found`.
///
/// Internal matcher state; exposed only because `Matcher` is a public enum.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct SavedMatchContext {
    /// Global index (relative to last `reset_window`) of the saved match.
    global_index: usize,
    /// Shaped payload computed at match time.
    payload: ShapedMatchPayload,
}

/// Stateful byte pattern matcher supporting literal, regex, and glob modes.
///
/// Call [`Matcher::push`] with each incoming chunk. When the concatenated
/// bytes contain a match, returns [`MatchResult::Found`] with the byte offset
/// relative to the total bytes fed since the last [`Matcher::reset_window`].
///
/// Bounded callers (raw `read` / raw `subscribe`) use [`Matcher::push_bounded`]
/// so the retained window never exceeds `max_buffered_bytes` plus the mode's
/// overlap allowance; [`Matcher::truncate_front`] remains the raw primitive
/// and advances the global base offset by the bytes removed.
///
/// Framed callers (`rx_consume`) keep using unbounded [`Matcher::push`] with a
/// [`Matcher::reset_window`] per frame, so frame match indexes stay frame-local;
/// [`Matcher::push`] also saves frame-local literal context for the matching
/// frame, bounded naturally by the frame's bytes.
#[derive(Debug)]
pub enum Matcher {
    /// Literal byte-substring match.
    Literal {
        needle: Vec<u8>,
        /// Rolling buffer mirroring the caller's accumulation, used for
        /// substring search.
        window: Vec<u8>,
        /// Pre-match context byte count for payload shaping.
        context_amount: Option<usize>,
        /// Absolute bytes discarded from the front of this window since the
        /// last `reset_window`. `Found(index)` = `base + window-local index`.
        base: usize,
        /// Saved literal context for the most recent push `Found`.
        last_match_context: Option<SavedMatchContext>,
    },
    /// Regular expression match on raw bytes.
    Regex {
        re: regex::bytes::Regex,
        window: Vec<u8>,
        context_amount: Option<usize>,
        /// Absolute bytes discarded from the front of this window since the
        /// last `reset_window`. `Found(index)` = `base + window-local index`.
        base: usize,
    },
    /// Glob pattern per-line match. Lines are split on `\n` and each
    /// line is tested against the glob pattern.
    Glob {
        pat: glob::Pattern,
        window: Vec<u8>,
        context_amount: Option<usize>,
        /// Absolute bytes discarded from the front of this window since the
        /// last `reset_window`. `Found(index)` = `base + window-local index`.
        base: usize,
        /// Whether the first retained line began before the window start
        /// (front truncation cut mid-line). Such a line is never treated as
        /// a complete line by `check`.
        first_line_partial: bool,
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
            base: 0,
            last_match_context: None,
        })
    }

    /// Create a new matcher with a context amount from an existing builder.
    /// Used by `validate_match_request` to set context after construction.
    pub fn with_context(self, context_amount: Option<usize>) -> Self {
        match self {
            Self::Literal {
                needle,
                window,
                base,
                last_match_context,
                ..
            } => Self::Literal {
                needle,
                window,
                context_amount,
                base,
                last_match_context,
            },
            Self::Regex {
                re, window, base, ..
            } => Self::Regex {
                re,
                window,
                context_amount,
                base,
            },
            Self::Glob {
                pat,
                window,
                base,
                first_line_partial,
                ..
            } => Self::Glob {
                pat,
                window,
                context_amount,
                base,
                first_line_partial,
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
    ///
    /// The returned `Found(index)` is relative to the total bytes fed since
    /// the last [`Matcher::reset_window`] (window-local index plus the
    /// internal base of bytes truncated from the front).
    pub fn check(&self) -> MatchResult {
        match self {
            Self::Literal {
                needle,
                window,
                base,
                ..
            } => find_subsequence(window, needle)
                .map_or(MatchResult::NoMatch, |i| MatchResult::Found(base + i)),
            Self::Regex {
                re, window, base, ..
            } => re.find(window).map_or(MatchResult::NoMatch, |m| {
                MatchResult::Found(base + m.start())
            }),
            Self::Glob {
                pat,
                window,
                base,
                first_line_partial,
                ..
            } => {
                let decoded = String::from_utf8_lossy(window);
                let mut byte_offset: usize = 0;
                let mut first_line = true;
                for line in decoded.split('\n') {
                    // Strip trailing \r for line content matching (handles
                    // both \n and \r\n line endings from UART/serial).
                    let line_content = line.strip_suffix('\r').unwrap_or(line);
                    // A first retained line that front truncation cut
                    // mid-line is never a complete line; do not test it.
                    let skip = first_line && *first_line_partial;
                    first_line = false;
                    if !skip && pat.matches(line_content) {
                        return MatchResult::Found(base + byte_offset);
                    }
                    // Advance past the raw bytes of this line plus the \n separator.
                    // split('\n') preserves \r, so line.len() counts both \r and
                    // the line content correctly.
                    byte_offset += line.len() + 1; // +1 for \n
                }
                MatchResult::NoMatch
            }
        }
    }

    /// Append a chunk to the internal window and check for a match in the
    /// combined data. Returns the byte offset within the total accumulated
    /// buffer where the match starts, or `NoMatch`.
    ///
    /// For literal matchers with configured context, a `Found` also saves the
    /// shaped payload over the current window. Framed callers reset the
    /// window per frame, so the configured context is bounded naturally by
    /// the bytes present in the matching frame. Each call first clears any
    /// previously saved context, so the saved payload always corresponds to
    /// the most recent push.
    pub fn push(&mut self, chunk: &[u8]) -> MatchResult {
        // Each push invalidates any previously saved literal context.
        if let Self::Literal {
            last_match_context, ..
        } = self
        {
            *last_match_context = None;
        }
        match self {
            Self::Literal { window, .. }
            | Self::Regex { window, .. }
            | Self::Glob { window, .. } => {
                window.extend_from_slice(chunk);
            }
        }
        let result = self.check();
        if let MatchResult::Found(global_index) = result {
            self.save_literal_context(global_index, None);
        }
        result
    }

    /// Retention cap for this matcher given the connection's
    /// `max_buffered_bytes`: the cap plus the mode's overlap allowance
    /// (literal: `needle.len() - 1`; regex/glob:
    /// [`REGEX_GLOB_OVERLAP_ALLOWANCE`]).
    pub fn retained_window_limit(&self, max_buffered_bytes: usize) -> usize {
        let allowance = match self {
            Self::Literal { needle, .. } => needle.len().saturating_sub(1),
            Self::Regex { .. } | Self::Glob { .. } => REGEX_GLOB_OVERLAP_ALLOWANCE,
        };
        max_buffered_bytes.saturating_add(allowance)
    }

    /// Save the shaped literal payload for a `Found` at `global_index`,
    /// computed over the current window.
    ///
    /// `context_cap` optionally caps the pre-match context byte count:
    /// bounded callers pass `max_buffered_bytes` (so requested context can
    /// never bypass the connection memory/result bound), framed callers pass
    /// `None` (full configured context, bounded naturally by the frame bytes
    /// in the window). No-op for regex/glob matchers and for literal
    /// matchers without configured context.
    fn save_literal_context(&mut self, global_index: usize, context_cap: Option<usize>) {
        if let Self::Literal {
            needle,
            window,
            context_amount: Some(context),
            base,
            last_match_context,
            ..
        } = self
        {
            let effective_context = context_cap.map_or(*context, |cap| (*context).min(cap));
            // Shape over the current window: translate the global index
            // through the window base.
            let local = global_index.saturating_sub(*base);
            let pre_start = local.saturating_sub(effective_context);
            let match_end = local.saturating_add(needle.len()).min(window.len());
            let payload = ShapedMatchPayload {
                data: window[pre_start..match_end].to_vec(),
                match_index: local - pre_start,
                needle_len: needle.len(),
            };
            *last_match_context = Some(SavedMatchContext {
                global_index,
                payload,
            });
        }
    }

    /// Append and check a chunk under the bounded-window policy.
    ///
    /// The combined data is checked first so matches spanning the previous
    /// retention boundary are still found; the returned `Found(index)` is
    /// global (relative to the last [`Matcher::reset_window`]). For literal
    /// matchers with configured context, the shaped payload for this match is
    /// saved with pre-match context capped at
    /// `min(configured_context, max_buffered_bytes)` — the payload still
    /// contains the full matched literal, but requested context can never
    /// bypass the connection memory/result bound. The retained window after
    /// the call never exceeds [`Matcher::retained_window_limit`], including
    /// after `NoMatch`.
    pub fn push_bounded(&mut self, chunk: &[u8], max_buffered_bytes: usize) -> MatchResult {
        // `push` clears any previously saved context and saves the full
        // configured-context shape on `Found`; overwrite that shape with the
        // bounded-cap shape (min(configured, max_buffered_bytes)) below.
        let result = self.push(chunk);

        if let MatchResult::Found(global_index) = result {
            self.save_literal_context(global_index, Some(max_buffered_bytes));
        }

        // Enforce retention after capturing the global result.
        let limit = self.retained_window_limit(max_buffered_bytes);
        if self.len() > limit {
            self.truncate_front(limit);
        }

        result
    }

    /// Truncate the internal window to keep at most `keep` bytes from the
    /// back. Advances the global base by exactly the bytes removed so
    /// subsequent `Found(index)` values stay stream-global. Call after
    /// consuming match data to prevent unbounded growth.
    #[cfg_attr(mutants, mutants::skip)]
    pub fn truncate_front(&mut self, keep: usize) {
        match self {
            Self::Literal { window, base, .. } | Self::Regex { window, base, .. } => {
                let drop = window.len().saturating_sub(keep);
                if drop > 0 {
                    window.drain(..drop);
                    *base += drop;
                }
            }
            Self::Glob {
                window,
                base,
                first_line_partial,
                ..
            } => {
                let drop = window.len().saturating_sub(keep);
                if drop > 0 {
                    // The new first retained line is a complete line only if
                    // it starts right after a line terminator (the byte that
                    // immediately precedes the new window start).
                    *first_line_partial = window[drop - 1] != b'\n';
                    window.drain(..drop);
                    *base += drop;
                }
            }
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

    /// Reset the internal window to start fresh matching: clears bytes,
    /// resets the global base to zero, clears any saved literal context, and
    /// marks the first glob line complete. Used for per-frame matching when
    /// framing is active.
    pub fn reset_window(&mut self) {
        match self {
            Self::Literal {
                window,
                base,
                last_match_context,
                ..
            } => {
                window.clear();
                *base = 0;
                *last_match_context = None;
            }
            Self::Regex { window, base, .. } => {
                window.clear();
                *base = 0;
            }
            Self::Glob {
                window,
                base,
                first_line_partial,
                ..
            } => {
                window.clear();
                *base = 0;
                *first_line_partial = false;
            }
        }
    }

    /// Return the matcher-owned literal context for the most recent `Found`
    /// at `global_match_index`, if the literal matcher has configured context
    /// and the index matches the saved match.
    ///
    /// Returns `None` for regex/glob matchers and for any index other than
    /// the most recent push `Found`. The shaped payload is a clone of the
    /// bytes retained at match time.
    pub fn shape_literal_match_context(
        &self,
        global_match_index: usize,
    ) -> Option<ShapedMatchPayload> {
        match self {
            Self::Literal {
                context_amount,
                last_match_context,
                ..
            } => {
                if context_amount.is_none() {
                    return None;
                }
                match last_match_context {
                    Some(ctx) if ctx.global_index == global_match_index => {
                        Some(ctx.payload.clone())
                    }
                    _ => None,
                }
            }
            Self::Regex { .. } | Self::Glob { .. } => None,
        }
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
                base: 0,
            })
        }
        MatchMode::Glob => {
            let pat = glob::Pattern::new(&String::from_utf8_lossy(&decoded))
                .map_err(|e| format!("Invalid glob pattern: {e}"))?;
            Ok(Matcher::Glob {
                pat,
                window: Vec::new(),
                context_amount: context,
                base: 0,
                first_line_partial: false,
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
        // Push more data; "BBBOK>" contains "OK>" at offset 3 within the
        // window, but 7 bytes have been fed since reset, so the returned
        // index is global 7, not local 3.
        assert_eq!(m.push(b"OK>"), MatchResult::Found(7));
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

    // ── Regex matching ─────────────────────────────────────────────────

    #[test]
    fn regex_matches_simple_pattern() {
        let mut m = Matcher::Regex {
            re: regex::bytes::Regex::new("world").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
        };
        assert_eq!(m.push(b"hello "), MatchResult::NoMatch);
        assert_eq!(m.push(b"world"), MatchResult::Found(6));
    }

    #[test]
    fn regex_matches_wildcard_dot() {
        let mut m = Matcher::Regex {
            re: regex::bytes::Regex::new("po.g").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
        };
        assert_eq!(m.push(b"test"), MatchResult::NoMatch);
        assert_eq!(m.push(b"pong"), MatchResult::Found(4));
    }

    #[test]
    fn regex_no_match_returns_no_match() {
        let mut m = Matcher::Regex {
            re: regex::bytes::Regex::new("world").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
        };
        assert_eq!(m.push(b"hello moon"), MatchResult::NoMatch);
    }

    #[test]
    fn validate_match_request_regex_utf8() {
        let req = MatchRequest {
            pattern: "po.g".into(),
            config: MatchConfig {
                mode: MatchMode::Regex,
                pattern_encoding: PatternEncoding::Utf8,
                context_amount_of_matched_bytes: None,
            },
        };
        let matcher = validate_match_request(&req).unwrap();
        assert!(matcher.needle_len().is_none()); // regex has no fixed needle
    }

    #[test]
    fn validate_match_request_regex_invalid_rejected() {
        let req = MatchRequest {
            pattern: "[invalid".into(),
            config: MatchConfig {
                mode: MatchMode::Regex,
                pattern_encoding: PatternEncoding::Utf8,
                context_amount_of_matched_bytes: None,
            },
        };
        assert!(validate_match_request(&req).is_err());
    }

    // ── Glob matching ──────────────────────────────────────────────────

    #[test]
    fn glob_matches_line() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("pong").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(m.push(b"boot\npong\nready"), MatchResult::Found(5));
    }

    #[test]
    fn glob_matches_line_with_crlf_endings() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("pong").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(m.push(b"boot\r\npong\r\nready"), MatchResult::Found(6));
    }

    #[test]
    fn glob_matches_first_line_with_crlf() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("boot").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(m.push(b"boot\r\npong\r\nready"), MatchResult::Found(0));
    }

    #[test]
    fn glob_matches_last_partial_line() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("pong").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        // No trailing newline — last line still tested
        assert_eq!(m.push(b"boot\r\npong"), MatchResult::Found(6));
    }

    #[test]
    fn glob_no_match_returns_no_match() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("pong").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(m.push(b"boot\r\nready\r\ndone"), MatchResult::NoMatch);
    }

    #[test]
    fn glob_matches_wildcard_pattern() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("error*").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(
            m.push(b"boot\r\nerror: flash failed\r\nready"),
            MatchResult::Found(6)
        );
    }

    #[test]
    fn validate_match_request_glob_invalid_rejected() {
        let req = MatchRequest {
            pattern: "[unclosed".into(),
            config: MatchConfig {
                mode: MatchMode::Glob,
                pattern_encoding: PatternEncoding::Utf8,
                context_amount_of_matched_bytes: None,
            },
        };
        assert!(validate_match_request(&req).is_err());
    }

    // ── Bounded-window policy (Phase 5) ────────────────────────────────

    #[test]
    fn retained_window_limit_uses_mode_overlap_allowance() {
        // Literal: cap + needle.len() - 1.
        let literal = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        assert_eq!(literal.retained_window_limit(16), 18);
        // One-byte literal: overlap saturates to zero.
        let one = Matcher::new_literal(b"O".to_vec()).unwrap();
        assert_eq!(one.retained_window_limit(16), 16);
        // Zero max: no underflow.
        assert_eq!(one.retained_window_limit(0), 0);
        // Regex/glob: cap + fixed 256-byte allowance.
        let re = Matcher::Regex {
            re: regex::bytes::Regex::new("o").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
        };
        assert_eq!(
            re.retained_window_limit(16),
            16 + REGEX_GLOB_OVERLAP_ALLOWANCE
        );
    }

    #[test]
    fn push_bounded_literal_spanning_chunks_matches_global_index() {
        // needle "XYZ" (len 3) -> limit = 8 + 2 = 10.
        let mut m = Matcher::new_literal(b"XYZ".to_vec())
            .map(|m| m.with_context(None))
            .unwrap();
        // 11 bytes fed: window truncates to the last 10 ("123456789A"), base 1.
        assert_eq!(m.push_bounded(b"0123456789A", 8), MatchResult::NoMatch);
        assert_eq!(m.len(), 10);
        assert_eq!(m.retained_window_limit(8), 10);
        // The match starts at the truncated boundary ("...A" + "BCXYZ");
        // "XYZ" is at window-local index 12, global = 1 + 12 = 13.
        assert_eq!(
            m.push_bounded(b"BCXYZ", 8),
            MatchResult::Found(13),
            "match index must be global, not window-local"
        );
    }

    #[test]
    fn push_bounded_repeated_no_match_keeps_window_within_limit() {
        let mut m = Matcher::new_literal(b"XYZ".to_vec()).unwrap();
        // Simulate a long stream of non-matching chunks; the window must
        // never exceed the computed limit, including after NoMatch.
        for i in 0..20u8 {
            let chunk = [i, i + 1, i + 2, i + 3, i + 4];
            assert_eq!(m.push_bounded(&chunk, 16), MatchResult::NoMatch);
            assert!(
                m.len() <= m.retained_window_limit(16),
                "window {} exceeds limit {} after push {}",
                m.len(),
                m.retained_window_limit(16),
                i
            );
        }
    }

    #[test]
    fn push_bounded_zero_and_one_byte_literal_no_underflow() {
        // One-byte needle: overlap 0, limit == max.
        let mut one = Matcher::new_literal(b"B".to_vec()).unwrap();
        assert_eq!(one.push_bounded(b"ab", 3), MatchResult::NoMatch);
        assert_eq!(one.push_bounded(b"cdB", 3), MatchResult::Found(4));
        assert_eq!(one.len(), 3, "retained window must not exceed max=3");

        // Zero max: everything is truncated, no panic, Found still global.
        let mut zero = Matcher::new_literal(b"A".to_vec()).unwrap();
        assert_eq!(zero.push_bounded(b"A", 0), MatchResult::Found(0));
        assert!(zero.is_empty());
        assert_eq!(zero.push_bounded(b"xyz", 0), MatchResult::NoMatch);
        assert!(zero.is_empty());
    }

    #[test]
    fn truncate_front_advances_global_base_exactly() {
        let mut m = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        assert_eq!(m.push(b"ABCDEF"), MatchResult::NoMatch);
        m.truncate_front(4); // drop 2 ("AB"), base 2
        assert_eq!(m.push(b"OK>"), MatchResult::Found(6)); // 2 + 4
        m.truncate_front(2); // drop 2 ("EF"), base 4
        assert_eq!(m.check(), MatchResult::NoMatch);
    }

    #[test]
    fn reset_window_restores_frame_local_index_zero() {
        let mut m = Matcher::new_literal(b"OK>".to_vec()).unwrap();
        m.push(b"AAAABBB");
        m.truncate_front(3);
        m.push(b"X");
        assert_eq!(m.push(b"OK>"), MatchResult::Found(8));
        m.reset_window();
        assert!(m.is_empty());
        assert_eq!(m.check(), MatchResult::NoMatch);
        // Frame-local again: fresh window, index restarts at zero.
        assert_eq!(m.push(b"OK>"), MatchResult::Found(0));
    }

    #[test]
    fn push_bounded_literal_context_capped_at_max_buffered_bytes() {
        // Configured context (4096) exceeds a small max (8): the saved
        // pre-match context must be capped at min(4096, 8) = 8, never the
        // full requested amount. The payload still contains the full matched
        // literal.
        let mut m = Matcher::new_literal(b"XYZ".to_vec())
            .map(|m| m.with_context(Some(4096)))
            .unwrap();
        // 100 bytes of history, no match: retained window truncates to
        // limit = 8 + 2 = 10 (last 10 a's), base 90.
        let hist = vec![b'a'; 100];
        assert_eq!(m.push_bounded(&hist, 8), MatchResult::NoMatch);
        assert_eq!(m.len(), 10);
        // Match at global 100 (window-local 10). Capped context 8 ->
        // window[2..13] = 8 a's + "XYZ".
        assert_eq!(m.push_bounded(b"XYZ", 8), MatchResult::Found(100));
        let shaped = m.shape_literal_match_context(100).expect("saved context");
        assert_eq!(shaped.data, b"aaaaaaaaXYZ");
        assert_eq!(shaped.match_index, 8);
        assert_eq!(shaped.needle_len, 3);
        // A smaller cap (3) shrinks the saved pre-context accordingly.
        let mut n = Matcher::new_literal(b"XYZ".to_vec())
            .map(|m| m.with_context(Some(4096)))
            .unwrap();
        assert_eq!(n.push_bounded(&hist, 8), MatchResult::NoMatch);
        assert_eq!(n.push_bounded(b"XYZ", 3), MatchResult::Found(100));
        let shaped = n.shape_literal_match_context(100).expect("saved context");
        assert_eq!(shaped.data, b"aaaXYZ");
        assert_eq!(shaped.match_index, 3);
        assert_eq!(shaped.needle_len, 3);
    }

    #[test]
    fn push_saves_frame_local_literal_context() {
        // Framed callers use unbounded `push` with `reset_window` per frame.
        // The matching frame's own bytes bound the shaped context, so no
        // cross-frame bytes can leak into the payload.
        let mut m = Matcher::new_literal(b"beta".to_vec())
            .map(|m| m.with_context(Some(16)))
            .unwrap();
        // Frame 1: no match. Saved context stays unset.
        assert_eq!(m.push(b"alpha"), MatchResult::NoMatch);
        assert!(m.shape_literal_match_context(0).is_none());
        m.reset_window();
        // Frame 2: match at frame-local index 2; only 2 pre-match bytes
        // exist, so the saved payload is exactly "xxbeta".
        assert_eq!(m.push(b"xxbeta"), MatchResult::Found(2));
        let shaped = m.shape_literal_match_context(2).expect("saved context");
        assert_eq!(shaped.data, b"xxbeta");
        assert_eq!(shaped.match_index, 2);
        assert_eq!(shaped.needle_len, 4);
        // A different index does not match the saved context.
        assert!(m.shape_literal_match_context(3).is_none());
        // The next push clears the stale saved context when it returns
        // NoMatch (front truncation first drops the old needle, so the
        // re-check cannot re-find it).
        m.truncate_front(2); // keep "ta", drop "xxbe", base 4
        assert_eq!(m.push(b"zz"), MatchResult::NoMatch);
        assert!(m.shape_literal_match_context(2).is_none());
        // reset_window clears window + base + saved context.
        m.reset_window();
        assert!(m.shape_literal_match_context(2).is_none());
        // Frame-local again: fresh window restarts indexes at zero.
        assert_eq!(m.push(b"beta"), MatchResult::Found(0));
        let shaped = m.shape_literal_match_context(0).expect("saved context");
        assert_eq!(shaped.data, b"beta");
        assert_eq!(shaped.match_index, 0);
        assert_eq!(shaped.needle_len, 4);
    }

    #[test]
    fn bounded_context_at_exact_truncation_boundary_is_exact() {
        // needle "XYZ" (len 3) -> limit = 6 + 2 = 8.
        let mut m = Matcher::new_literal(b"XYZ".to_vec())
            .map(|m| m.with_context(Some(4)))
            .unwrap();
        // 10 bytes fed, no match: window truncates to last 8 ("cdefghij"),
        // base 2.
        assert_eq!(m.push_bounded(b"abcdefghij", 6), MatchResult::NoMatch);
        assert_eq!(m.len(), 8);
        // "XYZ" at global 10 (window-local 8). Shaped before retention:
        // 4 context bytes before local 8 -> window[4..11] = "ghijXYZ".
        assert_eq!(m.push_bounded(b"XYZ", 6), MatchResult::Found(10));
        let shaped = m.shape_literal_match_context(10).expect("saved context");
        assert_eq!(shaped.data, b"ghijXYZ");
        assert_eq!(shaped.match_index, 4);
        assert_eq!(shaped.needle_len, 3);
        // A different index does not match the saved context.
        assert!(m.shape_literal_match_context(11).is_none());
        assert!(m.shape_literal_match_context(9).is_none());
        // The retained needle stays in the window (overlap allowance), so a
        // later push re-finds it at the same global index and re-saves the
        // context for that index.
        assert_eq!(m.push_bounded(b"qq", 6), MatchResult::Found(10));
        assert!(m.shape_literal_match_context(10).is_some());
        // reset_window drops the saved context along with the window.
        m.reset_window();
        assert!(m.shape_literal_match_context(10).is_none());
        // A fresh matcher that only ever saw NoMatch never exposes context.
        let mut fresh = Matcher::new_literal(b"XYZ".to_vec())
            .map(|m| m.with_context(Some(4)))
            .unwrap();
        assert_eq!(fresh.push_bounded(b"abc", 6), MatchResult::NoMatch);
        assert!(fresh.shape_literal_match_context(0).is_none());
        // Regex/glob never expose shaped context.
        let re = Matcher::Regex {
            re: regex::bytes::Regex::new("xyz").unwrap(),
            window: Vec::new(),
            context_amount: Some(4),
            base: 0,
        };
        assert!(re.shape_literal_match_context(0).is_none());
    }

    #[test]
    fn regex_matches_across_chunk_splits_within_retained_cap() {
        let mut m = Matcher::Regex {
            re: regex::bytes::Regex::new("world").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
        };
        assert_eq!(m.push_bounded(b"hello wor", 64), MatchResult::NoMatch);
        assert_eq!(m.push_bounded(b"ld", 64), MatchResult::Found(6));
    }

    #[test]
    fn glob_matches_across_chunk_splits_within_retained_cap() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("pong").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(m.push_bounded(b"boot\n", 64), MatchResult::NoMatch);
        assert_eq!(m.push_bounded(b"pong\nready", 64), MatchResult::Found(5));
    }

    #[test]
    fn glob_truncated_mid_line_does_not_false_match() {
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("dy").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        // "ready" truncated to its mid-line suffix "dy\n" must NOT match
        // the glob "dy": the first retained line is partial.
        assert_eq!(m.push(b"ready\n"), MatchResult::NoMatch);
        m.truncate_front(3); // keep "dy\n", drop "rea" (mid-line)
        assert_eq!(
            m.check(),
            MatchResult::NoMatch,
            "partial line must not match"
        );
        assert_eq!(m.push(b"xyz\n"), MatchResult::NoMatch);
        // A truncation that lands on a line boundary keeps testing complete
        // lines at correct global offsets.
        let mut n = Matcher::Glob {
            pat: glob::Pattern::new("zz").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(n.push(b"ab\ncd\n"), MatchResult::NoMatch);
        n.truncate_front(3); // keep "cd\n", drop "ab\n" (boundary), base 3
                             // "zz" starts at window-local 3, so global = 3 + 3 = 6.
        assert_eq!(n.push(b"zz\n"), MatchResult::Found(6));
    }

    #[test]
    fn glob_bounded_push_truncation_keeps_global_line_indexes() {
        // Regex/glob overlap is large, so feed enough bytes to truncate.
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("ready").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        // 400 bytes of non-matching lines, then "ready\n".
        let junk = vec![b'x'; 200];
        let mut fed = 0usize;
        let mut chunk = junk.clone();
        chunk.extend_from_slice(b"\n");
        assert_eq!(m.push_bounded(&chunk, 64), MatchResult::NoMatch);
        fed += chunk.len();
        let chunk2 = vec![b'y'; 200];
        let mut chunk2 = chunk2;
        chunk2.extend_from_slice(b"\nready\n");
        let result = m.push_bounded(&chunk2, 64);
        let ready_offset = fed + 200 + 1; // after second junk line + \n
        assert_eq!(result, MatchResult::Found(ready_offset));
        assert!(
            m.len() <= m.retained_window_limit(64),
            "window {} exceeds limit {}",
            m.len(),
            m.retained_window_limit(64)
        );
    }

    #[test]
    fn push_bounded_glob_partial_first_line_after_boundary_truncation() {
        // Truncation dropping through a '\n' starts the window at a fresh
        // line; subsequent complete lines still match at global indexes.
        let mut m = Matcher::Glob {
            pat: glob::Pattern::new("pong").unwrap(),
            window: Vec::new(),
            context_amount: None,
            base: 0,
            first_line_partial: false,
        };
        assert_eq!(m.push(b"boot\nxxxx\n"), MatchResult::NoMatch);
        m.truncate_front(5); // keep "xxxx\n", drop "boot\n" (boundary), base 5
        assert_eq!(
            m.push(b"pong\n"),
            MatchResult::Found(10),
            "complete line after boundary truncation matches at global index"
        );
    }
}
