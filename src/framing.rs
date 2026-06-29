//! Frame boundary detection and protocol parsing for RX and TX streams.
//!
//! Provides a [`FrameDecoder`] that splits a byte stream into structured
//! frames using one of four boundary modes (line, delimiter, length-prefixed,
//! start/end marker). Optional parsers interpret frame content (AT commands,
//! JSON lines, shell prompts). Used as an option on `read` and `subscribe`.
//!
//! Also provides TX framing via [`TxFramingMode`] which encodes payloads
//! with frame boundaries matching the RX modes. Used on `write`.

use crate::checksums::Checksum;
use crate::checksums::Lrc;
use crate::checksums::XorChecksum;
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
    /// Skip frames whose data is empty or whitespace-only (ASCII whitespace).
    /// Default: false. When true, empty/whitespace-only frames are not emitted
    /// and do not increment the frame index. Applies to all framing modes.
    /// The `ndjson` preset sets this true (NDJSON spec: skip blank lines).
    #[serde(default)]
    pub skip_empty: bool,
}

impl Default for RxFramingConfig {
    fn default() -> Self {
        Self {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
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
    /// `start` is a list of marker strings; RX matches ANY of them
    /// (finds the earliest one). TX uses `start[0]`.
    #[serde(rename_all = "snake_case")]
    StartEnd {
        /// Start marker(s) (decoded per `marker_encoding`).
        /// RX matches any marker in the list (earliest match wins).
        /// TX uses the first marker (`start[0]`).
        start: Vec<String>,
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
    /// COBS (Consistent Overhead Byte Stuffing) framing. Byte-stuffed payloads
    /// delimited by 0x00 (plain COBS, Cheshire/Baker). The delimiter never
    /// appears inside an encoded block. The PPP-COBS draft variant (0x7E) is
    /// not supported; it is tracked for a future PR.
    #[serde(rename_all = "snake_case")]
    Cobs,
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
    /// SLIP (RFC 1055) byte-stuffed framing, raw payload (no parser).
    Slip,
    /// One JSON value per line (line framing + JSON-lines parser).
    JsonLines,
    /// COBS (Consistent Overhead Byte Stuffing, plain 0x00-delimited) framing,
    /// raw payload (no parser).
    Cobs,
    /// NDJSON (newline-delimited JSON). Line framing (`auto` RX, `lf` TX) +
    /// JSON-lines parser, skipping empty/whitespace-only lines per the NDJSON
    /// spec. Differs from `json_lines` only in `skip_empty`.
    Ndjson,
    /// NMEA-0183 marine sentence protocol. StartEnd framing (start markers
    /// $ / !, end \r\n) + Nmea parser with checksum validation.
    Nmea0183,
    /// Modbus ASCII mode. StartEnd framing (: start, \r\n end) + ModbusAscii
    /// parser with LRC validation.
    ModbusAscii,
}

/// The TX framing implied by a protocol preset.
pub fn preset_tx_framing(p: ProtocolPreset) -> TxFramingConfig {
    match p {
        ProtocolPreset::AtCommand => TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Cr,
            },
        },
        ProtocolPreset::Slip => TxFramingConfig {
            mode: TxFramingMode::Slip,
        },
        ProtocolPreset::JsonLines | ProtocolPreset::Ndjson => TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Lf,
            },
        },
        ProtocolPreset::Cobs => TxFramingConfig {
            mode: TxFramingMode::Cobs,
        },
        ProtocolPreset::Nmea0183 => TxFramingConfig {
            mode: TxFramingMode::StartEnd {
                start: vec!["$".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
            },
        },
        ProtocolPreset::ModbusAscii => TxFramingConfig {
            mode: TxFramingMode::StartEnd {
                start: vec![":".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
            },
        },
    }
}

/// The RX framing implied by a protocol preset.
pub fn preset_rx_framing(p: ProtocolPreset) -> RxFramingConfig {
    let (mode, skip_empty) = match p {
        ProtocolPreset::AtCommand | ProtocolPreset::JsonLines => (
            RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            false,
        ),
        ProtocolPreset::Slip => (RxFramingMode::Slip, false),
        ProtocolPreset::Cobs => (RxFramingMode::Cobs, false),
        ProtocolPreset::Ndjson => (
            RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            true,
        ),
        ProtocolPreset::Nmea0183 => (
            RxFramingMode::StartEnd {
                start: vec!["$".into(), "!".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            false,
        ),
        ProtocolPreset::ModbusAscii => (
            RxFramingMode::StartEnd {
                start: vec![":".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            false,
        ),
    };
    RxFramingConfig {
        mode,
        max_frames: None,
        include_terminators: false,
        skip_empty,
    }
}

/// The RX parser implied by a protocol preset.
pub fn preset_rx_parser(p: ProtocolPreset) -> ParserConfig {
    let (parser_type, validate) = match p {
        ProtocolPreset::AtCommand => (ParserType::AtCommand, false),
        ProtocolPreset::Slip | ProtocolPreset::Cobs => (ParserType::Raw, false),
        ProtocolPreset::JsonLines | ProtocolPreset::Ndjson => (ParserType::JsonLines, false),
        ProtocolPreset::Nmea0183 => (ParserType::Nmea, true),
        ProtocolPreset::ModbusAscii => (ParserType::ModbusAscii, true),
    };
    ParserConfig {
        parser_type,
        custom_prompt: None,
        validate,
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
        /// Start marker(s) (decoded per `marker_encoding`).
        /// TX uses the first marker (`start[0]`).
        start: Vec<String>,
        /// End marker (decoded per `marker_encoding`).
        end: String,
        /// How to decode the marker strings into bytes.
        #[serde(default = "default_encoding")]
        marker_encoding: PatternEncoding,
    },
    /// SLIP (RFC 1055) framing. Encodes as `END [stuffed payload] END`.
    #[serde(rename_all = "snake_case")]
    Slip,
    /// COBS (Consistent Overhead Byte Stuffing) framing. Byte-stuffed payload
    /// followed by 0x00 delimiter (plain COBS). The delimiter never appears
    /// inside the encoded block. The PPP-COBS draft variant (0x7E) is not
    /// supported; it is tracked for a future PR.
    #[serde(rename_all = "snake_case")]
    Cobs,
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
                if start.is_empty() {
                    return Err("TX start markers must not be empty".into());
                }
                let start_bytes = codec::decode((*marker_encoding).into(), &start[0])
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
            TxFramingMode::Cobs => {
                let mut framed = vec![0x00];
                framed.extend_from_slice(&cobs_stuff(payload));
                framed.push(0x00);
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
    /// Whether to enforce a protocol checksum when present. Default: false.
    /// When true, a protocol parser that defines a checksum (currently NMEA's
    /// *XX XOR) validates it and surfaces a mismatch as a framing_error.
    /// A sentence/message WITHOUT a checksum is accepted regardless.
    #[serde(default)]
    pub validate: bool,
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
    /// Parse NMEA-0183 sentences: talker ID + sentence type + comma fields,
    /// with optional *XX XOR checksum validation.
    Nmea,
    /// Parse Modbus ASCII frames: hex-decode the body, validate the LRC,
    /// expose address + function code + data bytes.
    ModbusAscii,
}

// ---- Frame types -----------------------------------------------------------

/// A decoded frame with optional parsed content.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Frame {
    /// Raw frame bytes (without delimiters/terminators unless include_terminators is set).
    #[schemars(schema_with = "crate::schema_helpers::byte_array_schema")]
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
    Nmea {
        /// Talker ID (e.g. "GP" for GPS, "GN" for GLONASS, "AI" for AIS).
        /// Two characters for standard sentences; may be longer for proprietary.
        talker_id: String,
        /// Sentence type (e.g. "GGA", "RMC", "GLL", "AIVDM").
        /// Three characters for standard; variable for proprietary ($P...).
        sentence_type: String,
        /// Comma-separated data fields (the body after the address, before '*').
        fields: Vec<String>,
        /// Checksum status:
        /// - Some(true): checksum present and valid (or present, validate=false: not enforced but reported as valid-shape).
        /// - Some(false): checksum present and INVALID (only reachable when validate=false; when validate=true a mismatch returns Err, not Some(false)).
        /// - None: no checksum present in the sentence.
        #[serde(skip_serializing_if = "Option::is_none")]
        checksum_valid: Option<bool>,
    },
    ModbusAscii {
        /// Slave address (1-247), decoded from the first 2 hex chars of the body.
        /// 0 = broadcast. Stored as the decoded byte value (0-255).
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        address: u8,
        /// Function code (decoded byte value). 1-127 = normal; 128+ = exception
        /// response (the high bit set indicates an exception).
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        function_code: u8,
        /// Decoded data bytes (the PDU payload after address + function code,
        /// excluding the LRC). For an exception response, the first byte is the
        /// exception code.
        #[schemars(schema_with = "crate::schema_helpers::byte_array_schema")]
        data: Vec<u8>,
        /// LRC status:
        /// - Some(true): LRC present and valid (or present, validate=false: not enforced but reported as valid-shape).
        /// - Some(false): LRC present and INVALID (only reachable when validate=false; when validate=true a mismatch returns Err, not Some(false)).
        /// - None: no LRC present (a malformed frame shorter than 2 hex chars after the data — should not happen for valid frames, but defensive).
        checksum_valid: Option<bool>,
    },
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
    /// Skip frames whose data is empty or whitespace-only.
    skip_empty: bool,
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
        start: Vec<Vec<u8>>,
        end: Vec<u8>,
        include_markers: bool,
        in_frame: bool,
    },
    Slip {
        state: SlipState,
    },
    Cobs {
        state: CobsState,
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

/// COBS decoder state.
#[derive(Debug, Clone)]
enum CobsState {
    /// Discard bytes until the first 0x00 delimiter is seen (SLIP parity:
    /// skip junk before the first frame marker).
    BeforeFirstDelim,
    /// Inside a frame. `decoded` accumulates the reconstructed payload (with
    /// phantom zeros inserted by each non-0xFF code). `remaining` is data
    /// bytes still to copy for the current code block; when `remaining == 0`
    /// the next byte is a code byte (or the 0x00 delimiter ending the frame).
    /// `pending_zero` is true when the current code block (non-0xFF) has its
    /// implicit zero still to be appended after the remaining data bytes are
    /// copied.
    InFrame {
        decoded: Vec<u8>,
        remaining: u8,
        pending_zero: bool,
    },
}

trait FrameParser: Send + Sync {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError>;
}

/// SLIP decoder: consume `buf_outer` byte-by-byte according to current
/// [`SlipState`] in `mode`. Returns decoded frames, or `Err` for a
/// malformed escape sequence. Updates `mode` state in-place.
fn slip_decode(
    buf_outer: &mut Vec<u8>,
    frame_count: &mut usize,
    parser: &Option<Box<dyn FrameParser>>,
    mode: &mut DecoderMode,
    skip_empty: bool,
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
                                if !skip_empty || !is_blank_frame(&data) {
                                    *frame_count += 1;
                                    let parsed = match parser.as_ref().map(|p| p.parse(&data)) {
                                        Some(Ok(pf)) => Some(pf),
                                        Some(Err(e)) => {
                                            // Drain consumed bytes, clear state, return error.
                                            buf_outer.drain(..read_pos);
                                            *state = SlipState::BeforeFirstEnd;
                                            return Err(e);
                                        }
                                        None => None,
                                    };
                                    frames.push(Frame {
                                        data,
                                        index: *frame_count - 1,
                                        frame_type: "slip".into(),
                                        parsed,
                                    });
                                }
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
/// COBS decoder: consume `buf_outer` according to current [`CobsState`].
/// Returns decoded frames. A code byte of 0x00 (the delimiter) terminates the
/// frame; the trailing phantom zero (inserted by the final non-0xFF code) is
/// dropped before emitting the frame. Malformed code bytes surface as
/// [`FrameDecodeError::CobsInvalidCode`].
fn cobs_decode(
    buf_outer: &mut Vec<u8>,
    frame_count: &mut usize,
    parser: &Option<Box<dyn FrameParser>>,
    mode: &mut DecoderMode,
    skip_empty: bool,
) -> Result<Vec<Frame>, FrameDecodeError> {
    let mut frames = Vec::new();
    let state = match mode {
        DecoderMode::Cobs { ref mut state } => state,
        _ => return Ok(frames),
    };

    loop {
        match state {
            CobsState::BeforeFirstDelim => {
                if let Some(pos) = buf_outer.iter().position(|&b| b == 0x00) {
                    buf_outer.drain(..=pos);
                    *state = CobsState::InFrame {
                        decoded: Vec::new(),
                        remaining: 0,
                        pending_zero: false,
                    };
                    continue;
                }
                buf_outer.clear();
                return Ok(frames);
            }
            CobsState::InFrame {
                ref mut decoded,
                ref mut remaining,
                ref mut pending_zero,
            } => {
                let mut read_pos: usize = 0;
                let len = buf_outer.len();
                loop {
                    // 1. Flush pending zero first (does NOT consume a
                    //    byte). This handles the code==1 case (zero data
                    //    bytes) where the implicit zero is due immediately
                    //    before the next code byte. For code>1, this flush
                    //    happens on the iteration after the last data byte
                    //    was copied (when remaining just hit 0), and the
                    //    current b is the next code byte.
                    if *remaining == 0 && *pending_zero {
                        decoded.push(0x00);
                        *pending_zero = false;
                        // Fall through to code-byte handling (or the next
                        // iteration will read the next byte). Do NOT
                        // consume a byte from buf_outer for this flush.
                    }
                    // 2. If we have consumed all bytes in buf_outer, break.
                    if read_pos >= len {
                        break;
                    }
                    let b = buf_outer[read_pos];
                    read_pos += 1;

                    // 3. Copy data bytes for the current code block.
                    if *remaining > 0 {
                        decoded.push(b);
                        *remaining -= 1;
                        // (Pending-zero flush happens at top of next
                        //  iteration when remaining hits 0.)
                        continue;
                    }

                    // 4. Expecting a code byte (or the 0x00 delimiter).
                    if b == 0x00 {
                        // Frame delimiter: the final code's implicit zero
                        // is the phantom. Drop it and emit the frame.
                        let mut data = std::mem::take(decoded);
                        // The final code is always non-0xFF (the encoder
                        // always writes a final code for the phantom,
                        // which is 0x01-0xFE). Pop the trailing zero that
                        // the final code inserted. Defensive: only pop if
                        // the last byte is 0x00.
                        if data.last() == Some(&0x00) {
                            data.pop();
                        }
                        if !skip_empty || !is_blank_frame(&data) {
                            *frame_count += 1;
                            let parsed = match parser.as_ref().map(|p| p.parse(&data)) {
                                Some(Ok(pf)) => Some(pf),
                                Some(Err(e)) => {
                                    buf_outer.drain(..read_pos);
                                    *decoded = Vec::new();
                                    *remaining = 0;
                                    *pending_zero = false;
                                    *state = CobsState::BeforeFirstDelim;
                                    return Err(e);
                                }
                                None => None,
                            };
                            frames.push(Frame {
                                data,
                                index: *frame_count - 1,
                                frame_type: "cobs".into(),
                                parsed,
                            });
                        }
                        *state = CobsState::BeforeFirstDelim;
                        buf_outer.drain(..read_pos);
                        // Continue the outer loop to look for the next
                        // frame's leading delimiter (or run out of bytes).
                        break;
                    }

                    // 5. Code byte: 0x01-0xFE => (code-1) data bytes then
                    //    a zero; 0xFF => 254 data bytes, no zero.
                    if b == 0xFF {
                        *remaining = 254;
                        *pending_zero = false;
                    } else {
                        // b in 0x01-0xFE: set (code-1) data bytes,
                        // then insert a zero.
                        *remaining = b - 1;
                        *pending_zero = true;
                    }
                }
                // If we broke out of the inner loop via the delimiter,
                // the outer loop continues with BeforeFirstDelim.
                // Otherwise we consumed all bytes in buf_outer without
                // hitting a delimiter — drain and wait for more bytes.
                if !matches!(state, CobsState::BeforeFirstDelim) {
                    buf_outer.drain(..read_pos);
                    return Ok(frames);
                }
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
                if start.is_empty() {
                    return Err("Start markers must not be empty".into());
                }
                let mut start_bytes_vec: Vec<Vec<u8>> = Vec::with_capacity(start.len());
                for s in start {
                    let sb = codec::decode((*marker_encoding).into(), s)
                        .map_err(|e| format!("Invalid start marker encoding: {e}"))?;
                    if sb.is_empty() {
                        return Err("Start and end markers must not be empty".into());
                    }
                    start_bytes_vec.push(sb);
                }
                let end_bytes = codec::decode((*marker_encoding).into(), end)
                    .map_err(|e| format!("Invalid end marker encoding: {e}"))?;
                if end_bytes.is_empty() {
                    return Err("Start and end markers must not be empty".into());
                }
                DecoderMode::StartEnd {
                    start: start_bytes_vec,
                    end: end_bytes,
                    include_markers: *include_markers,
                    in_frame: false,
                }
            }
            RxFramingMode::Slip => DecoderMode::Slip {
                state: SlipState::BeforeFirstEnd,
            },
            RxFramingMode::Cobs => DecoderMode::Cobs {
                state: CobsState::BeforeFirstDelim,
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
            skip_empty: config.skip_empty,
            parser,
        })
    }

    /// Feed a chunk of bytes. Returns any complete frames found, or a
    /// [`FrameDecodeError`] for protocol violations (SLIP malformed escape).
    /// The caller is responsible for draining consumed bytes from their
    /// accumulation buffer.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, FrameDecodeError> {
        self.buf.extend_from_slice(chunk);
        // SLIP and COBS are handled separately via free functions to avoid
        // borrow conflicts between the mutable mode borrow and self.
        if matches!(self.mode, DecoderMode::Slip { .. }) {
            let frames = slip_decode(
                &mut self.buf,
                &mut self.frame_count,
                &self.parser,
                &mut self.mode,
                self.skip_empty,
            )?;
            return Ok(frames);
        }
        if matches!(self.mode, DecoderMode::Cobs { .. }) {
            let frames = cobs_decode(
                &mut self.buf,
                &mut self.frame_count,
                &self.parser,
                &mut self.mode,
                self.skip_empty,
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
                        // Find the earliest start marker among all candidates.
                        let mut best_pos: Option<(usize, usize)> = None; // (pos, marker_len)
                        for marker in &start {
                            if let Some(pos) = find_subsequence(&self.buf, marker) {
                                match best_pos {
                                    Some((bp, _)) if pos < bp => {
                                        best_pos = Some((pos, marker.len()));
                                    }
                                    None => {
                                        best_pos = Some((pos, marker.len()));
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if let Some((pos, marker_len)) = best_pos {
                            self.buf.drain(..pos);
                            if !include {
                                self.buf.drain(..marker_len);
                            }
                            *in_frame = true;
                        } else {
                            // Keep enough trailing bytes to allow a partial
                            // match of the longest start marker.
                            let keep = start
                                .iter()
                                .map(|m| m.len().saturating_sub(1))
                                .max()
                                .unwrap_or(0);
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
                DecoderMode::Cobs { .. } => unreachable!("COBS handled before match"),
            };

            match consumed {
                None => break,
                Some(frame_bytes) => {
                    if !self.skip_empty || !is_blank_frame(&frame_bytes) {
                        self.frame_count += 1;
                        let parsed = match self.parser.as_ref().map(|p| p.parse(&frame_bytes)) {
                            Some(Ok(pf)) => Some(pf),
                            Some(Err(e)) => return Err(e),
                            None => None,
                        };
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
            DecoderMode::Cobs { .. } => "cobs".into(),
        }
    }

    /// Bytes pending in the incomplete frame buffer.
    pub fn pending_len(&self) -> usize {
        self.buf.len()
    }

    /// Flush any remaining bytes as a partial frame. For SLIP and COBS, this
    /// drains the in-frame buffer; pending escaped/partial state is emitted as
    /// raw bytes.
    pub fn flush_partial(&mut self) -> Option<Frame> {
        // SLIP: drain the in-frame buffer instead of self.buf.
        if let DecoderMode::Slip {
            state: SlipState::InFrame { ref mut buf, .. },
        } = self.mode
        {
            if buf.is_empty() {
                return None;
            }
            // flush_partial does not apply skip_empty — partial
            // frames at flush are emitted regardless.
            let data = std::mem::take(buf);
            self.frame_count += 1;
            return Some(Frame {
                data,
                index: self.frame_count - 1,
                frame_type: "slip".into(),
                parsed: None,
            });
        }
        // COBS: drain the in-frame decoded buffer.
        if let DecoderMode::Cobs {
            state: CobsState::InFrame {
                ref mut decoded, ..
            },
        } = self.mode
        {
            if decoded.is_empty() {
                return None;
            }
            // flush_partial does not apply skip_empty — partial
            // frames at flush are emitted regardless.
            let data = std::mem::take(decoded);
            self.frame_count += 1;
            return Some(Frame {
                data,
                index: self.frame_count - 1,
                frame_type: "cobs".into(),
                parsed: None,
            });
        }
        if self.buf.is_empty() {
            return None;
        }
        // flush_partial does not apply skip_empty — partial
        // frames at flush are emitted regardless.
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
        ParserType::Nmea => Ok(Box::new(NmeaParser {
            validate: config.validate,
        })),
        ParserType::ModbusAscii => Ok(Box::new(ModbusAsciiParser {
            validate: config.validate,
        })),
    }
}

// AT command parser

struct AtCommandParser;

impl FrameParser for AtCommandParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        let text = String::from_utf8_lossy(data);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(ParsedFrame::Raw);
        }
        if let Some(cmd) = trimmed.strip_prefix('+') {
            if let Some(colon) = cmd.find(':') {
                let cmd_name = cmd[..colon].to_string();
                let fields: Vec<String> = cmd[colon + 1..]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                return Ok(ParsedFrame::AtCommand {
                    response_type: "response".into(),
                    command: Some(cmd_name),
                    status: None,
                    fields,
                });
            }
        }
        if trimmed == "OK" {
            return Ok(ParsedFrame::AtCommand {
                response_type: "status".into(),
                command: None,
                status: Some("OK".into()),
                fields: vec![],
            });
        }
        if trimmed == "ERROR" {
            return Ok(ParsedFrame::AtCommand {
                response_type: "error".into(),
                command: None,
                status: Some("ERROR".into()),
                fields: vec![],
            });
        }
        if trimmed.starts_with("+CME ERROR") || trimmed.starts_with("+CMS ERROR") {
            return Ok(ParsedFrame::AtCommand {
                response_type: "error".into(),
                command: None,
                status: Some(trimmed.to_string()),
                fields: vec![],
            });
        }
        Ok(ParsedFrame::AtCommand {
            response_type: "data".into(),
            command: None,
            status: None,
            fields: vec![trimmed.to_string()],
        })
    }
}

// JSON lines parser

struct JsonLinesParser;

impl FrameParser for JsonLinesParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        match serde_json::from_slice::<serde_json::Value>(data) {
            Ok(val) if val.is_object() => Ok(ParsedFrame::Json(val)),
            _ => Ok(ParsedFrame::Raw),
        }
    }
}

// Shell prompt parser

struct ShellPromptParser {
    custom: Option<regex::bytes::Regex>,
}

impl FrameParser for ShellPromptParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        let text = String::from_utf8_lossy(data);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return Ok(ParsedFrame::Raw);
        }
        if let Some(ref re) = self.custom {
            if re.is_match(data) {
                return Ok(ParsedFrame::ShellPrompt {
                    prompt: trimmed.to_string(),
                    prompt_type: "custom".into(),
                });
            }
        }
        if trimmed.ends_with("$ ") || trimmed.ends_with("$") {
            return Ok(ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "user".into(),
            });
        }
        if trimmed.ends_with("# ") || trimmed.ends_with("#") {
            return Ok(ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "root".into(),
            });
        }
        if trimmed.ends_with("> ") || trimmed.ends_with(">") {
            return Ok(ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "generic".into(),
            });
        }
        if let Some(at_pos) = trimmed.rfind('@') {
            if let Some(colon_pos) = trimmed[at_pos..].find(':') {
                let _user = &trimmed[..at_pos];
                let _host = &trimmed[at_pos + 1..at_pos + colon_pos];
                let suffix = &trimmed[at_pos + colon_pos + 1..];
                if suffix == "$ " || suffix == "$" || suffix == "# " || suffix == "#" {
                    return Ok(ParsedFrame::ShellPrompt {
                        prompt: trimmed.to_string(),
                        prompt_type: if suffix.starts_with('#') {
                            "root".to_string()
                        } else {
                            "user".to_string()
                        },
                    });
                }
            }
        }
        Ok(ParsedFrame::Raw)
    }
}

// Raw parser (passthrough)

struct RawParser;

impl FrameParser for RawParser {
    fn parse(&self, _data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        Ok(ParsedFrame::Raw)
    }
}

/// Strip a trailing `\r\n` or bare `\n` from `content` in place (defensive:
/// handles frames where the end marker was NOT stripped by the framing layer).
fn strip_trailing_newline(content: &mut Vec<u8>) {
    if content.ends_with(b"\r\n") {
        content.truncate(content.len() - 2);
    } else if content.ends_with(b"\n") {
        content.truncate(content.len() - 1);
    }
}

/// Strip a leading byte if it matches any of `markers` (defensive: handles
/// frames where the start marker was NOT stripped by the framing layer).
fn strip_leading_if_any(content: &mut Vec<u8>, markers: &[u8]) {
    if let Some(&first) = content.first() {
        if markers.contains(&first) {
            content.remove(0);
        }
    }
}

// NMEA-0183 parser

struct NmeaParser {
    validate: bool,
}

impl FrameParser for NmeaParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        // 1. Strip optional leading $ or ! and trailing \r\n.
        //    Defensive: when include_markers is true, data includes the start/end
        //    markers. The nmea0183 preset uses include_markers=false, but bare
        //    callers might set true. Handle both.
        let mut content = data.to_vec();
        strip_leading_if_any(&mut content, b"$!");
        strip_trailing_newline(&mut content);

        // 2. Validate that this looks like a NMEA sentence. The preset strips
        //    $/! via StartEnd framing, so the data may start directly with the
        //    talker ID (e.g. "GPGLL,..."). If include_markers were true, $/!
        //    was stripped by step 1 above. In either case, a valid NMEA
        //    sentence always contains at least one comma (separating address
        //    from fields). Non-NMEA frames (e.g. "hello world", "AT+CGMI")
        //    have no comma and return Raw — the parser is opt-in; bare callers
        //    mixing data types see Raw for non-matching frames.
        if !content.contains(&b',') {
            return Ok(ParsedFrame::Raw);
        }

        // 3. Split at '*': content before = sentence body, after = checksum hex.
        let (body, checksum_hex) = if let Some(star_pos) = content.iter().position(|&b| b == b'*') {
            let body = &content[..star_pos];
            let checksum_hex = &content[star_pos + 1..];
            (body.to_vec(), Some(checksum_hex.to_vec()))
        } else {
            (content, None)
        };

        // 4. Validate checksum if present and validate is true.
        let checksum_valid = match &checksum_hex {
            Some(hex) if hex.len() >= 2 => {
                let hex_str = String::from_utf8_lossy(hex);
                let received_val = match u8::from_str_radix(&hex_str[..2], 16) {
                    Ok(v) => v,
                    Err(_) => {
                        // Invalid hex in checksum — treat as mismatch.
                        if self.validate {
                            let computed = XorChecksum.compute(&body);
                            return Err(FrameDecodeError::ChecksumMismatch {
                                expected: computed,
                                received: hex.clone(),
                            });
                        }
                        return Ok(ParsedFrame::Nmea {
                            talker_id: String::new(),
                            sentence_type: String::new(),
                            fields: vec![],
                            checksum_valid: Some(false),
                        });
                    }
                };
                let computed = XorChecksum.compute(&body);
                let computed_val = computed[0];
                if self.validate {
                    if computed_val != received_val {
                        return Err(FrameDecodeError::ChecksumMismatch {
                            expected: computed,
                            received: vec![received_val],
                        });
                    }
                    Some(true)
                } else {
                    Some(computed_val == received_val)
                }
            }
            Some(hex) => {
                // Checksum present but too short (<2 hex chars). Treat as mismatch.
                if self.validate {
                    let computed = XorChecksum.compute(&body);
                    return Err(FrameDecodeError::ChecksumMismatch {
                        expected: computed,
                        received: hex.clone(),
                    });
                }
                Some(false)
            }
            None => None,
        };

        // 5. Parse the sentence body: split into address + comma fields.
        let body_str = String::from_utf8_lossy(&body);
        let body_owned = body_str.into_owned();

        let (address_part, fields_part) = match body_owned.find(',') {
            Some(comma_pos) => (
                body_owned[..comma_pos].to_string(),
                body_owned[comma_pos + 1..].to_string(),
            ),
            None => (body_owned, String::new()),
        };

        let (talker_id, sentence_type) = if address_part.len() >= 5 {
            // Standard NMEA: talker = first 2, type = chars 3 onward
            let tid = address_part[..2].to_string();
            let stype = address_part[2..].to_string();
            (tid, stype)
        } else if address_part.len() >= 2 {
            let tid = address_part[..2].to_string();
            let stype = address_part[2..].to_string();
            (tid, stype)
        } else {
            // < 2 chars: use whole as talker, no type
            (address_part, String::new())
        };

        let fields: Vec<String> = if fields_part.is_empty() {
            vec![]
        } else {
            fields_part.split(',').map(|s| s.to_string()).collect()
        };

        Ok(ParsedFrame::Nmea {
            talker_id,
            sentence_type,
            fields,
            checksum_valid,
        })
    }
}

// Modbus ASCII parser

struct ModbusAsciiParser {
    validate: bool,
}

impl FrameParser for ModbusAsciiParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        // 1. Strip optional leading ':' and trailing \r\n (defensive — the
        //    preset uses include_markers=false so data is the body between
        //    ':' and \r\n; a bare caller with include_markers=true would
        //    include them).
        let mut content = data.to_vec();
        strip_leading_if_any(&mut content, b":");
        strip_trailing_newline(&mut content);

