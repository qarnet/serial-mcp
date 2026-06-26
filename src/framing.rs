//! Frame boundary detection and protocol parsing for RX and TX streams.
//!
//! Provides a [`FrameDecoder`] that splits a byte stream into structured
//! frames using one of four boundary modes (line, delimiter, length-prefixed,
//! start/end marker). Optional parsers interpret frame content (AT commands,
//! JSON lines, shell prompts). Used as an option on `read` and `subscribe`.
//!
//! Also provides TX framing via [`TxFramingMode`] which encodes payloads
//! with frame boundaries matching the RX modes. Used on `write`.

use crate::codec;
use crate::match_config::PatternEncoding;
use crate::util::find_subsequence;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// ---- RX framing configuration ----------------------------------------------

/// Framing configuration for `read` and `subscribe`.
/// Specifies how to split the byte stream into frames and optionally parse
/// each frame's content.
///
/// The mode fields are flattened into the config struct so the JSON shape is:
/// `{"type": "line", "ending": "auto"}` rather than
/// `{"mode": {"type": "line"}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RxFramingConfig {
    /// Frame boundary detection mode. Flattened: its `type` discriminator and
    /// variant fields appear at the top level of the `rx_framing` object.
    #[serde(flatten)]
    pub mode: RxFramingMode,
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

impl Default for RxFramingConfig {
    fn default() -> Self {
        Self {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
        }
    }
}

/// How frame boundaries are detected in the byte stream.
/// Flattened into [`RxFramingConfig`] via `#[serde(flatten)]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum RxFramingMode {
    /// Split on line endings. Supports `auto` (LF or CRLF), `lf`, `cr`, `crlf`.
    #[serde(rename_all = "snake_case")]
    Line {
        /// Line ending style. Default: `auto` (recognizes LF and CRLF, strips
        /// preceding `\r` when splitting on `\n`).
        #[serde(default)]
        ending: LineEnding,
    },
    /// Split on a user-supplied byte delimiter sequence.
    #[serde(rename_all = "snake_case")]
    Delimiter {
        /// The delimiter as a string (decoded per `delimiter_encoding`).
        delimiter: String,
        /// How to decode the delimiter string into bytes.
        #[serde(default = "default_encoding")]
        delimiter_encoding: PatternEncoding,
    },
    /// Split based on a length prefix field at the start of each frame.
    #[serde(rename_all = "snake_case")]
    LengthPrefixed {
        /// Size of the length prefix field in bytes: 1, 2, or 4.
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
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
    #[serde(rename_all = "snake_case")]
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
    /// SLIP (RFC 1055) framing. Byte-stuffed payloads between END (0xC0) markers.
    #[serde(rename_all = "snake_case")]
    Slip,
}

/// Line ending style for RX line framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum LineEnding {
    /// Adaptive: starts as LF/CRLF (splits on `\n`, strips preceding `\r`).
    /// When a bare `\r` is detected (no `\n` follows in the same chunk), the
    /// decoder enters a pending state. If the next received byte is `\n`, the
    /// `\r\n` is treated as CRLF and decoding continues in LF/CRLF mode. If
    /// the next byte is anything else (including end-of-stream), the `\r` is
    /// confirmed as a bare CR line ending, the pending line is emitted, and
    /// the decoder promotes to CR-split mode for the remainder of the call.
    /// Promotion is per-call (resets on next read/subscribe).
    #[default]
    Auto,
    /// Split on `\n` only. Do NOT strip a preceding `\r`.
    Lf,
    /// Split on bare `\r` only.
    Cr,
    /// Split on exact `\r\n` only.
    Crlf,
}

fn default_encoding() -> PatternEncoding {
    PatternEncoding::Utf8
}

// ---- Protocol presets --------------------------------------------------------

/// Built-in protocol preset. A named bundle of framing/parser primitives
/// that a single `protocol` field expands into on `write`, `read`, and
/// `subscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProtocolPreset {
    /// AT-command modem protocol. TX appends `\r`, RX splits on line
    /// endings (auto), RX frames are parsed as AT command responses/URCs.
    AtCommand,
}

/// The TX framing implied by a protocol preset.
pub fn preset_tx_framing(p: ProtocolPreset) -> TxFramingConfig {
    match p {
        ProtocolPreset::AtCommand => TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Cr,
            },
        },
    }
}

/// The RX framing implied by a protocol preset.
pub fn preset_rx_framing(p: ProtocolPreset) -> RxFramingConfig {
    match p {
        ProtocolPreset::AtCommand => RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
        },
    }
}

/// The RX parser implied by a protocol preset.
pub fn preset_rx_parser(p: ProtocolPreset) -> ParserConfig {
    match p {
        ProtocolPreset::AtCommand => ParserConfig {
            parser_type: ParserType::AtCommand,
            custom_prompt: None,
        },
    }
}

// ---- TX framing configuration -----------------------------------------------

/// TX framing configuration for `write`.
/// Mirrors the RX modes but directionally appropriate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TxFramingConfig {
    /// TX frame boundary mode. Flattened: its `type` discriminator and
    /// variant fields appear at the top level of the `tx_framing` object.
    #[serde(flatten)]
    pub mode: TxFramingMode,
}

/// How TX frames are constructed around a payload.
/// Flattened into [`TxFramingConfig`] via `#[serde(flatten)]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum TxFramingMode {
    /// Append a line terminator. No `auto` — agents must be explicit.
    #[serde(rename_all = "snake_case")]
    Line {
        /// Line ending to append: `lf`, `cr`, or `crlf`.
        ending: TxLineEnding,
    },
    /// Append a delimiter byte sequence after the payload.
    #[serde(rename_all = "snake_case")]
    Delimiter {
        /// The delimiter as a string (decoded per `delimiter_encoding`).
        delimiter: String,
        /// How to decode the delimiter string into bytes.
        #[serde(default = "default_encoding")]
        delimiter_encoding: PatternEncoding,
    },
    /// Prepend a length prefix encoding the payload length, then the payload.
    #[serde(rename_all = "snake_case")]
    LengthPrefixed {
        /// Size of the length prefix field in bytes: 1, 2, or 4.
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        prefix_size: u8,
        /// Byte order of the length prefix.
        #[serde(default)]
        endianness: Endianness,
    },
    /// Write start marker, payload, end marker.
    #[serde(rename_all = "snake_case")]
    StartEnd {
        /// Start marker (decoded per `marker_encoding`).
        start: String,
        /// End marker (decoded per `marker_encoding`).
        end: String,
        /// How to decode the marker strings into bytes.
        #[serde(default = "default_encoding")]
        marker_encoding: PatternEncoding,
    },
    /// SLIP (RFC 1055) framing. Encodes as `END [stuffed payload] END`.
    #[serde(rename_all = "snake_case")]
    Slip,
}

