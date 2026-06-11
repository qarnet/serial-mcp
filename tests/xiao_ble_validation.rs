//! Hardware-in-the-loop integration tests for the XIAO BLE (nRF52840) running
//! the dedicated serial-mcp UART test firmware.
//!
//! The firmware exposes a simple UART CLI over the PicoProbe UART bridge on
//! `/dev/ttyACM0`.
//! Relevant commands:
//!
//! - `ping` → `pong\r\n`
//! - `spam <bytes> hex [last_data="\r\n"] [delay=10]`
//!   Sends `bytes` random hex chars in 256-byte packets separated by
//!   `delay` ms, then prints `"Spam complete: N bytes sent\r\n"`.
//! - `spam stop` → `"Spam stopped: N bytes sent\r\n"`
//! - `info` → `"Board: ...\r\nBuild time: ...\r\n"`
//!
//! These tests are marked `#[ignore]` and skipped in CI. Run explicitly:
//!
//! ```sh
//! cargo test --test xiao_ble_validation -- --ignored
//! # or with a custom port:
//! SERIAL_MCP_XIAO_PORT=/dev/ttyACM1 cargo test --test xiao_ble_validation -- --ignored
//! ```

use std::time::Duration;

use serde_json::json;

mod common;
use common::{args_object, connect_client, next_notification, tool_request, TestServer};

const PORT_ENV: &str = "SERIAL_MCP_XIAO_PORT";
const DEFAULT_PORT: &str = "/dev/ttyACM0";
const BAUD_RATE: u32 = 115200;
const NAME: &str = "xiao-uart";

fn xiao_port() -> String {
    std::env::var(PORT_ENV)
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_PORT.to_string())
}

async fn open_xiao(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    port: &str,
) -> String {
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": port,
                "name": NAME,
                "baud_rate": BAUD_RATE,
            }),
        ))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    let s = result.structured_content.expect("structured open");
    assert_eq!(s["name"], json!(NAME));
    s["connection_id"]
        .as_str()
        .expect("connection_id")
        .to_string()
}

async fn write_cmd(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    cmd: &str,
) {
    let result = client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": format!("{cmd}\r\n") }),
        ))
        .await
        .expect("write call");
    assert_ne!(result.is_error, Some(true), "write failed: {result:?}");
}

async fn write_raw(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    data: &str,
) {
    let result = client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": data }),
        ))
        .await
        .expect("write call");
    assert_ne!(result.is_error, Some(true), "write failed: {result:?}");
}

async fn flush_both(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
) {
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .expect("flush call");
}

async fn reset_firmware_state(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
) {
    // Disable trace/framing modes that may have been left on by prior tests.
    write_cmd(client, connection_id, "trace off").await;
    write_cmd(client, connection_id, "framing off").await;
    write_cmd(client, connection_id, "spam stop").await;
    write_cmd(client, connection_id, "rxbuf clear").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    flush_both(client, connection_id).await;
}

async fn close_connection(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
) {
    let result = client
        .peer()
        .call_tool(
            rmcp::model::CallToolRequestParams::new("close")
                .with_arguments(args_object(json!({ "connection_id": connection_id }))),
        )
        .await
        .expect("close call");
    assert_ne!(result.is_error, Some(true), "close failed: {result:?}");
}

// ── Test 1: ping roundtrip ───────────────────────────────────────────────────

/// Verify the board is alive and read responds correctly.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_ping_roundtrip() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    reset_firmware_state(&client, &id).await;
    write_cmd(&client, &id, "ping").await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 2000,
                "max_buffered_bytes": 64,
                "encoding": "utf8",
                "match": {
                    "pattern": "pong",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }),
        ))
        .await
        .expect("read call");

    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected pong: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

// ── Test 2: read match on spam completion ────────────────────────────────────

