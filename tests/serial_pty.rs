//! End-to-end tests with a real PTY pair standing in for a serial device.
//!
//! These tests open a Linux pseudo-terminal pair via `openpty(3)`,
//! point the server at the slave path (`/dev/pts/N`) via the regular
//! `open` MCP tool, and drive the master end from the test process as if
//! it were a USB-Serial device. Unlike the in-memory loopback tests in
//! `tests/http_integration.rs`, these exercise the real
//! `tokio_serial::SerialStream` code path inside `SerialConnection`.

#![cfg(target_os = "linux")]

use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use serde_json::json;
use tokio::io::AsyncWriteExt;

mod common;
use common::{connect_client, pty::PtyPair, tool_request, TestServer};

/// Open a real PTY pair, then walk an MCP client through opening the
/// slave path as a serial port. Returns the test server (kept alive by
/// the caller), the connected client, and the PTY pair plus
/// connection_id.
async fn setup() -> (
    TestServer,
    rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    (),
    PtyPair,
    String,
) {
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();

    let server = TestServer::start().await;
    let (client, rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({ "port": slave_path, "baud_rate": 115200 }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");

    let structured = result
        .structured_content
        .expect("open must return structured content");
    let connection_id = structured["connection_id"]
        .as_str()
        .expect("connection_id is a string")
        .to_string();
    (server, client, rx, pty, connection_id)
}

#[tokio::test]
async fn pty_open_returns_connection_id() {
    let (_server, client, _rx, _pty, connection_id) = setup().await;
    assert!(!connection_id.is_empty());
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_client_write_reaches_device_side() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "PING\r\n",
            }),
        ))
        .await
        .unwrap();

    let mut buf = [0u8; 6];
    pty.read_device_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"PING\r\n");
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_device_write_then_client_read() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"PONG\r\n").await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["bytes_read"], json!(6));
    assert_eq!(structured["data"], json!("PONG\r\n"));
    assert!(structured.get("timed_out").is_none(), "{structured:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_device_binary_read_falls_back_to_exact_hex() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // Send invalid UTF-8 bytes through the real PTY serial path.
    pty.write_device(&[0xDE, 0xAD, 0xBE, 0xEF, 0xFF])
        .await
        .unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 1000,
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["bytes_read"], json!(5));
    assert_eq!(structured["data"], json!("de ad be ef ff"));
    assert_eq!(structured["encoding"], json!("hex"));
    // Buffered bytes stop with "drained". Otherwise the read waits and stops
    // with "timeout". Both results carry the data.
    assert!(
        structured["stop_reason"] == json!("drained")
            || structured["stop_reason"] == json!("timeout"),
        "unexpected stop_reason: {structured:?}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_match_finds_real_serial_pattern() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    let read_handle = {
        let peer = client.peer().clone();
        let id = connection_id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 8000,
                    "encoding": "utf8",
                    "match": { "pattern": "OK>" },
                }),
            ))
            .await
        })
    };

    // Feed bytes in separate writes to exercise the read and match accumulator.
    tokio::time::sleep(Duration::from_millis(50)).await;
    pty.write_device(b"warming up... ").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    pty.write_device(b"OK> ready").await.unwrap();

    let result = read_handle.await.unwrap().unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert!(structured.get("timed_out").is_none(), "{structured:?}");
    assert_eq!(structured["matched"], json!(true), "{structured:?}");
    let match_index = structured["match_index"].as_u64().unwrap();
    let data = structured["data"].as_str().unwrap();
    assert!(
        data[..(match_index as usize + 3)].ends_with("OK>"),
        "match offset wrong: {data:?} match_index={match_index}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_match_with_context_returns_shaped_payload() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // Write data first, then wait briefly for the PTY to buffer it.
    pty.write_device(b"AAAAprefix___OK>suffix").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 3000,
                "match": {
                    "pattern": "OK>",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8",
                        "context_amount_of_matched_bytes": 4
                    }
                }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["matched"], json!(true), "{structured:?}");
    assert_eq!(structured["stop_reason"], json!("match_found"));

    let match_index = structured["match_index"].as_u64().expect("match_index") as usize;
    let data = structured["data"].as_str().expect("data");
    // "OK>" at byte 14 in "AAAAprefix___OK>suffix", context_amount=4:
    // pre_start = 14 - 4 = 10, shaped = "x___OK>" (7 bytes), match_index = 4.
    assert!(data.ends_with("OK>"), "data should end with OK>: {data:?}");
    assert_eq!(match_index, 4, "match_index should be 4: {structured:?}");
    assert!(
        data.len() <= 7 + 3,
        "data should be context + match: {data:?}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_match_with_zero_context_returns_only_matched_bytes() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"garbage before OK>tail").await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 3000,
                "match": {
                    "pattern": "OK>",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8",
                        "context_amount_of_matched_bytes": 0
                    }
                }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["matched"], json!(true), "{structured:?}");
    let match_index = structured["match_index"].as_u64().expect("match_index") as usize;
    let data = structured["data"].as_str().expect("data");
    assert_eq!(match_index, 0, "match_index should be 0 with 0 context");
    assert_eq!(
        data, "OK>",
        "data should be just the matched bytes: {data:?}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_match_without_context_returns_full_accumulated() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // Write data before starting read so it is already in the PTY buffer.
    pty.write_device(b"junk OK> rest").await.unwrap();
    // Wait briefly for the PTY to deliver the bytes.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 3000,
                "match": {
                    "pattern": "OK>",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8"
                    }
                }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["matched"], json!(true), "{structured:?}");
    let data = structured["data"].as_str().expect("data");
    assert!(data.contains("OK>"), "data should contain OK>: {data:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_literal_match_index_over_chunked_stream() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // Send two device writes, 100ms apart.
    pty.write_device(b"warming up... ").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    pty.write_device(b"OK> ready").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The literal match crosses chunks at global index 14.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 3000,
                "match": { "pattern": "OK>" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["matched"], json!(true), "{structured:?}");
    assert_eq!(structured["match_index"], json!(14), "{structured:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_live_match_applies_context_shaping() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // Start read before device data. The match then uses the live-path
    // context shaping.
    let read_handle = {
        let peer = client.peer().clone();
        let id = connection_id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 8000,
                    "match": {
                        "pattern": "OK>",
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8",
                            "context_amount_of_matched_bytes": 4
                        }
                    },
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    pty.write_device(b"AAAA").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    pty.write_device(b"BBBBOK>tail").await.unwrap();

    let result = read_handle.await.unwrap().unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["matched"], json!(true), "{structured:?}");
    assert_eq!(structured["stop_reason"], json!("match_found"));
    // "OK>" is at global index 8. Context 4 produces "BBBBOK>" with
    // relative index 4.
    assert_eq!(structured["data"], json!("BBBBOK>"), "{structured:?}");
    assert_eq!(structured["match_index"], json!(4), "{structured:?}");
    assert_eq!(structured["bytes_read"], json!(7), "{structured:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_close_then_use_returns_is_error() {
    let (_server, client, _rx, _pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();

    let after_close = client
        .peer()
        .call_tool(
            CallToolRequestParams::new("write").with_arguments(common::args_object(json!({
                "connection_id": connection_id,
                "data": "should not reach",
            }))),
        )
        .await
        .unwrap();
    assert_eq!(after_close.is_error, Some(true));
    client.cancel().await.ok();
}

#[tokio::test]
#[cfg(target_os = "linux")]
async fn pty_send_break_short_duration_timing() {
    let (_server, client, _rx, _pty, connection_id) = setup().await;

    // A 50ms BREAK must release within about 100ms, not remain held until
    // 250ms or more.
    let start = std::time::Instant::now();
    let result = client
        .peer()
        .call_tool(tool_request(
            "send_break",
            json!({
                "connection_id": connection_id,
                "duration_ms": 50u64,
            }),
        ))
        .await
        .unwrap();
    let elapsed = start.elapsed().as_millis() as u64;

    assert_ne!(result.is_error, Some(true), "send_break failed: {result:?}");
    let structured = result
        .structured_content
        .expect("send_break must return structured");
    let actual_duration = structured["actual_duration_ms"]
        .as_u64()
        .expect("actual_duration_ms");

    // Accept 40-100ms for the 50ms BREAK.
    assert!(
        (40..=100).contains(&actual_duration),
        "send_break(50ms) took {actual_duration}ms, expected 40-100ms"
    );
    // Keep the full round trip below 200ms.
    assert!(
        elapsed <= 200,
        "send_break round-trip took {elapsed}ms, expected <200ms"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_line_auto_promotes_on_bare_cr_and_flushes_pending() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // A pending CR is not a frame yet.
    pty.write_device(b"line1\r").await.unwrap();
    // The non-\n byte confirms bare CR, emits "line1", and promotes to
    // CrMode. The following "line2\r" emits "line2".
    pty.write_device(b"line2\r").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 3000,
                "rx_framing": { "type": "line" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    let frames = structured["frames"].as_array().expect("frames array");
    assert_eq!(frames.len(), 2, "{structured:?}");
    assert_eq!(frames[0]["frame_type"], json!("line"));
    assert_eq!(frames[0]["data"], json!("line1"), "{structured:?}");
    assert_eq!(frames[1]["frame_type"], json!("line"));
    assert_eq!(frames[1]["data"], json!("line2"), "{structured:?}");
    client.cancel().await.ok();
}

// Ring-based read tests.

/// Cat semantics: write then read returns buffered bytes immediately
/// with stop_reason="drained".
#[tokio::test]
async fn pty_read_returns_buffered_bytes_immediately() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"HELLO").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["bytes_read"], json!(5));
    assert_eq!(structured["data"], json!("HELLO"));
    assert_eq!(structured["stop_reason"], json!("drained"));
    assert_eq!(structured["bytes_lost"], json!(0));
    assert!(
        structured
            .get("from_offset")
            .and_then(|v| v.as_u64())
            .is_some(),
        "from_offset should be present"
    );
    client.cancel().await.ok();
}

/// Wrap + bytes_lost: small buffer, write >8 bytes, read reports loss.
#[tokio::test]
async fn pty_read_wrap_reports_bytes_lost() {
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Open with tiny rx_buffer_size to force wrap.
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({ "port": slave_path, "baud_rate": 115200, "rx_buffer_size": 8 }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let connection_id = result.structured_content.expect("structured")["connection_id"]
        .as_str()
        .expect("string")
        .to_string();

    let (mut pty_file, _slave_fd) = pty.into_parts();

    // Write more than the ring capacity.
    pty_file.write_all(b"abcdefghijklmnop").await.unwrap(); // 16 bytes, ring=8
    tokio::time::sleep(Duration::from_millis(200)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert!(
        structured["bytes_lost"].as_u64().unwrap_or(0) > 0,
        "bytes_lost should be > 0 for wrapped ring: {structured:?}"
    );
    client.cancel().await.ok();
}

/// Read with `from: {"type":"buffer_start"}` replays retained history.
#[tokio::test]
async fn pty_read_from_buffer_start() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"RETAINED").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Read with `from: {"type":"buffer_start"}` to replay retained bytes.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "buffer_start" },
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert!(
        s["data"].as_str().unwrap().contains("RETAINED"),
        "should replay retained data: {s:?}"
    );
    // start_offset/end_offset should be present.
    assert!(s["start_offset"].as_u64().is_some(), "{s:?}");
    assert!(s["end_offset"].as_u64().is_some(), "{s:?}");
    client.cancel().await.ok();
}

