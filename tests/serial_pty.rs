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
                "timeout_ms": 1500,
                "max_bytes": 64,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["bytes_read"], json!(6));
    assert_eq!(structured["data"], json!("PONG\r\n"));
    assert_eq!(structured["timed_out"], json!(false));
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
                "poll_interval_ms": 50,
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
async fn pty_wait_for_matches_real_serial_pattern() {
    let (_server, client, _rx, mut pty, connection_id) = setup().await;

    let writer = tokio::spawn(async move {
        // Slow-feed bytes to exercise the wait_for accumulator.
        pty.write_device(b"warming up... ").await.unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        pty.write_device(b"OK> ready").await.unwrap();
    });

    let result = client
        .peer()
        .call_tool(tool_request(
            "wait_for",
            json!({
                "connection_id": connection_id,
                "pattern": "OK>",
                "timeout_ms": 8000,
            }),
        ))
        .await
        .unwrap();
    writer.await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured");
    assert_eq!(structured["timed_out"], json!(false), "{structured:?}");
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