/// Line ending for TX framing. No `auto` — agents must pick one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TxLineEnding {
    /// Append `\n` (LF).
    Lf,
    /// Append `\r` (CR).
    Cr,
    /// Append `\r\n` (CRLF).
    Crlf,
}

impl TxFramingMode {
    /// Encode a decoded payload by applying this TX framing mode.
    /// Returns the framed bytes to send to the UART.
    pub fn encode(&self, payload: &[u8]) -> Result<Vec<u8>, String> {
        match self {
            TxFramingMode::Line { ending } => {
                let mut framed = payload.to_vec();
                match ending {
                    TxLineEnding::Lf => framed.push(b'\n'),
                    TxLineEnding::Cr => framed.push(b'\r'),
                    TxLineEnding::Crlf => framed.extend_from_slice(b"\r\n"),
                }
                Ok(framed)
            }
            TxFramingMode::Delimiter {
                delimiter,
                delimiter_encoding,
            } => {
                let delim_bytes = codec::decode((*delimiter_encoding).into(), delimiter)
                    .map_err(|e| format!("Invalid TX delimiter encoding: {e}"))?;
                if delim_bytes.is_empty() {
                    return Err("TX delimiter must not be empty".into());
                }
                let mut framed = payload.to_vec();
                framed.extend_from_slice(&delim_bytes);
                Ok(framed)
            }
            TxFramingMode::LengthPrefixed {
                prefix_size,
                endianness,
            } => {
                if !matches!(prefix_size, 1 | 2 | 4) {
                    return Err("TX prefix_size must be 1, 2, or 4".into());
                }
                let len = payload.len();
                let mut framed = Vec::with_capacity(*prefix_size as usize + len);
                match (prefix_size, endianness) {
                    (1, _) => {
                        if len > 255 {
                            return Err(format!(
                                "TX payload length {len} exceeds maximum 255 for prefix_size=1"
                            ));
                        }
                        framed.push(len as u8);
                    }
                    (2, Endianness::Big) => {
                        if len > 65535 {
                            return Err(format!(
                                "TX payload length {len} exceeds maximum 65535 for prefix_size=2"
                            ));
                        }
                        framed.extend_from_slice(&(len as u16).to_be_bytes());
                    }
                    (2, Endianness::Little) => {
                        if len > 65535 {
                            return Err(format!(
                                "TX payload length {len} exceeds maximum 65535 for prefix_size=2"
                            ));
                        }
                        framed.extend_from_slice(&(len as u16).to_le_bytes());
                    }
                    (4, Endianness::Big) => {
                        framed.extend_from_slice(&(len as u32).to_be_bytes());
                    }
                    (4, Endianness::Little) => {
                        framed.extend_from_slice(&(len as u32).to_le_bytes());
                    }
                    _ => unreachable!("prefix_size validated above"),
                }
                framed.extend_from_slice(payload);
                Ok(framed)
            }
            TxFramingMode::StartEnd {
                start,
                end,
                marker_encoding,
            } => {
                let start_bytes = codec::decode((*marker_encoding).into(), start)
                    .map_err(|e| format!("Invalid TX start marker encoding: {e}"))?;
                let end_bytes = codec::decode((*marker_encoding).into(), end)
                    .map_err(|e| format!("Invalid TX end marker encoding: {e}"))?;
                if start_bytes.is_empty() || end_bytes.is_empty() {
                    return Err("TX start and end markers must not be empty".into());
                }
                let mut framed =
                    Vec::with_capacity(start_bytes.len() + payload.len() + end_bytes.len());
                framed.extend_from_slice(&start_bytes);
                framed.extend_from_slice(payload);
                framed.extend_from_slice(&end_bytes);
                Ok(framed)
            }
            TxFramingMode::Slip => {
                let mut framed = vec![SLIP_END];
                framed.extend_from_slice(&slip_stuff(payload));
                framed.push(SLIP_END);
                Ok(framed)
            }
        }
    }
}

// ---- Shared types -----------------------------------------------------------

/// Byte order for length-prefixed framing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Endianness {
    #[default]
    Big,
    Little,
}

/// Parser configuration — what to do with each frame's content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    Line(LineState),
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
    Slip {
        state: SlipState,
    },
}

/// Internal line-decoder state.
///
/// `Lf`, `Cr`, `Crlf` are terminal (no promotion). `AutoLf` is the starting
/// state for `ending: auto`; it can transition to `PendingCr` when a bare `\r`
/// is detected, and from there to `CrMode` when the `\r` is confirmed not to
/// be part of a CRLF sequence.
enum LineState {
    Lf,
    Cr,
    Crlf,
    /// `auto` initial state: split on `\n`, strip preceding `\r` (CRLF-aware).
    AutoLf,
    /// Saw a `\r` at end of buffer with no trailing `\n` yet. Stores the index
    /// of the pending `\r` in `self.buf` at the moment of transition. Waiting
    /// for next byte to decide if it's CRLF or bare CR.
    PendingCr(usize),
    /// Promoted: split on bare `\r` only, ignore `\n` for the rest of the call.
    CrMode,
}

/// SLIP decoder state.
#[derive(Debug, Clone)]
enum SlipState {
    /// Discard bytes until the first END marker is seen.
    BeforeFirstEnd,
    /// Inside a frame. `buf` accumulates decoded payload bytes. `escaped` is
    /// true after an `ESC` byte — the next byte is the escape code.
    InFrame { buf: Vec<u8>, escaped: bool },
}

trait FrameParser: Send + Sync {
    fn parse(&self, data: &[u8]) -> ParsedFrame;
}

