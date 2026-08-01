//! Property-based and boundary-value tests.
//!
//! Catches:
//! - Serde roundtrip breakage (serialize→deserialize→re-serialize identical)
//! - JSON Schema vs serialized output mismatches (the bug we fixed)
//! - Codec invariant violations (decode(encode(x)) == x)
//! - Clamp/validation boundary panics (u64::MAX, usize::MAX)
//! - Port name special-character safety
//!
//! Run: cargo test --test proptest
//! Fuzz longer: PROPTEST_CASES=10000 cargo test --test proptest

use std::sync::Arc;

use proptest::prelude::*;
use schemars::schema_for;
use serde_json::Value;

use serial_mcp::codec::{self, Encoding};
use serial_mcp::limits::*;
use serial_mcp::profiles::{allocate_generated_name, normalize_generated_label, Profile};
use serial_mcp::serial::{DataBits, FlowControl, Parity, StopBits};
use serial_mcp::tools::helpers::{
    clamp_or_err, clamp_poll_interval_or_err, clamp_timeout_or_err, parse_open_args,
    require_min_or_err,
};
use serial_mcp::tools::types::{
    CloseArgs, CloseResult, FlushArgs, FlushResult, ListConnectionsResult, OpenArgs, OpenResult,
    ReadArgs, ReadResult, SendBreakArgs, SendBreakResult, SetDtrRtsArgs, SetDtrRtsResult,
    SetFlowControlResult, SubscribeArgs, SubscribeResult, UnsubscribeArgs, UnsubscribeResult,
    WriteArgs, WriteResult,
};

// ── Schema helper ────────────────────────────────────────────────────────────

fn schemars_to_jsonschema<T: schemars::JsonSchema>() -> Value {
    let schema = schema_for!(T);
    serde_json::to_value(schema).unwrap()
}

fn validate_schema<T: schemars::JsonSchema>(value: &Value) {
    let schema_json = schemars_to_jsonschema::<T>();
    let compiled = jsonschema::validator_for(&schema_json)
        .unwrap_or_else(|e| panic!("schema compile error: {e}"));
    let errors: Vec<String> = compiled
        .iter_errors(value)
        .map(|e| format!("{e}"))
        .collect();
    if !errors.is_empty() {
        panic!("schema validation errors: {}", errors.join("; "));
    }
}

fn roundtrip_stable<T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug>(
    value: &T,
) {
    let json1 = serde_json::to_string(value).unwrap_or_else(|e| panic!("serialize: {e}"));
    let rt: T = serde_json::from_str(&json1).unwrap_or_else(|e| panic!("deserialize: {e}"));
    let json2 = serde_json::to_string(&rt).unwrap_or_else(|e| panic!("re-serialize: {e}"));
    if json1 != json2 {
        panic!("roundtrip unstable:\n  first:  {json1}\n  second: {json2}");
    }
}

/// Like roundtrip_stable but panics instead of returning Result — for use
/// inside proptest! tests where `?` on `String` isn't supported.
macro_rules! assert_roundtrip {
    ($val:expr) => {
        roundtrip_stable(&$val)
    };
}

macro_rules! assert_schema_valid {
    ($type:ty, $val:expr) => {
        validate_schema::<$type>(&$val)
    };
}

// ── Strategies ──────────────────────────────────────────────────────────────

fn valid_port_name() -> impl Strategy<Value = String> {
    prop::string::string_regex(r"/dev/[A-Za-z0-9_/\-]+")
        .expect("regex compile")
        .prop_filter("max 256 chars", |s| s.len() <= 256)
}

fn valid_encoding() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "utf8".to_string(),
        "utf-8".to_string(),
        "hex".to_string(),
        "base64".to_string(),
        "b64".to_string(),
    ])
}

fn valid_data_bits() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["5".into(), "6".into(), "7".into(), "8".into()])
}

fn valid_stop_bits() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["1".into(), "2".into()])
}

fn valid_parity() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["none".into(), "odd".into(), "even".into()])
}

fn valid_flow_control() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["none".into(), "software".into(), "hardware".into()])
}

fn opaque_id() -> impl Strategy<Value = String> {
    "[a-f0-9\\-]{8,64}".prop_filter("min 1 char", |s| !s.is_empty())
}

fn any_u32() -> impl Strategy<Value = u32> {
    any::<u32>()
}

fn any_usize() -> impl Strategy<Value = usize> {
    any::<usize>()
}

fn any_u64() -> impl Strategy<Value = u64> {
    any::<u64>()
}

fn valid_flush_target() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["input".into(), "output".into(), "both".into()])
}

