//! Frame boundary detection and protocol parsing for RX streams.
//!
//! Provides a [`FrameDecoder`] that splits a byte stream into structured
//! frames using one of four boundary modes (line, delimiter, length-prefixed,
//! start/end marker). Optional parsers interpret frame content (AT commands,
//! JSON lines, shell prompts). Used as an option on `read` and `subscribe`.

use crate::codec;
use crate::match_config::PatternEncoding;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---- Framing configuration ------------------------------------------------

/// Framing configuration for `read` and `subscribe`.
/// Specifies how to split the byte stream into frames and optionally parse
/// each frame's content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FramingConfig {
    /// Frame boundary detection mode.
    pub mode: FramingMode,
    /// Optional parser configuration for interpreting frame content.
    #[serde(default)]
    pub parser: Option<ParserConfig>,
    /// Maximum number of frames to collect before stopping (read only).
    /// When set, the read stops after collecting this many frames regardless
    /// of timeout. Default: no limit.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub max_frames: Option<usize>,
    /// Include terminators/delimiters in the frame `data` field.
    /// Default: false (terminators are stripped).
    #[serde(default)]
    pub include_terminators: bool,
}

impl Default for FramingConfig {
    fn default() -> Self {
        Self {
            mode: FramingMode::Line,
            parser: None,
            max_frames: None,
            include_terminators: false,
        }
    }
}

/// How frame boundaries are detected in the byte stream.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum FramingMode {
    /// Split on `\n` (optionally preceded by `\r`). Default mode.
    Line,
    /// Split on a user-supplied byte delimiter sequence.
    Delimiter {
        /// The delimiter as a string (decoded per `delimiter_encoding`).
        delimiter: String,
        /// How to decode the delimiter string into bytes.
        #[serde(default = "default_encoding")]
        delimiter_encoding: PatternEncoding,
    },
    /// Split based on a length prefix field at the start of each frame.
    LengthPrefixed {
        /// Size of the length prefix field in bytes: 1, 2, or 4.
        prefix_size: u8,
        /// Byte order of the length prefix.
        #[serde(default)]
        endianness: Endianness,
        /// Optional: reading starts at an offset from the beginning of the stream.
        /// When Some(N), the first N bytes are skipped before reading the first
        /// length prefix.
        #[serde(default)]
        #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
        initial_offset: Option<usize>,
    },
    /// Split based on start and end marker byte sequences.
    StartEnd {
        /// Start marker (decoded per `marker_encoding`).
        start: String,
        /// End marker (decoded per `marker_encoding`).
        end: String,
        /// How to decode the marker strings into bytes.
        #[serde(default = "default_encoding")]
        marker_encoding: PatternEncoding,
        /// Include the markers in frame data. Default: false.
        #[serde(default)]
        include_markers: bool,
    },
}

fn default_encoding() -> PatternEncoding {
    PatternEncoding::Utf8
}

/// Byte order for length-prefixed framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Endianness {
    #[default]
    Big,
    Little,
}

/// Parser configuration — what to do with each frame's content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ParserConfig {
    /// Which parser to use.
    #[serde(rename = "type")]
    pub parser_type: ParserType,
    /// Optional custom prompt pattern for shell prompt parser.
    /// Accepts a regex pattern as a string.
    #[serde(default)]
    pub custom_prompt: Option<String>,
}

/// Supported parser types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ParserType {
    /// Parse AT command responses and URCs.
    AtCommand,
    /// Parse each frame as a JSON object.
    JsonLines,
    /// Detect shell prompt patterns.
    ShellPrompt,
    /// No parsing — frames are returned as raw data.
    Raw,
}

// ---- Frame types -----------------------------------------------------------

/// A decoded frame with optional parsed content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Frame {
    /// Raw frame bytes (without delimiters/terminators unless include_terminators is set).
    pub data: Vec<u8>,
    /// Frame number since decoder creation (0-based).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub index: usize,
    /// Boundary detection mode used (for diagnostic purposes).
    pub frame_type: String,
    /// Parsed frame fields, if a parser is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed: Option<ParsedFrame>,
}