/// SLIP decoder: consume `buf_outer` byte-by-byte according to current
/// [`SlipState`] in `mode`. Returns decoded frames, or `Err` for a
/// malformed escape sequence. Updates `mode` state in-place.
fn slip_decode(
    buf_outer: &mut Vec<u8>,
    frame_count: &mut usize,
    parser: &Option<Box<dyn FrameParser>>,
    mode: &mut DecoderMode,
) -> Result<Vec<Frame>, FrameDecodeError> {
    let mut frames = Vec::new();
    let state = match mode {
        DecoderMode::Slip { ref mut state } => state,
        _ => return Ok(frames),
    };

    loop {
        match state {
            SlipState::BeforeFirstEnd => {
                if let Some(pos) = buf_outer.iter().position(|&b| b == SLIP_END) {
                    buf_outer.drain(..=pos);
                    *state = SlipState::InFrame {
                        buf: Vec::new(),
                        escaped: false,
                    };
                    continue;
                }
                buf_outer.clear();
                return Ok(frames);
            }
            SlipState::InFrame {
                ref mut buf,
                ref mut escaped,
            } => {
                let mut read_pos: usize = 0;
                while read_pos < buf_outer.len() {
                    let b = buf_outer[read_pos];
                    read_pos += 1;

                    if *escaped {
                        match b {
                            SLIP_ESC_END => {
                                buf.push(SLIP_END);
                                *escaped = false;
                            }
                            SLIP_ESC_ESC => {
                                buf.push(SLIP_ESC);
                                *escaped = false;
                            }
                            _ => {
                                // Malformed escape: clear in-progress frame,
                                // reset escaped flag, and resync on next END.
                                // Drain only the consumed bytes (up to and
                                // including the malformed byte via read_pos);
                                // leave the remainder of buf_outer intact so
                                // BeforeFirstEnd can scan/discard it on the
                                // next push.
                                buf_outer.drain(..read_pos);
                                buf.clear();
                                *escaped = false;
                                *state = SlipState::BeforeFirstEnd;
                                return Err(FrameDecodeError::SlipInvalidEscape(b));
                            }
                        }
                    } else {
                        match b {
                            SLIP_END => {
                                let data = std::mem::take(buf);
                                *frame_count += 1;
                                let parsed = parser.as_ref().map(|p| p.parse(&data));
                                frames.push(Frame {
                                    data,
                                    index: *frame_count - 1,
                                    frame_type: "slip".into(),
                                    parsed,
                                });
                            }
                            SLIP_ESC => {
                                *escaped = true;
                            }
                            _ => {
                                buf.push(b);
                            }
                        }
                    }
                }
                // Consumed the whole buffer without hitting a terminal
                // return: drain everything read and fall through to the
                // outer loop.
                buf_outer.drain(..read_pos);
                return Ok(frames);
            }
        }
    }
}

// ---- Frame decoder implementation ------------------------------------------

impl FrameDecoder {
    /// Create a new frame decoder from an RX framing configuration and
    /// an optional parser configuration.
    pub fn new(
        config: &RxFramingConfig,
        parser_config: Option<&ParserConfig>,
    ) -> Result<Self, String> {
        let mode = match &config.mode {
            RxFramingMode::Line { ending } => {
                let state = match ending {
                    LineEnding::Auto => LineState::AutoLf,
                    LineEnding::Lf => LineState::Lf,
                    LineEnding::Cr => LineState::Cr,
                    LineEnding::Crlf => LineState::Crlf,
                };
                DecoderMode::Line(state)
            }
            RxFramingMode::Delimiter {
                delimiter,
                delimiter_encoding,
            } => {
                let bytes = codec::decode((*delimiter_encoding).into(), delimiter)
                    .map_err(|e| format!("Invalid delimiter encoding: {e}"))?;
                if bytes.is_empty() {
                    return Err("Delimiter must not be empty".into());
                }
                DecoderMode::Delimiter(bytes)
            }
            RxFramingMode::LengthPrefixed {
                prefix_size,
                endianness,
                initial_offset,
            } => {
                if !matches!(prefix_size, 1 | 2 | 4) {
                    return Err("prefix_size must be 1, 2, or 4".into());
                }
                DecoderMode::LengthPrefixed {
                    prefix_size: *prefix_size,
                    endianness: *endianness,
                    remaining_offset: initial_offset.unwrap_or(0),
                    next_payload_len: None,
                }
            }
            RxFramingMode::StartEnd {
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
                DecoderMode::StartEnd {
                    start: start_bytes,
                    end: end_bytes,
                    include_markers: *include_markers,
                    in_frame: false,
                }
            }
            RxFramingMode::Slip => DecoderMode::Slip {
                state: SlipState::BeforeFirstEnd,
            },
        };

        let parser: Option<Box<dyn FrameParser>> = match parser_config {
            Some(pc) => Some(build_parser(pc)?),
            None => None,
        };

        let buf = Vec::new();

        Ok(Self {
            buf,
            mode,
            frame_count: 0,
            include_terminators: config.include_terminators,
            parser,
        })
    }

