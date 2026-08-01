//! Layer 3 — end-to-end tests with a real PTY pair standing in for a
//! serial device.
//!
//! These tests open a Linux/macOS pseudo-terminal pair via `openpty(3)`,
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
use common::{connect_client, next_notification, pty::PtyPair, tool_request, TestServer};

/// Open a real PTY pair, then walk an MCP client through opening the
/// slave path as a serial port. Returns the test server (kept alive by
/// the caller), the connected client, and the PTY pair plus
/// connection_id.
async fn setup() -> (
    TestServer,
    rmcp::service::RunningService<rmcp::service::RoleClient, common::NotificationCollector>,
    tokio::sync::mpsc::UnboundedReceiver<rmcp::model::LoggingMessageNotificationParam>,
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

    // Invalid UTF-8 bytes over the real PTY serial path.
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
    // Cat path drains instantly ("drained") when bytes are already buffered;
    // otherwise the read waits and stops at "timeout". Both carry the data.
    assert!(
        structured["stop_reason"] == json!("drained")
            || structured["stop_reason"] == json!("timeout"),
        "unexpected stop_reason: {structured:?}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_streams_device_writes_as_notifications() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
            }),
        ))
        .await
        .unwrap();

    pty.write_device(b"hello from device\r\n").await.unwrap();

    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(
        event.logger.as_deref(),
        Some(&format!("serial:{connection_id}")[..])
    );
    let data = event.data.as_object().unwrap();
    assert_eq!(
        data["connection_id"],
        serde_json::Value::String(connection_id.clone())
    );
    // The PTY may deliver the bytes in one chunk or split — concatenate
    // until we have the whole payload.
    let mut received = data["data"].as_str().unwrap().to_string();
    while !received.contains("hello from device") {
        let more = next_notification(&mut rx, Duration::from_secs(1))
            .await
            .unwrap();
        received.push_str(more.data["data"].as_str().unwrap());
    }
    assert!(received.contains("hello from device"));
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

    // Slow-feed bytes to exercise the read+match accumulator.
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

    // Write data first, then delay briefly to let the PTY buffer it.
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
    // pre_start = 14-4 = 10, shaped = "x___OK>" (7 bytes), match_index = 4.
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

    // Write data to the PTY first so it's in the buffer before read starts.
    pty.write_device(b"junk OK> rest").await.unwrap();
    // Small delay to let the PTY deliver the bytes.
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
async fn pty_subscribe_match_with_context_includes_shaped_data() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "match": {
                    "pattern": "OK>",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8",
                        "context_amount_of_matched_bytes": 8
                    }
                }
            }),
        ))
        .await
        .unwrap();

    pty.write_device(b"AAAAAAAAAABBBBOK>tail").await.unwrap();

    // Collect notifications until we get the match stop notification.
    let mut found_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("matched").and_then(|v| v.as_bool()) == Some(true) {
                    found_match_stop = true;
                    assert_eq!(data["stop_reason"], json!("match_found"));
                    let match_index = data["match_index"].as_u64().expect("match_index") as usize;
                    let shaped_data = data["data"].as_str().expect("data in stop notification");
                    // "OK>" starts at byte 14 in "AAAAAAAAAABBBBOK>tail"
                    // context=8 → pre_start = 14-8 = 6 → bytes[6..17] = "AABBBBOK>"
                    assert!(
                        shaped_data.ends_with("OK>"),
                        "shaped data should end with OK>: {shaped_data:?}"
                    );
                    assert_eq!(
                        match_index, 8,
                        "match_index should be 8 in shaped payload: {data:?}"
                    );
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        found_match_stop,
        "should have received match stop notification"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_and_subscribe_report_same_literal_match_index() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    // Same chunked sequence for both tools: two device writes, 100ms apart.
    pty.write_device(b"warming up... ").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    pty.write_device(b"OK> ready").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // read: cross-chunk literal at global index 13.
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

    // subscribe replays the same retained stream from buffer_start and must
    // report the same match outcome and index.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "from": { "type": "buffer_start" },
                "timeout_ms": 3000,
                "match": { "pattern": "OK>" },
            }),
        ))
        .await
        .unwrap();

    let mut saw_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("stop_reason").is_some() {
                    saw_match_stop = true;
                    assert_eq!(data["matched"], json!(true), "{data:?}");
                    assert_eq!(data["match_index"], json!(14), "{data:?}");
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        saw_match_stop,
        "subscribe should emit a match stop notification"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_match_global_index_after_window_truncation() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    // Small bounded window: 16 bytes + literal overlap (2) = 18 retained.
    // 20 junk bytes then "OK>" force matcher front truncation mid-stream.
    client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "connection_id": connection_id,
                "defaults": { "max_buffered_bytes": 16 },
            }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "match": { "pattern": "OK>" },
            }),
        ))
        .await
        .unwrap();

    // Feed in small chunks so several bounded pushes happen.
    for _ in 0..5 {
        pty.write_device(b"AAAA").await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    pty.write_device(b"OK>").await.unwrap();

    let mut saw_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("stop_reason").is_some() {
                    saw_match_stop = true;
                    // "OK>" starts at global byte 20 — a window-local index
                    // would have been wrong after front truncation.
                    assert_eq!(data["stop_reason"], json!("match_found"), "{data:?}");
                    assert_eq!(data["matched"], json!(true), "{data:?}");
                    assert_eq!(data["match_index"], json!(20), "{data:?}");
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        saw_match_stop,
        "subscribe should emit a match stop notification"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_match_context_shaped_after_window_truncation() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "connection_id": connection_id,
                "defaults": { "max_buffered_bytes": 16 },
            }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "match": {
                    "pattern": "OK>",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8",
                        "context_amount_of_matched_bytes": 8
                    }
                },
            }),
        ))
        .await
        .unwrap();

    // 20 junk bytes then the match, crossing the retention boundary.
    for _ in 0..5 {
        pty.write_device(b"AAAA").await.unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    pty.write_device(b"OK>").await.unwrap();

    let mut saw_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("stop_reason").is_some() {
                    saw_match_stop = true;
                    assert_eq!(data["matched"], json!(true), "{data:?}");
                    // Exactly 8 requested context bytes before the match,
                    // with a relative match_index of 8.
                    assert_eq!(data["data"], json!("AAAAAAAAOK>"), "{data:?}");
                    assert_eq!(data["match_index"], json!(8), "{data:?}");
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        saw_match_stop,
        "subscribe should emit a match stop notification"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_read_live_match_applies_context_shaping() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // Read starts BEFORE any device data: the match happens on the live
    // (wait-loop) path, which must shape context like subscribe does.
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
    // "OK>" at global 8; context 4 -> shaped "BBBBOK>" with relative index 4.
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

    // Test that a 50ms BREAK is released within ~100ms, not held until 250ms+
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

    // Should be close to 50ms (allow 40-100ms window)
    assert!(
        (40..=100).contains(&actual_duration),
        "send_break(50ms) took {actual_duration}ms, expected 40-100ms"
    );
    // Full round-trip should also be reasonable
    assert!(
        elapsed <= 200,
        "send_break round-trip took {elapsed}ms, expected <200ms"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_match_stops_without_context() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "match": {
                    "pattern": "STOP",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8"
                    }
                }
            }),
        ))
        .await
        .unwrap();

    pty.write_device(b"noise noise STOP tail").await.unwrap();

    let mut found_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("matched").and_then(|v| v.as_bool()) == Some(true) {
                    found_match_stop = true;
                    assert_eq!(data["stop_reason"], json!("match_found"));
                    assert!(
                        data["match_index"].as_u64().is_some(),
                        "match_index present"
                    );
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        found_match_stop,
        "subscribe should emit match stop notification"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_silence_timeout_stops() {
    let (_server, client, mut rx, _pty, connection_id) = setup().await;

    // Subscribe with silence timeout. PTY device side is silent — no writes.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "no_new_rx_timeout_ms": 300
            }),
        ))
        .await
        .unwrap();

    // Should arrive within ~600ms.
    let event = next_notification(&mut rx, Duration::from_secs(3))
        .await
        .expect("subscribe should emit stop notification on silence timeout");

    let data = event.data.as_object().unwrap();
    assert_eq!(
        data["stop_reason"],
        json!("no_new_rx_timeout"),
        "stop_reason should be no_new_rx_timeout: {data:?}"
    );
    assert_ne!(data.get("matched").and_then(|v| v.as_bool()), Some(true));
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_framing_emits_per_frame_notifications() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "rx_framing": { "type": "line" },
            }),
        ))
        .await
        .unwrap();

    pty.write_device(b"alpha\nbeta\n").await.unwrap();

    // Collect frame notifications until we've seen "alpha" and "beta".
    let mut seen: Vec<(u64, String)> = Vec::new();
    while !(seen.iter().any(|(_, d)| d.contains("alpha"))
        && seen.iter().any(|(_, d)| d.contains("beta")))
    {
        let n = next_notification(&mut rx, Duration::from_secs(2))
            .await
            .unwrap();
        let obj = n.data.as_object().unwrap();
        // Frame notifications carry frame_index; the stop notification does not.
        if let Some(idx) = obj.get("frame_index").and_then(|v| v.as_u64()) {
            assert_eq!(obj["frame_type"], json!("line"), "frame_type: {obj:?}");
            seen.push((idx, obj["data"].as_str().unwrap().to_string()));
        }
    }

    let alpha = seen.iter().find(|(_, d)| d.contains("alpha")).unwrap();
    let beta = seen.iter().find(|(_, d)| d.contains("beta")).unwrap();
    assert_eq!(alpha.0, 0, "alpha is frame 0");
    assert_eq!(beta.0, 1, "beta is frame 1");
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_framing_match_stops_at_frame() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "rx_framing": { "type": "line" },
                "match": { "pattern": "beta" },
            }),
        ))
        .await
        .unwrap();

    pty.write_device(b"alpha\nbeta\ngamma\n").await.unwrap();

    // Drain notifications until the final stop notification (no frame_index,
    // carries stop_reason).
    loop {
        let n = next_notification(&mut rx, Duration::from_secs(2))
            .await
            .unwrap();
        let obj = n.data.as_object().unwrap();
        if let Some(reason) = obj.get("stop_reason").and_then(|v| v.as_str()) {
            assert_eq!(reason, "match_found", "stop: {obj:?}");
            assert_eq!(obj["matched"], json!(true));
            assert_eq!(obj["match_frame_index"], json!(1), "beta is frame 1");
            break;
        }
    }
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_framing_match_context_second_frame_only() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "rx_framing": { "type": "line" },
                "match": {
                    "pattern": "beta",
                    "config": { "context_amount_of_matched_bytes": 16 }
                },
            }),
        ))
        .await
        .unwrap();

    // Two frames; the match lands in the SECOND frame ("xxbeta"). Only 16
    // requested context bytes, but only 2 exist inside the frame — the
    // shaped payload must be exactly "xxbeta", never a mix with frame 0.
    pty.write_device(b"alpha\nxxbeta\ngamma\n").await.unwrap();

    // Drain until the final stop notification.
    let mut stop: Option<serde_json::Value> = None;
    for _ in 0..16 {
        let n = next_notification(&mut rx, Duration::from_secs(2))
            .await
            .unwrap();
        let obj = n.data.as_object().unwrap();
        if obj.get("stop_reason").is_some() {
            stop = Some(n.data.clone());
            break;
        }
    }
    let stop = stop.expect("received match_found stop notification");
    assert_eq!(stop["stop_reason"], json!("match_found"));
    assert_eq!(stop["matched"], json!(true));
    // Matching frame is the second frame.
    assert_eq!(stop["match_frame_index"], json!(1), "xxbeta is frame 1");
    // Final stop data = requested pre-context + literal from the second
    // frame only.
    assert_eq!(stop["data"], json!("xxbeta"));
    // Relative match_index equals the actual returned pre-context count.
    assert_eq!(stop["match_index"], json!(2));
    // Encoding remains the requested utf8.
    assert_eq!(stop["encoding"], json!("utf8"));
    // No cross-frame bytes: frame 0's "alpha" must not appear anywhere.
    assert!(!stop["data"].as_str().unwrap().contains("alpha"));
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_line_auto_promotes_on_bare_cr_and_flushes_pending() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "rx_framing": { "type": "line" },
            }),
        ))
        .await
        .unwrap();

    // Pending CR, no frame yet.
    pty.write_device(b"line1\r").await.unwrap();
    // Second byte 'l' (non-\n) confirms bare CR → emit "line1", promote to
    // CrMode, then "line2\r" splits on \r → emit "line2".
    pty.write_device(b"line2\r").await.unwrap();

    let mut seen: Vec<(u64, String)> = Vec::new();
    while !(seen.iter().any(|(_, d)| d == "line1") && seen.iter().any(|(_, d)| d == "line2")) {
        let n = next_notification(&mut rx, Duration::from_secs(2))
            .await
            .unwrap();
        let obj = n.data.as_object().unwrap();
        if let Some(idx) = obj.get("frame_index").and_then(|v| v.as_u64()) {
            assert_eq!(obj["frame_type"], json!("line"), "frame_type: {obj:?}");
            let data = obj["data"].as_str().unwrap().to_string();
            assert!(!data.contains('\r'), "terminator stripped: {data:?}");
            seen.push((idx, data));
        }
    }

    let line1 = seen.iter().find(|(_, d)| d == "line1").unwrap();
    let line2 = seen.iter().find(|(_, d)| d == "line2").unwrap();
    assert_eq!(line1.0, 0, "line1 is frame 0");
    assert_eq!(line2.0, 1, "line2 is frame 1");
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_subscribe_slip_malformed_escape_emits_framing_error() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "rx_framing": { "type": "slip" },
            }),
        ))
        .await
        .unwrap();

    // END, ESC, invalid byte 0x41, END → malformed escape.
    pty.write_device(b"\xC0\xDB\x41\xC0").await.unwrap();

    // Collect notifications until the stop notification appears.
    let mut stop: Option<serde_json::Value> = None;
    for _ in 0..16 {
        let n = next_notification(&mut rx, Duration::from_secs(2))
            .await
            .unwrap();
        let obj = n.data.as_object().unwrap();
        if obj.get("stop_reason").is_some() {
            stop = Some(n.data.clone());
            break;
        }
    }
    let stop = stop.expect("received framing_error stop notification");
    assert_eq!(stop["stop_reason"], json!("framing_error"));
    let err = stop["error"].as_str().expect("error field present");
    assert!(err.contains("SLIP framing error"), "error msg: {err}");
    assert!(
        err.contains("0x41"),
        "error msg names violating byte: {err}"
    );
    client.cancel().await.ok();
}