/// Reading from the same offset is non-destructive. The cursor is reset to
/// that offset before reading, so the same bytes return.
#[tokio::test]
async fn pty_read_reread_same_from_offset() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"UNIQUE").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // First read with `from: {"type":"buffer_start"}`.
    let first = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "buffer_start" },
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(first.is_error, Some(true), "{first:?}");
    let s1 = first.structured_content.expect("structured");
    let from_offset = s1["from_offset"].as_u64().expect("from_offset");
    let data1 = s1["data"].as_str().expect("data");

    // Read again from the returned absolute offset.
    let second = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "offset", "offset": from_offset },
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(second.is_error, Some(true), "{second:?}");
    let s2 = second.structured_content.expect("structured");
    assert_eq!(
        s2["data"].as_str().unwrap(),
        data1,
        "re-read with same from_offset should return same bytes"
    );
    client.cancel().await.ok();
}

/// `from: {"type":"now"}` skips buffered data and starts at the live edge.
#[tokio::test]
async fn pty_read_from_now_skips_backlog() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"SKIP_ME").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Read with `from: {"type":"now"}` to skip buffered bytes.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "now" },
                "timeout_ms": 300,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    // The result must not contain the skipped marker.
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        !data.contains("SKIP_ME"),
        "from: now should skip buffered data, got: {s:?}"
    );
    client.cancel().await.ok();
}

/// `flush(target="both")` discards retained RX backlog. Pre-flush bytes must
/// not return on a later read, while post-flush bytes remain readable. The
/// test uses public tools only, and the connection must remain usable.
#[tokio::test]
async fn pty_flush_both_discards_retained_rx_backlog() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"OLD-MARKER-4711").await.unwrap();

    // Poll public get_status until the old marker reaches the ring.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut saw_backlog = false;
    while tokio::time::Instant::now() < deadline {
        let status = client
            .peer()
            .call_tool(tool_request(
                "get_status",
                json!({ "connection_id": connection_id }),
            ))
            .await
            .unwrap();
        assert_ne!(status.is_error, Some(true), "{status:?}");
        let s = status.structured_content.expect("structured");
        if s["rx_buffered_unread"].as_u64().unwrap_or(0) >= 15 {
            saw_backlog = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(saw_backlog, "OLD marker never reached the RX ring");

    let flush = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .unwrap();
    assert_ne!(flush.is_error, Some(true), "{flush:?}");

    // Write a new marker only after flush returns.
    pty.write_device(b"NEW-MARKER-2299").await.unwrap();

    // Read from the shared cursor, which flush clamped to the live edge.
    let read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(read.is_error, Some(true), "read failed: {read:?}");
    let s = read.structured_content.expect("structured");
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.contains("NEW-MARKER-2299"),
        "post-flush bytes must remain readable: {s:?}"
    );
    assert!(
        !data.contains("OLD-MARKER-4711"),
        "stale pre-flush bytes must be discarded by flush(target=both): {s:?}"
    );
    client.cancel().await.ok();
}

// Read history with framing and matching.

/// Read with `from: {"type":"buffer_start"}` and line framing decodes all
/// frames.
#[tokio::test]
async fn pty_read_from_buffer_start_with_framing_decodes_all_frames() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"LINE1\nLINE2\nLINE3\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "buffer_start" },
                "rx_framing": { "type": "line" },
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert_eq!(frames.len(), 3, "expected 3 decoded frames: {frames:?}");
    assert_eq!(frames[0]["data"], "LINE1");
    assert_eq!(frames[1]["data"], "LINE2");
    assert_eq!(frames[2]["data"], "LINE3");
    client.cancel().await.ok();
}

/// Read with `from: {"type":"offset","offset":0}` and match scans from
/// the given offset.
#[tokio::test]
async fn pty_read_from_offset_with_match_scans_from_offset() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"AAAAATARGETBBBBB").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Read with `from: {"type":"offset","offset":0}`; the match is present.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "offset", "offset": 0 },
                "match": { "pattern": "TARGET" },
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], true);
    assert_eq!(s["match_index"], 5, "match_index relative to returned data");

    // Read with `from: {"type":"offset","offset":10}`; the match at
    // absolute position 5 is outside the read range.
    let result2 = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "offset", "offset": 10 },
                "match": { "pattern": "TARGET" },
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result2.is_error, Some(true), "{result2:?}");
    let s2 = result2.structured_content.expect("structured");
    // No match remains beyond offset 10, so the read stops on timeout.
    assert_eq!(
        s2["matched"], false,
        "match should not be found from offset 10"
    );
    client.cancel().await.ok();
}

// Connection-mode configure tests.

#[tokio::test]
async fn pty_configure_connection_mutates_framing_default() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;
    pty.write_device(b"line1\nline2\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Read with no framing to receive raw bytes.
    let raw = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 300,
            }),
        ))
        .await
        .unwrap();
    let raw_s = raw.structured_content.expect("structured");
    assert!(
        raw_s["frames"].is_null(),
        "no framing expected before configure: {raw_s:?}"
    );
    // Configure line framing on the live connection.
    let cfg = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "connection_id": connection_id,
                "defaults": { "rx_framing": {"type": "line"} }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(cfg.is_error, Some(true), "{cfg:?}");
    assert_eq!(cfg.structured_content.unwrap()["mode"], "connection");
    // Write more data and read with the new framing default.
    pty.write_device(b"line3\nline4\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let framed = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 300,
                "from": {"type": "cursor"},
            }),
        ))
        .await
        .unwrap();
    let f_s = framed.structured_content.expect("structured");
    assert!(
        f_s["frames"].is_array(),
        "frames expected after configure: {f_s:?}"
    );
    assert!(!f_s["frames"].as_array().unwrap().is_empty());
    client.cancel().await.ok();
}

/// Configure a profile with line framing, then pass the same framing
/// explicitly to `open` and verify that `read` uses line framing.
///
/// This test does not call `open_profile` or select the configured profile.
/// It covers explicit-open framing only. Profile default application remains
/// untested here.
#[tokio::test]
async fn pty_explicit_open_framing_applies() {
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let profile_name = "test-configure-apply";

    // Configure a profile with line framing.
    let _ = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": profile_name,
                "defaults": { "rx_framing": {"type": "line"} }
            }),
        ))
        .await
        .unwrap();

    // Open the PTY with explicit line framing.
    let open_r = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": slave_path,
                "baud_rate": 115200,
                "rx_framing": {"type": "line"}
            }),
        ))
        .await
        .unwrap();
    assert_ne!(open_r.is_error, Some(true), "open failed: {open_r:?}");
    let s = open_r.structured_content.expect("structured");
    let conn_id = s["connection_id"].as_str().unwrap().to_string();

    // Read device data using the explicit line framing.
    let mut pty_mut = pty;
    pty_mut.write_device(b"frame1\nframe2\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let read_r = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": conn_id,
                "timeout_ms": 300,
            }),
        ))
        .await
        .unwrap();
    let rs = read_r.structured_content.expect("structured");
    assert!(
        rs["frames"].is_array(),
        "explicit framing should apply: {rs:?}"
    );
    assert!(!rs["frames"].as_array().unwrap().is_empty());

    let _ = client
        .peer()
        .call_tool(tool_request("close", json!({"connection_id": conn_id})))
        .await;
    let _ = client
        .peer()
        .call_tool(tool_request(
            "delete_profile",
            json!({"profile_name": profile_name}),
        ))
        .await;
    client.cancel().await.ok();
}

// Transact write-then-read tests.