    /// Feed a chunk of bytes. Returns any complete frames found, or a
    /// [`FrameDecodeError`] for protocol violations (SLIP malformed escape).
    /// The caller is responsible for draining consumed bytes from their
    /// accumulation buffer.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, FrameDecodeError> {
        self.buf.extend_from_slice(chunk);
        // SLIP is handled separately via a free function to avoid borrow
        // conflicts between the mutable mode borrow and self.
        if matches!(self.mode, DecoderMode::Slip { .. }) {
            let frames = slip_decode(
                &mut self.buf,
                &mut self.frame_count,
                &self.parser,
                &mut self.mode,
            )?;
            return Ok(frames);
        }
        let mut frames = Vec::new();
        loop {
            let consumed = match &mut self.mode {
                DecoderMode::Line(state) => match state {
                    LineState::Lf => self.match_line_lf(),
                    LineState::Cr => self.match_line_cr(),
                    LineState::Crlf => self.match_line_crlf(),
                    LineState::AutoLf => self.match_auto_lf(),
                    LineState::PendingCr(_) => self.match_pending_cr(),
                    LineState::CrMode => self.match_line_cr(),
                },
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
                        None => break,
                    };
                    let header_len = *prefix_size as usize;
                    let total_needed = header_len + payload_len;
                    if self.buf.len() < total_needed {
                        break;
                    }
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
                        if let Some(pos) = find_subsequence(&self.buf, &start) {
                            self.buf.drain(..pos);
                            if !include {
                                self.buf.drain(..start.len());
                            }
                            *in_frame = true;
                        } else {
                            let keep = start.len().saturating_sub(1);
                            if self.buf.len() > keep {
                                self.buf.drain(..(self.buf.len() - keep));
                            }
                            break;
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
                            break;
                        }
                    } else {
                        None
                    }
                }
                DecoderMode::Slip { .. } => unreachable!("SLIP handled before match"),
            };

            match consumed {
                None => break,
                Some(frame_bytes) => {
                    self.frame_count += 1;
                    let parsed = self.parser.as_ref().map(|p| p.parse(&frame_bytes));
                    let frame_type = self.frame_type_str();
                    frames.push(Frame {
                        data: frame_bytes,
                        index: self.frame_count - 1,
                        frame_type,
                        parsed,
                    });
                }
            }
        }
        Ok(frames)
    }

    /// `auto` initial state: scan for `\n` (CRLF-aware), detect bare `\r`.
    ///
    /// If a `\r` is found at the end of the buffer with no `\n` following it in
    /// the same chunk, transitions to [`LineState::PendingCr`] and returns
    /// `None` to wait for more data. If a `\r` is immediately followed by a
    /// non-`\n` byte (bare CR confirmed in same chunk), transitions directly to
    /// [`LineState::CrMode`] and returns the frame before the `\r`.
    fn match_auto_lf(&mut self) -> Option<Vec<u8>> {
        // Scan for \n first — preserves existing eager-LF behavior.
        if let Some(lf_pos) = self.buf.iter().position(|&b| b == b'\n') {
            // Check if there's a bare \r before this \n that hasn't been
            // consumed yet. If no \r precedes this \n, or only the \r
            // immediately before \n, handle normally.
            // Walk backwards from lf_pos to check for any earlier \r that
            // isn't part of a CRLF.
            let end = if !self.include_terminators && lf_pos > 0 && self.buf[lf_pos - 1] == b'\r' {
                lf_pos - 1
            } else {
                lf_pos
            };
            let fb = if self.include_terminators {
                self.buf[..lf_pos + 1].to_vec()
            } else {
                self.buf[..end].to_vec()
            };
            self.buf.drain(..lf_pos + 1);
            return Some(fb);
        }

        // No \n found. Look for a bare \r.
        if let Some(cr_pos) = self.buf.iter().position(|&b| b == b'\r') {
            let next_is_lf = self.buf.get(cr_pos + 1) == Some(&b'\n');
            if next_is_lf {
                // CRLF: \n is in the buffer right after \r. This means \n was
                // found above. Unreachable in practice, but safe.
                let fb = if self.include_terminators {
                    self.buf[..cr_pos + 2].to_vec()
                } else {
                    self.buf[..cr_pos].to_vec()
                };
                self.buf.drain(..cr_pos + 2);
                return Some(fb);
            }
            // \r found, no \n follows in the buffer.
            if cr_pos + 1 < self.buf.len() {
                // Bytes after \r exist in this chunk → bare CR confirmed immediately.
                // Emit the line before \r, drain through \r, transition to CrMode.
                let fb = if self.include_terminators {
                    self.buf[..cr_pos + 1].to_vec()
                } else {
                    self.buf[..cr_pos].to_vec()
                };
                self.buf.drain(..cr_pos + 1);
                if let DecoderMode::Line(ref mut state) = self.mode {
                    *state = LineState::CrMode;
                }
                return Some(fb);
            }
            // \r is the last byte in buffer → transition to PendingCr.
            if let DecoderMode::Line(ref mut state) = self.mode {
                *state = LineState::PendingCr(cr_pos);
            }
            return None;
        }

        // No \n, no \r. Wait for more data.
        None
    }

    /// `PendingCr` state: buffer has a `\r` at a known position. The next byte
    /// after that `\r` decides.
    ///
    /// On next non-`\n` byte → bare CR confirmed, emit frame before `\r`,
    /// drain through `\r`, promote to [`LineState::CrMode`].
    /// On `\n` → CRLF, emit frame before `\r\n`, drain through `\r\n`, return
    /// to [`LineState::AutoLf`].
    fn match_pending_cr(&mut self) -> Option<Vec<u8>> {
        let cr_pos = match self.mode {
            DecoderMode::Line(LineState::PendingCr(pos)) => pos,
            _ => {
                // Shouldn't happen. Reset to AutoLf.
                if let DecoderMode::Line(ref mut state) = self.mode {
                    *state = LineState::AutoLf;
                }
                return None;
            }
        };

        if cr_pos + 1 >= self.buf.len() {
            // \r is still the last byte. Wait for more data.
            return None;
        }

        let next_byte = self.buf[cr_pos + 1];
        if next_byte == b'\n' {
            // CRLF confirmed. Emit frame (strip \r\n unless include_terminators).
            let fb = if self.include_terminators {
                self.buf[..cr_pos + 2].to_vec()
            } else {
                self.buf[..cr_pos].to_vec()
            };
            self.buf.drain(..cr_pos + 2);
            if let DecoderMode::Line(ref mut state) = self.mode {
                *state = LineState::AutoLf;
            }
            return Some(fb);
        }

        // Non-\n byte after \r → bare CR confirmed.
        // Emit frame before \r, drain through \r, promote to CrMode.
        let fb = if self.include_terminators {
            self.buf[..cr_pos + 1].to_vec()
        } else {
            self.buf[..cr_pos].to_vec()
        };
        self.buf.drain(..cr_pos + 1);
        if let DecoderMode::Line(ref mut state) = self.mode {
            *state = LineState::CrMode;
        }
        Some(fb)
    }

    /// Match a line with `lf` ending: split on `\n` only, do NOT strip `\r`.
    fn match_line_lf(&mut self) -> Option<Vec<u8>> {
        let pos = self.buf.iter().position(|&b| b == b'\n')?;
        let fb = if self.include_terminators {
            self.buf[..pos + 1].to_vec()
        } else {
            self.buf[..pos].to_vec()
        };
        self.buf.drain(..pos + 1);
        Some(fb)
    }

    /// Match a line with `cr` ending: split on bare `\r`.
    fn match_line_cr(&mut self) -> Option<Vec<u8>> {
        let pos = self.buf.iter().position(|&b| b == b'\r')?;
        let fb = if self.include_terminators {
            self.buf[..pos + 1].to_vec()
        } else {
            self.buf[..pos].to_vec()
        };
        self.buf.drain(..pos + 1);
        Some(fb)
    }

    /// Match a line with `crlf` ending: split on exact `\r\n`.
    fn match_line_crlf(&mut self) -> Option<Vec<u8>> {
        let pos = find_subsequence(&self.buf, b"\r\n")?;
        let fb = if self.include_terminators {
            self.buf[..pos + 2].to_vec()
        } else {
            self.buf[..pos].to_vec()
        };
        self.buf.drain(..pos + 2);
        Some(fb)
    }

    fn frame_type_str(&self) -> String {
        match &self.mode {
            DecoderMode::Line(_) => "line".into(),
            DecoderMode::Delimiter(_) => "delimiter".into(),
            DecoderMode::LengthPrefixed { .. } => "length_prefixed".into(),
            DecoderMode::StartEnd { .. } => "start_end".into(),
            DecoderMode::Slip { .. } => "slip".into(),
        }
    }

    /// Bytes pending in the incomplete frame buffer.
    pub fn pending_len(&self) -> usize {
        self.buf.len()
    }

    /// Flush any remaining bytes as a partial frame. For SLIP, this drains
    /// the in-frame buffer; pending escaped state is emitted as raw bytes.
    pub fn flush_partial(&mut self) -> Option<Frame> {
        // SLIP: drain the in-frame buffer instead of self.buf.
        if let DecoderMode::Slip {
            state: SlipState::InFrame { ref mut buf, .. },
        } = self.mode
        {
            if buf.is_empty() {
                return None;
            }
            let data = std::mem::take(buf);
            self.frame_count += 1;
            return Some(Frame {
                data,
                index: self.frame_count - 1,
                frame_type: "slip".into(),
                parsed: None,
            });
        }
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
            Ok(val) if val.is_object() => ParsedFrame::Json(val),
            _ => ParsedFrame::Raw,
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
        if let Some(ref re) = self.custom {
            if re.is_match(data) {
                return ParsedFrame::ShellPrompt {
                    prompt: trimmed.to_string(),
                    prompt_type: "custom".into(),
                };
            }
        }
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

/// SLIP framing error: a malformed escape sequence was encountered during
/// RX decode. Construction errors (e.g. invalid delimiter) are synchronous
/// and configurable by the agent; runtime decode errors like this indicate
/// stream corruption and are not recoverable by retrying the same bytes.
#[derive(Debug, Clone)]
pub enum FrameDecodeError {
    /// SLIP `ESC` (0xDB) followed by an invalid byte (not `ESC_END` 0xDC
    /// or `ESC_ESC` 0xDD).
    SlipInvalidEscape(u8),
}

impl std::fmt::Display for FrameDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameDecodeError::SlipInvalidEscape(b) => {
                write!(f, "SLIP framing error: invalid escape byte 0x{b:02X}")
            }
        }
    }
}