        // 2. Modbus ASCII body is all hex chars (0-9, A-F, a-f). Validate:
        //    if any non-hex byte is present, return Ok(ParsedFrame::Raw) —
        //    non-Modbus frames routed through this parser see Raw (mirror
        //    Nmea's non-NMEA → Raw behavior). The body MUST have an even
        //    number of hex chars (each decoded byte = 2 hex chars). Odd
        //    length → Raw (malformed).
        let body_str = match std::str::from_utf8(&content) {
            Ok(s) => s,
            Err(_) => return Ok(ParsedFrame::Raw),
        };
        if body_str.is_empty() || body_str.bytes().any(|b| !b.is_ascii_hexdigit()) {
            return Ok(ParsedFrame::Raw);
        }
        if body_str.len() % 2 != 0 {
            return Ok(ParsedFrame::Raw);
        }

        // 3. Hex-decode the body into bytes.
        let decoded: Vec<u8> = (0..body_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&body_str[i..i + 2], 16).unwrap_or(0))
            .collect();
        // decoded = [address, function_code, data..., lrc]

        // 4. Split off the LRC (last byte). Need at least 3 bytes
        //    (address + function + lrc) for a minimal frame; fewer → Raw.
        if decoded.len() < 3 {
            return Ok(ParsedFrame::Raw);
        }
        let lrc_received = decoded[decoded.len() - 1];
        let pdu = &decoded[..decoded.len() - 1]; // address + function + data
        let address = pdu[0];
        let function_code = pdu[1];
        let data = pdu[2..].to_vec();

        // 5. Validate LRC over the PDU (address + function + data — NOT the LRC byte).
        let computed = Lrc.compute(pdu);
        let computed_val = computed[0];
        let checksum_valid = if self.validate {
            if computed_val != lrc_received {
                return Err(FrameDecodeError::ChecksumMismatch {
                    expected: computed,
                    received: vec![lrc_received],
                });
            }
            Some(true)
        } else {
            Some(computed_val == lrc_received)
        };

        // 6. Return ParsedFrame::ModbusAscii.
        Ok(ParsedFrame::ModbusAscii {
            address,
            function_code,
            data,
            checksum_valid,
        })
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
    /// COBS invalid code byte (impossible run length or truncated frame).
    CobsInvalidCode(u8),
    /// A protocol checksum (e.g. NMEA *XX XOR) did not match the recomputed value.
    /// `expected` is the recomputed checksum bytes; `received` is the frame's value.
    ChecksumMismatch {
        expected: Vec<u8>,
        received: Vec<u8>,
    },
}