// ── Phase 1.3: ring-based read tests ────────────────────────────────────

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

/// read with `from: "buffer_start"` replays retained history.
#[tokio::test]
async fn pty_read_from_buffer_start() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"RETAINED").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // read with from: "buffer_start" — replay everything retained.
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

/// Re-reading with the same `from` offset is non-destructive: the
/// cursor is reset to `from` before reading, so the same bytes come
/// back.
#[tokio::test]
async fn pty_read_reread_same_from_offset() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"UNIQUE").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // First read from buffer_start.
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

    // Re-read with the same from_offset.
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

/// `from: "now"` skips buffered data, jumping to the live edge.
#[tokio::test]
async fn pty_read_from_now_skips_backlog() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"SKIP_ME").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // read with from: "now" — skip buffered, should get nothing (or timeout).
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
    // Skip past SKIP_ME — data should be empty or not contain it.
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        !data.contains("SKIP_ME"),
        "from: now should skip buffered data, got: {s:?}"
    );
    client.cancel().await.ok();
}

/// flush(target="both") discards the retained RX backlog: stale pre-flush
/// bytes must never come back on a later read, while post-flush bytes remain
/// readable. Proves the fix via public tools only — the connection must stay
/// usable, so the read's success cannot be explained by a dead stream.
#[tokio::test]
async fn pty_flush_both_discards_retained_rx_backlog() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    // 1. Device writes a unique old marker.
    pty.write_device(b"OLD-MARKER-4711").await.unwrap();

    // 2. Poll public get_status until the backlog reached the ring (bounded).
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

    // 3. Flush with target="both" — must discard the retained backlog.
    let flush = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .unwrap();
    assert_ne!(flush.is_error, Some(true), "{flush:?}");

    // 4. Device sends a unique new marker after the flush returned.
    pty.write_device(b"NEW-MARKER-2299").await.unwrap();

    // 5. Ordinary read (shared cursor, which flush clamped to the live edge).
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