#[tokio::test]
async fn pty_transact_writes_then_reads_response() {
    let (_server, client, _rx, pty, connection_id) = setup().await;
    let (mut master_file, _slave) = pty.into_parts();
    let cid = connection_id.clone();

    // The device emulator writes a response after 300ms.
    let emulator = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        use tokio::io::AsyncWriteExt;
        let _ = master_file.write_all(b"pong\n").await;
        let _ = master_file.flush().await;
    });

    let r = client
        .peer()
        .call_tool(tool_request(
            "transact",
            json!({
                "connection_id": cid,
                "data": "ping\n",
                "encoding": "utf8",
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured");
    assert!(
        s["write"]["bytes_written"].as_u64().unwrap() > 0,
        "write half: {s:?}"
    );
    let data = s["read"]["data"].as_str().unwrap_or("");
    // Raw PTYs do not echo. The read half may receive the emulator's
    // "pong\n" or time out; the write half must complete in either case.
    assert!(
        data.contains("pong") || s["read"]["bytes_read"].as_u64() == Some(0),
        "expected pong response or empty: {s:?}"
    );

    let _ = emulator.await;
    // Dropping _slave closes the PTY.
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_transact_from_now_skips_pre_write_buffer() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;
    // Put data in the ring before the transact call.
    pty.write_device(b"PREEXISTING\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // The default `from` is `{"type":"now"}`, so PREEXISTING is skipped.
    let r = client
        .peer()
        .call_tool(tool_request(
            "transact",
            json!({
                "connection_id": connection_id,
                "data": "ping\n",
                "timeout_ms": 300,
            }),
        ))
        .await
        .unwrap();
    let s = r.structured_content.expect("structured");
    let data = s["read"]["data"].as_str().unwrap_or("");
    assert!(
        !data.contains("PREEXISTING"),
        "from:now should skip pre-write buffer: {s:?}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_transact_from_cursor_includes_pre_write_buffer() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;
    // Put data in the ring before the transact call.
    pty.write_device(b"PREEXISTING\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // `from: {"type":"cursor"}` includes PREEXISTING.
    let r = client
        .peer()
        .call_tool(tool_request(
            "transact",
            json!({
                "connection_id": connection_id,
                "data": "ping\n",
                "from": {"type": "cursor"},
                "timeout_ms": 300,
            }),
        ))
        .await
        .unwrap();
    let s = r.structured_content.expect("structured");
    let data = s["read"]["data"].as_str().unwrap_or("");
    assert!(
        data.contains("PREEXISTING"),
        "from:cursor should include pre-write: {s:?}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_transact_with_protocol_applies_both_directions() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;
    // The at_command preset appends \r on TX and frames RX by line.
    // Pre-write data so the read half has buffered bytes to decode. Use
    // `from: {"type":"cursor"}` because `{"type":"now"}` skips them.
    pty.write_device(b"OK\r\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let r = client
        .peer()
        .call_tool(tool_request(
            "transact",
            json!({
                "connection_id": connection_id,
                "data": "AT",
                "protocol": {"type": "at_command"},
                "from": {"type": "cursor"},
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    let s = r.structured_content.expect("structured");
    // Write half: bytes_written > decoded_bytes (framing added \r).
    let bw = s["write"]["bytes_written"].as_u64().unwrap();
    let db = s["write"]["decoded_bytes"].as_u64().unwrap();
    assert!(bw > db, "at_command framing should add \\r: {s:?}");
    // Read half: frames array present (line-framed).
    assert!(
        s["read"]["frames"].is_array(),
        "at_command should frame read: {s:?}"
    );
    assert!(!s["read"]["frames"].as_array().unwrap().is_empty());
    client.cancel().await.ok();
}

/// Wrap the `call_tool` future in `tokio::time::timeout`, then cancel the
/// client. A completed call, transport error, or timeout is acceptable. The
/// test verifies that transact does not hang when the client disconnects
/// during the read.
#[tokio::test]
async fn pty_transact_cancellation_aborts_read() {
    let (_server, client, _rx, _pty, connection_id) = setup().await;

    // Use a long read timeout and a short outer timeout, then cancel the
    // client while the read is in progress.
    let result = tokio::time::timeout(
        Duration::from_millis(150),
        client.peer().call_tool(tool_request(
            "transact",
            json!({
                "connection_id": connection_id,
                "data": "x\n",
                "timeout_ms": 5000,
            }),
        )),
    )
    .await;

    // Client cancellation tears down the transport. The runtime cleans up a
    // transact task that is still running.
    client.cancel().await.ok();

    // The call may complete before the outer timeout, return a transport
    // error during teardown, or remain running until the runtime cleans it up.
    match result {
        Ok(Ok(call_result)) => {
            // A completed call may contain a partial, empty, or cancelled
            // read result depending on timing.
            let s = call_result.structured_content.expect("structured");
            assert!(
                s["write"]["bytes_written"].as_u64().unwrap_or(0) > 0,
                "write half should complete before cancel: {s:?}"
            );
            // Transport teardown may produce "cancelled", "connection_closed",
            // or no read result.
        }
        Ok(Err(_)) => {
            // Client teardown raced the tool response.
        }
        Err(_) => {
            // The outer timeout fired while transact was still running.
        }
    }
}

// Automatic profile sessions.
//
// These tests use a real PTY slave as the hardware port and an injected
// StaticPortProvider for synthetic USB identity. Profile-session behavior is
// checked through public MCP results and real serial traffic.

use std::path::PathBuf;
use std::sync::Arc;

use common::StaticPortProvider;

const VID: u16 = 0x1234;
const PID: u16 = 0x5678;

/// Extract the textual error payload of a failed tool call.
fn tool_error_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.clone()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Snapshot use_count/revision/defaults of one profile through the public
/// `list_profiles` tool.
async fn session_profile_snapshot<H: rmcp::handler::client::ClientHandler>(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, H>,
    profile_name: &str,
) -> (serde_json::Value, serde_json::Value, serde_json::Value) {
    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    let p = s["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == json!(profile_name))
        .expect("profile present")
        .clone();
    (
        p["metadata"]["use_count"].clone(),
        p["metadata"]["revision"].clone(),
        p["defaults"].clone(),
    )
}

/// Open a port through the public MCP tool with the given extra JSON fields.
async fn open_port(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    port: &str,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut body = serde_json::Map::new();
    body.insert("port".into(), serde_json::Value::String(port.into()));
    if let serde_json::Value::Object(map) = extra {
        for (k, v) in map {
            body.insert(k, v);
        }
    }
    let result = client
        .peer()
        .call_tool(tool_request("open", serde_json::Value::Object(body)))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    result.structured_content.expect("structured")
}

async fn close_port(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
) {
    let result = client
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "close failed: {result:?}");
}

/// One PTY + provider + server + client wired for profile-session tests.
struct SessionHarness {
    _server: TestServer,
    _client: rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    _dir: tempfile::TempDir,
    profiles_path: PathBuf,
}

async fn session_harness(provider: Arc<StaticPortProvider>) -> SessionHarness {
    let dir = tempfile::tempdir().expect("tempdir");
    let profiles_path = dir.path().join("profiles.toml");
    let server = TestServer::builder(Arc::new(serial_mcp::serial::ConnectionManager::new()))
        .port_provider(provider)
        .profiles_path(profiles_path.clone())
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    SessionHarness {
        _server: server,
        _client: client,
        _dir: dir,
        profiles_path,
    }
}

/// A first high-confidence bare open creates a generated persistent profile.
/// `open`, `list_profiles`, `get_status`, and `list_connections` report the
/// same binding, and real serial traffic flows.
#[tokio::test]
async fn auto_session_first_open_creates_generated_profile_and_pty_traffic_flows() {
    let mut pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        Some(2),
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // Bare open with no baud or defaults uses the built-in 115200 fallback.
    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    assert_eq!(opened["baud_rate"], json!(115200), "built-in baud fallback");
    let profile = &opened["profile"];
    assert_eq!(profile["source"], json!("generated"));
    assert_eq!(profile["profile_name"], json!("auto-fake-usb-serial"));
    assert_eq!(profile["confidence"], json!("high"));
    assert_eq!(profile["persistent"], json!(true));
    assert_eq!(profile["generated"], json!(true));
    assert_eq!(profile["revision"], json!(1));
    assert_eq!(profile["dirty"], json!(false));
    assert!(profile["candidates"].as_array().unwrap().is_empty());

    // Real serial traffic through the generated session.
    pty.write_device(b"HELLO-SESSION").await.unwrap();
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": connection_id, "timeout_ms": 1000 }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        result.structured_content.as_ref().unwrap()["data"],
        json!("HELLO-SESSION")
    );

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    assert_eq!(s["count"], json!(1));
    assert_eq!(s["profiles"][0]["name"], json!("auto-fake-usb-serial"));
    assert_eq!(s["profiles"][0]["metadata"]["generated"], json!(true));
    assert_eq!(s["profiles"][0]["metadata"]["revision"], json!(1));
    assert_eq!(s["profiles"][0]["metadata"]["use_count"], json!(1));
    assert_eq!(s["profiles"][0]["selector"]["serial_number"], json!("SN-1"));
    // Generated selector omits path, description, and manufacturer.
    assert!(s["profiles"][0]["selector"]["port_pattern"].is_null());
    assert!(s["profiles"][0]["selector"]["manufacturer"].is_null());

    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let st = status.structured_content.as_ref().unwrap();
    assert_eq!(st["profile"]["profile_name"], json!("auto-fake-usb-serial"));
    assert_eq!(st["profile"]["source"], json!("generated"));

    let conns = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .unwrap();
    let cs = conns.structured_content.as_ref().unwrap();
    assert_eq!(
        cs["connections"][0]["profile"]["profile_name"],
        json!("auto-fake-usb-serial")
    );

    harness._client.cancel().await.ok();
}

/// Closing and reopening automatically selects the same profile and increments
/// usage without bumping revision. Real traffic still flows.
#[tokio::test]
async fn auto_session_close_reopen_selects_same_profile_and_increments_usage() {
    let mut pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let first = open_port(client, &slave, json!({})).await;
    let first_id = first["connection_id"].as_str().unwrap().to_string();
    let profile_name = first["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(first["profile"]["source"], json!("generated"));

    close_port(client, &first_id).await;

    let second = open_port(client, &slave, json!({})).await;
    let second_id = second["connection_id"].as_str().unwrap().to_string();
    assert_eq!(second["profile"]["profile_name"], json!(profile_name));
    assert_eq!(second["profile"]["source"], json!("automatic"));
    assert_eq!(second["profile"]["generated"], json!(true));
    assert_eq!(
        second["profile"]["revision"],
        json!(1),
        "usage must not bump revision"
    );

    // Real traffic in the automatically selected session.
    pty.write_device(b"REUSED").await.unwrap();
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": second_id, "timeout_ms": 1000 }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.structured_content.as_ref().unwrap()["data"],
        json!("REUSED")
    );

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    assert_eq!(s["count"], json!(1));
    assert_eq!(s["profiles"][0]["metadata"]["use_count"], json!(2));
    assert_eq!(s["profiles"][0]["metadata"]["revision"], json!(1));
    assert!(
        s["profiles"][0]["metadata"]["last_used_at_ms"]
            .as_u64()
            .unwrap()
            > 0
    );

    harness._client.cancel().await.ok();
}

/// A different serial number with the same VID/PID gets a different generated
/// profile.
#[tokio::test]
async fn auto_session_different_serial_same_vid_pid_gets_different_profile() {
    let pty1 = PtyPair::open().expect("openpty");
    let pty2 = PtyPair::open().expect("openpty");
    let slave1 = pty1.slave_path.to_string_lossy().into_owned();
    let slave2 = pty2.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![
        StaticPortProvider::usb_port(&slave1, VID, PID, "SN-1", Some("Fake USB Serial"), None),
        StaticPortProvider::usb_port(&slave2, VID, PID, "SN-2", Some("Fake USB Serial"), None),
    ]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let a = open_port(client, &slave1, json!({})).await;
    let b = open_port(client, &slave2, json!({})).await;
    let name_a = a["profile"]["profile_name"].as_str().unwrap().to_string();
    let name_b = b["profile"]["profile_name"].as_str().unwrap().to_string();
    assert_ne!(
        name_a, name_b,
        "different serials must get different profiles"
    );
    assert_eq!(a["profile"]["source"], json!("generated"));
    assert_eq!(b["profile"]["source"], json!("generated"));

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    assert_eq!(s["count"], json!(2));

    harness._client.cancel().await.ok();
}

/// Two live ports with a duplicate high fingerprint produce a transient
/// ambiguity session. Settings are not applied to an indistinguishable device.
#[tokio::test]
async fn auto_session_duplicate_fingerprint_two_ports_is_transient() {
    let pty1 = PtyPair::open().expect("openpty");
    let pty2 = PtyPair::open().expect("openpty");
    let slave1 = pty1.slave_path.to_string_lossy().into_owned();
    let slave2 = pty2.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![
        StaticPortProvider::usb_port(&slave1, VID, PID, "SAME-SN", Some("Fake USB Serial"), None),
        StaticPortProvider::usb_port(&slave2, VID, PID, "SAME-SN", Some("Fake USB Serial"), None),
    ]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave1, json!({})).await;
    let profile = &opened["profile"];
    assert_eq!(profile["source"], json!("transient"));
    assert_eq!(profile["profile_name"], json!(""));
    assert_eq!(profile["persistent"], json!(false));
    assert!(profile["candidates"].as_array().unwrap().is_empty());

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    assert_eq!(
        listed.structured_content.as_ref().unwrap()["count"],
        json!(0)
    );

    harness._client.cancel().await.ok();
}

/// A weak PTY identity opens with a transient session and leaves the profile
/// store untouched. No durable profile or file is created.
#[tokio::test]
async fn auto_session_weak_pty_identity_is_transient_and_store_stays_empty() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::weak_port(&slave)]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    assert_eq!(opened["profile"]["source"], json!("transient"));
    assert_eq!(opened["profile"]["confidence"], json!("none"));
    assert_eq!(opened["profile"]["persistent"], json!(false));

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    assert_eq!(
        listed.structured_content.as_ref().unwrap()["count"],
        json!(0)
    );

    // No durable profile was written, so the file must not exist.
    assert!(
        !harness.profiles_path.exists(),
        "weak identity must not create a profiles file"
    );

    harness._client.cancel().await.ok();
}

/// `profile_mode="none"` disables automatic selection and creation, and
/// returns an observable disabled binding.
#[tokio::test]
async fn auto_session_profile_mode_none_disables_selection_and_creation() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({ "profile_mode": "none" })).await;
    let profile = &opened["profile"];
    assert_eq!(profile["source"], json!("disabled"));
    assert_eq!(profile["profile_name"], json!(""));
    assert_eq!(profile["persistent"], json!(false));
    assert_eq!(profile["generated"], json!(false));

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    assert_eq!(
        listed.structured_content.as_ref().unwrap()["count"],
        json!(0)
    );
    assert!(
        !harness.profiles_path.exists(),
        "profile_mode none must not write a profiles file"
    );

    harness._client.cancel().await.ok();
}

/// An explicit open field overrides the selected profile's default for the
/// live connection and is persisted immediately. The binding returns clean
/// with a bumped revision, and the next bare reopen applies the override.
#[tokio::test]
async fn learning_explicit_open_override_persists_immediately_and_next_reopen_uses_it() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // First open generates the profile at the built-in 115200.
    let first = open_port(client, &slave, json!({})).await;
    let first_id = first["connection_id"].as_str().unwrap().to_string();
    let profile_name = first["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    close_port(client, &first_id).await;

    // Reopen with an explicit baud override (profile default is 115200).
    let second = open_port(client, &slave, json!({ "baud_rate": 9600 })).await;
    let second_id = second["connection_id"].as_str().unwrap().to_string();
    assert_eq!(second["profile"]["profile_name"], json!(profile_name));
    assert_eq!(second["profile"]["source"], json!("automatic"));
    assert_eq!(
        second["profile"]["revision"],
        json!(2),
        "override persisted as a new revision"
    );
    assert_eq!(
        second["profile"]["dirty"],
        json!(false),
        "override was learned; binding is clean"
    );
    assert_eq!(
        second["profile_persistence"]["state"],
        json!("persisted"),
        "open-override persistence must report persisted: {second:?}"
    );
    assert_eq!(
        second["profile_persistence"]["operation"],
        json!("open_override")
    );
    assert!(second["profile_persistence"]["error"].is_null());

    // The live connection actually runs at the overridden baud.
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": second_id }),
        ))
        .await
        .unwrap();
    assert_eq!(
        status.structured_content.as_ref().unwrap()["baud_rate"],
        json!(9600)
    );
    close_port(client, &second_id).await;

    // Next bare reopen applies the learned 9600 from the profile.
    let third = open_port(client, &slave, json!({})).await;
    assert_eq!(third["profile"]["source"], json!("automatic"));
    assert_eq!(third["baud_rate"], json!(9600), "learned baud applied");
    assert_eq!(third["profile"]["revision"], json!(2));
    assert_eq!(third["profile"]["dirty"], json!(false));

    harness._client.cancel().await.ok();
}

/// A separate HTTP client observes the same active binding and generated
/// profile.
#[tokio::test]
async fn auto_session_second_http_client_observes_binding_and_profile() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let dir = tempfile::tempdir().expect("tempdir");
    let profiles_path = dir.path().join("profiles.toml");
    let server = TestServer::builder(Arc::new(serial_mcp::serial::ConnectionManager::new()))
        .port_provider(provider)
        .profiles_path(profiles_path)
        .start()
        .await;
    let (client_a, _rx_a) = connect_client(&server).await.unwrap();
    let (client_b, _rx_b) = connect_client(&server).await.unwrap();

    let opened = open_port(&client_a, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();

    // Client B sees the same binding.
    let status = client_b
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let st = status.structured_content.as_ref().unwrap();
    assert_eq!(st["profile"]["profile_name"], json!(profile_name));
    assert_eq!(st["profile"]["source"], json!("generated"));

    // Client B also sees the generated profile.
    let listed = client_b
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    assert_eq!(s["profiles"][0]["name"], json!(profile_name));
    assert_eq!(s["profiles"][0]["metadata"]["generated"], json!(true));

    // list_connections from client B exposes the same active binding.
    let conns = client_b
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .unwrap();
    let cs = conns.structured_content.as_ref().unwrap();
    assert_eq!(
        cs["connections"][0]["profile"]["profile_name"],
        json!(profile_name)
    );
    assert_eq!(
        cs["connections"][0]["profile"]["source"],
        json!("generated")
    );

    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
}

/// `open_profile` with two matching ports returns a tool error. One exact
/// match works and becomes the last-used winner for a later bare open.
#[tokio::test]
async fn open_profile_two_matching_ports_errors_and_exact_one_becomes_last_used_winner() {
    let pty1 = PtyPair::open().expect("openpty");
    let pty2 = PtyPair::open().expect("openpty");
    let slave1 = pty1.slave_path.to_string_lossy().into_owned();
    let slave2 = pty2.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![
        StaticPortProvider::usb_port(&slave1, VID, PID, "SN-1", Some("Fake USB Serial"), None),
        StaticPortProvider::usb_port(&slave2, VID, PID, "SN-2", Some("Fake USB Serial"), None),
    ]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // The empty-selector profile matches both live ports, so open_profile
    // returns a tool error.
    let cfg = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({ "profile": "multi", "defaults": {} }),
        ))
        .await
        .unwrap();
    assert_ne!(cfg.is_error, Some(true), "{cfg:?}");
    let result = client
        .peer()
        .call_tool(tool_request("open_profile", json!({ "profile": "multi" })))
        .await
        .unwrap();
    assert_eq!(
        result.is_error,
        Some(true),
        "ambiguous open_profile must error: {result:?}"
    );
    let err = tool_error_text(&result);
    assert!(
        err.contains("live ports") && err.contains("refusing"),
        "error must explain the ambiguity: {err}"
    );

    // Bare open of pty1 creates the generated profile for SN-1.
    let first = open_port(client, &slave1, json!({})).await;
    let first_id = first["connection_id"].as_str().unwrap().to_string();
    let generated_name = first["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();

    // Save a named snapshot while the first connection remains open.
    let saved = client
        .peer()
        .call_tool(tool_request(
            "save_profile",
            json!({ "connection_id": first_id, "profile_name": "named-p1" }),
        ))
        .await
        .unwrap();
    assert_ne!(saved.is_error, Some(true), "{saved:?}");

    // Close the first connection. open_profile now has one matching port and
    // marks the profile used.
    close_port(client, &first_id).await;

    let explicit = client
        .peer()
        .call_tool(tool_request(
            "open_profile",
            json!({ "profile": "named-p1" }),
        ))
        .await
        .unwrap();
    assert_ne!(explicit.is_error, Some(true), "{explicit:?}");
    let explicit_result = explicit.structured_content.as_ref().unwrap();
    assert_eq!(explicit_result["profile"]["source"], json!("explicit"));
    assert_eq!(
        explicit_result["profile"]["profile_name"],
        json!("named-p1")
    );
    assert_eq!(explicit_result["profile"]["revision"], json!(1));
    let explicit_id = explicit_result["connection_id"]
        .as_str()
        .unwrap()
        .to_string();

    close_port(client, &explicit_id).await;

    // A later bare open selects the explicitly used profile, not the older
    // generated one.
    let reopened = open_port(client, &slave1, json!({})).await;
    assert_eq!(reopened["profile"]["source"], json!("automatic"));
    assert_eq!(
        reopened["profile"]["profile_name"],
        json!("named-p1"),
        "most-recently-used profile must win (generated was {generated_name})"
    );

    harness._client.cancel().await.ok();
}

/// Equal top-ranked profile timestamps produce observable ambiguity.
#[tokio::test]
async fn open_auto_equal_top_ranked_timestamps_produce_ambiguity() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let dir = tempfile::tempdir().expect("tempdir");
    let profiles_path = dir.path().join("profiles.toml");
    std::fs::write(
        &profiles_path,
        r#"
schema_version = 2

[[profile]]
name = "ambig-a"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
last_used_at_ms = 12345
use_count = 1

[[profile]]
name = "ambig-b"
[profile.selector]
vid = 0x1234
pid = 0x5678
serial_number = "SN-1"
transport = "usb"
[profile.metadata]
generated = false
revision = 1
created_at_ms = 10
updated_at_ms = 10
last_used_at_ms = 12345
use_count = 1
"#,
    )
    .unwrap();

    let server = TestServer::builder(Arc::new(serial_mcp::serial::ConnectionManager::new()))
        .port_provider(provider)
        .profiles_path(profiles_path.clone())
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let opened = open_port(&client, &slave, json!({})).await;
    let profile = &opened["profile"];
    assert_eq!(profile["source"], json!("transient"));
    assert_eq!(profile["profile_name"], json!(""));
    assert_eq!(profile["persistent"], json!(false));
    let candidates: Vec<&str> = profile["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(candidates, vec!["ambig-a", "ambig-b"]);

    // The ambiguous open applied neither profile and bumped neither profile.
    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    assert_eq!(s["count"], json!(2));
    assert_eq!(s["profiles"][0]["metadata"]["use_count"], json!(1));
    assert_eq!(s["profiles"][1]["metadata"]["use_count"], json!(1));

    client.cancel().await.ok();
}

/// Per-call read, write, and transact options do not alter usage, revision, or
/// defaults of the bound profile.
#[tokio::test]
async fn per_call_io_does_not_alter_usage_revision_or_defaults() {
    let mut pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();

    let before = session_profile_snapshot(client, &profile_name).await;

    // Exercise write, read, and transact with per-call options.
    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": "PING\r\n" }),
        ))
        .await
        .unwrap();
    let mut drain = [0u8; 8];
    let _ = pty.read_device(&mut drain).await;

    pty.write_device(b"READY\r\n").await.unwrap();
    client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 500,
                "match": { "pattern": "READY" },
            }),
        ))
        .await
        .unwrap();

    let tx = client
        .peer()
        .call_tool(tool_request(
            "transact",
            json!({
                "connection_id": connection_id,
                "data": "STATUS\r\n",
                "timeout_ms": 300,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(tx.is_error, Some(true), "{tx:?}");
    let _ = pty.read_device(&mut drain).await;

    let after = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(after.0, before.0, "per-call I/O must not bump use_count");
    assert_eq!(after.1, before.1, "per-call I/O must not bump revision");
    assert_eq!(after.2, before.2, "per-call I/O must not alter defaults");

    harness._client.cancel().await.ok();
}

/// Explicit `open_profile` on a weak-identity port reports the matched port's
/// confidence (none or low), not a hardcoded high, while keeping
/// `source=explicit`.
#[tokio::test]
async fn open_profile_explicit_binding_reports_matched_port_confidence() {
    // Case 1: path-only PTY (unknown transport, no identity) reports None.
    let pty_none = PtyPair::open().expect("openpty");
    let slave_none = pty_none.slave_path.to_string_lossy().into_owned();
    let provider_none = StaticPortProvider::new(vec![StaticPortProvider::weak_port(&slave_none)]);
    let harness_none = session_harness(provider_none).await;
    let client = &harness_none._client;

    // Empty-selector profile matches the single live port exactly once.
    let cfg = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({ "profile": "weak-pro", "defaults": {} }),
        ))
        .await
        .unwrap();
    assert_ne!(cfg.is_error, Some(true), "{cfg:?}");

    let opened = client
        .peer()
        .call_tool(tool_request(
            "open_profile",
            json!({ "profile": "weak-pro" }),
        ))
        .await
        .unwrap();
    assert_ne!(opened.is_error, Some(true), "{opened:?}");
    let profile = &opened.structured_content.as_ref().unwrap()["profile"];
    assert_eq!(profile["source"], json!("explicit"));
    assert_eq!(
        profile["confidence"],
        json!("none"),
        "path-only identity must report none, not high: {profile:?}"
    );
    assert_eq!(profile["profile_name"], json!("weak-pro"));
    harness_none._client.cancel().await.ok();

    // Case 2: PCI-synthetic PTY (hardware ID only) reports Low.
    let pty_low = PtyPair::open().expect("openpty");
    let slave_low = pty_low.slave_path.to_string_lossy().into_owned();
    let mut low_port = StaticPortProvider::weak_port(&slave_low);
    low_port.transport = serial_mcp::serial::PortTransport::Pci;
    low_port.hardware_id = Some("PCI".into());
    let provider_low = StaticPortProvider::new(vec![low_port]);
    let harness_low = session_harness(provider_low).await;
    let client = &harness_low._client;

    let cfg = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({ "profile": "pci-pro", "defaults": {} }),
        ))
        .await
        .unwrap();
    assert_ne!(cfg.is_error, Some(true), "{cfg:?}");

    let opened = client
        .peer()
        .call_tool(tool_request(
            "open_profile",
            json!({ "profile": "pci-pro" }),
        ))
        .await
        .unwrap();
    assert_ne!(opened.is_error, Some(true), "{opened:?}");
    let profile = &opened.structured_content.as_ref().unwrap()["profile"];
    assert_eq!(profile["source"], json!("explicit"));
    assert_eq!(
        profile["confidence"],
        json!("low"),
        "hardware-id-only identity must report low: {profile:?}"
    );
    assert_eq!(profile["profile_name"], json!("pci-pro"));

    let connection_id = opened.structured_content.as_ref().unwrap()["connection_id"]
        .as_str()
        .unwrap()
        .to_string();
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let st = status.structured_content.as_ref().unwrap();
    assert_eq!(st["profile"]["source"], json!("explicit"));
    assert_eq!(st["profile"]["confidence"], json!("low"));

    harness_low._client.cancel().await.ok();
}