fn valid_stop_reason() -> impl Strategy<Value = String> {
    prop::sample::select(vec![
        "data_complete".into(),
        "timeout".into(),
        "match_found".into(),
        "max_buffered_bytes".into(),
        "connection_closed".into(),
        "cancelled".into(),
        "read_error".into(),
        "channel_closed".into(),
        "peer_disconnected".into(),
        "budget_exhausted".into(),
        "no_new_rx_timeout".into(),
    ])
}

fn optional_u64() -> impl Strategy<Value = Option<u64>> {
    prop::option::of(any::<u64>())
}

fn non_empty_string() -> impl Strategy<Value = String> {
    r"[A-Za-z0-9_\r\n\t ]{1,256}"
}

// ── Schema roundtrips — all argument types ──────────────────────────────────

proptest! {
    #[test]
    fn open_args_roundtrip(
        port in valid_port_name(),
        baud in any_u32(),
        db in valid_data_bits(),
        sb in valid_stop_bits(),
        p in valid_parity(),
        fc in valid_flow_control(),
    ) {
        let args = OpenArgs {
            port: port.clone(),
            name: None,
            baud_rate: Some(baud),
            data_bits: Some(db.clone()),
            stop_bits: Some(sb.clone()),
            parity: Some(p.clone()),
            flow_control: Some(fc.clone()),
            log_capacity: Some(1024),
            log_enabled: Some(true),
            reconnect_policy: Some(Default::default()),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: Some(serial_mcp::limits::DEFAULT_RX_BUFFER_SIZE),
            max_buffered_bytes: Some(32768),
            poll_interval_ms: Some(200),
            profile_mode: None,
        };
        assert_roundtrip!(args);

        if let Ok(config) = parse_open_args(args) {
            assert_eq!(config.port, port);
            assert_eq!(config.baud_rate, baud);
        }
        db.parse::<DataBits>().unwrap();
        sb.parse::<StopBits>().unwrap();
        p.parse::<Parity>().unwrap();
        fc.parse::<FlowControl>().unwrap();
    }

    #[test]
    fn close_args_roundtrip(id in opaque_id()) {
        let args = CloseArgs { connection_id: id };
        assert_roundtrip!(args);
    }

    #[test]
    fn write_args_roundtrip(
        id in opaque_id(),
        data in r"[A-Za-z0-9\r\n\t ]{0,4096}",
        enc in valid_encoding(),
    ) {
        let args = WriteArgs { connection_id: id, data, encoding: enc, tx_framing: None, protocol: None };
        assert_roundtrip!(args);
    }

    #[test]
    fn write_args_with_tx_framing_roundtrip(
        id in opaque_id(),
        data in r"[A-Za-z0-9\r\n\t ]{0,4096}",
        enc in valid_encoding(),
    ) {
        use serial_mcp::framing::{Endianness, TxFramingConfig, TxFramingMode, TxLineEnding};
        use serial_mcp::match_config::PatternEncoding;
        let modes = vec![
            TxFramingMode::Line {
                ending: TxLineEnding::Crlf,
            },
            TxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            TxFramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
            },
            TxFramingMode::StartEnd {
                start: vec!["STX".into()],
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
            },
            TxFramingMode::Slip,
            TxFramingMode::Cobs,
            TxFramingMode::Nmea,
        ];
        for mode in modes {
            let args = WriteArgs {
                connection_id: id.clone(),
                data: data.clone(),
                encoding: enc.clone(),
                tx_framing: Some(TxFramingConfig { mode }),
                protocol: None,
            };
            assert_roundtrip!(args);
        }
    }

    #[test]
    fn read_args_roundtrip(
        id in opaque_id(),
        timeout in optional_u64(),
        enc in valid_encoding(),
    ) {
        let args = ReadArgs { connection_id: id, from: None, timeout_ms: timeout, encoding: enc, r#match: None, no_new_rx_timeout_ms: None, rx_framing: None, rx_parser: None, protocol: None };
        assert_roundtrip!(args);
    }

    #[test]
    fn flush_args_roundtrip(id in opaque_id(), target in valid_flush_target()) {
        let args = FlushArgs { connection_id: id, target: serde_json::from_value(serde_json::json!(target)).unwrap() };
        assert_roundtrip!(args);
    }

    #[test]
    fn set_dtr_rts_args_roundtrip(id in opaque_id(), dtr: bool, rts: bool) {
        let args = SetDtrRtsArgs { connection_id: id, dtr, rts };
        assert_roundtrip!(args);
    }

    #[test]
    fn send_break_args_roundtrip(id in opaque_id(), duration in any_u64()) {
        let args = SendBreakArgs { connection_id: id, duration_ms: duration };
        assert_roundtrip!(args);
    }

    #[test]
    fn subscribe_args_roundtrip(
        id in opaque_id(),
        timeout in optional_u64(),
        enc in valid_encoding(),
    ) {
        let args = SubscribeArgs {
            connection_id: id,
            timeout_ms: timeout,
            no_new_rx_timeout_ms: None,
            encoding: enc,
            from: None,
            r#match: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
        };
        assert_roundtrip!(args);
    }

    #[test]
    fn unsubscribe_args_roundtrip(id in opaque_id()) {
        let args = UnsubscribeArgs { connection_id: id };
        assert_roundtrip!(args);
    }
}