impl std::error::Error for FrameDecodeError {}

// ---- SLIP (RFC 1055) constants and codec ------------------------------------

const SLIP_END: u8 = 0xC0;
const SLIP_ESC: u8 = 0xDB;
const SLIP_ESC_END: u8 = 0xDC;
const SLIP_ESC_ESC: u8 = 0xDD;

/// Byte-stuff a payload for SLIP TX framing. Replaces `END` (0xC0) with
/// `ESC ESC_END` and `ESC` (0xDB) with `ESC ESC_ESC`. All other bytes pass
/// through unchanged. The caller wraps the result in `END` markers.
fn slip_stuff(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + payload.len() / 10);
    for &b in payload {
        match b {
            SLIP_END => {
                out.push(SLIP_ESC);
                out.push(SLIP_ESC_END);
            }
            SLIP_ESC => {
                out.push(SLIP_ESC);
                out.push(SLIP_ESC_ESC);
            }
            _ => out.push(b),
        }
    }
    out
}

/// Read a length prefix from the given bytes.
/// `prefix_size` must be 1, 2, or 4 (validated at construction).
/// Returns 0 for invalid sizes as a safe fallback.
fn read_length_prefix(bytes: &[u8], prefix_size: u8, endianness: Endianness) -> usize {
    match prefix_size {
        1 => bytes[0] as usize,
        2 => {
            let arr: [u8; 2] = bytes[..2].try_into().unwrap_or([0; 2]);
            match endianness {
                Endianness::Big => u16::from_be_bytes(arr) as usize,
                Endianness::Little => u16::from_le_bytes(arr) as usize,
            }
        }
        4 => {
            let arr: [u8; 4] = bytes[..4].try_into().unwrap_or([0; 4]);
            match endianness {
                Endianness::Big => u32::from_be_bytes(arr) as usize,
                Endianness::Little => u32::from_le_bytes(arr) as usize,
            }
        }
        _ => {
            // Invalid prefix_size — should never happen because
            // FrameDecoder::new() rejects sizes other than 1/2/4.
            0
        }
    }
}