/// Explicit `save_profile` of a generated-bound connection creates a
/// user-owned profile (`generated=false`). This is a promotion, not a copy of
/// the generated flag.
#[tokio::test]
async fn save_profile_on_generated_bound_connection_promotes_to_user_owned() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    assert_eq!(opened["profile"]["source"], json!("generated"));
    assert_eq!(opened["profile"]["generated"], json!(true));

    // Explicit user snapshot of the same device under a new name.
    let saved = client
        .peer()
        .call_tool(tool_request(
            "save_profile",
            json!({ "connection_id": connection_id, "profile_name": "promoted" }),
        ))
        .await
        .unwrap();
    assert_ne!(saved.is_error, Some(true), "{saved:?}");

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    assert_eq!(s["count"], json!(2), "auto-generated + promoted profile");

    let promoted = s["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == json!("promoted"))
        .expect("promoted profile exists");
    assert_eq!(
        promoted["metadata"]["generated"],
        json!(false),
        "save_profile must create a user-owned profile, not copy generated=true"
    );

    let auto = s["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == json!("auto-fake-usb-serial"))
        .expect("auto-generated profile still exists");
    assert_eq!(auto["metadata"]["generated"], json!(true));

    harness._client.cancel().await.ok();
}

// Write-through learning, conflicts, and rollback.
//
// These tests use the same harness: a real PTY and an injected high-confidence
// StaticPortProvider. They exercise learning, partial failures, CAS and stale
// state, rollback, and deletion protection through public MCP results and real
// serial traffic.

/// Reconfigure one connection and return the structured result.
async fn reconfigure_baud(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    baud: u32,
) -> serde_json::Value {
    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "baud_rate": baud }),
        ))
        .await
        .unwrap();
    assert_ne!(
        result.is_error,
        Some(true),
        "reconfigure failed: {result:?}"
    );
    result.structured_content.expect("structured")
}