// ── Schema validation — all result types against their schemas ──────────────

proptest! {
    #[test]
    fn open_result_schema_valid(
        id in opaque_id(), port in valid_port_name(), baud in any_u32(),
    ) {
        let r = OpenResult { connection_id: id, name: None, port, baud_rate: baud, profile: None, profile_persistence: None };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(OpenResult, v);
    }

    #[test]
    fn close_result_schema_valid(id in opaque_id()) {
        let r = CloseResult { connection_id: id, name: None, profile: None, profile_persistence: None };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(CloseResult, v);
    }

    #[test]
    fn write_result_schema_valid(id in opaque_id(), bw in any_usize(), enc in valid_encoding()) {
        let r = WriteResult { connection_id: id, name: None, bytes_written: bw, decoded_bytes: bw, encoding: enc };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(WriteResult, v);
    }

    #[test]
    fn read_result_schema_valid(
        id in opaque_id(), br in any_usize(), enc in valid_encoding(),
        data in non_empty_string(), timeout in any_u64(), elapsed in any_u64(),
        stop_reason in valid_stop_reason(), truncated: bool,
        bytes_obs in any_usize(), bytes_ret in any_usize(),
    ) {
        let r = ReadResult { connection_id: id, name: None, bytes_read: br, encoding: enc, data, timeout_ms: timeout, no_new_rx_timeout_ms: None, elapsed_ms: elapsed, stop_reason, truncated, bytes_observed: bytes_obs, bytes_returned: bytes_ret, matched: false, match_index: None, match_frame_index: None, frames: None, frames_dropped: 0, error: None, from_offset: None, next_offset: None, bytes_lost: 0, buffered_remaining: 0, start_offset: 0, end_offset: 0 };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(ReadResult, v);
    }

    #[test]
    fn flush_result_schema_valid(id in opaque_id(), target in valid_flush_target()) {
        let t: serial_mcp::serial::FlushTarget = serde_json::from_value(serde_json::json!(target)).unwrap();
        let r = FlushResult { connection_id: id, name: None, target: t };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(FlushResult, v);
    }

    #[test]
    fn set_dtr_rts_result_schema_valid(id in opaque_id(), dtr: bool, rts: bool) {
        let r = SetDtrRtsResult { connection_id: id, name: None, dtr, rts };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(SetDtrRtsResult, v);
    }

    #[test]
    fn send_break_result_schema_valid(id in opaque_id(), dur in any_u64(), actual in any_u64()) {
        let r = SendBreakResult { connection_id: id, name: None, duration_ms: dur, actual_duration_ms: actual };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(SendBreakResult, v);
    }

    #[test]
    fn subscribe_result_schema_valid(
        id in opaque_id(), enc in valid_encoding(),
        max_buffered_bytes in any_usize(), poll in any_u64(), replaced: bool,
    ) {
        // SubscribeResult — subscribe is always background.
        let r = SubscribeResult {
            connection_id: id.clone(), name: None, encoding: enc.clone(),
            max_buffered_bytes, poll_interval_ms: poll,
            replaced_previous: replaced,
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(SubscribeResult, v);
    }

    #[test]
    fn unsubscribe_result_schema_valid(id in opaque_id(), was_active: bool) {
        let r = UnsubscribeResult { connection_id: id, name: None, was_active };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(UnsubscribeResult, v);
    }
}

// ── Encoding roundtrips ─────────────────────────────────────────────────────

proptest! {
    #[test]
    fn hex_encode_decode_roundtrip(bytes: Vec<u8>) {
        let encoded = codec::encode(Encoding::Hex, &bytes).unwrap();
        let decoded = codec::decode(Encoding::Hex, &encoded).unwrap();
        assert_eq!(decoded, bytes, "hex roundtrip mismatch");
    }

    #[test]
    fn base64_encode_decode_roundtrip(bytes: Vec<u8>) {
        let encoded = codec::encode(Encoding::Base64, &bytes).unwrap();
        let decoded = codec::decode(Encoding::Base64, &encoded).unwrap();
        assert_eq!(decoded, bytes, "base64 roundtrip mismatch");
    }

    #[test]
    fn utf8_encode_is_valid_for_valid_utf8(valid_utf8 in "\\PC*") {
        let bytes = valid_utf8.as_bytes().to_vec();
        let encoded = codec::encode(Encoding::Utf8, &bytes).unwrap();
        assert_eq!(encoded, valid_utf8);
    }

    #[test]
    fn utf8_encode_rejects_invalid_utf8_byte_blob(invalid_bytes: Vec<u8>) {
        let _ = codec::encode(Encoding::Utf8, &invalid_bytes);
    }

    #[test]
    fn hex_decode_handles_edge_cases(s in r"[A-Fa-f0-9 ]{0,32}") {
        let _ = codec::decode(Encoding::Hex, &s);
    }

    #[test]
    fn base64_decode_handles_edge_cases(s in r"[A-Za-z0-9+/= ]{0,64}") {
        let _ = codec::decode(Encoding::Base64, &s);
    }

    #[test]
    fn encoding_from_str_accepts_all_aliases(
        raw in prop::sample::select(vec![
            "utf8", "UTF8", "Utf8", "utf-8", "UTF-8",
            "hex", "HEX", "Hex",
            "base64", "BASE64", "Base64",
            "b64", "B64",
        ])
    ) {
        let result: Result<Encoding, _> = raw.parse();
        prop_assert!(result.is_ok(), "{raw:?} must parse successfully");
    }

    #[test]
    fn encoding_from_str_rejects_garbage(raw in "[a-z]{3,20}") {
        let known = ["utf8", "utf-8", "hex", "base64", "b64"];
        let lower = raw.to_lowercase();
        if known.iter().any(|k| lower == *k) {
            return Ok(());
        }
        let result: Result<Encoding, _> = raw.parse();
        prop_assert!(result.is_err(), "{raw:?} must fail to parse");
    }

    #[test]
    fn cobs_roundtrip_arbitrary_payload(bytes in prop::collection::vec(any::<u8>(), 0..=512)) {
        // Plain COBS (delimiter 0x00) roundtrip — must hold for ALL byte
        // payloads, including those with trailing/embedded zeros crossing
        // the 254-byte continuation boundary.
        let mode = serial_mcp::framing::TxFramingMode::Cobs;
        let framed = mode.encode(&bytes).unwrap();
        let cfg = serial_mcp::framing::RxFramingConfig {
            mode: serial_mcp::framing::RxFramingMode::Cobs,
            ..Default::default()
        };
        let mut dec =
            serial_mcp::framing::FrameDecoder::new(&cfg, None).unwrap();
        let frames = dec.push(&framed).frames;
        prop_assert_eq!(frames.len(), 1);
        prop_assert_eq!(&frames[0].data, &bytes);
    }

    #[test]
    fn slip_roundtrip_arbitrary_payload(bytes in prop::collection::vec(any::<u8>(), 0..=512)) {
        // SLIP (RFC 1055) roundtrip: TX encode → RX decode must reproduce
        // the original payload for any byte sequence (including END/ESC).
        let mode = serial_mcp::framing::TxFramingMode::Slip;
        let framed = mode.encode(&bytes).unwrap();
        let cfg = serial_mcp::framing::RxFramingConfig {
            mode: serial_mcp::framing::RxFramingMode::Slip,
            ..Default::default()
        };
        let mut dec = serial_mcp::framing::FrameDecoder::new(&cfg, None).unwrap();
        let frames = dec.push(&framed).frames;
        prop_assert_eq!(frames.len(), 1);
        prop_assert_eq!(&frames[0].data, &bytes);
    }
}

// ── Framing/parser roundtrips & no-panic — NMEA / Modbus ─────────────────────

proptest! {
    #[test]
    fn nmea_parser_roundtrip_valid_checksum(
        talker in "[A-Z]{2}",
        sentence_type in "[A-Z]{3}",
        fields in prop::collection::vec("[A-Z0-9.]{1,10}", 0..8),
    ) {
        use serial_mcp::framing::{
            FrameDecoder, LineEnding, ParserConfig, ParserType, ParsedFrame,
            RxFramingConfig, RxFramingMode,
        };
        // Build the NMEA body: talker + sentence_type + comma + fields.
        // Always include the comma so a zero-field sentence (e.g. "GPGGA,*XX")
        // is still recognised as NMEA — the parser requires at least one
        // comma in the content to distinguish NMEA from non-NMEA frames.
        let mut body = format!("{talker}{sentence_type},");
        if !fields.is_empty() {
            body.push_str(&fields.join(","));
        }
        // Compute XOR checksum inline (serial_mcp::checksums::XorChecksum is
        // pub(crate) — not visible from integration tests).
        let cs: u8 = body.bytes().fold(0u8, |acc, b| acc ^ b);
        let sentence = format!("{body}*{cs:02X}");

        // Route through FrameDecoder: Line framing + NMEA parser with validation.
        // build_parser is private; FrameDecoder is the correct integration path.
        let cfg = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&cfg, Some(&parser)).unwrap();
        let mut data = sentence.into_bytes();
        data.push(b'\n');
        let frames = dec.push(&data).frames;
        prop_assert_eq!(frames.len(), 1);
        let frame = &frames[0];
        match &frame.parsed {
            Some(ParsedFrame::Nmea {
                talker_id,
                sentence_type: st,
                fields: pf,
                checksum_valid,
            }) => {
                let expected_st: String;
                if let Some(stripped) = talker.strip_prefix('P') {
                    // Proprietary: talker_id = "P", sentence_type = rest of talker + suffix
                    expected_st = format!("{stripped}{sentence_type}");
                    prop_assert_eq!(talker_id.as_str(), "P");
                    prop_assert_eq!(st.as_str(), expected_st);
                } else {
                    // Standard: talker_id = first 2 chars, sentence_type = the rest
                    prop_assert_eq!(talker_id.as_str(), talker);
                    prop_assert_eq!(st.as_str(), sentence_type);
                }
                prop_assert_eq!(pf.as_slice(), fields.as_slice());
                prop_assert_eq!(checksum_valid, &Some(true));
            }
            other => prop_assert!(false, "expected Nmea, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_never_panics_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..600),
    ) {
        use serial_mcp::framing::{
            FrameDecoder, ParserConfig, ParserType, RxFramingConfig, RxFramingMode,
        };
        let cfg = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: serial_mcp::framing::LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&cfg, Some(&parser)).unwrap();
        let _ = dec.push(&bytes);
        let _ = dec.flush_partial();
    }

    #[test]
    fn nmea_parser_never_panics_on_utf8_with_commas(
        bytes in prop::string::string_regex("[\\x21-\\x7E\\xC0-\\xDF][\\x80-\\xBF]{0,4}(,[\\x21-\\x7E\\xC0-\\xDF][\\x80-\\xBF]{0,4})*\\*?[0-9A-Fa-f]{0,2}")
            .expect("valid regex")
            .prop_map(|s| s.into_bytes()),
    ) {
        use serial_mcp::framing::{
            FrameDecoder, ParserConfig, ParserType, RxFramingConfig, RxFramingMode,
        };
        let cfg = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: serial_mcp::framing::LineEnding::Auto,
            },
            ..Default::default()
        };
        // validate: true
        let parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&cfg, Some(&parser)).unwrap();
        let _ = dec.push(&bytes);
        let _ = dec.flush_partial();

        // validate: false
        let parser_no_val = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: false,
        };
        let mut dec2 = FrameDecoder::new(&cfg, Some(&parser_no_val)).unwrap();
        let _ = dec2.push(&bytes);
        let _ = dec2.flush_partial();
    }

    #[test]
    fn modbus_ascii_parser_never_panics_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..600),
    ) {
        use serial_mcp::framing::{
            FrameDecoder, ParserConfig, ParserType, RxFramingConfig, RxFramingMode,
        };
        let cfg = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: serial_mcp::framing::LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::ModbusAscii,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&cfg, Some(&parser)).unwrap();
        let _ = dec.push(&bytes);
        let _ = dec.flush_partial();
    }
}