/// read waits first, then a later write still reaches the board promptly.
/// Exercises real-device TX while the RX session already owns the connection.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_pending_read_then_write_ping_roundtrip() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    reset_firmware_state(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 128,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;

    let start = tokio::time::Instant::now();
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    let elapsed = start.elapsed();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected pong: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));
    assert!(
        elapsed < Duration::from_secs(1),
        "write+response took too long with pending read: {elapsed:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

/// Split write calls must stay ordered on the real device so the board still
/// sees one valid CLI command.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_split_writes_preserve_command_order() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    reset_firmware_state(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 128,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_raw(&client, &id, "pi").await;
    write_raw(&client, &id, "ng").await;
    write_raw(&client, &id, "\r\n").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected pong: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

/// Framing mode should report one committed `ping` line even when the command
/// arrives through multiple write calls.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_framing_reports_single_split_command() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    reset_firmware_state(&client, &id).await;
    write_cmd(&client, &id, "framing on").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_raw(&client, &id, "pi").await;
    write_raw(&client, &id, "ng").await;
    write_raw(&client, &id, "\r\n").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.contains("LINE len=4 data=\"ping\""),
        "expected one framed ping line, got: {data:?}"
    );
    assert!(
        data.contains("pong"),
        "expected pong after framed line: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

/// Trace mode should expose exact RX byte order for split writes on real
/// hardware, including CRLF terminator bytes.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_trace_reports_exact_split_byte_sequence() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    reset_firmware_state(&client, &id).await;
    write_cmd(&client, &id, "trace on").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 2048,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_raw(&client, &id, "pi").await;
    write_raw(&client, &id, "ng").await;
    write_raw(&client, &id, "\r\n").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let data = s["data"].as_str().unwrap_or("");
    for expected in [
        "RX[0]=0x70",
        "RX[1]=0x69",
        "RX[2]=0x6e",
        "RX[3]=0x67",
        "RX[4]=0x0d",
        "RX[5]=0x0a",
    ] {
        assert!(
            data.contains(expected),
            "missing trace {expected} in {data:?}"
        );
    }
    assert!(
        data.contains("pong"),
        "expected pong after traced bytes: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

/// read(match=...) stops on "Spam complete" after a 1024-byte hex spam.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_read_match_on_spam_complete() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    flush_both(&client, &id).await;

    // Spawn the read first so it's waiting when spam data arrives.
    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 5000,
                    "max_buffered_bytes": 8192,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "Spam complete",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    // 1024 hex bytes, 4 × 256-byte packets at 10ms interval → ~40ms total.
    write_cmd(&client, &id, "spam 1024 hex").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected match: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));
    assert_eq!(s["name"], json!(NAME));
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.contains("Spam complete"),
        "data should contain stop phrase: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

// ── Test 3: subscribe match stops on spam completion ─────────────────────────

/// subscribe(match=...) self-stops with match_found when "Spam complete"
/// appears mid-stream. Exercises the subscribe match stop bug fix.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_subscribe_match_stops_on_spam_complete() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    flush_both(&client, &id).await;

    // Subscribe with match on the completion phrase.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 8192,
                "match": {
                    "pattern": "Spam complete",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8",
                        "context_amount_of_matched_bytes": 64
                    }
                }
            }),
        ))
        .await
        .unwrap();

    // Small spam — 1024 hex bytes, done in ~40ms. Subscribe should auto-stop.
    write_cmd(&client, &id, "spam 1024 hex").await;

    let mut found_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(3)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("matched").and_then(|v| v.as_bool()) == Some(true) {
                    found_match_stop = true;
                    assert_eq!(data["stop_reason"], json!("match_found"), "{data:?}");
                    assert!(
                        data["match_index"].as_u64().is_some(),
                        "match_index present"
                    );
                    let shaped = data.get("data").and_then(|v| v.as_str()).unwrap_or("");
                    assert!(
                        shaped.contains("Spam complete"),
                        "shaped payload should contain stop phrase: {shaped:?}"
                    );
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        found_match_stop,
        "subscribe should have emitted match_found stop notification"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

// ── Test 4: subscribe silence timeout after spam ends ────────────────────────

/// subscribe(no_new_rx_timeout_ms=500) stops with no_new_rx_timeout once
/// the spam finishes and the board goes silent.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_subscribe_silence_timeout_after_spam() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    flush_both(&client, &id).await;

    // Run a small spam first to produce data, then let the board go silent.
    write_cmd(&client, &id, "spam 512 hex").await;
    // Wait for spam to complete (~20ms data + some margin).
    tokio::time::sleep(Duration::from_millis(300)).await;
    flush_both(&client, &id).await;

    // Now subscribe with silence timeout — board is quiet.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 1024,
                "no_new_rx_timeout_ms": 500
            }),
        ))
        .await
        .unwrap();

    // Stop notification should arrive within ~1s.
    let event = next_notification(&mut rx, Duration::from_secs(5))
        .await
        .expect("subscribe should emit stop notification");

    let data = event.data.as_object().unwrap();
    assert_eq!(
        data["stop_reason"],
        json!("no_new_rx_timeout"),
        "expected silence timeout stop: {data:?}"
    );
    assert_ne!(data.get("matched").and_then(|v| v.as_bool()), Some(true));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