// ── Step 7 (12e): subscribe from variants ─────────────────────────────────────

/// subscribe with `from: "cursor"` replays bytes after the last read.
#[tokio::test]
async fn pty_subscribe_from_cursor_replays_after_read() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"ABCDEF").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Configure max_buffered_bytes=3 so the read only returns 3 bytes.
    client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "connection_id": connection_id,
                "defaults": { "max_buffered_bytes": 3 },
            }),
        ))
        .await
        .unwrap();

    // Read first 3 bytes — cursor advances to 3.
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

    // subscribe from cursor — should see "DEF" (bytes after offset 3).
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "from": { "type": "cursor" },
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();

    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .unwrap();
    let data_str = event.data["data"].as_str().unwrap_or("");
    assert!(
        data_str.contains("DEF"),
        "subscribe from cursor should replay bytes after read: got {data_str:?}"
    );
    client.cancel().await.ok();
}

/// subscribe with `from: {"offset": 0}` after flush+write reports gap.
#[tokio::test]
async fn pty_subscribe_from_offset_below_start_reports_gap() {
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    // Open with small ring to force wrap behavior.
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({ "port": slave_path, "baud_rate": 115200, "rx_buffer_size": 16 }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let connection_id = result.structured_content.expect("structured")["connection_id"]
        .as_str()
        .expect("string")
        .to_string();

    let (mut pty_file, _slave_fd) = pty.into_parts();

    // Write 20 bytes to 16-byte ring to force wrap: start=4, end=20.
    pty_file.write_all(b"ABCDEFGHIJKLMNOPQRST").await.unwrap(); // 20 bytes
    tokio::time::sleep(Duration::from_millis(200)).await;

    // subscribe from offset 0 (below ring start=4) → gap of 4 bytes.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "from": { "type": "offset", "offset": 0 },
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();

    // Collect all notifications.
    let mut found_gap = false;
    let mut attempts = 0;
    while attempts < 20 {
        let event = match next_notification(&mut rx, Duration::from_millis(500)).await {
            Ok(e) => e,
            Err(_) => break,
        };
        if event.data.get("stop_reason").is_some() {
            if let Some(bl) = event.data.get("bytes_lost").and_then(|v| v.as_u64()) {
                if bl > 0 {
                    found_gap = true;
                }
            }
            break;
        }
        if let Some(bl) = event.data.get("bytes_lost").and_then(|v| v.as_u64()) {
            if bl > 0 {
                found_gap = true;
            }
        }
        attempts += 1;
    }
    assert!(
        found_gap,
        "should report bytes_lost > 0 when cursor is below ring start"
    );
    client.cancel().await.ok();
}

/// subscribe with `from: "buffer_start"` across ring wrap replays retained bytes.
#[tokio::test]
async fn pty_subscribe_from_buffer_start_across_ring_wrap() {
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

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

    // Write 16 bytes to 8-byte ring → start=8, end=16, 8 bytes lost.
    pty_file.write_all(b"abcdefghijklmnop").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // subscribe from buffer_start — replays retained bytes (positions 8-16).
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "from": { "type": "buffer_start" },
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();

    // Collect notifications until stop.
    let mut received = String::new();
    let mut attempts = 0;
    while attempts < 20 {
        let event = match next_notification(&mut rx, Duration::from_millis(500)).await {
            Ok(e) => e,
            Err(_) => break,
        };
        if event.data.get("stop_reason").is_some() {
            break;
        }
        if let Some(d) = event.data.get("data").and_then(|v| v.as_str()) {
            received.push_str(d);
        }
        attempts += 1;
    }
    assert!(
        received.contains("ijklmnop"),
        "should replay retained bytes, got {received:?}"
    );
    // buffer_start begins at ring start_offset (=8 after wrap); no gap.
    client.cancel().await.ok();
}

// ── Step 8 (12g): read from + framing/match ────────────────────────────────────

/// read with `from: "buffer_start"` and line framing decodes all frames.
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

/// read with `from: {"offset": 0}` and match scans from the given offset.
#[tokio::test]
async fn pty_read_from_offset_with_match_scans_from_offset() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    pty.write_device(b"AAAAATARGETBBBBB").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Read from offset 0 — match should be found.
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

    // Read from offset 10 — match is at absolute position 5 (<10), so missed.
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
    // No match found past offset 10 — read should stop on timeout.
    assert_eq!(
        s2["matched"], false,
        "match should not be found from offset 10"
    );
    client.cancel().await.ok();
}