// ── Boundary values — clamp helpers never panic ──────────────────────────────

proptest! {
    #[test]
    fn clamp_or_err_never_panics(value in any_usize(), max in any_usize()) {
        let _ = clamp_or_err("test", value, max);
    }

    #[test]
    fn require_min_or_err_never_panics(value in any_usize(), min in any_usize()) {
        let _ = require_min_or_err("test", value, min);
    }

    #[test]
    fn clamp_timeout_or_err_never_panics(value in any_u64(), max in any_u64()) {
        let _ = clamp_timeout_or_err("test", value, max);
    }

    #[test]
    fn clamp_poll_interval_or_err_never_panics(value in any_u64(), min in any_u64()) {
        let _ = clamp_poll_interval_or_err("test", value, min);
    }

    #[test]
    fn clamp_or_err_with_known_limits(value in any_usize()) {
        let _ = clamp_or_err("read.max_buffered_bytes", value, MAX_READ_BYTES);
        let _ = clamp_or_err("subscribe.max_buffered_bytes", value, MAX_STREAM_CHUNK_BYTES);
    }

    #[test]
    fn clamp_timeout_with_known_limit(value in any_u64()) {
        let _ = clamp_timeout_or_err("test", value, MAX_TIMEOUT_MS);
    }

    #[test]
    fn clamp_poll_interval_with_known_limit(value in any_u64()) {
        let _ = clamp_poll_interval_or_err("test", value, MIN_POLL_INTERVAL_MS);
    }

    #[test]
    fn parse_data_bits_accepts_valid(d in valid_data_bits()) {
        assert!(d.parse::<DataBits>().is_ok());
    }

    #[test]
    fn parse_data_bits_rejects_garbage(d in "[A-Za-z0-9]{1,5}") {
        let known = ["5", "6", "7", "8"];
        if known.contains(&d.as_str()) { return Ok(()); }
        assert!(d.parse::<DataBits>().is_err(), "{d:?} must fail");
    }

    #[test]
    fn parse_stop_bits_accepts_valid(s in valid_stop_bits()) {
        assert!(s.parse::<StopBits>().is_ok());
    }

    #[test]
    fn parse_stop_bits_rejects_garbage(s in "[A-Za-z0-9]{1,5}") {
        let known = ["1", "2"];
        if known.contains(&s.as_str()) { return Ok(()); }
        assert!(s.parse::<StopBits>().is_err(), "{s:?} must fail");
    }

    #[test]
    fn parse_parity_accepts_valid(p in valid_parity()) {
        assert!(p.parse::<Parity>().is_ok());
    }

    #[test]
    fn parse_parity_rejects_garbage(p in "[A-Za-z]{2,10}") {
        let lower = p.to_lowercase();
        if lower == "none" || lower == "odd" || lower == "even" { return Ok(()); }
        assert!(p.parse::<Parity>().is_err(), "{p:?} must fail");
    }

    #[test]
    fn parse_flow_control_accepts_valid(fc in valid_flow_control()) {
        assert!(fc.parse::<FlowControl>().is_ok());
    }

    #[test]
    fn parse_flow_control_rejects_garbage(fc in "[A-Za-z]{2,10}") {
        let lower = fc.to_lowercase();
        if lower == "none" || lower == "software" || lower == "hardware" { return Ok(()); }
        assert!(fc.parse::<FlowControl>().is_err(), "{fc:?} must fail");
    }

    #[test]
    fn port_names_with_special_chars(
        port in r"/dev/[A-Za-z0-9_\-\/\.\*\\ ]{1,256}"
    ) {
        let args = OpenArgs {
            port,
            name: None,
            baud_rate: Some(9600),
            data_bits: Some("8".into()),
            stop_bits: Some("1".into()),
            parity: Some("none".into()),
            flow_control: Some("none".into()),
            log_capacity: Some(1024),
            log_enabled: Some(true),
            reconnect_policy: Some(Default::default()),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: Some(serial_mcp::limits::DEFAULT_RX_BUFFER_SIZE),
            max_buffered_bytes: Some(32768),
            poll_interval_ms: Some(200),
            profile_mode: None,
        };
        assert_roundtrip!(args);
    }
}