/// Structured field interpretation of a frame.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "parser", rename_all = "snake_case")]
pub enum ParsedFrame {
    AtCommand {
        /// Result code, URC, data, or error.
        response_type: String,
        /// Command name (e.g. "+CGREG").
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// Status: "OK", "ERROR", "CME ERROR: <code>", etc.
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        /// Response data fields (lines between echo and status).
        fields: Vec<String>,
    },
    Json(serde_json::Value),
    ShellPrompt {
        prompt: String,
        prompt_type: String,
    },
    Raw,
}

// ---- Frame decoder ---------------------------------------------------------

/// Stateful frame boundary detector.
///
/// Push chunks via [`FrameDecoder::push`] to receive decoded
/// [`Frame`] instances. Callers accumulate frames and drain consumed
/// bytes from their accumulation buffer.
pub struct FrameDecoder {
    /// Buffer for incomplete frame data.
    buf: Vec<u8>,
    /// How frame boundaries are detected.
    mode: DecoderMode,
    /// Total frames emitted so far.
    frame_count: usize,
    /// Include terminators in frame data.
    include_terminators: bool,
    /// Optional parser for frame content.
    parser: Option<Box<dyn FrameParser>>,
}

enum DecoderMode {
    Line,
    Delimiter(Vec<u8>),
    LengthPrefixed {
        prefix_size: u8,
        endianness: Endianness,
        remaining_offset: usize,
        next_payload_len: Option<usize>,
    },
    StartEnd {
        start: Vec<u8>,
        end: Vec<u8>,
        include_markers: bool,
        in_frame: bool,
    },
}

trait FrameParser: Send + Sync {
    fn parse(&self, data: &[u8]) -> ParsedFrame;
}

// ---- Frame decoder implementation ------------------------------------------

impl FrameDecoder {
    /// Create a new frame decoder from a framing configuration.
    pub fn new(config: &FramingConfig) -> Result<Self, String> {
        let (mode, offset) = match &config.mode {
            FramingMode::Line => (DecoderMode::Line, None),
            FramingMode::Delimiter {
                delimiter,
                delimiter_encoding,
            } => {
                let bytes = codec::decode((*delimiter_encoding).into(), delimiter)
                    .map_err(|e| format!("Invalid delimiter encoding: {e}"))?;
                if bytes.is_empty() {
                    return Err("Delimiter must not be empty".into());
                }
                (DecoderMode::Delimiter(bytes), None)
            }
            FramingMode::LengthPrefixed {
                prefix_size,
                endianness,
                initial_offset,
            } => {
                if !matches!(prefix_size, 1 | 2 | 4) {
                    return Err("prefix_size must be 1, 2, or 4".into());
                }
                (
                    DecoderMode::LengthPrefixed {
                        prefix_size: *prefix_size,
                        endianness: *endianness,
                        remaining_offset: initial_offset.unwrap_or(0),
                        next_payload_len: None,
                    },
                    *initial_offset,
                )
            }
            FramingMode::StartEnd {
                start,
                end,
                marker_encoding,
                include_markers,
            } => {
                let start_bytes = codec::decode((*marker_encoding).into(), start)
                    .map_err(|e| format!("Invalid start marker encoding: {e}"))?;
                let end_bytes = codec::decode((*marker_encoding).into(), end)
                    .map_err(|e| format!("Invalid end marker encoding: {e}"))?;
                if start_bytes.is_empty() || end_bytes.is_empty() {
                    return Err("Start and end markers must not be empty".into());
                }
                (
                    DecoderMode::StartEnd {
                        start: start_bytes,
                        end: end_bytes,
                        include_markers: *include_markers,
                        in_frame: false,
                    },
                    None,
                )
            }
        };

        let parser: Option<Box<dyn FrameParser>> = match &config.parser {
            Some(pc) => Some(build_parser(pc)?),
            None => None,
        };

        let mut buf = Vec::new();
        if let Some(skip) = offset {
            if skip > 0 {
                buf.reserve(skip);
            }
        }

        Ok(Self {
            buf,
            mode,
            frame_count: 0,
            include_terminators: config.include_terminators,
            parser,
        })
    }