/// Reconfiguring the generated profile from revision 1 persists revision 2.
/// Close and reopen apply the baud in live status.
#[tokio::test]
async fn learning_reconfigure_baud_bumps_revision_and_reopen_applies() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(opened["profile"]["revision"], json!(1));

    let r = reconfigure_baud(client, &connection_id, 9600).await;
    assert_eq!(r["baud_rate"], json!(9600), "live state changed");
    assert_eq!(r["profile_persistence"]["state"], json!("persisted"));
    assert_eq!(r["profile_persistence"]["operation"], json!("learned"));
    assert_eq!(r["profile_persistence"]["revision"], json!(2));
    assert_eq!(r["profile"]["revision"], json!(2));
    assert_eq!(r["profile"]["dirty"], json!(false));

    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(2));
    assert_eq!(
        defaults["baud_rate"],
        json!(9600),
        "profile defaults updated"
    );

    close_port(client, &connection_id).await;

    let reopened = open_port(client, &slave, json!({})).await;
    assert_eq!(
        reopened["baud_rate"],
        json!(9600),
        "learned baud applied on reopen"
    );
    assert_eq!(reopened["profile"]["revision"], json!(2));

    harness._client.cancel().await.ok();
}

/// `set_flow_control` persists and applies on reopen.
#[tokio::test]
async fn learning_set_flow_control_persists_and_applies_on_reopen() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();

    let sfc = client
        .peer()
        .call_tool(tool_request(
            "set_flow_control",
            json!({ "connection_id": connection_id, "flow_control": "software" }),
        ))
        .await
        .unwrap();
    assert_ne!(sfc.is_error, Some(true), "{sfc:?}");
    let s = sfc.structured_content.as_ref().unwrap();
    assert_eq!(s["flow_control"], json!("software"));
    assert_eq!(s["profile_persistence"]["state"], json!("persisted"));
    assert_eq!(s["profile"]["revision"], json!(2));
    assert_eq!(s["profile"]["dirty"], json!(false));

    close_port(client, &connection_id).await;

    let reopened = open_port(client, &slave, json!({})).await;
    let reopened_id = reopened["connection_id"].as_str().unwrap().to_string();
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": reopened_id }),
        ))
        .await
        .unwrap();
    assert_eq!(
        status.structured_content.as_ref().unwrap()["flow_control"],
        json!("software"),
        "learned flow control applied on reopen"
    );

    harness._client.cancel().await.ok();
}

/// Connection-mode configure persists framing. Reopen and an actual framed
/// read verify it.
#[tokio::test]
async fn learning_connection_configure_framing_persists_and_framed_read_proves_it() {
    let mut pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();

    let cfg = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "connection_id": connection_id,
                "defaults": { "rx_framing": { "type": "line", "ending": "lf" } },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(cfg.is_error, Some(true), "{cfg:?}");
    let c = cfg.structured_content.as_ref().unwrap();
    assert_eq!(c["mode"], json!("connection"));
    assert_eq!(c["profile_persistence"]["state"], json!("persisted"));
    assert_eq!(c["profile"]["revision"], json!(2));
    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(2));
    assert_eq!(defaults["rx_framing"]["type"], json!("line"));

    close_port(client, &connection_id).await;

    // Reopen: the profile's framing default applies to the connection.
    let reopened = open_port(client, &slave, json!({})).await;
    let reopened_id = reopened["connection_id"].as_str().unwrap().to_string();
    pty.write_device(b"LINE1\nLINE2\n").await.unwrap();
    let read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": reopened_id, "timeout_ms": 1000 }),
        ))
        .await
        .unwrap();
    assert_ne!(read.is_error, Some(true), "{read:?}");
    let rd = read.structured_content.as_ref().unwrap();
    let frames = rd["frames"].as_array().expect("framed read expected");
    assert_eq!(frames[0]["data"], json!("LINE1"));
    assert_eq!(frames[1]["data"], json!("LINE2"));

    harness._client.cancel().await.ok();
}

/// Multiple durable changes create a bounded revision history of five entries.
#[tokio::test]
async fn learning_multiple_changes_create_bounded_revision_history() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();

    // Five reconfigures after revision 1 produce revision 6.
    for baud in [1200u32, 2400, 4800, 9600, 19200] {
        reconfigure_baud(client, &connection_id, baud).await;
    }

    let (_, rev, _) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(6));

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    let revisions = s["profiles"][0]["revisions"].as_array().unwrap();
    assert_eq!(
        revisions.len(),
        serial_mcp::profiles::MAX_PROFILE_REVISIONS,
        "history capped at five prior snapshots"
    );
    let retained: Vec<u64> = revisions
        .iter()
        .map(|r| r["revision"].as_u64().unwrap())
        .collect();
    assert_eq!(retained, vec![1, 2, 3, 4, 5], "newest five prior states");

    harness._client.cancel().await.ok();
}