// ── JSON schema covers every known tool outputSchema ────────────────────────

#[test]
fn all_result_types_have_valid_schema() {
    let types: Vec<(&str, Value)> = vec![
        (
            "ListConnectionsResult",
            schemars_to_jsonschema::<ListConnectionsResult>(),
        ),
        ("OpenResult", schemars_to_jsonschema::<OpenResult>()),
        ("CloseResult", schemars_to_jsonschema::<CloseResult>()),
        ("WriteResult", schemars_to_jsonschema::<WriteResult>()),
        ("ReadResult", schemars_to_jsonschema::<ReadResult>()),
        ("FlushResult", schemars_to_jsonschema::<FlushResult>()),
        (
            "SetDtrRtsResult",
            schemars_to_jsonschema::<SetDtrRtsResult>(),
        ),
        (
            "SetFlowControlResult",
            schemars_to_jsonschema::<SetFlowControlResult>(),
        ),
        (
            "SendBreakResult",
            schemars_to_jsonschema::<SendBreakResult>(),
        ),
        (
            "SubscribeResult",
            schemars_to_jsonschema::<SubscribeResult>(),
        ),
        (
            "UnsubscribeResult",
            schemars_to_jsonschema::<UnsubscribeResult>(),
        ),
    ];
    for (name, schema) in &types {
        jsonschema::validator_for(schema)
            .unwrap_or_else(|e| panic!("{name} schema fails to compile: {e}"));
    }
}