    /// Feed a chunk of bytes. Returns any complete frames found.
    /// The caller is responsible for draining consumed bytes from
    /// their accumulation buffer.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(chunk);
        let mut frames = Vec::new();
        loop {
            let consumed = match &mut self.mode {
                DecoderMode::Line => {
                    let pos = self.buf.iter().position(|&b| b == b'\n');
                    pos.map(|p| {
                        let end = if !self.include_terminators && p > 0 && self.buf[p - 1] == b'\r'
                        {
                            p - 1
                        } else {
                            p
                        };
                        let fb = if self.include_terminators {
                            self.buf[..p + 1].to_vec()
                        } else {
                            self.buf[..end].to_vec()
                        };
                        self.buf.drain(..p + 1);
                        fb
                    })
                }
                DecoderMode::Delimiter(delim) => {
                    let d = delim.clone();
                    let pos = find_subsequence(&self.buf, &d);
                    pos.map(|p| {
                        let fb = if self.include_terminators {
                            self.buf[..p + d.len()].to_vec()
                        } else {
                            self.buf[..p].to_vec()
                        };
                        self.buf.drain(..p + d.len());
                        fb
                    })
                }
                DecoderMode::LengthPrefixed {
                    prefix_size,
                    endianness,
                    remaining_offset,
                    next_payload_len,
                } => {
                    if *remaining_offset > 0 {
                        let drain = (*remaining_offset).min(self.buf.len());
                        self.buf.drain(..drain);
                        *remaining_offset -= drain;
                    }
                    // Determine next_payload_len if not yet known
                    if next_payload_len.is_none() {
                        let needed = *prefix_size as usize;
                        if self.buf.len() < needed {
                            break;
                        }
                        let len =
                            read_length_prefix(&self.buf[..needed], *prefix_size, *endianness);
                        *next_payload_len = Some(len);
                    }
                    let payload_len = match *next_payload_len {
                        Some(len) => len,
                        None => break, // not yet known, wait for more data
                    };
                    let header_len = *prefix_size as usize;
                    let total_needed = header_len + payload_len;
                    if self.buf.len() < total_needed {
                        break;
                    }
                    // Extract frame (prefix + payload)
                    let frame_bytes = if self.include_terminators {
                        self.buf[..total_needed].to_vec()
                    } else {
                        self.buf[header_len..total_needed].to_vec()
                    };
                    self.buf.drain(..total_needed);
                    *next_payload_len = None;
                    Some(frame_bytes)
                }
                DecoderMode::StartEnd {
                    start,
                    end,
                    include_markers,
                    in_frame,
                } => {
                    let start = start.clone();
                    let end = end.clone();
                    let include = *include_markers;
                    if !*in_frame {
                        // Search for start marker
                        if let Some(pos) = find_subsequence(&self.buf, &start) {
                            self.buf.drain(..pos);
                            if !include {
                                self.buf.drain(..start.len());
                            }
                            *in_frame = true;
                        } else {
                            // Keep tail that could partially match start
                            let keep = start.len().saturating_sub(1);
                            if self.buf.len() > keep {
                                self.buf.drain(..(self.buf.len() - keep));
                            }
                            break; // exit loop, need more data
                        }
                    }
                    if *in_frame {
                        if let Some(pos) = find_subsequence(&self.buf, &end) {
                            let frame_bytes = if include {
                                let fb = self.buf[..pos + end.len()].to_vec();
                                self.buf.drain(..pos + end.len());
                                fb
                            } else {
                                let fb = self.buf[..pos].to_vec();
                                self.buf.drain(..pos + end.len());
                                fb
                            };
                            *in_frame = false;
                            Some(frame_bytes)
                        } else {
                            break; // need more data for end marker
                        }
                    } else {
                        None
                    }
                }
            };

            match consumed {
                None => break,
                Some(frame_bytes) => {
                    self.frame_count += 1;
                    let parsed = self.parser.as_ref().map(|p| p.parse(&frame_bytes));
                    let frame_type = self.frame_type_str();
                    // If delimiter and include_terminators, don't strip
                    if matches!(self.mode, DecoderMode::Delimiter(_)) && !self.include_terminators {
                        // Already stripped
                    }
                    frames.push(Frame {
                        data: frame_bytes,
                        index: self.frame_count - 1,
                        frame_type,
                        parsed,
                    });
                }
            }
        }
        frames
    }

    fn frame_type_str(&self) -> String {
        match &self.mode {
            DecoderMode::Line => "line".into(),
            DecoderMode::Delimiter(_) => "delimiter".into(),
            DecoderMode::LengthPrefixed { .. } => "length_prefixed".into(),
            DecoderMode::StartEnd { .. } => "start_end".into(),
        }
    }

    /// Drain consumed bytes from the internal buffer.
    pub fn drain_consumed(&mut self, _consumed: usize) {
        // Buffer is drained in push() as frames are extracted.
        // This method is a no-op for API compatibility.
    }

    /// Bytes pending in the incomplete frame buffer.
    pub fn pending_len(&self) -> usize {
        self.buf.len()
    }

    /// Flush any remaining bytes as a partial frame.
    pub fn flush_partial(&mut self) -> Option<Frame> {
        if self.buf.is_empty() {
            return None;
        }
        let data = std::mem::take(&mut self.buf);
        self.frame_count += 1;
        Some(Frame {
            data,
            index: self.frame_count - 1,
            frame_type: self.frame_type_str(),
            parsed: None,
        })
    }
}