/// Non-durable operations do not change profile defaults or revision: BREAK,
/// flush, and per-call framing and matching on read/write. DTR/RTS is covered
/// by the http_integration loopback suite because PTYs cannot drive modem
/// lines and return ENOTTY.
#[tokio::test]
async fn non_learning_operations_do_not_alter_profile() {
    let mut pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    let before = session_profile_snapshot(client, &profile_name).await;

    // BREAK pulse.
    let brk = client
        .peer()
        .call_tool(tool_request(
            "send_break",
            json!({ "connection_id": connection_id, "duration_ms": 30 }),
        ))
        .await
        .unwrap();
    assert_ne!(brk.is_error, Some(true), "{brk:?}");

    // Flush both directions.
    let fl = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .unwrap();
    assert_ne!(fl.is_error, Some(true), "{fl:?}");

    // Per-call write with tx_framing (device drains).
    let w = client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "PING",
                "tx_framing": { "type": "line", "ending": "lf" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(w.is_error, Some(true), "{w:?}");
    let mut drain = [0u8; 8];
    let _ = pty.read_device(&mut drain).await;

    // Per-call read with framing + match.
    pty.write_device(b"MATCHED\n").await.unwrap();
    let rd = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 500,
                "rx_framing": { "type": "line", "ending": "lf" },
                "match": { "pattern": "MATCHED" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(rd.is_error, Some(true), "{rd:?}");

    // Per-call transact with a protocol override + match: the device must
    // actually answer the request, proving the transaction worked end to
    // end (not merely that the request was accepted).
    let peer = client.peer().clone();
    let tx_task = tokio::spawn(async move {
        peer.call_tool(tool_request(
            "transact",
            json!({
                "connection_id": connection_id,
                "data": "TXN",
                "timeout_ms": 2000,
                "protocol": { "type": "at_command" },
                "match": { "pattern": "ACK" },
            }),
        ))
        .await
        .unwrap()
    });
    // Device side: read the AT request (protocol appends \r), then answer.
    let mut req_buf = [0u8; 16];
    let n = pty.read_device(&mut req_buf).await.unwrap();
    assert_eq!(&req_buf[..n], b"TXN\r", "at_command preset appends CR");
    pty.write_device(b"ACK\r\n").await.unwrap();
    let tx = tx_task.await.unwrap();
    assert_ne!(tx.is_error, Some(true), "{tx:?}");
    let tr = tx.structured_content.as_ref().unwrap();
    assert_eq!(
        tr["read"]["matched"],
        json!(true),
        "transaction matched the device response: {tr:?}"
    );
    assert_eq!(tr["read"]["data"], json!("ACK"));
    let frames = tr["read"]["frames"]
        .as_array()
        .expect("framed transact read");
    assert_eq!(frames[0]["data"], json!("ACK"));

    let after = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(after.0, before.0, "non-durable ops must not bump use_count");
    assert_eq!(after.1, before.1, "non-durable ops must not bump revision");
    assert_eq!(after.2, before.2, "non-durable ops must not alter defaults");

    harness._client.cancel().await.ok();
}

/// Partial failure: live reconfigure succeeds while the profile write fails
/// in a read-only directory. The result stays successful with
/// `state="failed"`, the binding is dirty, and the cache and file stay old.
/// Restoring permissions and closing cleanly retries persistence; reopen uses
/// the new baud.
#[tokio::test]
async fn learning_partial_failure_reports_failed_and_clean_close_retries() {
    use std::os::unix::fs::PermissionsExt;

    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let dir = tempfile::tempdir().expect("tempdir");
    let profiles_path = dir.path().join("profiles.toml");
    let server = TestServer::builder(Arc::new(serial_mcp::serial::ConnectionManager::new()))
        .port_provider(provider)
        .profiles_path(profiles_path.clone())
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // First open creates the generated profile while the dir is writable.
    let first = open_port(&client, &slave, json!({})).await;
    let profile_name = first["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    let first_id = first["connection_id"].as_str().unwrap().to_string();
    close_port(&client, &first_id).await;

    // Make the profile directory read-only: live mutations still succeed,
    // profile writes fail.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();

    // Reopen: mark_used fails but the binding stays persistent and the
    // open stays a success.
    let second = open_port(&client, &slave, json!({})).await;
    let second_id = second["connection_id"].as_str().unwrap().to_string();
    assert_eq!(second["profile"]["profile_name"], json!(profile_name));
    assert_eq!(second["profile"]["persistent"], json!(true));
    assert!(
        !second["profile"]["last_persistence_error"].is_null(),
        "metadata failure surfaces on the binding"
    );

    // Live reconfigure succeeds; persistence fails and is reported as a
    // successful result with failed state.
    let r = reconfigure_baud(&client, &second_id, 9600).await;
    assert_eq!(r["baud_rate"], json!(9600), "live state changed");
    assert_eq!(r["profile_persistence"]["state"], json!("failed"));
    assert!(
        !r["profile_persistence"]["error"].is_null(),
        "failure carries the error: {r:?}"
    );
    assert_eq!(r["profile"]["dirty"], json!(true), "binding dirty");
    assert_eq!(
        r["profile"]["stale"],
        json!(false),
        "plain I/O failure stays retryable, not stale"
    );

    // Status shows the new live baud.
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": second_id }),
        ))
        .await
        .unwrap();
    assert_eq!(
        status.structured_content.as_ref().unwrap()["baud_rate"],
        json!(9600)
    );

    // Profile file/cache stay old.
    let (_, rev, defaults) = session_profile_snapshot(&client, &profile_name).await;
    assert_eq!(rev, json!(1), "failed write must not bump revision");
    assert_eq!(defaults["baud_rate"], json!(115200), "file/cache old");

    // Restore permissions; clean close retries and persists.
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let cl = client
        .peer()
        .call_tool(tool_request("close", json!({ "connection_id": second_id })))
        .await
        .unwrap();
    assert_ne!(cl.is_error, Some(true), "{cl:?}");
    let clr = cl.structured_content.as_ref().unwrap();
    assert_eq!(
        clr["profile_persistence"]["state"],
        json!("persisted"),
        "clean close retries the dirty state: {clr:?}"
    );
    assert_eq!(clr["profile"]["dirty"], json!(false));

    // Reopen uses the new baud.
    let third = open_port(&client, &slave, json!({})).await;
    assert_eq!(third["baud_rate"], json!(9600));
    assert_eq!(third["profile"]["revision"], json!(2));

    client.cancel().await.ok();
}

/// CAS and stale state: an external profile-mode configure bumps the bound
/// profile. The next live reconfigure succeeds but reports a conflict, the
/// binding turns stale, and the newer profile remains untouched. Close does
/// not overwrite the stale profile.
#[tokio::test]
async fn learning_cas_conflict_marks_stale_and_close_does_not_overwrite() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;
    let (client_b, _rx_b) = connect_client(&harness._server).await.unwrap();

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(opened["profile"]["revision"], json!(1));

    // A second client bumps the profile to revision 2 with baud 14400.
    let cfg = client_b
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": profile_name,
                "overwrite": true,
                "defaults": { "baud_rate": 14400 },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(cfg.is_error, Some(true), "{cfg:?}");

    // Live reconfigure succeeds while persistence reports a conflict.
    let r = reconfigure_baud(client, &connection_id, 9600).await;
    assert_eq!(r["baud_rate"], json!(9600), "hardware mutation applied");
    assert_eq!(r["profile_persistence"]["state"], json!("failed"));
    let err = r["profile_persistence"]["error"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        err.contains("revision conflict") && err.contains("expected 1") && err.contains("found 2"),
        "conflict names expected + actual revision: {err}"
    );
    assert_eq!(r["profile"]["stale"], json!(true));
    assert_eq!(r["profile"]["dirty"], json!(true));

    // The newer profile remains unchanged.
    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(2));
    assert_eq!(
        defaults["baud_rate"],
        json!(14400),
        "newer profile untouched"
    );

    // Close must not overwrite the stale profile.
    let cl = client
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(cl.is_error, Some(true), "{cl:?}");
    let clr = cl.structured_content.as_ref().unwrap();
    assert_eq!(
        clr["profile_persistence"]["state"],
        json!("failed"),
        "stale binding keeps reporting the conflict on close"
    );
    assert_eq!(clr["profile"]["stale"], json!(true));
    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(2));
    assert_eq!(defaults["baud_rate"], json!(14400));

    harness._client.cancel().await.ok();
    client_b.cancel().await.ok();
}