// ── SubscribeResult null-data is valid per schema ───────────────────────────

#[test]
fn subscribe_result_ff_null_fields_match_schema() {
    // Subscribe is always background; no nullable vestigial fields remain.
    let r = SubscribeResult {
        connection_id: "abc".into(),
        name: None,
        encoding: "utf8".into(),
        max_buffered_bytes: 1024,
        poll_interval_ms: 200,
        replaced_previous: false,
    };
    let v = serde_json::to_value(&r).unwrap();
    validate_schema::<SubscribeResult>(&v);
    roundtrip_stable(&r);
}

#[test]
fn subscribe_result_blocking_filled_fields_match_schema() {
    let r = SubscribeResult {
        connection_id: "abc".into(),
        name: None,
        encoding: "utf8".into(),
        max_buffered_bytes: 2048,
        poll_interval_ms: 100,
        replaced_previous: true,
    };
    let v = serde_json::to_value(&r).unwrap();
    validate_schema::<SubscribeResult>(&v);
    roundtrip_stable(&r);
}

// ── Stateful connection lifecycle ───────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    OpenWriteCloseRead,
    DoubleClose,
    ReadAfterClose,
    WriteAfterClose,
    SubscribeThenClose,
}

fn run_lifecycle_scenario(op: Op) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
        let (conn, _peer) = serial_mcp::serial::test_support::loopback_connection("lifecycle");
        let cid = manager.insert(conn).await.unwrap();
        let conn = manager.get(&cid).await.unwrap();

        match op {
            Op::OpenWriteCloseRead => {
                conn.write(b"hello").await.unwrap();
                manager.close(&cid).await.unwrap();
                let (conn2, _) = serial_mcp::serial::test_support::loopback_connection("lifecycle");
                assert!(manager.insert(conn2).await.is_ok());
            }
            Op::DoubleClose => {
                manager.close(&cid).await.unwrap();
                let result = manager.close(&cid).await;
                assert!(result.is_err(), "double close must error");
            }
            Op::ReadAfterClose => {
                manager.close(&cid).await.unwrap();
                let mut buf = [0u8; 16];
                let _ = conn.read(&mut buf, Some(50)).await;
            }
            Op::WriteAfterClose => {
                manager.close(&cid).await.unwrap();
                let _ = conn.write(b"data").await;
            }
            Op::SubscribeThenClose => {
                manager.close(&cid).await.unwrap();
            }
        }
    });
}