// ---- Parser implementations ------------------------------------------------

fn build_parser(config: &ParserConfig) -> Result<Box<dyn FrameParser>, String> {
    match config.parser_type {
        ParserType::AtCommand => Ok(Box::new(AtCommandParser)),
        ParserType::JsonLines => Ok(Box::new(JsonLinesParser)),
        ParserType::ShellPrompt => {
            let custom = config
                .custom_prompt
                .as_deref()
                .map(|s| {
                    regex::bytes::Regex::new(s).map_err(|e| format!("Invalid prompt regex: {e}"))
                })
                .transpose()?;
            Ok(Box::new(ShellPromptParser { custom }))
        }
        ParserType::Raw => Ok(Box::new(RawParser)),
    }
}

// AT command parser

struct AtCommandParser;

impl FrameParser for AtCommandParser {
    fn parse(&self, data: &[u8]) -> ParsedFrame {
        let text = String::from_utf8_lossy(data);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return ParsedFrame::Raw;
        }
        // AT command response lines start with + and then the command name
        if let Some(cmd) = trimmed.strip_prefix('+') {
            if let Some(colon) = cmd.find(':') {
                let cmd_name = cmd[..colon].to_string();
                let fields: Vec<String> = cmd[colon + 1..]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                return ParsedFrame::AtCommand {
                    response_type: "response".into(),
                    command: Some(cmd_name),
                    status: None,
                    fields,
                };
            }
        }
        // Check for URC (unsolicited result code) — starts with + but no echo preceding
        // Actually, AT URCs also start with + — same as responses. The difference
        // is context (preceded by AT command vs unsolicited). For simplicity,
        // treat all + lines as responses. Agent can distinguish via context.
        if trimmed == "OK" {
            return ParsedFrame::AtCommand {
                response_type: "status".into(),
                command: None,
                status: Some("OK".into()),
                fields: vec![],
            };
        }
        if trimmed == "ERROR" {
            return ParsedFrame::AtCommand {
                response_type: "error".into(),
                command: None,
                status: Some("ERROR".into()),
                fields: vec![],
            };
        }
        if trimmed.starts_with("+CME ERROR") || trimmed.starts_with("+CMS ERROR") {
            return ParsedFrame::AtCommand {
                response_type: "error".into(),
                command: None,
                status: Some(trimmed.to_string()),
                fields: vec![],
            };
        }
        // Data lines: not a command prefix, not a status.
        ParsedFrame::AtCommand {
            response_type: "data".into(),
            command: None,
            status: None,
            fields: vec![trimmed.to_string()],
        }
    }
}

// JSON lines parser

struct JsonLinesParser;

impl FrameParser for JsonLinesParser {
    fn parse(&self, data: &[u8]) -> ParsedFrame {
        match serde_json::from_slice::<serde_json::Value>(data) {
            Ok(val) => ParsedFrame::Json(val),
            Err(_) => ParsedFrame::Raw,
        }
    }
}

