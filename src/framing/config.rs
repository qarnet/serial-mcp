//! Framing and parser configuration types, defaults, and protocol presets.
//!
//! RX framing configuration (`RxFramingConfig`/`RxFramingMode`/`LineEnding`),
//! TX framing configuration (`TxFramingConfig`/`TxFramingMode`/`TxLineEnding`),
//! parser configuration (`ParserConfig`/`ParserType`), the shared
//! `Endianness` type, and the `ProtocolPreset` expansion functions.

use crate::match_config::PatternEncoding;
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
            mode: TxFramingMode::Nmea,
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
    /// inside an encoded block. The PPP-COBS draft variant (0x7E) is not
    /// supported; it is tracked for a future PR.
    #[serde(rename_all = "snake_case")]
    Cobs,
    /// NMEA-0183 sentence framing: `$<payload>*XX\r\n` with the `*XX` XOR
    /// checksum auto-appended over the payload bytes. Used by the `nmea0183`
    /// preset TX path. If the payload already ends in `*HH` (two hex chars
    /// after a `*`), the existing checksum is validated and a mismatch
    /// errors (no double-append). If the payload already starts with `$` or
    /// `!`, that leading char is used (AIS `!` sentences); otherwise `$` is
    /// prepended.
    #[serde(rename_all = "snake_case")]
    Nmea,
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
    /// *XX XOR, Modbus ASCII LRC) drops mismatched frames instead of emitting
    /// them. The dropped frame is counted in `PushOutcome.frames_dropped` and
    /// does NOT halt the read or subscribe (stream-fatal errors like SLIP
    /// malformed escapes still stop the decode). When false, the frame is
    /// emitted with `checksum_valid: Some(false)` (no-op for the caller).
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

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── Protocol preset tests (table-driven) ────────────────────────────────

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

    /// One row per protocol preset: expected framing/parser expansions + roundtrip label.
    struct PresetTestRow {
        preset: ProtocolPreset,
        wire_name: &'static str,
        expected_tx: TxFramingConfig,
        expected_rx: RxFramingConfig,
        expected_parser: ParserConfig,
        /// `None` = run the equivalence check. `Some(reason)` = skip (documented
        /// exemption for presets whose expansion cannot be expressed as a single
        /// bare framing/parser config).
        equivalence_skip: Option<&'static str>,
    }

    fn preset_test_table() -> Vec<PresetTestRow> {
        vec![
            PresetTestRow {
                preset: ProtocolPreset::AtCommand,
                wire_name: "at_command",
                expected_tx: TxFramingConfig {
                    mode: TxFramingMode::Line {
                        ending: TxLineEnding::Cr,
                    },
                },
                expected_rx: RxFramingConfig {
                    mode: RxFramingMode::Line {
                        ending: LineEnding::Auto,
                    },
                    max_frames: None,
                    include_terminators: false,
                    skip_empty: false,
                },
                expected_parser: ParserConfig {
                    parser_type: ParserType::AtCommand,
                    custom_prompt: None,
                    validate: false,
                },
                equivalence_skip: None,
            },
            PresetTestRow {
                preset: ProtocolPreset::Slip,
                wire_name: "slip",
                expected_tx: TxFramingConfig {
                    mode: TxFramingMode::Slip,
                },
                expected_rx: RxFramingConfig {
                    mode: RxFramingMode::Slip,
                    max_frames: None,
                    include_terminators: false,
                    skip_empty: false,
                },
                expected_parser: ParserConfig {
                    parser_type: ParserType::Raw,
                    custom_prompt: None,
                    validate: false,
                },
                equivalence_skip: None,
            },
            PresetTestRow {
                preset: ProtocolPreset::JsonLines,
                wire_name: "json_lines",
                expected_tx: TxFramingConfig {
                    mode: TxFramingMode::Line {
                        ending: TxLineEnding::Lf,
                    },
                },
                expected_rx: RxFramingConfig {
                    mode: RxFramingMode::Line {
                        ending: LineEnding::Auto,
                    },
                    max_frames: None,
                    include_terminators: false,
                    skip_empty: false,
                },
                expected_parser: ParserConfig {
                    parser_type: ParserType::JsonLines,
                    custom_prompt: None,
                    validate: false,
                },
                equivalence_skip: None,
            },
            PresetTestRow {
                preset: ProtocolPreset::Cobs,
                wire_name: "cobs",
                expected_tx: TxFramingConfig {
                    mode: TxFramingMode::Cobs,
                },
                expected_rx: RxFramingConfig {
                    mode: RxFramingMode::Cobs,
                    max_frames: None,
                    include_terminators: false,
                    skip_empty: false,
                },
                expected_parser: ParserConfig {
                    parser_type: ParserType::Raw,
                    custom_prompt: None,
                    validate: false,
                },
                equivalence_skip: None,
            },
            PresetTestRow {
                preset: ProtocolPreset::Ndjson,
                wire_name: "ndjson",
                expected_tx: TxFramingConfig {
                    mode: TxFramingMode::Line {
                        ending: TxLineEnding::Lf,
                    },
                },
                expected_rx: RxFramingConfig {
                    mode: RxFramingMode::Line {
                        ending: LineEnding::Auto,
                    },
                    max_frames: None,
                    include_terminators: false,
                    skip_empty: true,
                },
                expected_parser: ParserConfig {
                    parser_type: ParserType::JsonLines,
                    custom_prompt: None,
                    validate: false,
                },
                equivalence_skip: None,
            },
            PresetTestRow {
                preset: ProtocolPreset::Nmea0183,
                wire_name: "nmea0183",
                expected_tx: TxFramingConfig {
                    mode: TxFramingMode::Nmea,
                },
                expected_rx: RxFramingConfig {
                    mode: RxFramingMode::StartEnd {
                        start: vec!["$".into(), "!".into()],
                        end: "\r\n".into(),
                        marker_encoding: PatternEncoding::Utf8,
                        include_markers: false,
                    },
                    max_frames: None,
                    include_terminators: false,
                    skip_empty: false,
                },
                expected_parser: ParserConfig {
                    parser_type: ParserType::Nmea,
                    custom_prompt: None,
                    validate: true,
                },
                equivalence_skip: None,
            },
            PresetTestRow {
                preset: ProtocolPreset::ModbusAscii,
                wire_name: "modbus_ascii",
                expected_tx: TxFramingConfig {
                    mode: TxFramingMode::StartEnd {
                        start: vec![":".into()],
                        end: "\r\n".into(),
                        marker_encoding: PatternEncoding::Utf8,
                    },
                },
                expected_rx: RxFramingConfig {
                    mode: RxFramingMode::StartEnd {
                        start: vec![":".into()],
                        end: "\r\n".into(),
                        marker_encoding: PatternEncoding::Utf8,
                        include_markers: false,
                    },
                    max_frames: None,
                    include_terminators: false,
                    skip_empty: false,
                },
                expected_parser: ParserConfig {
                    parser_type: ParserType::ModbusAscii,
                    custom_prompt: None,
                    validate: true,
                },
                equivalence_skip: None,
            },
        ]
    }

    #[test]
    fn preset_tx_framing_matches_table() {
        for row in preset_test_table() {
            let cfg = preset_tx_framing(row.preset);
            assert_eq!(cfg.mode, row.expected_tx.mode);
        }
    }

    #[test]
    fn preset_rx_framing_matches_table() {
        for row in preset_test_table() {
            let cfg = preset_rx_framing(row.preset);
            assert_eq!(cfg.mode, row.expected_rx.mode);
            assert_eq!(cfg.skip_empty, row.expected_rx.skip_empty);
        }
    }

    #[test]
    fn preset_rx_parser_matches_table() {
        for row in preset_test_table() {
            let cfg = preset_rx_parser(row.preset);
            assert_eq!(cfg.parser_type, row.expected_parser.parser_type);
            assert_eq!(cfg.validate, row.expected_parser.validate);
        }
    }

    #[test]
    fn protocol_preset_tagged_object_roundtrips() {
        for row in preset_test_table() {
            assert_preset_roundtrip(row.wire_name, row.preset);
        }
    }

    #[test]
    fn presets_equivalent_to_bare_configs() {
        for row in preset_test_table() {
            if let Some(reason) = row.equivalence_skip {
                // Documented exemption, not a missing test.
                eprintln!("skipping equivalence for {}: {}", row.wire_name, reason);
                continue;
            }
            let preset_tx = preset_tx_framing(row.preset);
            assert_eq!(preset_tx, row.expected_tx, "{} TX", row.wire_name);

            let preset_rx = preset_rx_framing(row.preset);
            assert_eq!(preset_rx, row.expected_rx, "{} RX", row.wire_name);

            let preset_parser = preset_rx_parser(row.preset);
            assert_eq!(
                preset_parser, row.expected_parser,
                "{} parser",
                row.wire_name
            );
        }
    }

    #[test]
    fn skip_empty_default_is_false() {
        assert!(!RxFramingConfig::default().skip_empty);
    }

    #[test]
    fn skip_empty_off_by_default_in_preset_json_lines() {
        assert!(!preset_rx_framing(ProtocolPreset::JsonLines).skip_empty);
    }

    #[test]
    fn skip_empty_on_in_preset_ndjson() {
        assert!(preset_rx_framing(ProtocolPreset::Ndjson).skip_empty);
    }
}