// ── Test 5: buffer budget under hex flood ────────────────────────────────────

/// read(max_buffered_bytes=256) stops cleanly with max_buffered_bytes
/// while a large hex flood is in progress.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_read_buffer_budget_stops_under_flood() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 5000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    // Large spam — 65536 hex bytes at 10ms/packet — will outlast the read.
    write_cmd(&client, &id, "spam 65536 hex").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["stop_reason"], json!("max_buffered_bytes"), "{s:?}");
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.len() <= 256,
        "data should be ≤ 256 bytes, got {}",
        data.len()
    );

    // Stop the flood.
    write_cmd(&client, &id, "spam stop").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

// ── Test 6: subscribe wall-clock timeout stops under active flood ─────────────

/// subscribe(timeout_ms=800) stops on wall-clock timeout even while spam data
/// is actively flowing. Verifies that the timeout stop condition fires correctly
/// when the stream is not silent (complementary to the silence-timeout test).
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_subscribe_timeout_stops_under_flood() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    flush_both(&client, &id).await;

    // Subscribe with a 800ms wall-clock timeout (no match, no silence condition).
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 16384,
                "timeout_ms": 800,
            }),
        ))
        .await
        .unwrap();

    // Large spam — should not finish within the 800ms window.
    write_cmd(&client, &id, "spam 1000000 hex").await;

    // Collect notifications until stop.
    let mut total_bytes: u64 = 0;
    let mut stop_reason = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(3)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if let Some(n) = data.get("bytes_read").and_then(|v| v.as_u64()) {
                    total_bytes += n;
                }
                if let Some(reason) = data.get("stop_reason").and_then(|v| v.as_str()) {
                    stop_reason = reason.to_string();
                    break;
                }
            }
            Err(_) => break,
        }
    }

    assert_eq!(
        stop_reason, "timeout",
        "expected timeout stop: {stop_reason:?}"
    );
    assert!(
        total_bytes > 0,
        "should have received some bytes before timeout"
    );

    // Stop the flood so subsequent tests start clean.
    write_cmd(&client, &id, "spam stop").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    close_connection(&client, &id).await;
    client.cancel().await.ok();
}

// ── Test 7: close while subscribe active ─────────────────────────────────────

/// Close the connection while a subscribe is streaming hex flood.
/// Connection cleanup should be clean; list_connections returns 0.
#[tokio::test]
#[ignore = "requires XIAO BLE on /dev/ttyACM0"]
async fn xiao_close_while_subscribe_active() {
    let port = xiao_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_xiao(&client, &port).await;

    flush_both(&client, &id).await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 4096,
            }),
        ))
        .await
        .unwrap();

    write_cmd(&client, &id, "spam 1000000 hex delay=5").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Stop the spam before closing so the firmware is quiet for subsequent tests.
    // "Spam stopped" is never printed by the firmware (spam_tick returns early when
    // spam_running=false), but the flood does stop.
    write_cmd(&client, &id, "spam stop").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    close_connection(&client, &id).await;

    let list = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .expect("list_connections");
    assert_ne!(list.is_error, Some(true), "{list:?}");
    let s = list.structured_content.expect("structured");
    assert_eq!(
        s["count"],
        json!(0),
        "expected 0 connections after close: {s:?}"
    );

    client.cancel().await.ok();
}