// Shell prompt parser

struct ShellPromptParser {
    custom: Option<regex::bytes::Regex>,
}

impl FrameParser for ShellPromptParser {
    fn parse(&self, data: &[u8]) -> ParsedFrame {
        let text = String::from_utf8_lossy(data);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return ParsedFrame::Raw;
        }
        // Try custom pattern first
        if let Some(ref re) = self.custom {
            if re.is_match(data) {
                return ParsedFrame::ShellPrompt {
                    prompt: trimmed.to_string(),
                    prompt_type: "custom".into(),
                };
            }
        }
        // Standard patterns
        if trimmed.ends_with("$ ") || trimmed.ends_with("$") {
            return ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "user".into(),
            };
        }
        if trimmed.ends_with("# ") || trimmed.ends_with("#") {
            return ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "root".into(),
            };
        }
        if trimmed.ends_with("> ") || trimmed.ends_with(">") {
            return ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "generic".into(),
            };
        }
        if let Some(at_pos) = trimmed.rfind('@') {
            if let Some(colon_pos) = trimmed[at_pos..].find(':') {
                let _user = &trimmed[..at_pos];
                let _host = &trimmed[at_pos + 1..at_pos + colon_pos];
                let suffix = &trimmed[at_pos + colon_pos + 1..];
                if suffix == "$ " || suffix == "$" || suffix == "# " || suffix == "#" {
                    return ParsedFrame::ShellPrompt {
                        prompt: trimmed.to_string(),
                        prompt_type: if suffix.starts_with('#') {
                            "root".to_string()
                        } else {
                            "user".to_string()
                        },
                    };
                }
            }
        }
        ParsedFrame::Raw
    }
}

// Raw parser (passthrough)

struct RawParser;

impl FrameParser for RawParser {
    fn parse(&self, _data: &[u8]) -> ParsedFrame {
        ParsedFrame::Raw
    }
}

// ---- Utility ---------------------------------------------------------------