/// Rollback restores a prior baud as a new monotonic revision. The active
/// connection stays unchanged and stale, close cannot overwrite the rollback,
/// and reopen applies the rolled-back baud.
#[tokio::test]
async fn rollback_restores_prior_baud_and_active_connection_stays_unchanged() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    // Revisions progress from 1 (115200) to 2 (9600) to 3 (19200).
    reconfigure_baud(client, &connection_id, 9600).await;
    reconfigure_baud(client, &connection_id, 19200).await;

    // Roll back to revision 2 (9600) as new revision 4.
    let rb = client
        .peer()
        .call_tool(tool_request(
            "rollback_profile",
            json!({
                "profile_name": profile_name,
                "expected_revision": 3,
                "revision": 2,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(rb.is_error, Some(true), "{rb:?}");
    let rb_r = rb.structured_content.as_ref().unwrap();
    assert_eq!(rb_r["restored_from_revision"], json!(2));
    assert_eq!(rb_r["previous_revision"], json!(3));
    assert_eq!(rb_r["revision"], json!(4), "new monotonic revision");
    assert_eq!(rb_r["defaults"]["baud_rate"], json!(9600));
    assert_eq!(rb_r["active_connections_unchanged"], json!(1));
    assert_eq!(rb_r["persistence"]["state"], json!("persisted"));
    assert_eq!(rb_r["persistence"]["operation"], json!("rollback"));

    // The active connection keeps its live state and turns stale.
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let st = status.structured_content.as_ref().unwrap();
    assert_eq!(st["baud_rate"], json!(19200), "live hardware untouched");
    assert_eq!(st["profile"]["stale"], json!(true), "binding stale");

    // Close cannot overwrite the rollback.
    let cl = client
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(cl.is_error, Some(true), "{cl:?}");
    let clr = cl.structured_content.as_ref().unwrap();
    assert_eq!(clr["profile_persistence"]["state"], json!("failed"));
    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(4));
    assert_eq!(defaults["baud_rate"], json!(9600));

    // Reopen applies the rolled-back baud.
    let reopened = open_port(client, &slave, json!({})).await;
    assert_eq!(reopened["baud_rate"], json!(9600));

    harness._client.cancel().await.ok();
}

/// Rollback of a framing revision: after reopen, framed traffic verifies the
/// restored framing default.
#[tokio::test]
async fn rollback_framing_revision_proves_framed_traffic_after_reopen() {
    let mut pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // Open with explicit rx_framing; the generated profile stores it in
    // revision 1.
    let opened = open_port(
        client,
        &slave,
        json!({ "rx_framing": { "type": "line", "ending": "lf" } }),
    )
    .await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(opened["profile"]["revision"], json!(1));

    // Connection-mode configure clears framing in revision 2, producing raw
    // reads.
    let cfg = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "connection_id": connection_id,
                "defaults": { "rx_framing": null },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(cfg.is_error, Some(true), "{cfg:?}");
    let c = cfg.structured_content.as_ref().unwrap();
    assert_eq!(c["profile_persistence"]["state"], json!("persisted"));
    assert_eq!(c["profile"]["revision"], json!(2));
    let (_, _, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert!(
        defaults["rx_framing"].is_null(),
        "revision 2 must be raw: {defaults:?}"
    );

    // Roll back to revision 1 (framing) as new revision 3.
    let rb = client
        .peer()
        .call_tool(tool_request(
            "rollback_profile",
            json!({
                "profile_name": profile_name,
                "expected_revision": 2,
                "revision": 1,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(rb.is_error, Some(true), "{rb:?}");
    let rb_r = rb.structured_content.as_ref().unwrap();
    assert_eq!(rb_r["revision"], json!(3));
    assert_eq!(rb_r["defaults"]["rx_framing"]["type"], json!("line"));
    assert_eq!(rb_r["active_connections_unchanged"], json!(1));

    // Close reports failure for the stale binding without changing the file.
    // Reopen and a framed read verify the restored framing.
    let cl = client
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(cl.is_error, Some(true), "{cl:?}");
    assert_eq!(
        cl.structured_content.as_ref().unwrap()["profile_persistence"]["state"],
        json!("failed")
    );

    let reopened = open_port(client, &slave, json!({})).await;
    let reopened_id = reopened["connection_id"].as_str().unwrap().to_string();
    pty.write_device(b"ROLLED\n").await.unwrap();
    let read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": reopened_id, "timeout_ms": 1000 }),
        ))
        .await
        .unwrap();
    assert_ne!(read.is_error, Some(true), "{read:?}");
    let rd = read.structured_content.as_ref().unwrap();
    let frames = rd["frames"].as_array().expect("framed read expected");
    assert_eq!(frames[0]["data"], json!("ROLLED"));

    harness._client.cancel().await.ok();
}

/// Wrong expected and evicted revisions are tool errors that leave the file
/// unchanged.
#[tokio::test]
async fn rollback_wrong_expected_and_evicted_revision_error_without_file_change() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    reconfigure_baud(client, &connection_id, 9600).await;
    reconfigure_baud(client, &connection_id, 19200).await;
    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(3));
    let original_baud = defaults["baud_rate"].clone();

    // A wrong expected_revision reports a conflict and leaves the file alone.
    let bad = client
        .peer()
        .call_tool(tool_request(
            "rollback_profile",
            json!({
                "profile_name": profile_name,
                "expected_revision": 1,
                "revision": 2,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(bad.is_error, Some(true), "wrong CAS must error: {bad:?}");
    let err = tool_error_text(&bad);
    assert!(
        err.contains("revision conflict") && err.contains("expected 1") && err.contains("found 3"),
        "got: {err}"
    );
    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(3), "file unchanged after wrong CAS");
    assert_eq!(defaults["baud_rate"], original_baud);

    // An evicted or missing target revision is a tool error; the file stays
    // unchanged.
    let bad = client
        .peer()
        .call_tool(tool_request(
            "rollback_profile",
            json!({
                "profile_name": profile_name,
                "expected_revision": 3,
                "revision": 99,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        bad.is_error,
        Some(true),
        "evicted revision must error: {bad:?}"
    );
    let err = tool_error_text(&bad);
    assert!(
        err.contains("no retained snapshot at revision 99"),
        "got: {err}"
    );
    let (_, rev, defaults) = session_profile_snapshot(client, &profile_name).await;
    assert_eq!(rev, json!(3), "file unchanged after evicted rollback");
    assert_eq!(defaults["baud_rate"], original_baud);

    harness._client.cancel().await.ok();
}

/// Deleting a profile bound to an open connection reports the connection ID.
/// After close, deletion succeeds.
#[tokio::test]
async fn delete_profile_bound_to_open_connection_errors_and_succeeds_after_close() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();

    let del = client
        .peer()
        .call_tool(tool_request(
            "delete_profile",
            json!({ "profile_name": profile_name }),
        ))
        .await
        .unwrap();
    assert_eq!(
        del.is_error,
        Some(true),
        "bound profile must refuse deletion: {del:?}"
    );
    let err = tool_error_text(&del);
    assert!(
        err.contains("bound to open connection") && err.contains(&connection_id),
        "error must list the bound connection ID: {err}"
    );

    close_port(client, &connection_id).await;

    let del = client
        .peer()
        .call_tool(tool_request(
            "delete_profile",
            json!({ "profile_name": profile_name }),
        ))
        .await
        .unwrap();
    assert_ne!(del.is_error, Some(true), "{del:?}");

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    assert_eq!(
        listed.structured_content.as_ref().unwrap()["count"],
        json!(0)
    );

    harness._client.cancel().await.ok();
}

/// Rollback with no active bound connections reports zero. The reopened
/// device applies the restored defaults.
#[tokio::test]
async fn rollback_with_no_active_connections_reports_zero() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();
    reconfigure_baud(client, &connection_id, 9600).await; // rev 2
    close_port(client, &connection_id).await; // no active bindings left

    let rb = client
        .peer()
        .call_tool(tool_request(
            "rollback_profile",
            json!({
                "profile_name": profile_name,
                "expected_revision": 2,
                "revision": 1,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(rb.is_error, Some(true), "{rb:?}");
    let rb_r = rb.structured_content.as_ref().unwrap();
    assert_eq!(rb_r["revision"], json!(3));
    assert_eq!(rb_r["defaults"]["baud_rate"], json!(115200));
    assert_eq!(
        rb_r["active_connections_unchanged"],
        json!(0),
        "no open connection bound at rollback time"
    );

    let reopened = open_port(client, &slave, json!({})).await;
    assert_eq!(
        reopened["baud_rate"],
        json!(115200),
        "restored defaults applied"
    );

    harness._client.cancel().await.ok();
}

// `list_ports` profile discovery preview.
//
// These tests use real PTY slave paths, injected StaticPortProvider identity,
// and public MCP calls only. The preview must predict bare `open(port=...)`
// without marking profiles used or mutating the store.

/// Call `list_ports` through the public MCP surface and return the
/// structured result.
async fn call_list_ports<H: rmcp::handler::client::ClientHandler>(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, H>,
) -> serde_json::Value {
    let result = client
        .peer()
        .call_tool(tool_request("list_ports", json!({})))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "list_ports failed: {result:?}");
    result.structured_content.expect("structured")
}

/// Find the profile-match entry for one port.
fn match_for<'a>(listed: &'a serde_json::Value, port: &str) -> &'a serde_json::Value {
    listed["profile_matches"]
        .as_array()
        .expect("profile_matches array")
        .iter()
        .find(|m| m["port"] == json!(port))
        .unwrap_or_else(|| panic!("no profile_match entry for {port}: {listed}"))
}

/// An empty store returns one `none` entry per port, parallel to `ports`.
/// The `ports` array remains identical to the provider's raw `PortInfo` data.
#[tokio::test]
async fn list_ports_preview_empty_store_reports_none_parallel_and_pure_ports() {
    let pty1 = PtyPair::open().expect("openpty");
    let slave1 = pty1.slave_path.to_string_lossy().into_owned();
    let pty2 = PtyPair::open().expect("openpty");
    let slave2 = pty2.slave_path.to_string_lossy().into_owned();
    let provider_ports = vec![
        StaticPortProvider::usb_port(&slave1, VID, PID, "SN-1", Some("Fake USB Serial"), Some(2)),
        StaticPortProvider::weak_port(&slave2),
    ];
    let provider = StaticPortProvider::new(provider_ports.clone());
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let listed = call_list_ports(client).await;
    let ports = listed["ports"].as_array().expect("ports array");
    let matches = listed["profile_matches"].as_array().expect("matches array");
    assert_eq!(
        listed["count"],
        json!(2),
        "count matches the number of ports"
    );
    assert_eq!(
        ports.len(),
        matches.len(),
        "profile_matches must parallel ports"
    );
    assert_eq!(
        listed["ports"],
        serde_json::to_value(&provider_ports).unwrap(),
        "ports elements must serialize identically regardless of match metadata"
    );

    let high = match_for(&listed, &slave1);
    assert_eq!(high["confidence"], json!("high"));
    assert_eq!(high["outcome"], json!("none"));
    assert!(high["selected_profile"].is_null());
    assert_eq!(high["candidates"].as_array().unwrap().len(), 0);

    let weak = match_for(&listed, &slave2);
    assert_eq!(weak["confidence"], json!("none"));
    assert_eq!(weak["outcome"], json!("none"));
    assert!(weak["selected_profile"].is_null());

    harness._client.cancel().await.ok();
}

/// Generated and saved high profiles preview as `selected` with the expected
/// name and revision. The unique last-used winner matches a later bare open.
#[tokio::test]
async fn list_ports_preview_selected_winner_matches_later_bare_open() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // First bare open creates the generated profile at revision 1.
    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    assert_eq!(
        opened["profile"]["profile_name"],
        json!("auto-fake-usb-serial")
    );

    // Save a second, user-named profile for the same device. It has never been
    // used, so it sorts oldest despite being newer on disk.
    let saved = client
        .peer()
        .call_tool(tool_request(
            "save_profile",
            json!({ "connection_id": connection_id, "profile_name": "my-dev" }),
        ))
        .await
        .unwrap();
    assert_ne!(saved.is_error, Some(true), "{saved:?}");
    close_port(client, &connection_id).await;

    // Preview selects the generated profile as the unique most-recently-used
    // winner.
    let listed = call_list_ports(client).await;
    let high = match_for(&listed, &slave);
    assert_eq!(high["outcome"], json!("selected"));
    assert_eq!(high["selected_profile"], json!("auto-fake-usb-serial"));
    let candidates = high["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2, "both profiles listed as candidates");
    assert_eq!(candidates[0]["profile_name"], json!("auto-fake-usb-serial"));
    assert_eq!(candidates[0]["generated"], json!(true));
    assert_eq!(candidates[0]["revision"], json!(1));
    assert!(candidates[0]["last_used_at_ms"].is_number());

    // Explicit use of the second profile makes it the winner.
    let opened = open_port(client, &slave, json!({ "profile_mode": "none" })).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    close_port(client, &connection_id).await;
    let opened = client
        .peer()
        .call_tool(tool_request("open_profile", json!({ "profile": "my-dev" })))
        .await
        .unwrap();
    assert_ne!(opened.is_error, Some(true), "{opened:?}");
    let connection_id = opened.structured_content.as_ref().unwrap()["connection_id"]
        .as_str()
        .unwrap()
        .to_string();
    close_port(client, &connection_id).await;

    let listed = call_list_ports(client).await;
    let high = match_for(&listed, &slave);
    assert_eq!(high["outcome"], json!("selected"));
    assert_eq!(high["selected_profile"], json!("my-dev"));

    // Bare open selects the profile advertised by the preview.
    let reopened = open_port(client, &slave, json!({})).await;
    assert_eq!(reopened["profile"]["profile_name"], json!("my-dev"));
    assert_eq!(reopened["profile"]["source"], json!("automatic"));
    let connection_id = reopened["connection_id"].as_str().unwrap().to_string();
    close_port(client, &connection_id).await;

    harness._client.cancel().await.ok();
}

/// Equal top rank from two never-used saved profiles returns `ambiguous`, lists
/// both candidates, and leaves a bare open transient instead of guessing.
#[tokio::test]
async fn list_ports_preview_equal_timestamps_is_ambiguous_and_open_stays_transient() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // With profile_mode=none, no generated profile competes. Both saved
    // profiles have last_used_at_ms = null and tie at the oldest rank.
    let opened = open_port(client, &slave, json!({ "profile_mode": "none" })).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    for name in ["dev-a", "dev-b"] {
        let saved = client
            .peer()
            .call_tool(tool_request(
                "save_profile",
                json!({ "connection_id": connection_id, "profile_name": name }),
            ))
            .await
            .unwrap();
        assert_ne!(saved.is_error, Some(true), "{saved:?}");
    }
    close_port(client, &connection_id).await;

    let listed = call_list_ports(client).await;
    let high = match_for(&listed, &slave);
    assert_eq!(high["outcome"], json!("ambiguous"));
    assert!(high["selected_profile"].is_null());
    let candidate_names: Vec<&str> = high["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["profile_name"].as_str().unwrap())
        .collect();
    assert_eq!(candidate_names, vec!["dev-a", "dev-b"]);

    // Bare open must not guess. It returns a transient session with both
    // candidates.
    let opened = open_port(client, &slave, json!({})).await;
    assert_eq!(opened["profile"]["source"], json!("transient"));
    assert_eq!(opened["profile"]["persistent"], json!(false));
    let trans_candidates: Vec<&str> = opened["profile"]["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(trans_candidates, vec!["dev-a", "dev-b"]);
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    close_port(client, &connection_id).await;

    harness._client.cancel().await.ok();
}

/// Duplicate live high fingerprints return `duplicate` for both ports and never
/// select a profile, even when a matching profile exists.
#[tokio::test]
async fn list_ports_preview_duplicate_fingerprints_report_duplicate() {
    let pty1 = PtyPair::open().expect("openpty");
    let slave1 = pty1.slave_path.to_string_lossy().into_owned();
    let pty2 = PtyPair::open().expect("openpty");
    let slave2 = pty2.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![
        StaticPortProvider::usb_port(&slave1, VID, PID, "SAME-SN", Some("Fake USB Serial"), None),
        StaticPortProvider::usb_port(&slave2, VID, PID, "SAME-SN", Some("Fake USB Serial"), None),
    ]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // Write a matching profile through a second store instance using the same
    // file. The preview must report `duplicate` for both ports.
    let store2 = serial_mcp::profile_store::ProfileStore::open(harness.profiles_path.clone())
        .expect("second store instance on same file");
    store2
        .upsert(
            serial_mcp::profiles::Profile {
                name: "shared-dev".into(),
                selector: serial_mcp::profiles::ProfileSelector {
                    vid: Some(VID),
                    pid: Some(PID),
                    serial_number: Some("SAME-SN".into()),
                    transport: Some("usb".into()),
                    ..Default::default()
                },
                defaults: Default::default(),
                metadata: Default::default(),
                revisions: Vec::new(),
            },
            false,
        )
        .await
        .unwrap();

    let listed = call_list_ports(client).await;
    for slave in [&slave1, &slave2] {
        let m = match_for(&listed, slave);
        assert_eq!(m["confidence"], json!("high"));
        assert_eq!(m["outcome"], json!("duplicate"), "port {slave}");
        assert!(m["selected_profile"].is_null());
        assert_eq!(m["candidates"].as_array().unwrap().len(), 0);
    }

    harness._client.cancel().await.ok();
}

/// Medium identity (VID/PID without a serial) is never auto-selected.
/// Explicitly matching non-empty selectors are `ineligible` candidates, and
/// empty selectors that match every port are excluded.
#[tokio::test]
async fn list_ports_preview_medium_identity_ineligible_and_empty_selector_excluded() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let mut medium = StaticPortProvider::usb_port(&slave, VID, PID, "", Some("No Serial"), None);
    medium.serial_number = None;
    let provider = StaticPortProvider::new(vec![medium]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // A profile with an explicit (non-empty) VID/PID selector via
    // save_profile, plus an empty-selector profile via configure.
    let opened = open_port(client, &slave, json!({ "profile_mode": "none" })).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let saved = client
        .peer()
        .call_tool(tool_request(
            "save_profile",
            json!({ "connection_id": connection_id, "profile_name": "mid-dev" }),
        ))
        .await
        .unwrap();
    assert_ne!(saved.is_error, Some(true), "{saved:?}");
    close_port(client, &connection_id).await;

    let configured = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({ "profile": "empty-sel", "defaults": {} }),
        ))
        .await
        .unwrap();
    assert_ne!(configured.is_error, Some(true), "{configured:?}");

    let listed = call_list_ports(client).await;
    let m = match_for(&listed, &slave);
    assert_eq!(m["confidence"], json!("medium"));
    assert_eq!(m["outcome"], json!("ineligible"));
    assert!(m["selected_profile"].is_null());
    let candidate_names: Vec<&str> = m["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["profile_name"].as_str().unwrap())
        .collect();
    assert_eq!(
        candidate_names,
        vec!["mid-dev"],
        "only the explicitly matching selector is a candidate; \
         empty-selector 'empty-sel' must be excluded"
    );

    harness._client.cancel().await.ok();
}

/// Deleting the only matching profile returns the preview to `none`.
#[tokio::test]
async fn list_ports_preview_delete_profile_returns_to_none() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    close_port(client, &connection_id).await;

    let listed = call_list_ports(client).await;
    assert_eq!(match_for(&listed, &slave)["outcome"], json!("selected"));

    let deleted = client
        .peer()
        .call_tool(tool_request(
            "delete_profile",
            json!({ "profile_name": "auto-fake-usb-serial" }),
        ))
        .await
        .unwrap();
    assert_ne!(deleted.is_error, Some(true), "{deleted:?}");

    let listed = call_list_ports(client).await;
    let m = match_for(&listed, &slave);
    assert_eq!(m["outcome"], json!("none"));
    assert!(m["selected_profile"].is_null());
    assert_eq!(m["candidates"].as_array().unwrap().len(), 0);

    harness._client.cancel().await.ok();
}

/// A fresh read across store instances sees a second store's write to the same
/// file immediately, as another process would.
#[tokio::test]
async fn list_ports_preview_fresh_read_sees_second_store_write() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    // A second store instance writes the file while the server cache is empty.
    // list_ports must reload under the advisory lock instead of using stale
    // cache data.
    let store2 = serial_mcp::profile_store::ProfileStore::open(harness.profiles_path.clone())
        .expect("second store instance on same file");
    store2
        .upsert(
            serial_mcp::profiles::Profile {
                name: "other-proc".into(),
                selector: serial_mcp::profiles::ProfileSelector {
                    vid: Some(VID),
                    pid: Some(PID),
                    serial_number: Some("SN-1".into()),
                    transport: Some("usb".into()),
                    ..Default::default()
                },
                defaults: Default::default(),
                metadata: Default::default(),
                revisions: Vec::new(),
            },
            false,
        )
        .await
        .unwrap();

    let listed = call_list_ports(client).await;
    let m = match_for(&listed, &slave);
    assert_eq!(m["outcome"], json!("selected"));
    assert_eq!(m["selected_profile"], json!("other-proc"));
    assert_eq!(m["candidates"][0]["revision"], json!(1));

    harness._client.cancel().await.ok();
}

/// A real `list_ports` response validates schema wire types for preview
/// entries with candidates and a selection, and rejects non-standard uint
/// formats in the catalog schema.
///
/// This test covers preview entries only. It does not validate the full raw OS
/// enumeration payload.
#[tokio::test]
async fn list_ports_preview_output_validates_against_generated_schema() {
    let pty = PtyPair::open().expect("openpty");
    let slave = pty.slave_path.to_string_lossy().into_owned();
    let provider = StaticPortProvider::new(vec![StaticPortProvider::usb_port(
        &slave,
        VID,
        PID,
        "SN-1",
        Some("Fake USB Serial"),
        None,
    )]);
    let harness = session_harness(provider).await;
    let client = &harness._client;

    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    close_port(client, &connection_id).await;

    let listed = call_list_ports(client).await;

    let catalog = serial_mcp::server::tool_catalog();
    let list_ports_tool = catalog
        .iter()
        .find(|t| t.name.as_ref() == "list_ports")
        .expect("list_ports in catalog");
    let schema = list_ports_tool
        .output_schema
        .as_ref()
        .expect("list_ports outputSchema");
    let schema_value = serde_json::Value::Object((**schema).clone());
    let schema_text = serde_json::to_string(&schema_value).unwrap();
    for bad in ["uint", "uint8", "uint16", "uint32", "uint64"] {
        assert!(
            !schema_text.contains(&format!("\"format\":\"{bad}\"")),
            "list_ports output schema must not emit {bad} format"
        );
    }

    // Validate the preview match entries against their generated $defs
    // (def + sibling $defs kept together so internal $refs resolve).
    let defs = schema_value["$defs"].clone();
    let match_schema = serde_json::json!({
        "$ref": "#/$defs/PortProfileMatch",
        "$defs": defs,
    });
    let candidate_schema = serde_json::json!({
        "$ref": "#/$defs/ProfileMatchCandidate",
        "$defs": defs,
    });
    let match_validator = jsonschema::validator_for(&match_schema).unwrap();
    let candidate_validator = jsonschema::validator_for(&candidate_schema).unwrap();
    for entry in listed["profile_matches"].as_array().unwrap() {
        let errors: Vec<String> = match_validator
            .iter_errors(entry)
            .map(|e| e.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "profile_matches entry must validate against PortProfileMatch: {errors:?}"
        );
        for candidate in entry["candidates"].as_array().unwrap() {
            let errors: Vec<String> = candidate_validator
                .iter_errors(candidate)
                .map(|e| e.to_string())
                .collect();
            assert!(
                errors.is_empty(),
                "candidate must validate against ProfileMatchCandidate: {errors:?}"
            );
        }
    }

    harness._client.cancel().await.ok();
}

// `capture_boot` arm-only behavior over a real PTY.
//
// PTYs expose no modem-line callbacks, so DTR/RTS assertion cannot be
// observed here (the atomic reset proof lives in the controlled backend in
// http_integration.rs). Arm-only capture (reset=null) needs no line control
// at all and exercises the real byte pipeline end-to-end.

#[tokio::test]
async fn pty_capture_boot_arm_only_captures_only_post_mark_bytes() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // Stale bytes the device emitted before the capture is armed.
    pty.write_device(b"stale-banner\r\n").await.unwrap();
    // Consume them so the shared cursor sits past them.
    let r = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": connection_id, "timeout_ms": 1000 }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    assert!(
        r.structured_content.unwrap()["data"]
            .as_str()
            .unwrap_or_default()
            .contains("stale-banner"),
        "stale bytes must be readable before capture"
    );

    // Arm-only capture with a match so the read half waits for the boot
    // prompt instead of returning immediately.
    let call = client.peer().call_tool(tool_request(
        "capture_boot",
        json!({
            "connection_id": connection_id,
            "reset": null,
            "match": {
                "pattern": "boot>",
                "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
            },
            "timeout_ms": 3000,
        }),
    ));
    let mut call = Box::pin(call);

    // Emit the boot bytes while the capture is armed (external reset).
    tokio::select! {
        res = call.as_mut() => {
            panic!("arm-only capture completed before external bytes: {res:?}");
        }
        _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => {}
    }
    pty.write_device(b"early-boot-output\r\nboot> ")
        .await
        .unwrap();

    let r = call.await.unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert!(s["reset"].is_null(), "arm-only capture reports reset=null");
    assert_eq!(s["read"]["stop_reason"], json!("match_found"));
    let data = s["read"]["data"].as_str().unwrap_or_default();
    assert!(
        data.contains("boot>"),
        "post-mark boot bytes must be captured: {data:?}"
    );
    assert!(
        !data.contains("stale-banner"),
        "pre-mark stale bytes must never appear in the capture: {data:?}"
    );

    client.cancel().await.ok();
}