// ---- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Line decoder (auto — original behavior) ──────────────────────────

    #[test]
    fn line_decoder_single_line() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[0].frame_type, "line");
    }

    #[test]
    fn line_decoder_crlf() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\nworld\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[1].data, b"world");
    }

    #[test]
    fn line_decoder_partial_across_chunks() {
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hel").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"lo\nwor").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
        let frames = dec.push(b"ld\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"world");
    }

    #[test]
    fn line_decoder_empty_lines() {
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\n\n\n").unwrap();
        assert_eq!(frames.len(), 3);
        for f in &frames {
            assert!(f.data.is_empty());
        }
    }

    #[test]
    fn line_decoder_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello\r\n");
    }

    // ── Line decoder: new ending modes ────────────────────────────────────

    #[test]
    fn line_decoder_lf_preserves_cr() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Lf,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello\r");
        assert_eq!(frames[0].frame_type, "line");
    }

    #[test]
    fn line_decoder_cr_splits_on_bare_cr() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Cr,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\rb\r").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
    }

    #[test]
    fn line_decoder_cr_with_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Cr,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\r").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a\r");
    }

    #[test]
    fn line_decoder_crlf_exact_only() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        // CRLF split across chunks: "\r" in first, "\n" in second.
        let frames = dec.push(b"a\r").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"\nb").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
    }

    #[test]
    fn line_decoder_crlf_no_split_on_bare_cr() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\rb\n").unwrap();
        assert_eq!(frames.len(), 0, "bare \\r should not split in crlf mode");
    }

    #[test]
    fn line_decoder_crlf_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello\r\n");
    }

    // ── Line ending default: `auto` when `ending` omitted ────────────────

    #[test]
    fn line_ending_default_is_auto() {
        // When Line is constructed without `ending`, it should default to Auto.
        let val = serde_json::json!({"type": "line"});
        let mode: RxFramingMode = serde_json::from_value(val).unwrap();
        assert!(matches!(
            mode,
            RxFramingMode::Line {
                ending: LineEnding::Auto
            }
        ));
    }

    // ── Line decoder: auto promotion (bare-CR detection) ──────────────────

    fn auto_config() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        }
    }

    fn auto_config_include_terms() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            include_terminators: true,
            ..Default::default()
        }
    }

    #[test]
    fn auto_does_not_promote_on_crlf() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        let frames = dec.push(b"a\r\nb\r\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
        // After CRLF, decoder stays in AutoLf — next bare \r still triggers promotion.
        let frames = dec.push(b"c\r").unwrap();
        assert!(frames.is_empty(), "pending CR after CRLF");
        // Push "d" — confirmation byte stays buffered as start of next line.
        let frames = dec.push(b"d").unwrap();
        assert_eq!(frames.len(), 1, "bare CR confirmed on next non-\\n byte");
        assert_eq!(frames[0].data, b"c");
        // Now in CrMode. Buffer has "d". Push "e\r" → "de\r" → frame "de".
        let frames = dec.push(b"e\r").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"de");
    }

    #[test]
    fn auto_does_not_promote_on_lf() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        let frames = dec.push(b"a\nb\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
    }

    #[test]
    fn auto_promotes_on_next_non_lf_byte() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Push "line1\r" → \r at end, no frame emitted, enters PendingCr.
        let frames = dec.push(b"line1\r").unwrap();
        assert!(frames.is_empty());
        // Push "x" → non-\n byte confirms bare CR. Emit "line1", enter CrMode.
        // The "x" stays buffered as the start of the next line.
        let frames = dec.push(b"x").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"line1");
        // In CrMode: push "more\r" → split on \r. Buffer had "x" + "more\r".
        let frames = dec.push(b"more\r").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"xmore");
    }

    #[test]
    fn auto_crlf_after_pending_cr_cancels_promotion() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Push "a\r" → pending CR.
        let frames = dec.push(b"a\r").unwrap();
        assert!(frames.is_empty());
        // Push "\nb" → \n arrives, CRLF recognized. "b" stays buffered.
        let frames = dec.push(b"\nb").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
        // Back to AutoLf. Buffer has "b". Push "c\n" → "bc\n" → frame "bc".
        let frames = dec.push(b"c\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"bc");
    }

    #[test]
    fn auto_flush_partial_emits_pending_cr() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        let frames = dec.push(b"tail\r").unwrap();
        assert!(frames.is_empty());
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.data, b"tail\r");
        assert_eq!(partial.frame_type, "line");
    }

    #[test]
    fn auto_flush_partial_emits_pending_cr_include_terminators() {
        let mut dec = FrameDecoder::new(&auto_config_include_terms(), None).unwrap();
        let frames = dec.push(b"tail\r").unwrap();
        assert!(frames.is_empty());
        let partial = dec.flush_partial().expect("partial frame");
        // include_terminators=true → the \r is included (already in buffer).
        assert_eq!(partial.data, b"tail\r");
    }

    #[test]
    fn auto_promotes_and_stays_in_cr_mode() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Promote to CrMode.
        dec.push(b"a\r").unwrap();
        dec.push(b"b").unwrap();
        // In CrMode: \n is NOT a terminator, \r is.
        // Buffer has "b" from confirmation, then "x\ny\r" → "bx\ny\r" → split on \r.
        let frames = dec.push(b"x\ny\r").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"bx\ny");
    }

    #[test]
    fn auto_promotion_include_terminators() {
        let mut dec = FrameDecoder::new(&auto_config_include_terms(), None).unwrap();
        let frames = dec.push(b"line1\r").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"x").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"line1\r");
    }

    #[test]
    fn auto_pending_cr_then_flush_keeps_frame_index_monotonic() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Two LF lines.
        let frames = dec.push(b"a\nb\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[1].index, 1);
        // Pending CR, then flush.
        dec.push(b"c\r").unwrap();
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.index, 2);
    }

    // ── Protocol preset tests ──────────────────────────────────────────────

    #[test]
    fn preset_tx_framing_returns_line_cr() {
        let cfg = preset_tx_framing(ProtocolPreset::AtCommand);
        assert!(matches!(
            cfg.mode,
            TxFramingMode::Line {
                ending: TxLineEnding::Cr
            }
        ));
    }

    #[test]
    fn preset_rx_framing_returns_line_auto() {
        let cfg = preset_rx_framing(ProtocolPreset::AtCommand);
        assert!(matches!(
            cfg.mode,
            RxFramingMode::Line {
                ending: LineEnding::Auto
            }
        ));
    }

    #[test]
    fn preset_rx_parser_returns_at_command() {
        let cfg = preset_rx_parser(ProtocolPreset::AtCommand);
        assert_eq!(cfg.parser_type, ParserType::AtCommand);
    }

    #[test]
    fn protocol_preset_tagged_object_roundtrip() {
        let json = serde_json::json!({ "type": "at_command" });
        let p: ProtocolPreset = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(p, ProtocolPreset::AtCommand);
        let back = serde_json::to_value(p).unwrap();
        assert_eq!(back, json, "must round-trip as tagged object");
        // Bare string form must be rejected.
        assert!(
            serde_json::from_value::<ProtocolPreset>(serde_json::json!("at_command")).is_err(),
            "bare string form must be rejected after adding tag"
        );
    }

    // ── SLIP (RFC 1055) tests ─────────────────────────────────────────────

    #[test]
    fn slip_stuff_replaces_end_and_esc() {
        let payload = &[SLIP_END, SLIP_ESC, 0x41];
        let stuffed = slip_stuff(payload);
        assert_eq!(
            stuffed,
            &[SLIP_ESC, SLIP_ESC_END, SLIP_ESC, SLIP_ESC_ESC, 0x41]
        );
    }

    #[test]
    fn tx_slip_encodes_end_end() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(b"hi").unwrap();
        assert_eq!(framed, &[SLIP_END, b'h', b'i', SLIP_END]);
    }

    #[test]
    fn tx_slip_stuffs_payload_with_end() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(&[SLIP_END]).unwrap();
        assert_eq!(framed, &[SLIP_END, SLIP_ESC, SLIP_ESC_END, SLIP_END]);
    }

    #[test]
    fn tx_slip_stuffs_payload_with_esc() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(&[SLIP_ESC]).unwrap();
        assert_eq!(framed, &[SLIP_END, SLIP_ESC, SLIP_ESC_ESC, SLIP_END]);
    }

    fn slip_rx_config() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Slip,
            ..Default::default()
        }
    }

    #[test]
    fn rx_slip_skips_to_first_end() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"junk\xC0hi\xC0").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");
    }

    #[test]
    fn rx_slip_decodes_basic_frame() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0hello\xC0").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn rx_slip_decodes_esc_end() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0\xDB\xDC\xC0").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"\xC0");
    }

    #[test]
    fn rx_slip_decodes_esc_esc() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0\xDB\xDD\xC0").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"\xDB");
    }

    #[test]
    fn rx_slip_malformed_escape_returns_err() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let result = dec.push(b"\xC0\xDB\x41\xC0");
        match result {
            Ok(_) => panic!("expected decode error"),
            Err(FrameDecodeError::SlipInvalidEscape(b)) => assert_eq!(b, 0x41),
        }
    }

    #[test]
    fn rx_slip_resyncs_after_malformed_escape() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        // Malformed escape.
        let result = dec.push(b"\xC0\xDB\x41\xC0");
        assert!(result.is_err());
        // After resync, decoder is in BeforeFirstEnd. The trailing END from
        // the malformed chunk remains in buf_outer. Push a valid frame —
        // two consecutive ENDs produce one empty frame then "ok".
        let frames = dec.push(b"\xC0ok\xC0").unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert_eq!(frames[1].data, b"ok");
    }

    #[test]
    fn rx_slip_resync_clears_stale_in_progress_buf() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        // Partial frame "hello", then malformed escape.
        let result = dec.push(b"\xC0hello\xDB\x41");
        assert!(result.is_err());
        // After resync, "hello" must be cleared. Push a new frame.
        let frames = dec.push(b"\xC0world\xC0").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"world");
    }

    #[test]
    fn rx_slip_two_frames_in_one_chunk() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0aa\xC0bb\xC0").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"aa");
        assert_eq!(frames[1].data, b"bb");
    }

    #[test]
    fn rx_slip_cross_chunk_frame() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0hel").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"lo\xC0").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn rx_slip_truncated_escape_holds_pending() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0\xDB").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"\xDC\xC0").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"\xC0");
    }

    #[test]
    fn rx_slip_flush_partial_emits_pending() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0hel").unwrap();
        assert!(frames.is_empty());
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.data, b"hel");
        assert_eq!(partial.frame_type, "slip");
    }

    #[test]
    fn roundtrip_slip_arbitrary_binary() {
        let payload: &[u8] = &[SLIP_END, SLIP_ESC, 0x41, SLIP_ESC, SLIP_ESC_END, SLIP_END];
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_slip_empty_payload() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(b"").unwrap();
        assert_eq!(framed, &[SLIP_END, SLIP_END]);
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.is_empty());
    }

    #[test]
    fn rx_slip_decodes_large_payload_preserves_bytes() {
        // Push a 4096-byte SLIP frame with a known repeating pattern
        // (0x00..=0xFF cycling) stuffed via slip_stuff, wrapped in END
        // markers. Assert exactly one decoded frame whose payload matches
        // the original byte-for-byte, and frame_type == "slip".
        let payload: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
        let stuffed = slip_stuff(&payload);
        let mut framed = vec![SLIP_END];
        framed.extend_from_slice(&stuffed);
        framed.push(SLIP_END);

        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1, "expected exactly one frame");
        assert_eq!(
            frames[0].data, payload,
            "decoded payload must match original"
        );
        assert_eq!(frames[0].frame_type, "slip");
    }

    // ── Delimiter decoder ───────────────────────────────────────────────

    #[test]
    fn delimiter_decoder_basic() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a|b|c|").unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
        assert_eq!(frames[2].data, b"c");
    }

    #[test]
    fn delimiter_decoder_multi_byte() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "AA".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"xAAyAAz").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"x");
        assert_eq!(frames[1].data, b"y");
    }

    #[test]
    fn delimiter_decoder_partial_delimiter() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "AB".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"xA").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"By").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"x");
    }

    // ── Length-prefixed decoder ─────────────────────────────────────────

    #[test]
    fn length_prefixed_basic() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x05hello\x02wo\x02rb").unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[1].data, b"wo");
        assert_eq!(frames[2].data, b"rb");
    }

    #[test]
    fn length_prefixed_u16_big_endian() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let mut buf = vec![0x00, 0x05];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    // ── Start/end marker decoder ────────────────────────────────────────

    #[test]
    fn start_end_basic() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "STX".into(),
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"noiseSTXdataETXjunkSTXmoreETX").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"data");
        assert_eq!(frames[1].data, b"more");
    }

    #[test]
    fn start_end_include_markers() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "<".into(),
                end: ">".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: true,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"<data>").unwrap();
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
    fn json_parser_non_object_is_raw() {
        let p = JsonLinesParser;
        assert!(matches!(p.parse(b"[1,2,3]"), ParsedFrame::Raw));
        assert!(matches!(p.parse(b"42"), ParsedFrame::Raw));
        assert!(matches!(p.parse(b"\"hi\""), ParsedFrame::Raw));
        assert!(matches!(p.parse(b"{\"k\":1}"), ParsedFrame::Json(_)));
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
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::AtCommand,
            custom_prompt: None,
        };
        let mut dec = FrameDecoder::new(&config, Some(&parser)).unwrap();
        let frames = dec.push(b"OK\nERROR\n+CGREG: 0,1\n").unwrap();
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
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello").unwrap();
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
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("empty delimiter should be rejected"),
            Err(err) => assert!(err.contains("Delimiter must not be empty"), "got: {err}"),
        }
    }

    #[test]
    fn length_prefixed_zero_payload() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x00\x05hello").unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert_eq!(frames[1].data, b"hello");
    }

    #[test]
    fn length_prefixed_incomplete_payload() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x0aABC").unwrap();
        assert!(frames.is_empty());
        assert!(dec.pending_len() >= 3);
    }

    #[test]
    fn length_prefixed_invalid_prefix_size() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 3,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("prefix_size=3 should be rejected"),
            Err(err) => assert!(err.contains("prefix_size must be 1, 2, or 4"), "got: {err}"),
        }
    }

    #[test]
    fn length_prefixed_u32_big_endian() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 4,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let mut buf = vec![0x00, 0x00, 0x00, 0x05];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn length_prefixed_u32_little_endian() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 4,
                endianness: Endianness::Little,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let mut buf = vec![0x05, 0x00, 0x00, 0x00];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn start_end_no_start_marker() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "STX".into(),
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"noise_without_markers").unwrap();
        assert!(frames.is_empty());
        assert!(dec.pending_len() <= 2);
    }

    #[test]
    fn start_end_start_no_end_then_flush() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "<".into(),
                end: ">".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"<data_without_end").unwrap();
        assert!(frames.is_empty(), "no end marker yet");
        let partial = dec.flush_partial().expect("partial frame after flush");
        assert_eq!(partial.data, b"data_without_end");
    }

    #[test]
    fn start_end_empty_markers_rejected() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "".into(),
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("empty markers should be rejected"),
            Err(err) => assert!(
                err.contains("Start and end markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    #[test]
    fn start_end_start_split_across_chunks() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "ABC".into(),
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"AB").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"CdX").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"d");
    }

    #[test]
    fn delimiter_invalid_encoding_rejected() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "!!!".into(),
                delimiter_encoding: crate::match_config::PatternEncoding::Base64,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("expected error for invalid delimiter encoding"),
            Err(err) => assert!(err.contains("Invalid delimiter encoding"), "got: {err}"),
        }
    }

    #[test]
    fn start_end_invalid_encoding_rejected() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "!!!".into(),
                end: "X".into(),
                marker_encoding: crate::match_config::PatternEncoding::Base64,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
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
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::ShellPrompt,
            custom_prompt: Some("[invalid".to_string()),
        };
        match FrameDecoder::new(&config, Some(&parser)) {
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
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: Some(0),
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn length_prefixed_initial_offset() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: Some(4),
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"XXXX\x05hello").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn delimiter_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a|b|").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a|");
        assert_eq!(frames[1].data, b"b|");
    }

    #[test]
    fn flush_partial_empty_buffer() {
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        assert!(dec.flush_partial().is_none(), "empty buf => no frame");
    }

    #[test]
    fn combined_line_json_parser() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::JsonLines,
            custom_prompt: None,
        };
        let mut dec = FrameDecoder::new(&config, Some(&parser)).unwrap();
        let frames = dec.push(b"{\"a\":1}\n{\"b\":2}\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0].parsed, Some(ParsedFrame::Json(_))));
        assert!(matches!(frames[1].parsed, Some(ParsedFrame::Json(_))));
    }

    // ── Coverage gap tests ──────────────────────────────────────────────

    #[test]
    fn delimiter_decoder_empty_segments() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"||").unwrap();
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert!(frames[1].data.is_empty());

        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a||b|").unwrap();
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"a");
        assert!(frames[1].data.is_empty());
        assert_eq!(frames[2].data, b"b");
    }

    #[test]
    fn length_prefixed_prefix_split_across_chunks() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x00").unwrap();
        assert!(frames.is_empty());
        let frames = dec.push(b"\x05hello").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn start_end_end_marker_split_across_chunks() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "STX".into(),
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"STXdataET").unwrap();
        assert!(frames.is_empty(), "end marker ETX not yet complete");
        let frames = dec.push(b"X").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"data");
    }

    // ── TX framing unit tests ────────────────────────────────────────────

    #[test]
    fn tx_line_lf() {
        let mode = TxFramingMode::Line {
            ending: TxLineEnding::Lf,
        };
        let framed = mode.encode(b"AT+CGMI").unwrap();
        assert_eq!(framed, b"AT+CGMI\n");
    }

    #[test]
    fn tx_line_cr() {
        let mode = TxFramingMode::Line {
            ending: TxLineEnding::Cr,
        };
        let framed = mode.encode(b"AT+CGMI").unwrap();
        assert_eq!(framed, b"AT+CGMI\r");
    }

    #[test]
    fn tx_line_crlf() {
        let mode = TxFramingMode::Line {
            ending: TxLineEnding::Crlf,
        };
        let framed = mode.encode(b"AT+CGMI").unwrap();
        assert_eq!(framed, b"AT+CGMI\r\n");
    }

    #[test]
    fn tx_delimiter_utf8() {
        let mode = TxFramingMode::Delimiter {
            delimiter: "|".into(),
            delimiter_encoding: PatternEncoding::Utf8,
        };
        let framed = mode.encode(b"data").unwrap();
        assert_eq!(framed, b"data|");
    }

    #[test]
    fn tx_delimiter_empty_rejected() {
        let mode = TxFramingMode::Delimiter {
            delimiter: "".into(),
            delimiter_encoding: PatternEncoding::Utf8,
        };
        match mode.encode(b"data") {
            Ok(_) => panic!("empty TX delimiter should be rejected"),
            Err(err) => assert!(err.contains("TX delimiter must not be empty"), "got: {err}"),
        }
    }

    #[test]
    fn tx_length_prefixed_u8() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 1,
            endianness: Endianness::Big,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(framed, b"\x05hello");
    }

    #[test]
    fn tx_length_prefixed_u16_big() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Big,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(&framed[..2], &[0x00, 0x05]);
        assert_eq!(&framed[2..], b"hello");
    }

    #[test]
    fn tx_length_prefixed_u16_little() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Little,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(&framed[..2], &[0x05, 0x00]);
        assert_eq!(&framed[2..], b"hello");
    }

    #[test]
    fn tx_length_prefixed_u32() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 4,
            endianness: Endianness::Big,
        };
        let framed = mode.encode(b"hello").unwrap();
        assert_eq!(&framed[..4], &[0x00, 0x00, 0x00, 0x05]);
        assert_eq!(&framed[4..], b"hello");
    }

    #[test]
    fn tx_length_prefixed_invalid_prefix_size() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 3,
            endianness: Endianness::Big,
        };
        match mode.encode(b"data") {
            Ok(_) => panic!("prefix_size=3 should be rejected"),
            Err(err) => assert!(
                err.contains("TX prefix_size must be 1, 2, or 4"),
                "got: {err}"
            ),
        }
    }

    #[test]
    fn tx_length_prefixed_u8_overflow() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 1,
            endianness: Endianness::Big,
        };
        let payload = vec![0u8; 300];
        match mode.encode(&payload) {
            Ok(_) => panic!("payload too large for prefix_size=1 should be rejected"),
            Err(err) => assert!(err.contains("exceeds maximum 255"), "got: {err}"),
        }
    }

    #[test]
    fn tx_length_prefixed_u16_overflow() {
        let mode = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Big,
        };
        let payload = vec![0u8; 65536];
        match mode.encode(&payload) {
            Ok(_) => panic!("payload too large for prefix_size=2 should be rejected"),
            Err(err) => assert!(err.contains("exceeds maximum 65535"), "got: {err}"),
        }
    }

    #[test]
    fn tx_start_end() {
        let mode = TxFramingMode::StartEnd {
            start: "<".into(),
            end: ">".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let framed = mode.encode(b"data").unwrap();
        assert_eq!(framed, b"<data>");
    }

    #[test]
    fn tx_start_end_empty_markers_rejected() {
        let mode = TxFramingMode::StartEnd {
            start: "".into(),
            end: ">".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        match mode.encode(b"data") {
            Ok(_) => panic!("empty markers should be rejected"),
            Err(err) => assert!(
                err.contains("TX start and end markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    // ── Round-trip tests (TX encode → RX decode) ──────────────────────

    #[test]
    fn roundtrip_line_lf() {
        let tx = TxFramingMode::Line {
            ending: TxLineEnding::Lf,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Lf,
            },
            ..Default::default()
        };
        let payload = b"hello world";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_line_cr() {
        let tx = TxFramingMode::Line {
            ending: TxLineEnding::Cr,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Cr,
            },
            ..Default::default()
        };
        let payload = b"hello world";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_line_crlf() {
        let tx = TxFramingMode::Line {
            ending: TxLineEnding::Crlf,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            ..Default::default()
        };
        let payload = b"hello world";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_delimiter() {
        let tx = TxFramingMode::Delimiter {
            delimiter: "|".into(),
            delimiter_encoding: PatternEncoding::Utf8,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let payload = b"data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_length_prefixed_u8() {
        let tx = TxFramingMode::LengthPrefixed {
            prefix_size: 1,
            endianness: Endianness::Big,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let payload = b"binary data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_length_prefixed_u16_be() {
        let tx = TxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Big,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let payload = b"binary data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_start_end() {
        let tx = TxFramingMode::StartEnd {
            start: "STX".into(),
            end: "ETX".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: "STX".into(),
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let payload = b"data";
        let framed = tx.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&rx_config, None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    // ── TX framing JSON deserialization ──────────────────────────────────

    #[test]
    fn tx_framing_line_crlf_deserialize() {
        let json = serde_json::json!({
            "type": "line",
            "ending": "crlf"
        });
        let cfg: TxFramingConfig = serde_json::from_value(json).unwrap();
        assert!(matches!(
            cfg.mode,
            TxFramingMode::Line {
                ending: TxLineEnding::Crlf
            }
        ));
    }

    #[test]
    fn tx_framing_delimiter_deserialize() {
        let json = serde_json::json!({
            "type": "delimiter",
            "delimiter": "|",
            "delimiter_encoding": "utf8"
        });
        let cfg: TxFramingConfig = serde_json::from_value(json).unwrap();
        assert!(matches!(cfg.mode, TxFramingMode::Delimiter { .. }));
    }
}
