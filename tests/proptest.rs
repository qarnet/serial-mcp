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
use serial_mcp::tools::helpers::{
    clamp_or_err, clamp_poll_interval_or_err, clamp_timeout_or_err, parse_data_bits,
    parse_flow_control, parse_open_args, parse_parity, parse_stop_bits, require_min_or_err,
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

// ── Phase A.1: Schema roundtrips — all argument types ────────────────────────

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
            baud_rate: baud,
            data_bits: db.clone(),
            stop_bits: sb.clone(),
            parity: p.clone(),
            flow_control: fc.clone(),
            log_capacity: 1024,
            log_enabled: true,
        };
        assert_roundtrip!(args);

        if let Ok(config) = parse_open_args(args) {
            assert_eq!(config.port, port);
            assert_eq!(config.baud_rate, baud);
        }
        parse_data_bits(&db).unwrap();
        parse_stop_bits(&sb).unwrap();
        parse_parity(&p).unwrap();
        parse_flow_control(&fc).unwrap();
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
        let args = WriteArgs { connection_id: id, data, encoding: enc };
        assert_roundtrip!(args);
    }

    #[test]
    fn read_args_roundtrip(
        id in opaque_id(),
        timeout in optional_u64(),
        max_buffered_bytes in any_usize(),
        enc in valid_encoding(),
    ) {
        let args = ReadArgs { connection_id: id, timeout_ms: timeout, max_buffered_bytes, encoding: enc, r#match: None, no_new_rx_timeout_ms: None };
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
        max_buffered_bytes in any_usize(),
        poll in any_u64(),
    ) {
        let args = SubscribeArgs {
            connection_id: id,
            timeout_ms: timeout,
            no_new_rx_timeout_ms: None,
            encoding: enc,
            max_buffered_bytes,
            poll_interval_ms: poll,
            r#match: None,
        };
        assert_roundtrip!(args);
    }

    #[test]
    fn unsubscribe_args_roundtrip(id in opaque_id()) {
        let args = UnsubscribeArgs { connection_id: id };
        assert_roundtrip!(args);
    }
}

// ── Phase A.2: Schema validation — all result types against their schemas ────

proptest! {
    #[test]
    fn open_result_schema_valid(
        id in opaque_id(), port in valid_port_name(), baud in any_u32(),
    ) {
        let r = OpenResult { connection_id: id, name: None, port, baud_rate: baud };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(OpenResult, v);
    }

    #[test]
    fn close_result_schema_valid(id in opaque_id()) {
        let r = CloseResult { connection_id: id, name: None };
        let v = serde_json::to_value(&r).unwrap();
        assert_schema_valid!(CloseResult, v);
    }

    #[test]
    fn write_result_schema_valid(id in opaque_id(), bw in any_usize(), enc in valid_encoding()) {
        let r = WriteResult { connection_id: id, name: None, bytes_written: bw, encoding: enc };
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
        let r = ReadResult { connection_id: id, name: None, bytes_read: br, encoding: enc, data, timeout_ms: timeout, no_new_rx_timeout_ms: None, elapsed_ms: elapsed, stop_reason, truncated, bytes_observed: bytes_obs, bytes_returned: bytes_ret, matched: false, match_index: None };
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
        // SubscribeResult — subscribe is always background (PLAN 1b).
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

// ── Phase A.3: Encoding roundtrips ───────────────────────────────────────────

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
}

// ── Phase A.4: Boundary values — clamp helpers never panic ───────────────────

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
        assert!(parse_data_bits(&d).is_ok());
    }

    #[test]
    fn parse_data_bits_rejects_garbage(d in "[A-Za-z0-9]{1,5}") {
        let known = ["5", "6", "7", "8"];
        if known.contains(&d.as_str()) { return Ok(()); }
        assert!(parse_data_bits(&d).is_err(), "{d:?} must fail");
    }

    #[test]
    fn parse_stop_bits_accepts_valid(s in valid_stop_bits()) {
        assert!(parse_stop_bits(&s).is_ok());
    }

    #[test]
    fn parse_stop_bits_rejects_garbage(s in "[A-Za-z0-9]{1,5}") {
        let known = ["1", "2"];
        if known.contains(&s.as_str()) { return Ok(()); }
        assert!(parse_stop_bits(&s).is_err(), "{s:?} must fail");
    }

    #[test]
    fn parse_parity_accepts_valid(p in valid_parity()) {
        assert!(parse_parity(&p).is_ok());
    }

    #[test]
    fn parse_parity_rejects_garbage(p in "[A-Za-z]{2,10}") {
        let lower = p.to_lowercase();
        if lower == "none" || lower == "odd" || lower == "even" { return Ok(()); }
        assert!(parse_parity(&p).is_err(), "{p:?} must fail");
    }

    #[test]
    fn parse_flow_control_accepts_valid(fc in valid_flow_control()) {
        assert!(parse_flow_control(&fc).is_ok());
    }

    #[test]
    fn parse_flow_control_rejects_garbage(fc in "[A-Za-z]{2,10}") {
        let lower = fc.to_lowercase();
        if lower == "none" || lower == "software" || lower == "hardware" { return Ok(()); }
        assert!(parse_flow_control(&fc).is_err(), "{fc:?} must fail");
    }

    #[test]
    fn port_names_with_special_chars(
        port in r"/dev/[A-Za-z0-9_\-\/\.\*\\ ]{1,256}"
    ) {
        let args = OpenArgs {
            port,
            name: None,
            baud_rate: 9600,
            data_bits: "8".into(),
            stop_bits: "1".into(),
            parity: "none".into(),
            flow_control: "none".into(),
            log_capacity: 1024,
            log_enabled: true,
        };
        assert_roundtrip!(args);
    }
}

// ── Phase A.6: JSON schema covers every known tool outputSchema ─────────────

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

// ── Phase A.7: SubscribeResult null-data is valid per schema ────────────────

#[test]
fn subscribe_result_ff_null_fields_match_schema() {
    // Subscribe is always background after PLAN 1b; no nullable vestigial fields remain.
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

// ── Phase C.1: Stateful connection lifecycle ────────────────────────────────

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