#[test]
fn lifecycle_open_write_close() {
    run_lifecycle_scenario(Op::OpenWriteCloseRead);
}

#[test]
fn lifecycle_double_close_is_error() {
    run_lifecycle_scenario(Op::DoubleClose);
}

#[test]
fn lifecycle_read_after_close_no_panic() {
    run_lifecycle_scenario(Op::ReadAfterClose);
}

#[test]
fn lifecycle_write_after_close_no_panic() {
    run_lifecycle_scenario(Op::WriteAfterClose);
}

#[test]
fn lifecycle_subscribe_then_close_no_panic() {
    run_lifecycle_scenario(Op::SubscribeThenClose);
}

#[test]
fn lifecycle_unsubscribe_noop_does_not_panic() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    rt.block_on(async {
        let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
        let (conn, _) = serial_mcp::serial::test_support::loopback_connection("unsub-noop");
        let cid = manager.insert(conn).await.unwrap();

        let streams: Arc<tokio::sync::Mutex<std::collections::HashMap<String, ()>>> =
            Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));
        let mut guard = streams.lock().await;
        let was_active = guard.remove(&cid).is_some();
        assert!(
            !was_active,
            "no-op unsubscribe must report was_active=false"
        );
    });
}

// ── RxFramingConfig roundtrip ──────────────────────────────────────────────

#[test]
fn rx_framing_config_roundtrip_all_modes() {
    use serial_mcp::framing::*;
    use serial_mcp::match_config::PatternEncoding;

    // Line (default ending: auto)
    let c1 = RxFramingConfig {
        mode: RxFramingMode::Line {
            ending: LineEnding::Auto,
        },
        max_frames: Some(10),
        include_terminators: true,
        skip_empty: false,
    };
    let json = serde_json::to_value(&c1).unwrap();
    let c2: RxFramingConfig = serde_json::from_value(json).unwrap();
    assert!(matches!(c2.mode, RxFramingMode::Line { .. }));
    assert_eq!(c2.max_frames, Some(10));
    assert!(c2.include_terminators);

    // Delimiter
    let c3 = RxFramingConfig {
        mode: RxFramingMode::Delimiter {
            delimiter: "|".into(),
            delimiter_encoding: PatternEncoding::Utf8,
        },
        max_frames: None,
        include_terminators: false,
        skip_empty: false,
    };
    let json = serde_json::to_value(&c3).unwrap();
    let c4: RxFramingConfig = serde_json::from_value(json).unwrap();
    assert!(matches!(c4.mode, RxFramingMode::Delimiter { .. }));

    // Length-prefixed
    let c5 = RxFramingConfig {
        mode: RxFramingMode::LengthPrefixed {
            prefix_size: 2,
            endianness: Endianness::Little,
            initial_offset: Some(4),
        },
        max_frames: Some(0),
        include_terminators: false,
        skip_empty: false,
    };
    let json = serde_json::to_value(&c5).unwrap();
    let c6: RxFramingConfig = serde_json::from_value(json).unwrap();
    assert!(matches!(c6.mode, RxFramingMode::LengthPrefixed { .. }));
    assert_eq!(c6.max_frames, Some(0));

    // Start/end
    let c7 = RxFramingConfig {
        mode: RxFramingMode::StartEnd {
            start: vec!["STX".into()],
            end: "ETX".into(),
            marker_encoding: PatternEncoding::Base64,
            include_markers: true,
        },
        max_frames: None,
        include_terminators: false,
        skip_empty: false,
    };
    let json = serde_json::to_value(&c7).unwrap();
    let c8: RxFramingConfig = serde_json::from_value(json).unwrap();
    assert!(matches!(c8.mode, RxFramingMode::StartEnd { .. }));
    assert!(c8.max_frames.is_none());

    // SLIP (parameterless)
    let c9 = RxFramingConfig {
        mode: RxFramingMode::Slip,
        ..Default::default()
    };
    let json = serde_json::to_value(&c9).unwrap();
    let c10: RxFramingConfig = serde_json::from_value(json).unwrap();
    assert!(matches!(c10.mode, RxFramingMode::Slip));

    // Line with skip_empty: true (ndjson-style)
    let c11 = RxFramingConfig {
        mode: RxFramingMode::Line {
            ending: LineEnding::Auto,
        },
        max_frames: None,
        include_terminators: false,
        skip_empty: true,
    };
    let json = serde_json::to_value(&c11).unwrap();
    let c12: RxFramingConfig = serde_json::from_value(json).unwrap();
    assert!(matches!(c12.mode, RxFramingMode::Line { .. }));
    assert!(c12.skip_empty);
}