impl std::fmt::Display for FrameDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameDecodeError::SlipInvalidEscape(b) => {
                write!(f, "SLIP framing error: invalid escape byte 0x{b:02X}")
            }
            FrameDecodeError::CobsInvalidCode(b) => {
                write!(f, "COBS framing error: invalid code byte 0x{b:02X}")
            }
            FrameDecodeError::ChecksumMismatch { expected, received } => {
                let exp = expected
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join("");
                let rcv = received
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join("");
                write!(f, "checksum mismatch: expected {exp}, received {rcv}")
            }
        }
    }
}

impl std::error::Error for FrameDecodeError {}

/// Whether a frame's data should be skipped under `skip_empty`: true when the
/// data is empty or contains only ASCII whitespace bytes (space, \t, \r, \n,
/// \x0b, \x0c).
fn is_blank_frame(data: &[u8]) -> bool {
    data.iter().all(|&b| b.is_ascii_whitespace())
}

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

/// COBS-encode a payload for TX framing using plain COBS (delimiter 0x00).
/// The encoded block contains no 0x00 bytes. The caller wraps the result as
/// `[0x00][block][0x00]` (SLIP-parity framing).
///
/// Plain COBS treats the payload as ending with a phantom trailing 0x00.
/// Codes 0x01-0xFE mean "the next 0x00 is `code` bytes ahead"; the receiver
/// inserts a 0x00 after `code-1` data bytes. Code 0xFF means "254 non-zero
/// data bytes follow, no implicit zero" (continuation for runs > 253).
fn cobs_stuff(payload: &[u8]) -> Vec<u8> {
    // Worst-case overhead: ceil(payload_len / 254) + 1 code bytes.
    let mut out: Vec<u8> = Vec::with_capacity(payload.len() + payload.len() / 254 + 2);
    // `code_index` tracks the placeholder position for the current block's
    // code byte; `code` counts bytes since the last zero (the distance to the
    // next zero, including the zero itself).
    let mut code_index: usize = 0;
    out.push(0x00); // placeholder for the first code byte
    let mut code: u8 = 1;

    for &b in payload {
        if b == 0x00 {
            // End of a run: write the code at the placeholder, start a new
            // block with a fresh placeholder.
            out[code_index] = code;
            code = 1;
            code_index = out.len();
            out.push(0x00); // placeholder for the next code
        } else {
            out.push(b);
            code = code.saturating_add(1);
            // 254 non-zero bytes with no zero: emit a continuation block
            // (code 0xFF = "254 data bytes, no implicit zero") and start a
            // new block. This keeps code values in 0x01-0xFF and bounds the
            // overhead.
            if code == 0xFF {
                out[code_index] = 0xFF;
                code = 1;
                code_index = out.len();
                out.push(0x00); // placeholder for the next code
            }
        }
    }
    // Final code for the phantom trailing zero.
    out[code_index] = code;
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

    /// Assert that a ProtocolPreset variant round-trips through the tagged-object
    /// JSON shape `{"type": "<variant_str>"}` and rejects the bare-string form.
    fn assert_preset_roundtrip(variant_str: &str, expected: ProtocolPreset) {
        let val = serde_json::json!({ "type": variant_str });
        let p: ProtocolPreset = serde_json::from_value(val.clone()).unwrap();
        assert_eq!(p, expected);
        assert_eq!(serde_json::to_value(p).unwrap(), val);
        assert!(
            serde_json::from_value::<ProtocolPreset>(serde_json::json!(variant_str)).is_err(),
            "bare string form {variant_str:?} must be rejected (tagged-object shape required)"
        );
    }

    #[test]
    fn protocol_preset_tagged_object_roundtrip() {
        assert_preset_roundtrip("at_command", ProtocolPreset::AtCommand);
    }

    #[test]
    fn preset_tx_framing_slip_returns_slip_mode() {
        let cfg = preset_tx_framing(ProtocolPreset::Slip);
        assert!(matches!(cfg.mode, TxFramingMode::Slip));
    }

    #[test]
    fn preset_tx_framing_json_lines_returns_line_lf() {
        let cfg = preset_tx_framing(ProtocolPreset::JsonLines);
        assert!(matches!(
            cfg.mode,
            TxFramingMode::Line {
                ending: TxLineEnding::Lf
            }
        ));
    }

    #[test]
    fn preset_rx_framing_slip_returns_slip_mode() {
        let cfg = preset_rx_framing(ProtocolPreset::Slip);
        assert!(matches!(cfg.mode, RxFramingMode::Slip));
    }

    #[test]
    fn preset_rx_framing_json_lines_returns_line_auto() {
        let cfg = preset_rx_framing(ProtocolPreset::JsonLines);
        assert!(matches!(
            cfg.mode,
            RxFramingMode::Line {
                ending: LineEnding::Auto
            }
        ));
    }

    #[test]
    fn preset_rx_parser_slip_returns_raw() {
        let cfg = preset_rx_parser(ProtocolPreset::Slip);
        assert_eq!(cfg.parser_type, ParserType::Raw);
    }

    #[test]
    fn preset_rx_parser_json_lines_returns_json_lines() {
        let cfg = preset_rx_parser(ProtocolPreset::JsonLines);
        assert_eq!(cfg.parser_type, ParserType::JsonLines);
    }

    #[test]
    fn protocol_preset_slip_tagged_object_roundtrip() {
        assert_preset_roundtrip("slip", ProtocolPreset::Slip);
    }

    #[test]
    fn protocol_preset_json_lines_tagged_object_roundtrip() {
        assert_preset_roundtrip("json_lines", ProtocolPreset::JsonLines);
    }

    #[test]
    fn slip_preset_equivalent_to_bare_slip_framing() {
        // TX: preset must match hand-built TxFramingConfig with Slip mode.
        let preset_tx = preset_tx_framing(ProtocolPreset::Slip);
        let bare_tx = TxFramingConfig {
            mode: TxFramingMode::Slip,
        };
        assert_eq!(preset_tx, bare_tx);
        // RX: preset must match hand-built RxFramingConfig with Slip mode.
        let preset_rx = preset_rx_framing(ProtocolPreset::Slip);
        let bare_rx = RxFramingConfig {
            mode: RxFramingMode::Slip,
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        assert_eq!(preset_rx, bare_rx);
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
            Err(_) => panic!("unexpected error variant"),
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

    #[test]
    fn slip_parser_error_propagates_and_resets_state() {
        // SLIP-frame a NMEA sentence with a BAD checksum. The NmeaParser with
        // validate:true returns Err(ChecksumMismatch) when it sees the bad *XX.
        // This exercises slip_decode's parser-Err path (src/framing.rs:770-774):
        // drain consumed bytes, reset state to BeforeFirstEnd, return Err.
        // Then push a well-formed SLIP-framed NMEA sentence and confirm the
        // decoder recovers (state was reset — no stale escaped/buf corruption).
        use crate::checksums::XorChecksum;

        let bad_body = b"GPGLL,3751.65,N,12226.54,W*00"; // wrong checksum (correct is 7E)
        let good_body = b"GPGLL,3751.65,N,12226.54,W";
        let good_cs = XorChecksum.compute(good_body)[0];
        let good_sentence = format!("GPGLL,3751.65,N,12226.54,W*{good_cs:02X}");

        let framing = RxFramingConfig {
            mode: RxFramingMode::Slip,
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&framing, Some(&parser)).unwrap();

        // Push 1: bad-checksum frame → Err(ChecksumMismatch).
        let mut push1 = vec![SLIP_END];
        push1.extend_from_slice(bad_body);
        push1.push(SLIP_END);
        let result = dec.push(&push1);
        assert!(
            result.is_err(),
            "SLIP+parser must propagate the ChecksumMismatch"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, FrameDecodeError::ChecksumMismatch { .. }),
            "expected ChecksumMismatch, got {err:?}"
        );

        // Push 2: good-checksum frame → one clean Nmea frame (state was reset
        // to BeforeFirstEnd by the error path; no stale escaped/buf corruption).
        let mut push2 = vec![SLIP_END];
        push2.extend_from_slice(good_sentence.as_bytes());
        push2.push(SLIP_END);
        let frames = dec.push(&push2).unwrap();
        assert_eq!(
            frames.len(),
            1,
            "decoder must recover after the parser error"
        );
        assert!(
            matches!(
                &frames[0].parsed,
                Some(ParsedFrame::Nmea {
                    checksum_valid: Some(true),
                    ..
                })
            ),
            "expected a clean Nmea frame after recovery, got {:?}",
            frames[0].parsed
        );
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
                start: vec!["STX".into()],
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
                start: vec!["<".into()],
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
        let result = p.parse(b"OK").unwrap();
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
        let result = p.parse(b"ERROR").unwrap();
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
        let result = p.parse(b"+CGREG: 0,1").unwrap();
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
        let result = p.parse(b"{\"key\":\"value\"}").unwrap();
        assert!(matches!(result, ParsedFrame::Json(_)));
    }

    #[test]
    fn json_parser_invalid() {
        let p = JsonLinesParser;
        let result = p.parse(b"not json").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn json_parser_non_object_is_raw() {
        let p = JsonLinesParser;
        assert!(matches!(p.parse(b"[1,2,3]").unwrap(), ParsedFrame::Raw));
        assert!(matches!(p.parse(b"42").unwrap(), ParsedFrame::Raw));
        assert!(matches!(p.parse(b"\"hi\"").unwrap(), ParsedFrame::Raw));
        assert!(matches!(
            p.parse(b"{\"k\":1}").unwrap(),
            ParsedFrame::Json(_)
        ));
    }

    #[test]
    fn shell_prompt_user() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"$ ").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "user")
        );
    }

    #[test]
    fn shell_prompt_root() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"# ").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "root")
        );
    }

    #[test]
    fn shell_prompt_host() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"root@host:~# ").unwrap();
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
            validate: false,
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
                start: vec!["STX".into()],
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
                start: vec!["<".into()],
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
                start: vec!["".into()],
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
                start: vec!["ABC".into()],
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
                start: vec!["!!!".into()],
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
        let result = p.parse(b"").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn at_parser_cme_error() {
        let p = AtCommandParser;
        let result = p.parse(b"+CME ERROR: 100").unwrap();
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
        let result = p.parse(b"+CMS ERROR: 500").unwrap();
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
        let result = p.parse(b"").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn shell_prompt_empty_input() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"").unwrap();
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
            validate: false,
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
        let result = p.parse(b">>> ").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "custom")
        );
    }

    #[test]
    fn raw_parser_passthrough() {
        let p = RawParser;
        let result = p.parse(b"anything").unwrap();
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
            validate: false,
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
                start: vec!["STX".into()],
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
            start: vec!["<".into()],
            end: ">".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let framed = mode.encode(b"data").unwrap();
        assert_eq!(framed, b"<data>");
    }

    #[test]
    fn tx_start_end_empty_markers_rejected() {
        let mode = TxFramingMode::StartEnd {
            start: vec!["".into()],
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
            start: vec!["STX".into()],
            end: "ETX".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let rx_config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["STX".into()],
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

    // ── COBS framing tests ───────────────────────────────────────────────

    fn cobs_rx_config() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Cobs,
            ..Default::default()
        }
    }

    #[test]
    fn cobs_stuff_preserves_payload_without_delimiter() {
        // Encode a payload containing the 0x00 delimiter byte; the encoded block
        // must NOT contain the 0x00 delimiter byte.
        let stuffed = cobs_stuff(&[0x00, 0x41, 0x00]);
        assert!(
            !stuffed.contains(&0x00),
            "encoded block must not contain delimiter"
        );
    }

    #[test]
    fn tx_cobs_encodes_block_then_delimiter() {
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"hi").unwrap();
        // Format: [delim] [stuffed] [delim]. Body between delimiters
        // must contain no 0x00.
        assert_eq!(framed.first(), Some(&0x00));
        assert_eq!(framed.last(), Some(&0x00));
        let body = &framed[1..framed.len() - 1];
        assert!(!body.contains(&0x00), "body must not contain delimiter");
        assert!(
            !body.is_empty(),
            "body must not be empty for non-empty payload"
        );
    }

    #[test]
    fn tx_cobs_stuffs_payload_with_delimiter_byte() {
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&[0x00]).unwrap();
        // Payload [0x00]: two code blocks of 0x01 (distance to next 0x00 is 1).
        // Stuffed = [0x01, 0x01]. Framed = [0x00, 0x01, 0x01, 0x00].
        assert_eq!(framed, &[0x00, 0x01, 0x01, 0x00]);
    }

    #[test]
    fn rx_cobs_skips_to_first_delimiter() {
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Prepend junk to a valid COBS-encoded frame.
        let mode = TxFramingMode::Cobs;
        let good_frame = mode.encode(b"hi").unwrap();
        let mut input = b"junk".to_vec();
        input.extend_from_slice(&good_frame);
        let frames = dec.push(&input).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");
    }

    #[test]
    fn rx_cobs_decodes_basic_frame() {
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"hello").unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn rx_cobs_decodes_delimiter_byte_in_payload() {
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Payload: [0x41, 0x00, 0x42]. Encoded: code 0x02 (dist to first 0x00
        // is 2: 1 data byte 'A' + the zero), then 'A', then code 0x02 (dist
        // to phantom zero: 'B' + phantom = 2), then 'B'.
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&[0x41, 0x00, 0x42]).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, &[0x41, 0x00, 0x42]);
    }

    #[test]
    fn roundtrip_cobs_arbitrary_binary() {
        let payload: &[u8] = &[0x00, 0xFF, 0x41, 0x00, 0x00, 0xFF, 0x7E];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_empty_payload() {
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"").unwrap();
        // Empty payload: [delim][code 0x01][delim] → [0x00, 0x01, 0x00].
        assert_eq!(framed, &[0x00, 0x01, 0x00]);
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.is_empty());
    }

    #[test]
    fn roundtrip_cobs_max_overhead() {
        // 254-byte all-zero payload (all delimiter bytes). This is the
        // worst-case overhead: every input byte is a delimiter, so each
        // code block encodes 0 data bytes with code 0x01.
        let payload = vec![0x00u8; 254];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_long_zero_run_300() {
        // 300 zero bytes: exercises multiple COBS code blocks and the
        // phantom-zero reinsertion. Regression guard for the 0x7E bug class
        // (the bug inserted the delimiter as the phantom; with 0x00 the
        // phantom is 0x00 and this round-trips correctly).
        let payload = vec![0u8; 300];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn rx_cobs_consecutive_delimiters_emit_empty_frame() {
        // With the canonical plain-COBS decoder, every byte value 0x01-0xFF
        // is a valid code and 0x00 is the frame delimiter, so CobsInvalidCode
        // is unreachable in practice. This test verifies that consecutive
        // delimiter bytes are handled correctly: the first is consumed as a
        // leading delimiter, the second terminates the (empty) frame.
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Two delimiter bytes: first consumed by BeforeFirstDelim, second
        // emits an empty frame.
        let frames = dec.push(b"\x00\x00").unwrap();
        assert_eq!(
            frames.len(),
            1,
            "2 consecutive delimiters yield 1 empty frame"
        );
        assert!(frames[0].data.is_empty());
    }

    #[test]
    fn rx_cobs_handles_between_frame_delimiters() {
        // After emitting a frame on a delimiter, the decoder transitions to
        // BeforeFirstDelim and skips any additional delimiters until the next
        // frame's stuffed block. This test verifies that a stream of
        // delimiters + a valid frame + more delimiters is handled correctly.
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Two consecutive delimiter bytes → one empty frame, then sync for
        // next frame.
        let frames = dec.push(b"\x00\x00").unwrap();
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.is_empty());
        // Subsequent valid frame decodes correctly.
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"ok").unwrap();
        let frames = dec.push(&framed).unwrap();
        assert!(!frames.is_empty());
        // The last frame should be "ok".
        assert_eq!(frames.last().unwrap().data, b"ok");
    }

    #[test]
    fn cobs_parser_error_propagates_and_resets_state() {
        // COBS-frame a NMEA sentence with a BAD checksum. The NmeaParser with
        // validate:true returns Err(ChecksumMismatch) when it sees the bad *XX.
        // This exercises cobs_decode's parser-Err path (src/framing.rs:892-898):
        // drain consumed bytes, clear decoded/remaining/pending_zero, reset
        // state to BeforeFirstDelim, return Err.
        // Then push a well-formed COBS-framed NMEA sentence and confirm the
        // decoder recovers (state was reset — no stale decoded/remaining/
        // pending_zero corruption).
        use crate::checksums::XorChecksum;

        let bad_body = b"GPGLL,3751.65,N,12226.54,W*00"; // wrong checksum (correct is 7E)
        let good_body = b"GPGLL,3751.65,N,12226.54,W";
        let good_cs = XorChecksum.compute(good_body)[0];
        let good_sentence = format!("GPGLL,3751.65,N,12226.54,W*{good_cs:02X}");

        // COBS-encode each sentence via TxFramingMode::Cobs (which produces
        // [0x00][cobs-stuffed block][0x00]).
        let bad_framed = TxFramingMode::Cobs.encode(bad_body).unwrap();
        let good_framed = TxFramingMode::Cobs
            .encode(good_sentence.as_bytes())
            .unwrap();

        let framing = RxFramingConfig {
            mode: RxFramingMode::Cobs,
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&framing, Some(&parser)).unwrap();

        // Push 1: bad-checksum frame → Err(ChecksumMismatch).
        let result = dec.push(&bad_framed);
        assert!(
            result.is_err(),
            "COBS+parser must propagate ChecksumMismatch"
        );
        assert!(
            matches!(
                result.unwrap_err(),
                FrameDecodeError::ChecksumMismatch { .. }
            ),
            "expected ChecksumMismatch"
        );

        // Push 2: good-checksum frame → one clean Nmea frame (state was reset
        // to BeforeFirstDelim by the error path; no stale decoded/remaining/
        // pending_zero corruption).
        let frames = dec.push(&good_framed).unwrap();
        assert_eq!(
            frames.len(),
            1,
            "decoder must recover after the parser error"
        );
        assert!(
            matches!(
                &frames[0].parsed,
                Some(ParsedFrame::Nmea {
                    checksum_valid: Some(true),
                    ..
                })
            ),
            "expected a clean Nmea frame after recovery, got {:?}",
            frames[0].parsed
        );
    }

    // ── COBS preset mapping tests ────────────────────────────────────────

    #[test]
    fn preset_tx_framing_cobs_returns_cobs() {
        let cfg = preset_tx_framing(ProtocolPreset::Cobs);
        assert!(matches!(cfg.mode, TxFramingMode::Cobs));
    }

    #[test]
    fn preset_rx_framing_cobs_returns_cobs() {
        let cfg = preset_rx_framing(ProtocolPreset::Cobs);
        assert!(matches!(cfg.mode, RxFramingMode::Cobs));
    }

    #[test]
    fn preset_rx_parser_cobs_returns_raw() {
        let cfg = preset_rx_parser(ProtocolPreset::Cobs);
        assert_eq!(cfg.parser_type, ParserType::Raw);
    }

    #[test]
    fn protocol_preset_cobs_tagged_object_roundtrip() {
        assert_preset_roundtrip("cobs", ProtocolPreset::Cobs);
    }

    #[test]
    fn cobs_preset_equivalent_to_bare_cobs_framing() {
        // TX: preset must match hand-built TxFramingConfig with Cobs mode.
        let preset_tx = preset_tx_framing(ProtocolPreset::Cobs);
        let bare_tx = TxFramingConfig {
            mode: TxFramingMode::Cobs,
        };
        assert_eq!(preset_tx, bare_tx);
        // RX: preset must match hand-built RxFramingConfig with Cobs mode.
        let preset_rx = preset_rx_framing(ProtocolPreset::Cobs);
        let bare_rx = RxFramingConfig {
            mode: RxFramingMode::Cobs,
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        assert_eq!(preset_rx, bare_rx);
    }

    #[test]
    fn roundtrip_cobs_255_ones() {
        let payload = vec![1u8; 255];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_254_ones_then_zero() {
        // Regression: 254 non-zero bytes then a trailing zero. This crossed
        // the continuation boundary and dropped the trailing zero in the
        // pre-canonical implementation.
        let mut payload = vec![1u8; 254];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_255_ones_then_zero() {
        let mut payload = vec![1u8; 255];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_256_ones_then_zero() {
        // Crosses two continuation blocks.
        let mut payload = vec![1u8; 256];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_510_ones_then_zero() {
        // Two full continuation blocks then a zero.
        let mut payload = vec![1u8; 510];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_alternating_ones_zeros_300() {
        // 300 bytes alternating 0x01 / 0x00 — heavy zero density across the
        // continuation boundary.
        let payload: Vec<u8> = (0..300).map(|i| if i % 2 == 0 { 1 } else { 0 }).collect();
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_all_byte_values_256() {
        // Every byte value 0x00-0xFF once. Exercises embedded 0x00 and 0xFF
        // (which is a non-zero data byte, NOT a code, when it appears in the
        // data portion).
        let payload: Vec<u8> = (0..=255u8).collect();
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    // ── skip_empty framing option ──────────────────────────────────────────

    #[test]
    fn skip_empty_default_is_false() {
        assert!(!RxFramingConfig::default().skip_empty);
    }

    #[test]
    fn skip_empty_drops_empty_line_frames() {
        // Line framing (auto), input b"a\n\nb\n", skip_empty: true → 2 frames.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\n\nb\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");

        // skip_empty: false → 3 frames (a, empty, b).
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"a\n\nb\n").unwrap();
        assert_eq!(frames_off.len(), 3);
        assert_eq!(frames_off[0].data, b"a");
        assert!(frames_off[1].data.is_empty());
        assert_eq!(frames_off[2].data, b"b");
    }

    #[test]
    fn skip_empty_drops_whitespace_only_frames() {
        let input = b"a\n   \nb\n";
        // skip_empty: true → middle line (3 spaces) skipped.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(input).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");

        // skip_empty: false → 3 frames.
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(input).unwrap();
        assert_eq!(frames_off.len(), 3);
        assert_eq!(frames_off[0].data, b"a");
        assert_eq!(frames_off[1].data, b"   ");
        assert_eq!(frames_off[2].data, b"b");
    }

    #[test]
    fn skip_empty_drops_leading_and_trailing_blank_lines() {
        // Input has leading, inner, and trailing blank lines.
        let input = b"\n{\"k\":1}\n\n{\"k\":2}\n\n";
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let parser = ParserConfig {
            parser_type: ParserType::JsonLines,
            custom_prompt: None,
            validate: false,
        };
        let mut dec = FrameDecoder::new(&config, Some(&parser)).unwrap();
        let frames = dec.push(input).unwrap();
        // skip_empty: true → 2 Json frames, blank lines skipped.
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0].parsed, Some(ParsedFrame::Json(_))));
        assert!(matches!(frames[1].parsed, Some(ParsedFrame::Json(_))));
    }

    #[test]
    fn skip_empty_does_not_count_skipped_frames_toward_max_frames() {
        // line framing, input b"\na\n\nb\n", skip_empty: true →
        // the empty lines do not appear in push output and do not increment
        // frame_count. Only the 2 non-blank frames (a, b) are emitted.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\na\n\nb\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[1].data, b"b");
        assert_eq!(frames[1].index, 1);
        // With skip_empty: false, the empty lines appear and consume
        // indices: frame 0 empty (leading \n), frame 1 'a', frame 2 empty
        // (inner \n), frame 3 'b'.
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"\na\n\nb\n").unwrap();
        assert_eq!(frames_off.len(), 4);
        assert!(frames_off[0].data.is_empty());
        assert_eq!(frames_off[0].index, 0);
        assert_eq!(frames_off[1].data, b"a");
        assert_eq!(frames_off[1].index, 1);
        assert!(frames_off[2].data.is_empty());
        assert_eq!(frames_off[2].index, 2);
        assert_eq!(frames_off[3].data, b"b");
        assert_eq!(frames_off[3].index, 3);
    }

    #[test]
    fn skip_empty_preserves_frame_index_continuity() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\n\nb\n").unwrap();
        assert_eq!(frames.len(), 2);
        // The skipped blank line does NOT consume an index.
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[1].index, 1);
    }

    #[test]
    fn skip_empty_applies_to_slip() {
        // Two consecutive END bytes produce one empty frame, then 'a'.
        // skip_empty: true → empty skipped, only 'a' emitted.
        let framed: &[u8] = &[SLIP_END, SLIP_END, b'a', SLIP_END];
        let config = RxFramingConfig {
            mode: RxFramingMode::Slip,
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[0].index, 0);
    }

    #[test]
    fn skip_empty_applies_to_cobs() {
        // Leading 0x00 sync, then back-to-back 0x00 delimiters for an empty
        // frame, then another 0x00 sync + COBS-stuffed 'a' + 0x00 delimiter.
        // skip_empty: true → empty skipped, only 'a' emitted.
        let framed: &[u8] = &[0x00, 0x00, 0x00, 0x02, 0x61, 0x00];
        let config = RxFramingConfig {
            mode: RxFramingMode::Cobs,
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(framed).unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[0].index, 0);
    }

    #[test]
    fn skip_empty_off_by_default_in_preset_json_lines() {
        assert!(!preset_rx_framing(ProtocolPreset::JsonLines).skip_empty);
    }

    #[test]
    fn skip_empty_on_in_preset_ndjson() {
        assert!(preset_rx_framing(ProtocolPreset::Ndjson).skip_empty);
    }

    #[test]
    fn preset_tx_framing_ndjson_returns_line_lf() {
        let tx = preset_tx_framing(ProtocolPreset::Ndjson);
        assert!(matches!(
            tx.mode,
            TxFramingMode::Line {
                ending: TxLineEnding::Lf
            }
        ));
    }

    #[test]
    fn preset_rx_framing_ndjson_returns_line_auto_skip_empty() {
        let rx = preset_rx_framing(ProtocolPreset::Ndjson);
        assert!(matches!(
            rx.mode,
            RxFramingMode::Line {
                ending: LineEnding::Auto
            }
        ));
        assert!(rx.skip_empty);
    }

    #[test]
    fn preset_rx_parser_ndjson_returns_json_lines() {
        let parser = preset_rx_parser(ProtocolPreset::Ndjson);
        assert!(matches!(parser.parser_type, ParserType::JsonLines));
    }

    #[test]
    fn protocol_preset_ndjson_tagged_object_roundtrip() {
        assert_preset_roundtrip("ndjson", ProtocolPreset::Ndjson);
    }

    #[test]
    fn ndjson_preset_equivalent_to_bare_config() {
        // TX config.
        let preset_tx = preset_tx_framing(ProtocolPreset::Ndjson);
        let bare_tx = TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Lf,
            },
        };
        assert_eq!(preset_tx, bare_tx);

        // RX config.
        let preset_rx = preset_rx_framing(ProtocolPreset::Ndjson);
        let bare_rx = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        assert_eq!(preset_rx, bare_rx);

        // Parser.
        let preset_parser = preset_rx_parser(ProtocolPreset::Ndjson);
        let bare_parser = ParserConfig {
            parser_type: ParserType::JsonLines,
            custom_prompt: None,
            validate: false,
        };
        assert_eq!(preset_parser, bare_parser);
    }

    #[test]
    fn flush_partial_does_not_apply_skip_empty() {
        // Line framing, skip_empty: true. Push partial content, then flush.
        // The partial frame is emitted even though it's a single character
        // (not blank), so this test verifies flush_partial emits frames
        // regardless of skip_empty. The bypass is structural: flush_partial
        // does not check skip_empty at all.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        // Push "b" without a newline — no frame emitted by push.
        let frames = dec.push(b"b").unwrap();
        assert!(frames.is_empty());
        // flush_partial emits the pending content regardless of skip_empty.
        let partial = dec.flush_partial();
        assert!(partial.is_some());
        assert_eq!(partial.unwrap().data, b"b");
    }

    // ── NMEA parser tests ──────────────────────────────────────────────

    /// Helper: build a NMEA sentence body with a computed XOR checksum.
    fn nmea_checksum_body(body: &[u8]) -> String {
        let cs = XorChecksum.compute(body);
        format!("{:02X}", cs[0])
    }

    #[test]
    fn nmea_parser_valid_gga_sentence() {
        let body = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
        let cs_hex = nmea_checksum_body(body);
        let sentence =
            format!("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GGA");
                assert_eq!(
                    fields,
                    vec![
                        "123519",
                        "4807.038",
                        "N",
                        "01131.000",
                        "E",
                        "1",
                        "08",
                        "0.9",
                        "545.4",
                        "M",
                        "46.9",
                        "M",
                        "",
                        ""
                    ]
                );
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_valid_gll_sentence() {
        // Known-good sentence from checksums::tests::xor_checksum_known_nmea_sentence
        // Body: "GPGLL,3751.65,N,12226.54,W" → XOR checksum 0x7E
        let sentence = b"GPGLL,3751.65,N,12226.54,W*7E";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(fields, vec!["3751.65", "N", "12226.54", "W"]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_ais_sentence_starts_with_bang() {
        // Simple AIS sentence body.
        let body = b"AIVDM,1,1,,B,15M67FC000G?ufbE`H9P<In,0";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("AIVDM,1,1,,B,15M67FC000G?ufbE`H9P<In,0*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                checksum_valid,
                ..
            } => {
                assert_eq!(talker_id, "AI");
                assert_eq!(sentence_type, "VDM");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_bad_checksum_returns_error_when_validate_true() {
        // GLL sentence with correct checksum 0x7E, but we pass *00 (wrong).
        let sentence = b"GPGLL,3751.65,N,12226.54,W*00";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence);
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected, vec![0x7E]);
                assert_eq!(received, vec![0x00]);
            }
            other => panic!("expected ChecksumMismatch error, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_bad_checksum_returns_some_false_when_validate_false() {
        let sentence = b"GPGLL,3751.65,N,12226.54,W*00";
        let p = NmeaParser { validate: false };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea { checksum_valid, .. } => {
                assert_eq!(checksum_valid, Some(false));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_no_checksum_accepted() {
        let sentence = b"GPGLL,3751.65,N,12226.54,W";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea { checksum_valid, .. } => {
                assert_eq!(checksum_valid, None);
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
        // Also test with validate: false
        let p2 = NmeaParser { validate: false };
        let result2 = p2.parse(sentence).unwrap();
        match result2 {
            ParsedFrame::Nmea { checksum_valid, .. } => {
                assert_eq!(checksum_valid, None);
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_non_nmea_frame_returns_raw() {
        let p = NmeaParser { validate: true };
        let result = p.parse(b"hello world").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));

        let result2 = p.parse(b"AT+CGMI").unwrap();
        assert!(matches!(result2, ParsedFrame::Raw));
    }

    #[test]
    fn nmea_parser_strips_leading_start_char_if_present() {
        // Simulate include_markers: true — the $ is included in the data.
        let body = b"GPGLL,3751.65,N,12226.54,W";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("$GPGLL,3751.65,N,12226.54,W*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                checksum_valid,
                ..
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_strips_trailing_crlf_if_present() {
        // Simulate include_markers: true — the trailing \r\n is included.
        let body = b"GPGLL,3751.65,N,12226.54,W";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("GPGLL,3751.65,N,12226.54,W*{cs_hex}\r\n");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                checksum_valid,
                ..
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_proprietary_sentence() {
        // $PGRMZ proprietary sentence without the $.
        let body = b"PGRMZ,2010,f,3";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("PGRMZ,2010,f,3*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                // Address is "PGRMZ" (5 chars): talker = first 2 = "PG", type = rest = "RMZ"
                assert_eq!(talker_id, "PG");
                assert_eq!(sentence_type, "RMZ");
                assert_eq!(fields, vec!["2010", "f", "3"]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    // ── StartEnd multi-marker tests ─────────────────────────────────────

    #[test]
    fn start_end_matches_either_of_multiple_start_markers() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$".into(), "!".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        // Input with $ marker
        let frames = dec.push(b"junk$hi\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");
        // Input with ! marker
        let frames = dec.push(b"junk!ok\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"ok");
    }

    #[test]
    fn start_end_tx_uses_first_marker() {
        let mode = TxFramingMode::StartEnd {
            start: vec!["<".into(), ">".into()],
            end: "|".into(),
            marker_encoding: PatternEncoding::Utf8,
        };
        let framed = mode.encode(b"hi").unwrap();
        // Uses start[0] = "<"
        assert_eq!(framed, b"<hi|");
    }

    #[test]
    fn start_end_empty_start_vec_rejected_at_construction() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec![],
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("empty start vec should be rejected"),
            Err(err) => assert!(
                err.contains("Start markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    #[test]
    fn start_end_existing_single_marker_still_works() {
        // Re-test start_end_basic but with start wrapped in vec![].
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["STX".into()],
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

    // ── NMEA preset tests ──────────────────────────────────────────────

    #[test]
    fn preset_tx_framing_nmea0183_returns_start_end_dollar() {
        let tx = preset_tx_framing(ProtocolPreset::Nmea0183);
        assert!(matches!(
            tx.mode,
            TxFramingMode::StartEnd {
                start,
                end,
                marker_encoding: PatternEncoding::Utf8,
            } if start == vec!["$".to_string()] && end == "\r\n"
        ));
    }

    #[test]
    fn preset_rx_framing_nmea0183_returns_start_end_dollar_bang() {
        let rx = preset_rx_framing(ProtocolPreset::Nmea0183);
        match &rx.mode {
            RxFramingMode::StartEnd {
                start,
                end,
                marker_encoding,
                include_markers,
            } => {
                assert_eq!(start, &vec!["$".to_string(), "!".to_string()]);
                assert_eq!(end, "\r\n");
                assert_eq!(*marker_encoding, PatternEncoding::Utf8);
                assert!(!include_markers);
            }
            other => panic!("expected StartEnd, got {other:?}"),
        }
    }

    #[test]
    fn preset_rx_parser_nmea0183_returns_nmea_validate_true() {
        let parser = preset_rx_parser(ProtocolPreset::Nmea0183);
        assert!(matches!(parser.parser_type, ParserType::Nmea));
        assert!(parser.validate);
    }

    #[test]
    fn protocol_preset_nmea0183_tagged_object_roundtrip() {
        assert_preset_roundtrip("nmea0183", ProtocolPreset::Nmea0183);
    }

    #[test]
    fn nmea0183_preset_equivalent_to_bare_config() {
        // TX
        let preset_tx = preset_tx_framing(ProtocolPreset::Nmea0183);
        let bare_tx = TxFramingConfig {
            mode: TxFramingMode::StartEnd {
                start: vec!["$".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
            },
        };
        assert_eq!(preset_tx, bare_tx);

        // RX
        let preset_rx = preset_rx_framing(ProtocolPreset::Nmea0183);
        let bare_rx = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$".into(), "!".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        assert_eq!(preset_rx, bare_rx);

        // Parser
        let preset_parser = preset_rx_parser(ProtocolPreset::Nmea0183);
        let bare_parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        assert_eq!(preset_parser, bare_parser);
    }

    // ── Checksum-failure-surfacing via push ────────────────────────────

    #[test]
    fn nmea_checksum_failure_surfaces_as_framing_error_via_push() {
        // Build a FrameDecoder with nmea0183 preset framing + parser.
        let rx_config = preset_rx_framing(ProtocolPreset::Nmea0183);
        let parser_config = preset_rx_parser(ProtocolPreset::Nmea0183);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        // Push a full NMEA sentence with bad checksum ($...*00\r\n)
        let result = dec.push(b"$GPGLL,3751.65,N,12226.54,W*00\r\n");
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected, vec![0x7E]);
                assert_eq!(received, vec![0x00]);
            }
            other => panic!("expected ChecksumMismatch error, got {other:?}"),
        }
    }

    #[test]
    fn nmea_valid_sentence_decodes_to_frame_with_parsed_nmea() {
        // GLL with correct checksum *7E
        let rx_config = preset_rx_framing(ProtocolPreset::Nmea0183);
        let parser_config = preset_rx_parser(ProtocolPreset::Nmea0183);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        let frames = dec.push(b"$GPGLL,3751.65,N,12226.54,W*7E\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        match &frames[0].parsed {
            Some(ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            }) => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(
                    fields,
                    &vec![
                        "3751.65".to_string(),
                        "N".to_string(),
                        "12226.54".to_string(),
                        "W".to_string()
                    ]
                );
                assert_eq!(*checksum_valid, Some(true));
            }
            other => panic!("expected Nmea parsed frame, got {other:?}"),
        }
    }

    // ── Modbus ASCII parser unit tests ───────────────────────────────────

    // Helper: compute LRC over a PDU byte slice and return the 1-byte hex string.
    fn modbus_lrc(pdu: &[u8]) -> String {
        let cs = Lrc.compute(pdu);
        format!("{:02X}", cs[0])
    }

    // Helper: build a Modbus ASCII hex body from PDU and append LRC.
    fn modbus_body(hex_pdu: &str) -> String {
        let pdu_bytes: Vec<u8> = (0..hex_pdu.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_pdu[i..i + 2], 16).unwrap())
            .collect();
        let lrc_hex = modbus_lrc(&pdu_bytes);
        format!("{hex_pdu}{lrc_hex}")
    }

    #[test]
    fn modbus_ascii_parser_valid_read_holding_registers() {
        // :010300000001FB\r\n — address=1, fc=3, data=[0,0,0,1], LRC=FB
        let body = modbus_body("010300000001");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                data,
                checksum_valid,
            } => {
                assert_eq!(address, 1);
                assert_eq!(function_code, 3);
                assert_eq!(data, vec![0, 0, 0, 1]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_valid_write_single_register() {
        // Address 0x01, FC 0x06, register 0x000A, value 0x0001
        // PDU = [0x01, 0x06, 0x00, 0x0A, 0x00, 0x01]
        // LRC = compute in test
        let pdu = [0x01u8, 0x06, 0x00, 0x0A, 0x00, 0x01];
        let lrc_hex = modbus_lrc(&pdu);
        let body = format!("0106000A0001{lrc_hex}");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                data,
                checksum_valid,
            } => {
                assert_eq!(address, 1);
                assert_eq!(function_code, 6);
                assert_eq!(data, vec![0x00, 0x0A, 0x00, 0x01]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_exception_response() {
        // Exception response: address 0x01, FC 0x83 (0x03 | 0x80), exception code 0x02
        let pdu = [0x01u8, 0x83, 0x02];
        let lrc_hex = modbus_lrc(&pdu);
        let body = format!("018302{lrc_hex}");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                data,
                checksum_valid,
            } => {
                assert_eq!(address, 1);
                assert_eq!(function_code, 131); // 0x83
                assert_eq!(data, vec![2]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_broadcast_address_zero() {
        let pdu = [0x00u8, 0x03, 0x00, 0x00, 0x00, 0x01];
        let lrc_hex = modbus_lrc(&pdu);
        let body = format!("000300000001{lrc_hex}");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                ..
            } => {
                assert_eq!(address, 0);
                assert_eq!(function_code, 3);
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_bad_lrc_returns_error_when_validate_true() {
        // Correct LRC is 0xFB, corrupt to 0x00
        let body = b"01030000000100";
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body);
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected, vec![0xFB]);
                assert_eq!(received, vec![0x00]);
            }
            other => panic!("expected ChecksumMismatch error, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_bad_lrc_returns_some_false_when_validate_false() {
        let body = b"01030000000100";
        let p = ModbusAsciiParser { validate: false };
        let result = p.parse(body).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                checksum_valid: Some(false),
                ..
            } => {}
            other => panic!("expected ModbusAscii with checksum_valid: false, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_lowercase_hex_accepted() {
        let body = b"010300000001fb";
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                checksum_valid: Some(true),
                ..
            } => {}
            other => panic!("expected valid ModbusAscii, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_non_hex_returns_raw() {
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"hello world").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_odd_length_returns_raw() {
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"010").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_too_short_returns_raw() {
        // "01" → 1 decoded byte, < 3
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"01").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_strips_leading_colon_and_trailing_crlf() {
        // Body with markers included — defensive stripping should still work.
        let pdu = [0x01u8, 0x03, 0x00, 0x00, 0x00, 0x01];
        let lrc_hex = modbus_lrc(&pdu);
        let frame = format!(":010300000001{lrc_hex}\r\n");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(frame.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                checksum_valid: Some(true),
                ..
            } => {}
            other => panic!("expected valid ModbusAscii, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_empty_body_returns_raw() {
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_minimal_frame_3_bytes() {
        // address 0x01, function 0xFF, LRC: sum=0x01+0xFF=0x00 (wrap), LRC=0x00
        let body = b"01FF00";
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 255,
                data,
                checksum_valid: Some(true),
            } if data.is_empty() => {}
            other => panic!("expected minimal ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_checksum_failure_surfaces_as_framing_error_via_push() {
        let rx_config = preset_rx_framing(ProtocolPreset::ModbusAscii);
        let parser_config = preset_rx_parser(ProtocolPreset::ModbusAscii);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        // Push a full frame with bad LRC: :01030000000100\r\n  (correct LRC is FB, here 00)
        let result = dec.push(b":01030000000100\r\n");
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected, vec![0xFB]);
                assert_eq!(received, vec![0x00]);
            }
            other => panic!("expected ChecksumMismatch error, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_valid_frame_decodes_to_frame_with_parsed_modbus_ascii() {
        let rx_config = preset_rx_framing(ProtocolPreset::ModbusAscii);
        let parser_config = preset_rx_parser(ProtocolPreset::ModbusAscii);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        // :010300000001FB\r\n  (valid read holding registers)
        let frames = dec.push(b":010300000001FB\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        match &frames[0].parsed {
            Some(ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                data,
                checksum_valid: Some(true),
            }) => {
                assert_eq!(data, &vec![0, 0, 0, 1]);
            }
            other => panic!("expected ModbusAscii parsed frame, got {other:?}"),
        }
    }

    // ── Modbus ASCII preset tests ──────────────────────────────────────

    #[test]
    fn preset_tx_framing_modbus_ascii_returns_start_end_colon() {
        let tx = preset_tx_framing(ProtocolPreset::ModbusAscii);
        assert!(matches!(
            tx.mode,
            TxFramingMode::StartEnd {
                start,
                end,
                marker_encoding: PatternEncoding::Utf8,
            } if start == vec![":".to_string()] && end == "\r\n"
        ));
    }

    #[test]
    fn preset_rx_framing_modbus_ascii_returns_start_end_colon_crlf() {
        let rx = preset_rx_framing(ProtocolPreset::ModbusAscii);
        match &rx.mode {
            RxFramingMode::StartEnd {
                start,
                end,
                marker_encoding,
                include_markers,
            } => {
                assert_eq!(start, &vec![":".to_string()]);
                assert_eq!(end, "\r\n");
                assert_eq!(*marker_encoding, PatternEncoding::Utf8);
                assert!(!include_markers);
            }
            other => panic!("expected StartEnd, got {other:?}"),
        }
    }

    #[test]
    fn preset_rx_parser_modbus_ascii_returns_modbus_ascii_validate_true() {
        let parser = preset_rx_parser(ProtocolPreset::ModbusAscii);
        assert!(matches!(parser.parser_type, ParserType::ModbusAscii));
        assert!(parser.validate);
    }

    #[test]
    fn protocol_preset_modbus_ascii_tagged_object_roundtrip() {
        assert_preset_roundtrip("modbus_ascii", ProtocolPreset::ModbusAscii);
    }

    #[test]
    fn modbus_ascii_preset_equivalent_to_bare_config() {
        // TX
        let preset_tx = preset_tx_framing(ProtocolPreset::ModbusAscii);
        let bare_tx = TxFramingConfig {
            mode: TxFramingMode::StartEnd {
                start: vec![":".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
            },
        };
        assert_eq!(preset_tx, bare_tx);

        // RX
        let preset_rx = preset_rx_framing(ProtocolPreset::ModbusAscii);
        let bare_rx = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec![":".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        assert_eq!(preset_rx, bare_rx);

        // Parser
        let preset_parser = preset_rx_parser(ProtocolPreset::ModbusAscii);
        let bare_parser = ParserConfig {
            parser_type: ParserType::ModbusAscii,
            custom_prompt: None,
            validate: true,
        };
        assert_eq!(preset_parser, bare_parser);
    }

    // ── Medium-risk gap tests (P3d) ───────────────────────────────────────

    #[test]
    fn skip_empty_applies_to_delimiter() {
        // Delimiter mode + skip_empty: empty frames between consecutive
        // delimiters are dropped. Mirror skip_empty_drops_empty_line_frames.
        let config_on = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config_on, None).unwrap();
        let frames = dec.push(b"a||b|").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");

        // skip_empty: false → 3 frames (a, empty, b).
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"a||b|").unwrap();
        assert_eq!(frames_off.len(), 3);
        assert_eq!(frames_off[0].data, b"a");
        assert!(frames_off[1].data.is_empty());
        assert_eq!(frames_off[2].data, b"b");
    }

    #[test]
    fn skip_empty_applies_to_start_end() {
        // StartEnd mode + skip_empty: the empty frame between a bare
        // "$" .. "\r\n" is dropped. Input b"$\r\n$hi\r\n" → two frames
        // (empty, "hi"). With skip_empty:true only the second frame emits.
        let config_on = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config_on, None).unwrap();
        let frames = dec.push(b"$\r\n$hi\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");

        // skip_empty: false → 2 frames (empty, "hi").
        let config_off = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"$\r\n$hi\r\n").unwrap();
        assert_eq!(frames_off.len(), 2);
        assert!(frames_off[0].data.is_empty());
        assert_eq!(frames_off[1].data, b"hi");
    }

    #[test]
    fn start_end_picks_earliest_of_overlapping_markers() {
        // Two start markers that share a prefix: "$G" and "$GPGGA".
        // Input: b"$GPGGA,x\r\n". Both markers match at position 0.
        // Tie-break: the first-listed marker in the start vec at the
        // earliest position wins. With start: ["$G", "$GPGGA"], "$G"
        // wins because it appears first in the list and both start at
        // offset 0. After draining the 2-byte "$G" marker, the body is
        // "PGGA,x" (the "$G" wins, not "$GPGGA").
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$G".into(), "$GPGGA".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"$GPGGA,x\r\n").unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"PGGA,x");
    }

    #[test]
    fn nmea_parser_multi_fragment_ais_first_sentence() {
        use crate::checksums::XorChecksum;

        // AIS multi-fragment messages are NOT reassembled by the parser —
        // each fragment is its own frame. This test pins the first-fragment
        // parse and documents the no-reassembly behavior.
        //
        // Verify the XOR checksum of the body between ! and * (exclusive).
        let checksum_body = b"AIVDM,2,1,3,B,55?MbV02;H0000,0";
        let computed = XorChecksum.compute(checksum_body);
        // The handoff documented checksum 0x5C but the correct XOR is 0x22.
        assert_eq!(computed, [0x22], "AIS sentence checksum is 0x22, not 0x5C");

        let sentence = b"!AIVDM,2,1,3,B,55?MbV02;H0000,0*22";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "AI");
                assert_eq!(sentence_type, "VDM");
                assert_eq!(fields, vec!["2", "1", "3", "B", "55?MbV02;H0000", "0"]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_preserves_whitespace_in_fields() {
        use crate::checksums::XorChecksum;

        // The parser uses split(',') without per-field trimming; this test
        // pins that whitespace inside a field is preserved. A future
        // "helpful" refactor adding .trim() per field would break this.
        let body_without_star = b"GPVTG,  48.7  ,T,,,N,,K,N";
        let cs = XorChecksum.compute(body_without_star);
        let cs_hex = format!("{:02X}", cs[0]);
        // cs_hex = "58" for this body.
        let sentence = format!(
            "${}*{}",
            std::str::from_utf8(body_without_star).unwrap(),
            cs_hex
        );
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "VTG");
                // The first data field (fields[0]) has leading + trailing
                // spaces — they are NOT trimmed.
                assert_eq!(fields[0], "  48.7  ");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_max_adu_length() {
        use crate::checksums::Lrc;

        // A 253-byte data payload near the Modbus ADU max. Pins no-length-cap
        // behavior. The frame is routed through a full FrameDecoder (StartEnd
        // framing with ':' start and "\r\n" end) + ModbusAscii parser.
        let address = 0x01u8;
        let function = 0x03u8;
        let data: Vec<u8> = (0..253u8).collect();
        let mut pdu = vec![address, function];
        pdu.extend_from_slice(&data);
        let lrc = Lrc.compute(&pdu);
        let lrc_byte = lrc[0];

        let mut all_bytes = pdu.clone();
        all_bytes.push(lrc_byte);
        let hex_body: String = all_bytes.iter().map(|b| format!("{b:02X}")).collect();
        let frame = format!(":{hex_body}\r\n");

        let rx_config = preset_rx_framing(ProtocolPreset::ModbusAscii);
        let parser_config = preset_rx_parser(ProtocolPreset::ModbusAscii);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        let frames = dec.push(frame.as_bytes()).unwrap();
        assert_eq!(frames.len(), 1);
        match &frames[0].parsed {
            Some(ParsedFrame::ModbusAscii {
                address: a,
                function_code: fc,
                data: d,
                checksum_valid: Some(true),
            }) => {
                assert_eq!(*a, 1);
                assert_eq!(*fc, 3);
                assert_eq!(d.len(), 253);
            }
            other => panic!("expected ModbusAscii parsed frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_non_ascii_body_returns_raw() {
        // Two branches in ModbusAsciiParser::parse that lead to Raw:
        //
        // Case 1: valid UTF-8 but non-ASCII → is_ascii_hexdigit fails.
        // b"01\xC3\xA900" = "01é00" where é is U+00E9 (valid UTF-8, non-
        // hex-digit byte 0xC3). The parser calls from_utf8 (Ok), then
        // is_ascii_hexdigit fails → Raw.
        let p = ModbusAsciiParser { validate: true };
        let body1 = b"01\xC3\xA900";
        let result1 = p.parse(body1).unwrap();
        assert!(
            matches!(result1, ParsedFrame::Raw),
            "non-ASCII valid-UTF-8 body should return Raw, got {result1:?}"
        );

        // Case 2: invalid UTF-8 → from_utf8 returns Err → Raw.
        // b"01\xFF00" — 0xFF is a lone byte >0x7F and not a valid UTF-8
        // continuation. from_utf8 returns Err immediately.
        let body2 = b"01\xFF00";
        let result2 = p.parse(body2).unwrap();
        assert!(
            matches!(result2, ParsedFrame::Raw),
            "invalid-UTF-8 body should return Raw, got {result2:?}"
        );

        // Sanity: a valid hex body with the same parser DOES parse to ModbusAscii.
        let pdu = [0x01u8, 0x03, 0x00, 0x00, 0x00, 0x01];
        let lrc_hex = {
            let cs = Lrc.compute(&pdu);
            format!("{:02X}", cs[0])
        };
        let body_valid = format!("010300000001{lrc_hex}");
        let result_valid = p.parse(body_valid.as_bytes()).unwrap();
        assert!(
            matches!(result_valid, ParsedFrame::ModbusAscii { .. }),
            "valid hex body should parse to ModbusAscii, got {result_valid:?}"
        );
    }
}