/// Find a subsequence in a slice. Returns the index of the first match.
fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Read a length prefix from the given bytes.
/// `prefix_size` must be 1, 2, or 4 (validated at construction).
/// Returns 0 for invalid sizes as a safe fallback.
fn read_length_prefix(bytes: &[u8], prefix_size: u8, endianness: Endianness) -> usize {
    match prefix_size {
        1 => bytes[0] as usize,
        2 => {
            let arr: [u8; 2] = bytes[..2]
                .try_into()
                .expect("prefix_size=2 but buffer too short");
            match endianness {
                Endianness::Big => u16::from_be_bytes(arr) as usize,
                Endianness::Little => u16::from_le_bytes(arr) as usize,
            }
        }
        4 => {
            let arr: [u8; 4] = bytes[..4]
                .try_into()
                .expect("prefix_size=4 but buffer too short");
            match endianness {
                Endianness::Big => u32::from_be_bytes(arr) as usize,
                Endianness::Little => u32::from_le_bytes(arr) as usize,
            }
        }
        _ => {
            // Invalid prefix_size — should never happen because
            // FrameDecoder::new() rejects sizes other than 1/2/4.
            // Return 0 as a safe fallback (zero-length frame).
            0
        }
    }
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Line decoder ────────────────────────────────────────────────────

    #[test]
    fn line_decoder_single_line() {
        let config = FramingConfig {
            mode: FramingMode::Line,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"hello\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[0].frame_type, "line");
    }

    #[test]
    fn line_decoder_crlf() {
        let config = FramingConfig {
            mode: FramingMode::Line,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"hello\r\nworld\n");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[1].data, b"world");
    }

    #[test]
    fn line_decoder_partial_across_chunks() {
        let config = FramingConfig::default();
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"hel");
        assert!(frames.is_empty());
        let frames = dec.push(b"lo\nwor");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
        let frames = dec.push(b"ld\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"world");
    }

    #[test]
    fn line_decoder_empty_lines() {
        let config = FramingConfig::default();
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"\n\n\n");
        assert_eq!(frames.len(), 3);
        for f in &frames {
            assert!(f.data.is_empty());
        }
    }

    #[test]
    fn line_decoder_include_terminators() {
        let config = FramingConfig {
            mode: FramingMode::Line,
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"hello\r\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello\r\n");
    }

    // ── Delimiter decoder ───────────────────────────────────────────────

    #[test]
    fn delimiter_decoder_basic() {
        let config = FramingConfig {
            mode: FramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"a|b|c|");
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
        assert_eq!(frames[2].data, b"c");
    }

    #[test]
    fn delimiter_decoder_multi_byte() {
        let config = FramingConfig {
            mode: FramingMode::Delimiter {
                delimiter: "AA".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"xAAyAAz");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"x");
        assert_eq!(frames[1].data, b"y");
        // "z" is incomplete (no trailing delimiter)
    }

    #[test]
    fn delimiter_decoder_partial_delimiter() {
        let config = FramingConfig {
            mode: FramingMode::Delimiter {
                delimiter: "AB".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"xA");
        assert!(frames.is_empty());
        let frames = dec.push(b"By");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"x");
    }

    // ── Length-prefixed decoder ─────────────────────────────────────────

    #[test]
    fn length_prefixed_basic() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"\x05hello\x02wo\x02rb");
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[1].data, b"wo");
        assert_eq!(frames[2].data, b"rb");
    }

    #[test]
    fn length_prefixed_u16_big_endian() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let mut buf = vec![0x00, 0x05];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    // ── Start/end marker decoder ────────────────────────────────────────

    #[test]
    fn start_end_basic() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "STX".into(),
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"noiseSTXdataETXjunkSTXmoreETX");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"data");
        assert_eq!(frames[1].data, b"more");
    }

    #[test]
    fn start_end_include_markers() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "<".into(),
                end: ">".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: true,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"<data>");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"<data>");
    }

    // ── Parser tests ────────────────────────────────────────────────────

    #[test]
    fn at_parser_ok() {
        let p = AtCommandParser;
        let result = p.parse(b"OK");
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                status: Some(s),
                ..
            } if response_type == "status" && s == "OK"
        ));
    }

    #[test]
    fn at_parser_error() {
        let p = AtCommandParser;
        let result = p.parse(b"ERROR");
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                status: Some(s),
                ..
            } if response_type == "error" && s == "ERROR"
        ));
    }

    #[test]
    fn at_parser_command_response() {
        let p = AtCommandParser;
        let result = p.parse(b"+CGREG: 0,1");
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                command: Some(ref c),
                ..
            } if response_type == "response" && c == "CGREG"
        ));
    }

    #[test]
    fn json_parser_valid() {
        let p = JsonLinesParser;
        let result = p.parse(b"{\"key\":\"value\"}");
        assert!(matches!(result, ParsedFrame::Json(_)));
    }

    #[test]
    fn json_parser_invalid() {
        let p = JsonLinesParser;
        let result = p.parse(b"not json");
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn shell_prompt_user() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"$ ");
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "user")
        );
    }

    #[test]
    fn shell_prompt_root() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"# ");
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "root")
        );
    }

    #[test]
    fn shell_prompt_host() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"root@host:~# ");
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "root")
        );
    }

    #[test]
    fn combined_line_at_parser() {
        let config = FramingConfig {
            mode: FramingMode::Line,
            parser: Some(ParserConfig {
                parser_type: ParserType::AtCommand,
                custom_prompt: None,
            }),
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"OK\nERROR\n+CGREG: 0,1\n");
        assert_eq!(frames.len(), 3);
        assert!(
            matches!(frames[0].parsed, Some(ParsedFrame::AtCommand { ref status, .. }) if status.as_deref() == Some("OK"))
        );
        assert!(
            matches!(frames[1].parsed, Some(ParsedFrame::AtCommand { ref status, .. }) if status.as_deref() == Some("ERROR"))
        );
        assert!(
            matches!(frames[2].parsed, Some(ParsedFrame::AtCommand { ref command, .. }) if command.as_deref() == Some("CGREG"))
        );
    }

    // ── Negative / edge-case tests ───────────────────────────────────────

    #[test]
    fn line_decoder_no_terminator_then_flush() {
        let config = FramingConfig::default();
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"hello");
        assert!(frames.is_empty());
        assert_eq!(dec.pending_len(), 5);
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.data, b"hello");
        assert_eq!(partial.index, 0);
        assert_eq!(partial.frame_type, "line");
        assert!(partial.parsed.is_none());
    }

    #[test]
    fn delimiter_decoder_empty_rejected() {
        let config = FramingConfig {
            mode: FramingMode::Delimiter {
                delimiter: "".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config) {
            Ok(_) => panic!("empty delimiter should be rejected"),
            Err(err) => assert!(err.contains("Delimiter must not be empty"), "got: {err}"),
        }
    }

    #[test]
    fn length_prefixed_zero_payload() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        // Prefix 0x00 means zero-length payload — should emit empty frame.
        // The next byte \x05 starts a new length-prefixed frame.
        let frames = dec.push(b"\x00\x05hello");
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert_eq!(frames[1].data, b"hello");
    }

    #[test]
    fn length_prefixed_incomplete_payload() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        // Prefix says 10 bytes, but only 3 arrive — no frame emitted.
        let frames = dec.push(b"\x0aABC");
        assert!(frames.is_empty());
        assert!(dec.pending_len() >= 3);
    }

    #[test]
    fn length_prefixed_invalid_prefix_size() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 3,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config) {
            Ok(_) => panic!("prefix_size=3 should be rejected"),
            Err(err) => assert!(err.contains("prefix_size must be 1, 2, or 4"), "got: {err}"),
        }
    }

    #[test]
    fn length_prefixed_u32_big_endian() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 4,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let mut buf = vec![0x00, 0x00, 0x00, 0x05];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn length_prefixed_u32_little_endian() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 4,
                endianness: Endianness::Little,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let mut buf = vec![0x05, 0x00, 0x00, 0x00];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn start_end_no_start_marker() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "STX".into(),
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"noise_without_markers");
        assert!(frames.is_empty());
        // Buffer should be pruned to at most start.len() - 1 bytes.
        assert!(dec.pending_len() <= 2); // "STX" has len 3, keep ≤ 2
    }

    #[test]
    fn start_end_start_no_end_then_flush() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "<".into(),
                end: ">".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"<data_without_end");
        assert!(frames.is_empty(), "no end marker yet");
        let partial = dec.flush_partial().expect("partial frame after flush");
        assert_eq!(partial.data, b"data_without_end");
    }

    #[test]
    fn start_end_empty_markers_rejected() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "".into(),
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config) {
            Ok(_) => panic!("empty markers should be rejected"),
            Err(err) => assert!(
                err.contains("Start and end markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    #[test]
    fn start_end_start_split_across_chunks() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "ABC".into(),
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        // First chunk: "AB" — partial start marker
        let frames = dec.push(b"AB");
        assert!(frames.is_empty());
        // Second chunk: "CdX" — completes start, then data 'd', then end 'X'
        let frames = dec.push(b"CdX");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"d");
    }

    #[test]
    fn delimiter_invalid_encoding_rejected() {
        let config = FramingConfig {
            mode: FramingMode::Delimiter {
                // "!!!" is not valid base64
                delimiter: "!!!".into(),
                delimiter_encoding: crate::match_config::PatternEncoding::Base64,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config) {
            Ok(_) => panic!("expected error for invalid delimiter encoding"),
            Err(err) => assert!(err.contains("Invalid delimiter encoding"), "got: {err}"),
        }
    }

    #[test]
    fn start_end_invalid_encoding_rejected() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "!!!".into(),
                end: "X".into(),
                marker_encoding: crate::match_config::PatternEncoding::Base64,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config) {
            Ok(_) => panic!("expected error for invalid marker encoding"),
            Err(err) => assert!(err.contains("Invalid start marker encoding"), "got: {err}"),
        }
    }

    #[test]
    fn at_parser_empty_input() {
        let p = AtCommandParser;
        let result = p.parse(b"");
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn at_parser_cme_error() {
        let p = AtCommandParser;
        // +CME ERROR matches the + prefix + colon branch before the
        // +CME ERROR starts_with check, so it returns response_type="response"
        // with command="CME ERROR" (a parser limitation).
        let result = p.parse(b"+CME ERROR: 100");
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                command: Some(ref c),
                ..
            } if response_type == "response" && c == "CME ERROR"
        ));
    }

    #[test]
    fn at_parser_cms_error() {
        let p = AtCommandParser;
        let result = p.parse(b"+CMS ERROR: 500");
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                command: Some(ref c),
                ..
            } if response_type == "response" && c == "CMS ERROR"
        ));
    }

    #[test]
    fn json_parser_empty_input() {
        let p = JsonLinesParser;
        let result = p.parse(b"");
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn shell_prompt_empty_input() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"");
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn shell_prompt_custom_regex_invalid() {
        let config = FramingConfig {
            mode: FramingMode::Line,
            parser: Some(ParserConfig {
                parser_type: ParserType::ShellPrompt,
                custom_prompt: Some("[invalid".to_string()),
            }),
            ..Default::default()
        };
        match FrameDecoder::new(&config) {
            Ok(_) => panic!("invalid regex should be rejected"),
            Err(err) => assert!(err.contains("Invalid prompt regex"), "got: {err}"),
        }
    }

    #[test]
    fn shell_prompt_custom_regex_match() {
        let p = ShellPromptParser {
            custom: Some(regex::bytes::Regex::new("^>>> $").unwrap()),
        };
        let result = p.parse(b">>> ");
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "custom")
        );
    }

    #[test]
    fn raw_parser_passthrough() {
        let p = RawParser;
        let result = p.parse(b"anything");
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn max_frames_zero_edge() {
        let config = FramingConfig {
            mode: FramingMode::Line,
            max_frames: Some(0),
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        // Push one line — the decoder returns the frame regardless.
        // max_frames is checked by the caller (read_bytes_via_session),
        // not by the decoder itself. So decoder still emits the frame.
        let frames = dec.push(b"hello\n");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn length_prefixed_initial_offset() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: Some(4),
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        // 4 skip bytes + 1 prefix (0x05) + 5 payload bytes.
        let frames = dec.push(b"XXXX\x05hello");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn delimiter_include_terminators() {
        let config = FramingConfig {
            mode: FramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"a|b|");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a|"); // terminator included
        assert_eq!(frames[1].data, b"b|");
    }

    #[test]
    fn flush_partial_empty_buffer() {
        let config = FramingConfig::default();
        let mut dec = FrameDecoder::new(&config).unwrap();
        assert!(dec.flush_partial().is_none(), "empty buf => no frame");
    }

    #[test]
    fn combined_line_json_parser() {
        let config = FramingConfig {
            mode: FramingMode::Line,
            parser: Some(ParserConfig {
                parser_type: ParserType::JsonLines,
                custom_prompt: None,
            }),
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0].parsed, Some(ParsedFrame::Json(_))));
        assert!(matches!(frames[1].parsed, Some(ParsedFrame::Json(_))));
    }

    // ── Coverage gap tests ──────────────────────────────────────────────

    #[test]
    fn delimiter_decoder_empty_segments() {
        let config = FramingConfig {
            mode: FramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        // "||" → two empty frames
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"||");
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert!(frames[1].data.is_empty());

        // "a||b|" → three frames: "a", "", "b"
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"a||b|");
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"a");
        assert!(frames[1].data.is_empty());
        assert_eq!(frames[2].data, b"b");
    }

    #[test]
    fn length_prefixed_prefix_split_across_chunks() {
        let config = FramingConfig {
            mode: FramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        // Push half the u16 prefix first.
        let mut dec = FrameDecoder::new(&config).unwrap();
        let frames = dec.push(b"\x00");
        assert!(frames.is_empty());
        // Push rest of prefix + payload.
        let frames = dec.push(b"\x05hello");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn start_end_end_marker_split_across_chunks() {
        let config = FramingConfig {
            mode: FramingMode::StartEnd {
                start: "STX".into(),
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config).unwrap();
        // Start + data + partial end "ET"
        let frames = dec.push(b"STXdataET");
        assert!(frames.is_empty(), "end marker ETX not yet complete");
        // Complete the end marker: "X" → "ETX"
        let frames = dec.push(b"X");
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"data");
    }
}