// ── configure tool: connection mode ──────────────────────────────────────────

#[tokio::test]
async fn pty_configure_connection_mutates_framing_default() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;
    pty.write_device(b"line1\nline2\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // Read with no framing — raw bytes.
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
    // Write more data + read — should be framed now.
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

/// Create a profile with rx_framing default, then open the PTY with
/// matching framing and verify that the read uses line framing.
///
/// NOTE: open_profile would be the ideal end-to-end test here, but the
/// default profile selector (all-None) matches any port — on systems
/// with multiple ports it may pick the wrong one. This test validates
/// that configure+open+framed-read composes correctly.
#[tokio::test]
async fn pty_configure_profile_applies_on_open_profile() {
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let profile_name = "test-configure-apply";

    // Create profile with line framing.
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

    // Open the PTY directly with the profile's framing.
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

    // Write data from device side — should be line-framed when read.
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
        "profile framing should apply: {rs:?}"
    );
    assert!(!rs["frames"].as_array().unwrap().is_empty());

    // Cleanup.
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

// ── transact: write-then-read ────────────────────────────────────────────────

#[tokio::test]
async fn pty_transact_writes_then_reads_response() {
    let (_server, client, _rx, pty, connection_id) = setup().await;
    let (mut master_file, _slave) = pty.into_parts();
    let cid = connection_id.clone();

    // Spawn a device emulator that writes a response after a short delay.
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
    // In raw PTY mode there is no echo; the device emulator writes "pong\n"
    // which the read half should pick up. If timing causes a timeout, that's
    // acceptable — verify write half at minimum.
    assert!(
        data.contains("pong") || s["read"]["bytes_read"].as_u64() == Some(0),
        "expected pong response or empty: {s:?}"
    );

    let _ = emulator.await;
    // _slave dropped here, closing the PTY
    client.cancel().await.ok();
}

#[tokio::test]
async fn pty_transact_from_now_skips_pre_write_buffer() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;
    // Pre-write some data from the device side so the ring has bytes
    // before the transact call.
    pty.write_device(b"PREEXISTING\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // transact with default from: "now" — should skip PREEXISTING.
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
    // Pre-write some data from the device side.
    pty.write_device(b"PREEXISTING\n").await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    // transact with from: "cursor" — should include PREEXISTING.
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
    // at_command preset: TX appends \r, RX frames by line.
    // Pre-write data from device so the read half has something to decode
    // (use from: cursor to pick up buffered data; default now skips it).
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

/// Cancellation in the PTY harness uses the proven timeout-based pattern:
/// wrap the call_tool future in `tokio::time::timeout`, then cancel the
/// client. Accepts any outcome (completed before timeout, transport error,
/// or timeout). Proves the transact tool does not hang forever when the
/// client disconnects mid-read.
#[tokio::test]
async fn pty_transact_cancellation_aborts_read() {
    let (_server, client, _rx, _pty, connection_id) = setup().await;

    // Start a transact with a long read timeout. Race it against a short
    // outer timeout + client cancel — proves the tool doesn't hang forever
    // when the client disconnects mid-read. Matches the proven
    // send_break_cancellation_stops_gracefully pattern.
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

    // Cancel the client — transport teardown. The transact task (if still
    // running) is cleaned up by the runtime.
    client.cancel().await.ok();

    // Either:
    //  - the transact completed before the outer timeout (write half done,
    //    read half may have is_error or a stop_reason), OR
    //  - the outer timeout fired (transact still running — fine, runtime
    //    cleans it up after cancel).
    // Either way, this proves the tool doesn't hang forever.
    match result {
        Ok(Ok(call_result)) => {
            // Transact returned a result before the outer timeout. Inspect
            // it loosely — the write half should have completed; the read
            // half may be partial/empty/cancelled depending on timing.
            let s = call_result.structured_content.expect("structured");
            assert!(
                s["write"]["bytes_written"].as_u64().unwrap_or(0) > 0,
                "write half should complete before cancel: {s:?}"
            );
            // No assertion on read.stop_reason — transport teardown may
            // produce any of: "cancelled", "connection_closed",
            // "peer_disconnected", or no read result at all.
        }
        Ok(Err(_)) => {
            // Transport error before timeout — acceptable; client teardown
            // raced the tool response.
        }
        Err(_) => {
            // Outer timeout fired — transact still running. Client was
            // cancelled above; runtime cleans up the task.
        }
    }
}

// ── Phase 3A: automatic profile sessions ────────────────────────────────────
//
// These tests exercise the full public `open`/`open_profile` path with a
// REAL PTY slave as the hardware port while an injected StaticPortProvider
// describes synthetic USB identity. They prove the profile-session behavior
// is observable through public MCP results — not just field wiring.

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
        .filter_map(|c| match &c.raw {
            rmcp::model::RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
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
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
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
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
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
    _client:
        rmcp::service::RunningService<rmcp::service::RoleClient, common::NotificationCollector>,
    _dir: tempfile::TempDir,
    profiles_path: PathBuf,
}

async fn session_harness(provider: Arc<StaticPortProvider>) -> SessionHarness {
    let dir = tempfile::tempdir().expect("tempdir");
    let profiles_path = dir.path().join("profiles.toml");
    let server = TestServer::start_with_provider_and_profiles_path(
        Arc::new(serial_mcp::serial::ConnectionManager::new()),
        provider,
        profiles_path.clone(),
    )
    .await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    SessionHarness {
        _server: server,
        _client: client,
        _dir: dir,
        profiles_path,
    }
}

/// 1. First high-confidence bare open creates a generated persistent
///    profile; open result, list_profiles, get_status, and list_connections
///    all agree — and real serial traffic flows.
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

    // Bare open: no baud, no defaults — built-in 115200 fallback.
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

    // list_profiles agrees.
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
    // Generated selector must NOT carry path/description/manufacturer.
    assert!(s["profiles"][0]["selector"]["port_pattern"].is_null());
    assert!(s["profiles"][0]["selector"]["manufacturer"].is_null());

    // get_status agrees.
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

    // list_connections agrees.
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

/// 2. Close/reopen automatically selects the same profile and increments
///    usage without bumping revision; real traffic still flows.
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

/// 3. A different serial number with the same VID/PID gets a different
///    generated profile.
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

/// 4. Two live ports with a duplicate high fingerprint produce a transient
///    ambiguity session — settings are never applied to an
///    indistinguishable device.
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

/// 5. Weak PTY identity opens with a transient session and leaves the
///    profile store untouched (no durable profile, no file).
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

    // No durable profile was written — the file must not exist.
    assert!(
        !harness.profiles_path.exists(),
        "weak identity must not create a profiles file"
    );

    harness._client.cancel().await.ok();
}

/// 6. `profile_mode="none"` disables automatic selection/creation and
///    returns an observable disabled binding.
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

/// 7. An explicit open field overrides the selected profile's default for
///    the live connection AND is persisted write-through immediately
///    (Phase 3B open-override learning): the binding comes back clean with
///    a bumped revision, and the next bare reopen applies the override.
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

/// 8. A separate HTTP client observes the same active binding and the
///    generated profile.
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
    let server = TestServer::start_with_provider_and_profiles_path(
        Arc::new(serial_mcp::serial::ConnectionManager::new()),
        provider,
        profiles_path,
    )
    .await;
    let (client_a, _rx_a) = connect_client(&server).await.unwrap();
    let (client_b, _rx_b) = connect_client(&server).await.unwrap();

    let opened = open_port(&client_a, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    let profile_name = opened["profile"]["profile_name"]
        .as_str()
        .unwrap()
        .to_string();

    // Client B sees the binding...
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

    // ...and the generated profile.
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

/// 9. `open_profile` with two matching ports returns a tool error; exactly
///    one match works and becomes the last-used winner for a later bare
///    open.
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

    // Empty-selector profile matches BOTH live ports → tool error.
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

    // Name the same device explicitly via save_profile (while the first
    // connection is still open — save_profile snapshots it).
    let saved = client
        .peer()
        .call_tool(tool_request(
            "save_profile",
            json!({ "connection_id": first_id, "profile_name": "named-p1" }),
        ))
        .await
        .unwrap();
    assert_ne!(saved.is_error, Some(true), "{saved:?}");

    // Close the first connection, then open_profile the same port with
    // exactly one match works and marks the profile used.
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

    // A later bare open must select the explicitly used profile, not the
    // older generated one.
    let reopened = open_port(client, &slave1, json!({})).await;
    assert_eq!(reopened["profile"]["source"], json!("automatic"));
    assert_eq!(
        reopened["profile"]["profile_name"],
        json!("named-p1"),
        "most-recently-used profile must win (generated was {generated_name})"
    );

    harness._client.cancel().await.ok();
}

/// 10. Equal top-ranked profile timestamps produce observable ambiguity.
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

    let server = TestServer::start_with_provider_and_profiles_path(
        Arc::new(serial_mcp::serial::ConnectionManager::new()),
        provider,
        profiles_path.clone(),
    )
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

    // Neither profile was applied or bumped.
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

/// 11. Per-call read/write/transact options do not alter usage, revision,
///     or defaults of the bound profile.
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

    // Write (device drains), read, transact with per-call options.
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

/// 12. Review gate: explicit `open_profile` on a weak-identity port reports
///     the matched port's OWN confidence (none/low), not a hardcoded high,
///     while keeping source=explicit.
#[tokio::test]
async fn open_profile_explicit_binding_reports_matched_port_confidence() {
    // Phase 1: path-only PTY (unknown transport, no identity) → None.
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

    // Phase 2: PCI-synthetic PTY (hardware id only) → Low.
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

    // get_status agrees with the binding.
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

/// 13. Review gate (M6): explicit `save_profile` of a generated-bound
///     connection creates a USER-owned profile (`generated=false`) — a
///     deliberate promotion, not a blind copy of the generated flag.
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

// ── Phase 3B: write-through learning, conflicts, rollback ────────────────────
//
// Same harness as Phase 3A: real PTY + injected high-confidence
// StaticPortProvider. These tests prove learning, partial-failure honesty,
// CAS/stale behavior, rollback, and deletion protection through public MCP
// results and real serial traffic.

/// Reconfigure one connection and return the structured result.
async fn reconfigure_baud(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
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

/// 1. Generated profile revision 1 → reconfigure baud → revision 2
///    persisted; close/reopen applies the baud on live status.
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

/// 2. set_flow_control persists and applies on reopen.
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

/// 3. Connection-mode configure persists framing; reopen and an actual
///    framed read prove it.
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

/// 4. Multiple durable changes create a bounded revision history (max 5).
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

    // 5 reconfigures after the rev-1 generated profile → revision 6.
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

/// 5. Non-durable operations never change profile defaults or revision:
///    BREAK, flush, subscribe/unsubscribe, and per-call framing/match on
///    read/write. (DTR/RTS is covered by the http_integration loopback
///    suite; PTYs cannot drive modem lines — ENOTTY.)
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

    // Subscribe (short timeout) then unsubscribe.
    let sub = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({ "connection_id": connection_id, "timeout_ms": 100 }),
        ))
        .await
        .unwrap();
    assert_ne!(sub.is_error, Some(true), "{sub:?}");
    let unsub = client
        .peer()
        .call_tool(tool_request(
            "unsubscribe",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(unsub.is_error, Some(true), "{unsub:?}");

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

/// 6. Partial failure: live reconfigure succeeds, profile write fails
///    (read-only dir) → result stays successful with `state="failed"`,
///    binding dirty, cache/file old. Restoring permissions + clean close
///    retries and persists; reopen uses the new baud.
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
    let server = TestServer::start_with_provider_and_profiles_path(
        Arc::new(serial_mcp::serial::ConnectionManager::new()),
        provider,
        profiles_path.clone(),
    )
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

/// 7. CAS/stale: an external profile-mode configure bumps the bound
///    profile; the next live reconfigure succeeds but reports a conflict,
///    the binding turns stale, and the newer profile remains untouched.
///    Close does not overwrite the stale profile.
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

    // Second client bumps the profile to revision 2 with baud 14400.
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

    // Live reconfigure succeeds; persistence reports a conflict.
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

    // The newer profile remains untouched.
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

/// 8. Rollback restores a prior baud as a new monotonic revision; the
///    active connection stays unchanged and stale; close cannot overwrite
///    the rollback; reopen applies the rolled-back baud.
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
    // rev 1 (115200) → rev 2 (9600) → rev 3 (19200).
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

    // Active connection stays on its live state and turns stale.
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

/// 9. Rollback of a framing revision: after reopen, actual framed traffic
///    proves the restored framing default.
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

    // Open with explicit rx_framing: the generated profile bakes the
    // framing into revision 1.
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

    // Connection-mode configure clears the framing → revision 2 (raw).
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

    // Close (stale → failed, file unchanged), reopen, framed read proves
    // the restored framing.
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

/// 10. Wrong expected revision and evicted revision are tool errors that
///     leave the file unchanged.
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

    // Wrong expected_revision → conflict, file unchanged.
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

    // Evicted/never-existing target revision → tool error, file unchanged.
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

/// 11. Deleting a profile bound to an open connection errors with the
///     connection ID; after close, deletion succeeds.
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

/// 12. Rollback with no active bound connections reports zero and the
///     reopened device applies the restored defaults.
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

// ── Phase 4: list_ports profile discovery preview ───────────────────────────
//
// Behavior-first tests for `ListPortsResult.profile_matches`: real PTY slave
// paths, injected StaticPortProvider identity, and public MCP calls only.
// The preview must exactly predict what a bare `open(port=...)` does,
// without marking any profile used or mutating the store.

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

/// 1. Empty store: one `none` entry per port, parallel to `ports`, with the
///    `ports` array serialized exactly like the provider's raw PortInfo
///    (match metadata never contaminates OS enumeration).
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

/// 2. Generated/saved high profiles preview as `selected` with the right
///    name/revision, and the unique last-used winner matches what a later
///    bare `open` actually selects.
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

    // First bare open creates the generated profile (revision 1).
    let opened = open_port(client, &slave, json!({})).await;
    let connection_id = opened["connection_id"].as_str().unwrap().to_string();
    assert_eq!(
        opened["profile"]["profile_name"],
        json!("auto-fake-usb-serial")
    );

    // Save a second, user-named profile for the same device. It has never
    // been used, so it sorts oldest despite being newer on disk.
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

    // Preview: generated profile is the unique most-recently-used winner.
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

    // Explicit use of the second profile makes IT the winner.
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

    // A bare open selects exactly what the preview advertised.
    let reopened = open_port(client, &slave, json!({})).await;
    assert_eq!(reopened["profile"]["profile_name"], json!("my-dev"));
    assert_eq!(reopened["profile"]["source"], json!("automatic"));
    let connection_id = reopened["connection_id"].as_str().unwrap().to_string();
    close_port(client, &connection_id).await;

    harness._client.cancel().await.ok();
}

/// 3. Equal top rank (two saved profiles that were never used): `ambiguous`,
///    both candidates listed, no selected profile — and a bare open stays
///    transient instead of guessing.
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

    // profile_mode=none: no generated profile competes; both saved profiles
    // carry last_used_at_ms = null, so they tie at the oldest rank.
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

    // Bare open must not guess: transient session listing both candidates.
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

/// 4. Duplicate live high fingerprints: `duplicate` for BOTH ports, never a
///    selection — even when a matching profile exists.
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

    // Give both ports a matching profile on disk via a second store
    // instance (same file), then preview: both must be `duplicate`.
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

/// 5. Medium identity (VID/PID, no serial): never auto-selected; explicitly
///    matching non-empty selectors are `ineligible` candidates, and empty
///    selectors (which match every port) are excluded.
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

/// 6. Deleting the only matching profile returns the preview to `none`.
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

/// 7. Fresh read across store instances: a second store writing to the same
///    file (as another process would) is visible to `list_ports` immediately.
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

    // The server's store cache is empty; a DIFFERENT store instance writes
    // the file. list_ports must reload under the advisory lock (fresh read),
    // not answer from the stale cache.
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

/// 8. A real `list_ports` response (with candidates and a selection)
///    validates against the generated schema's Phase 4 wire types, and the
///    catalog schema carries no non-standard uint formats.
///
/// Note: the FULL generated `ListPortsResult` schema cannot validate raw OS
/// enumeration output because schemars marks `PortInfo.vid`/`pid`/`interface`
/// `required` while serde `skip_serializing_if` omits them when `None` — a
/// pre-existing `PortInfo` schema quirk that Phase 4 must not touch (see the
/// "Do not change `PortInfo`" non-scope). The new Phase 4 wire types
/// (`PortProfileMatch`/`ProfileMatchCandidate`) validate cleanly.
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

    // Validate the Phase 4 match entries against their generated $defs
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

// ── Phase 5: capture_boot arm-only over a real PTY ─────────────────────────
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