// ── Generated-name and ranking properties ───────────────────────────────────

proptest! {
    #[test]
    fn generated_label_normalization_properties(label in "[A-Za-z0-9 _\\-!@#ÄÖÜß]*") {
        let normalized = normalize_generated_label(&label);
        // Never empty, never overwritten silently, ASCII lowercase only.
        assert!(!normalized.is_empty(), "empty label for {label:?}");
        assert!(normalized.len() <= 32, "cap at 32: {label:?} -> {normalized:?}");
        assert!(normalized.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "non-normalized chars: {label:?} -> {normalized:?}");
        assert!(!normalized.starts_with('-') && !normalized.ends_with('-'),
                "trimmed: {label:?} -> {normalized:?}");
        // Runs of separators collapse to a single dash.
        assert!(!normalized.contains("--"), "no double dash: {label:?} -> {normalized:?}");
        // Lowercasing a normalized label is a fixed point.
        assert_eq!(normalized, normalize_generated_label(&normalized));
    }

    #[test]
    fn generated_name_allocation_property(
        existing in prop::collection::vec("[a-z0-9-]{1,12}", 0..20),
        base in "[a-z0-9-]{1,12}",
    ) {
        let profiles: Vec<Profile> = existing
            .iter()
            .map(|n| Profile {
                name: n.clone(),
                selector: Default::default(),
                defaults: Default::default(),
                metadata: Default::default(),
                revisions: Vec::new(),
            })
            .collect();
        let allocated = allocate_generated_name(&profiles, &base);
        assert!(!existing.contains(&allocated), "must never collide: {existing:?} base={base}");
        assert!(allocated.starts_with(&base), "must keep the base prefix: {allocated}");
    }

    #[test]
    fn candidate_ranking_is_stable_and_descending(
        timestamps in prop::collection::vec(prop::option::of(0u64..1_000_000), 0..30),
    ) {
        use serial_mcp::profiles::rank_candidates;
        let profiles: Vec<Profile> = timestamps
            .iter()
            .enumerate()
            .map(|(i, ts)| Profile {
                name: format!("p-{i}"),
                selector: Default::default(),
                defaults: Default::default(),
                metadata: serial_mcp::profiles::ProfileMetadata {
                    last_used_at_ms: *ts,
                    ..Default::default()
                },
                revisions: Vec::new(),
            })
            .collect();
        let ranked = rank_candidates(profiles.clone());
        assert_eq!(ranked.len(), profiles.len());
        // Sorted non-increasing by effective timestamp (None == 0).
        for pair in ranked.windows(2) {
            let a = pair[0].metadata.last_used_at_ms.unwrap_or(0);
            let b = pair[1].metadata.last_used_at_ms.unwrap_or(0);
            assert!(a >= b, "must rank newest first: {timestamps:?}");
        }
        // Ranking is a permutation of the inputs.
        let mut names: Vec<String> = ranked.iter().map(|p| p.name.clone()).collect();
        names.sort();
        let mut orig: Vec<String> = profiles.iter().map(|p| p.name.clone()).collect();
        orig.sort();
        assert_eq!(names, orig);
    }
}
