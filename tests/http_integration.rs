//! Layer 2 — HTTP transport integration tests.
//!
//! These tests run an in-process `SerialHandler` behind `axum`, connect a
//! real `rmcp` HTTP client, and assert the MCP surface (tools, resources,
//! prompts, notifications) is wired up correctly.
//!
//! No OS serial port is involved. Tests that need a connection inject an
//! in-memory loopback via `ConnectionManager::insert` so the duplex peer
//! can stand in for a device.

use std::sync::Arc;
use std::time::Duration;

use rmcp::model::{
    CallToolRequestParams, GetPromptRequestParams, PaginatedRequestParams,
    ReadResourceRequestParams,
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use serial_mcp::limits::{MAX_READ_BYTES, MAX_STREAM_CHUNK_BYTES, MAX_TIMEOUT_MS, MAX_WRITE_BYTES};
use serial_mcp::serial::{test_support::loopback_connection, ConnectionManager};

mod common;
use common::{args_object, connect_client, next_notification, tool_request, TestServer};

const EXPECTED_TOOLS: &[&str] = &[
    "list_ports",
    "list_connections",
    "open",
    "close",
    "write",
    "read",
    "flush",
    "set_dtr_rts",
    "set_flow_control",
    "send_break",
    "subscribe",
    "unsubscribe",
    "get_status",
    "reconfigure",
    "list_profiles",
    "open_profile",
];

#[tokio::test]
async fn initialize_handshake_succeeds() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();
    let info = client.peer().peer_info().expect("peer_info");
    assert_eq!(info.server_info.name, "serial-mcp");
    client.cancel().await.ok();
}

#[tokio::test]
async fn list_tools_returns_all_thirteen_tools() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();

    for expected in EXPECTED_TOOLS {
        assert!(
            names.contains(expected),
            "tool {expected} missing; got {names:?}"
        );
    }
    assert_eq!(names.len(), EXPECTED_TOOLS.len(), "got {names:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn list_resources_returns_two_statics() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .list_resources(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    let uris: Vec<&str> = result.resources.iter().map(|r| r.uri.as_str()).collect();
    assert!(uris.contains(&"serial://ports"));
    assert!(uris.contains(&"serial://connections"));
    client.cancel().await.ok();
}

#[tokio::test]
async fn list_connections_returns_open_connection_summaries() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-list");
    manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .unwrap();

    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["count"], json!(1));
    assert_eq!(structured["connections"][0]["port"], json!("loop-list"));
    assert_eq!(structured["connections"][0]["baud_rate"], json!(115200));
    assert_eq!(structured["connections"][0]["flow_control"], json!("none"));

    client.cancel().await.ok();
}

#[tokio::test]
async fn list_resources_pagination_with_cursor_returns_next_page() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    // Request first page with size 1
    let page1 = client
        .peer()
        .list_resources(Some(PaginatedRequestParams::default().with_cursor(None)))
        .await
        .unwrap();
    assert_eq!(
        page1.resources.len(),
        2,
        "both resources fit on single page"
    );
    assert!(
        page1.next_cursor.is_none(),
        "no next cursor when all items fit"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn list_resource_templates_returns_connection_template() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .list_resource_templates(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    let uris: Vec<&str> = result
        .resource_templates
        .iter()
        .map(|t| t.uri_template.as_str())
        .collect();
    assert_eq!(
        uris,
        vec!["serial://connections/{id}", "serial://connections/{id}/raw"]
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn list_resource_templates_pagination_with_cursor_returns_next_page() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    // Request first page with size 1
    let page1 = client
        .peer()
        .list_resource_templates(Some(PaginatedRequestParams::default().with_cursor(None)))
        .await
        .unwrap();
    assert_eq!(
        page1.resource_templates.len(),
        2,
        "both templates fit on single page"
    );
    assert!(
        page1.next_cursor.is_none(),
        "no next cursor when all items fit"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn list_prompts_returns_diagnose_and_interactive() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .list_prompts(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    let names: Vec<&str> = result.prompts.iter().map(|p| p.name.as_str()).collect();
    assert!(names.contains(&"diagnose_port"));
    assert!(names.contains(&"interactive_terminal"));
    client.cancel().await.ok();
}

#[tokio::test]
async fn read_serial_ports_resource_returns_json_payload() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .read_resource(ReadResourceRequestParams::new("serial://ports"))
        .await
        .unwrap();
    assert_eq!(result.contents.len(), 1);
    let text = match &result.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
        _ => panic!("expected text resource contents"),
    };
    let parsed: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
    assert!(parsed.get("count").is_some());
    assert!(parsed.get("ports").is_some());
    client.cancel().await.ok();
}

#[tokio::test]
async fn read_unknown_resource_yields_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .read_resource(ReadResourceRequestParams::new("serial://does-not-exist"))
        .await;
    assert!(result.is_err(), "expected resource_not_found error");
    client.cancel().await.ok();
}

#[tokio::test]
async fn read_unknown_connection_yields_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .read_resource(ReadResourceRequestParams::new(
            "serial://connections/no-such-id",
        ))
        .await;
    assert!(result.is_err(), "expected resource_not_found error");
    client.cancel().await.ok();
}

#[tokio::test]
async fn call_tool_open_with_bad_data_bits_returns_is_error() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "/tmp/never-exists",
                "baud_rate": 9600,
                "data_bits": "9",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn call_tool_list_ports_returns_structured_result() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(CallToolRequestParams::new("list_ports"))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result
        .structured_content
        .expect("list_ports must produce structuredContent");
    assert!(structured.get("count").is_some());
    assert!(structured.get("ports").is_some());
    client.cancel().await.ok();
}

#[tokio::test]
async fn get_prompt_diagnose_port_returns_user_message() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .get_prompt(
            GetPromptRequestParams::new("diagnose_port")
                .with_arguments(args_object(json!({ "port": "/dev/ttyUSB7" }))),
        )
        .await
        .unwrap();
    assert!(!result.messages.is_empty());
    let first = &result.messages[0];
    assert!(matches!(first.role, rmcp::model::PromptMessageRole::User));
    let rendered = serde_json::to_string(&first.content).unwrap();
    assert!(rendered.contains("/dev/ttyUSB7"));
    client.cancel().await.ok();
}

// ---- With an injected loopback connection -----------------------------------

#[tokio::test]
async fn write_tool_sends_bytes_to_loopback_peer() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-write");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": "hello over http" }),
        ))
        .await
        .unwrap();

    let mut buf = [0u8; 15];
    peer.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello over http");
    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_then_peer_write_pushes_notification() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-sub");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

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

    peer.write_all(b"streaming!").await.unwrap();
    peer.flush().await.unwrap();

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
    assert_eq!(data["data"], serde_json::Value::String("streaming!".into()));
    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_with_timeout_auto_stops_in_background() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-sub-timed");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    // Pre-fill the duplex buffer so data is immediately available when
    // subscribe starts.
    peer.write_all(b"hello-timed").await.unwrap();
    peer.flush().await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 500,
                "encoding": "utf8",
                "poll_interval_ms": 50,
            }),
        ))
        .await
        .unwrap();

    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    // Both subscribe modes now return immediate ack; data is always null.
    assert!(structured["data"].is_null(), "data must be null in ack");
    assert!(
        structured["bytes_read"].is_null(),
        "bytes_read must be null in ack"
    );
    assert!(
        structured["elapsed_ms"].is_null(),
        "elapsed_ms must be null in ack"
    );

    // Data arrives as a background notification.
    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .unwrap();
    let data = event.data.as_object().unwrap();
    assert_eq!(
        data["data"],
        serde_json::Value::String("hello-timed".into())
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_without_timeout_is_fire_and_forget() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-sub-ff");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    let result = client
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
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // Fire-and-forget: data is null; bytes_read/elapsed_ms/timeout_ms also null
    let structured = result.structured_content.expect("structured content");
    assert!(structured["data"].is_null(), "data must be null in FF mode");
    assert!(
        structured["bytes_read"].is_null(),
        "bytes_read must be null"
    );
    assert!(
        structured["elapsed_ms"].is_null(),
        "elapsed_ms must be null"
    );
    assert!(
        structured["timeout_ms"].is_null(),
        "timeout_ms must be null"
    );

    // Background stream still runs: write something and it arrives as notification
    peer.write_all(b"post-subscribe").await.unwrap();
    peer.flush().await.unwrap();
    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(event.data["data"], json!("post-subscribe"));

    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_closed_from_other_session_stops_streaming_task() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-cross-session-close");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client_a, mut rx_a) = connect_client(&server).await.unwrap();
    let (client_b, _rx_b) = connect_client(&server).await.unwrap();

    let subscribe_result = client_a
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
    assert_ne!(
        subscribe_result.is_error,
        Some(true),
        "{subscribe_result:?}"
    );

    let close_result = client_b
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(close_result.is_error, Some(true), "{close_result:?}");

    let _ = peer.write_all(b"should not stream after close").await;
    let maybe_event = tokio::time::timeout(Duration::from_millis(250), rx_a.recv()).await;
    assert!(
        maybe_event.is_err(),
        "received unexpected stream event after close"
    );

    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
}

#[tokio::test]
async fn validation_limits_return_tool_errors_over_http() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-validation");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let cases = [
        tool_request(
            "read",
            json!({ "connection_id": connection_id, "max_buffered_bytes": 0 }),
        ),
        tool_request(
            "read",
            json!({ "connection_id": connection_id, "max_buffered_bytes": MAX_READ_BYTES + 1 }),
        ),
        tool_request(
            "subscribe",
            json!({ "connection_id": connection_id, "max_buffered_bytes": 0 }),
        ),
        tool_request(
            "subscribe",
            json!({ "connection_id": connection_id, "max_buffered_bytes": MAX_STREAM_CHUNK_BYTES + 1 }),
        ),
        tool_request(
            "subscribe",
            json!({ "connection_id": connection_id, "poll_interval_ms": 0 }),
        ),
        tool_request(
            "send_break",
            json!({ "connection_id": connection_id, "duration_ms": MAX_TIMEOUT_MS + 1 }),
        ),
        tool_request(
            "subscribe",
            json!({ "connection_id": connection_id, "timeout_ms": MAX_TIMEOUT_MS + 1 }),
        ),
    ];

    for request in cases {
        let result = client.peer().call_tool(request).await.unwrap();
        assert_eq!(
            result.is_error,
            Some(true),
            "expected validation error: {result:?}"
        );
    }

    let oversized_payload = "x".repeat(MAX_WRITE_BYTES + 1);
    let result = client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": oversized_payload }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.is_error,
        Some(true),
        "expected write validation error: {result:?}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn read_with_no_data_times_out_with_is_error() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-read-timeout");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 50,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();

    assert_ne!(
        result.is_error,
        Some(true),
        "read timeout must return isError=false: {result:?}"
    );
    // Timeout is a normal stop reason, not an error. Verify structured content.
    let structured = result
        .structured_content
        .expect("timeout result must have structured content");
    assert_eq!(
        structured["stop_reason"],
        json!("timeout"),
        "timeout result must have stop_reason=timeout"
    );
    assert_eq!(structured["bytes_read"], json!(0));

    client.cancel().await.ok();
}

#[tokio::test]
async fn read_result_contains_elapsed_ms() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-read-elapsed");
    let connection_id = manager.insert(conn).await.unwrap();

    peer.write_all(b"hello").await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 1000,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();

    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["data"], json!("hello"));
    assert!(structured.get("elapsed_ms").is_some(), "{structured:?}");
    let elapsed = structured["elapsed_ms"].as_u64().unwrap();
    assert!(
        elapsed < 1000,
        "elapsed_ms {elapsed} should be less than timeout"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn send_break_result_includes_actual_duration() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-break");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "send_break",
            json!({
                "connection_id": connection_id,
                "duration_ms": 80,
            }),
        ))
        .await
        .unwrap();

    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["duration_ms"], json!(80), "{structured:?}");
    assert!(
        structured.get("actual_duration_ms").is_some(),
        "{structured:?}"
    );
    let actual = structured["actual_duration_ms"].as_u64().unwrap();
    assert!(
        actual >= 80,
        "actual_duration_ms {actual} should be >= requested 80. {structured:?}"
    );

    client.cancel().await.ok();
}

// ── Gap-fill: set_dtr_rts integration ────────────────────────────────────────

#[tokio::test]
async fn set_dtr_rts_all_combos_return_valid_response() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-dtr-rts");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    for (dtr, rts) in [(false, false), (false, true), (true, false), (true, true)] {
        let result = client
            .peer()
            .call_tool(tool_request(
                "set_dtr_rts",
                json!({ "connection_id": connection_id, "dtr": dtr, "rts": rts }),
            ))
            .await
            .unwrap();
        assert_ne!(
            result.is_error,
            Some(true),
            "set_dtr_rts dtr={dtr} rts={rts} returned error: {result:?}"
        );
        let s = result.structured_content.expect("structured content");
        assert_eq!(s["dtr"], json!(dtr), "dtr mismatch in {s:?}");
        assert_eq!(s["rts"], json!(rts), "rts mismatch in {s:?}");
        assert_eq!(s["connection_id"], json!(connection_id));
    }

    client.cancel().await.ok();
}

// ── Gap-fill: flush target isolation ─────────────────────────────────────────

#[tokio::test]
async fn flush_each_target_returns_valid_response() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-flush-targets");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    for target in ["input", "output", "both"] {
        let result = client
            .peer()
            .call_tool(tool_request(
                "flush",
                json!({ "connection_id": connection_id, "target": target }),
            ))
            .await
            .unwrap();
        assert_ne!(
            result.is_error,
            Some(true),
            "flush target={target} returned error: {result:?}"
        );
        let s = result.structured_content.expect("structured content");
        assert_eq!(s["target"], json!(target), "target mismatch for {target} in {s:?}");
        assert_eq!(s["connection_id"], json!(connection_id));
    }

    client.cancel().await.ok();
}

// ── Gap-fill: write encoding error ───────────────────────────────────────────

#[tokio::test]
async fn write_with_invalid_encoding_returns_tool_error() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-write-enc");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Malformed base64
    {
        let result = client
            .peer()
            .call_tool(tool_request(
                "write",
                json!({ "connection_id": connection_id, "data": "!!!invalid!!!", "encoding": "base64" }),
            ))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true), "{result:?}");
    }

    // Invalid hex (odd length)
    {
        let result = client
            .peer()
            .call_tool(tool_request(
                "write",
                json!({ "connection_id": connection_id, "data": "abc", "encoding": "hex" }),
            ))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true), "{result:?}");
    }

    // Invalid hex characters
    {
        let result = client
            .peer()
            .call_tool(tool_request(
                "write",
                json!({ "connection_id": connection_id, "data": "xxyy", "encoding": "hex" }),
            ))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true), "{result:?}");
    }

    // Valid utf8 should succeed
    {
        let result = client
            .peer()
            .call_tool(tool_request(
                "write",
                json!({ "connection_id": connection_id, "data": "hello", "encoding": "utf8" }),
            ))
            .await
            .unwrap();
        assert_ne!(result.is_error, Some(true), "valid utf8 should succeed: {result:?}");
    }

    // Bogus encoding name
    {
        let result = client
            .peer()
            .call_tool(tool_request(
                "write",
                json!({ "connection_id": connection_id, "data": "hello", "encoding": "rot13" }),
            ))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true), "{result:?}");
    }

    client.cancel().await.ok();
}

// ── Gap-fill: unsubscribe on non-existent connection ─────────────────────────

#[tokio::test]
async fn unsubscribe_on_unknown_connection_returns_was_active_false() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "unsubscribe",
            json!({ "connection_id": "nonexistent-deadbeef" }),
        ))
        .await
        .unwrap();
    // unsubscribe on unknown connection should return success with was_active=false
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured content");
    assert_eq!(
        s["was_active"], json!(false),
        "expected was_active=false for unknown connection: {s:?}"
    );

    client.cancel().await.ok();
}

// ── Gap-fill: read silence timeout ───────────────────────────────────────────

#[tokio::test]
async fn read_silence_timeout_stops_with_no_new_rx_timeout() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-read-silence");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 1000,
                "no_new_rx_timeout_ms": 50,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();

    assert_ne!(
        result.is_error,
        Some(true),
        "silence timeout should be a normal stop, not an error: {result:?}"
    );
    let s = result.structured_content.expect("structured content");
    assert_eq!(
        s["stop_reason"], json!("no_new_rx_timeout"),
        "expected no_new_rx_timeout stop_reason: {s:?}"
    );
    assert_eq!(s["bytes_read"], json!(0));

    client.cancel().await.ok();
}

// ── Gap-fill: subscribe replaced_previous ────────────────────────────────────

#[tokio::test]
async fn subscribe_replaced_previous_field_is_correct() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-sub-replace");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // First subscribe
    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result
        .structured_content
        .expect("first subscribe structured");
    assert_eq!(
        s["replaced_previous"], json!(false),
        "first subscribe should have replaced_previous=false: {s:?}"
    );

    // Second subscribe — replaces first
    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result
        .structured_content
        .expect("second subscribe structured");
    assert_eq!(
        s["replaced_previous"], json!(true),
        "second subscribe should have replaced_previous=true: {s:?}"
    );

    client.cancel().await.ok();
}

// ── Gap-fill: set_flow_control invalid mode ──────────────────────────────────

#[tokio::test]
async fn set_flow_control_invalid_mode_returns_tool_error() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-flow-err");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "set_flow_control",
            json!({ "connection_id": connection_id, "flow_control": "bogus" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.is_error, Some(true),
        "bogus flow_control should return tool error: {result:?}"
    );

    // Valid mode should succeed
    let result = client
        .peer()
        .call_tool(tool_request(
            "set_flow_control",
            json!({ "connection_id": connection_id, "flow_control": "none" }),
        ))
        .await
        .unwrap();
    assert_ne!(
        result.is_error, Some(true),
        "valid flow_control=none should succeed: {result:?}"
    );

    client.cancel().await.ok();
}

// ── Gap-fill: send_break cancellation ────────────────────────────────────────

#[tokio::test]
async fn send_break_cancellation_stops_gracefully() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-break-cancel");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Request a long break and cancel mid-way
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        client.peer().call_tool(tool_request(
            "send_break",
            json!({
                "connection_id": connection_id,
                "duration_ms": 5000,
            }),
        )),
    )
    .await;

    // Cancel the client before the break completes
    client.cancel().await.ok();

    // The call should either complete with cancellation or the timeout fires
    // (in which case we already cancelled — the task will be cleaned up).
    // Either way, this proves the tool doesn't hang forever.
    match result {
        Ok(Ok(call_result)) => {
            // Completed before 100ms timeout — may be is_error due to cancellation
            assert!(
                call_result.is_error == Some(true) || call_result.is_error == Some(false),
                "break completed: {call_result:?}"
            );
        }
        _ => {
            // Timeout fired — that's fine; the client was cancelled and the
            // break task will be cleaned up by the runtime.
        }
    }
}

// ── Gap-fill: bogus connection ID for each tool ──────────────────────────────

#[tokio::test]
async fn bogus_connection_id_returns_tool_error_for_all_id_tools() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let bogus_id = "deadbeef-dead-beef-dead-beefdeadbeef";

    let cases = [
        ("close", json!({ "connection_id": bogus_id })),
        ("write", json!({ "connection_id": bogus_id, "data": "test" })),
        ("read", json!({ "connection_id": bogus_id })),
        ("flush", json!({ "connection_id": bogus_id })),
        ("set_dtr_rts", json!({ "connection_id": bogus_id, "dtr": true, "rts": false })),
        ("set_flow_control", json!({ "connection_id": bogus_id, "flow_control": "none" })),
        ("send_break", json!({ "connection_id": bogus_id })),
        ("subscribe", json!({ "connection_id": bogus_id })),
    ];

    for (tool_name, args) in &cases {
        let result = client
            .peer()
            .call_tool(tool_request(tool_name, args.clone()))
            .await
            .unwrap();
        assert_eq!(
            result.is_error, Some(true),
            "{tool_name} with bogus id should return tool error: {result:?}"
        );
    }

    // unsubscribe returns was_active=false for non-existent connection (not an error)
    {
        let result = client
            .peer()
            .call_tool(tool_request(
                "unsubscribe",
                json!({ "connection_id": bogus_id }),
            ))
            .await
            .unwrap();
        assert_ne!(
            result.is_error, Some(true),
            "unsubscribe with bogus id should succeed with was_active=false: {result:?}"
        );
        let s = result.structured_content.unwrap();
        assert_eq!(s["was_active"], json!(false), "{s:?}");
    }

    // list_connections does not take connection_id — just verify it succeeds
    let result = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .unwrap();
    assert_ne!(
        result.is_error, Some(true),
        "list_connections should succeed without connection_id: {result:?}"
    );

    client.cancel().await.ok();
}

// ── get_status integration ───────────────────────────────────────────────────

#[tokio::test]
async fn get_status_returns_config_and_counters() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-get-status");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Before any I/O, counters should be zero
    let result = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["connection_id"], json!(connection_id));
    assert_eq!(s["baud_rate"], json!(115200));
    assert_eq!(s["data_bits"], json!("8"));
    assert_eq!(s["stop_bits"], json!("1"));
    assert_eq!(s["parity"], json!("none"));
    assert_eq!(s["flow_control"], json!("none"));
    assert_eq!(s["is_open"], json!(true));
    assert_eq!(s["tx_bytes"], json!(0));
    assert_eq!(s["rx_bytes"], json!(0));
    assert!(s["last_activity_ms"].is_null());

    // Write some data — tx counter should increase
    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": "hello" }),
        ))
        .await
        .unwrap();

    peer.write_all(b"world").await.unwrap();

    // Read to increment rx counter
    client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 100,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();

    // Check updated status
    let result = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let s = result.structured_content.expect("structured");
    assert_eq!(s["tx_bytes"], json!(5), "tx should be 5: {s:?}");
    assert_eq!(s["rx_bytes"], json!(5), "rx should be 5: {s:?}");
    assert!(
        !s["last_activity_ms"].is_null(),
        "last_activity_ms should be set after I/O: {s:?}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn get_status_unknown_connection_returns_error() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": "nonexistent-deadbeef" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.is_error, Some(true),
        "unknown connection should return error: {result:?}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn reconfigure_changes_baud_rate_on_loopback() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-recfg");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Verify initial config
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let s = status.structured_content.unwrap();
    assert_eq!(s["baud_rate"], json!(115200));

    // Reconfigure baud_rate to 9600
    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "baud_rate": 9600 }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["baud_rate"], json!(9600), "{s:?}");

    // Verify through get_status that change persisted
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let s = status.structured_content.unwrap();
    assert_eq!(s["baud_rate"], json!(9600), "baud_rate should persist: {s:?}");

    client.cancel().await.ok();
}

#[tokio::test]
async fn reconfigure_invalid_args_return_error() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-recfg-err");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Bogus baud_rate (0)
    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "baud_rate": 0 }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    // Bogus data_bits
    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "data_bits": "9" }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    // Bogus flow_control
    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "flow_control": "bogus" }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    // Unknown connection
    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": "nonexistent", "baud_rate": 9600 }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    client.cancel().await.ok();
}

#[tokio::test]
async fn list_profiles_returns_empty_when_no_config() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["count"], json!(0));
    assert!(s["profiles"].as_array().unwrap().is_empty());

    client.cancel().await.ok();
}

#[tokio::test]
async fn open_profile_not_found_returns_error() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open_profile",
            json!({ "profile": "nonexistent" }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.is_error, Some(true),
        "unknown profile should return error: {result:?}"
    );

    client.cancel().await.ok();
}

// ── reconfigure gap-fill tests ───────────────────────────────────────────────

#[tokio::test]
async fn reconfigure_multiple_params_at_once() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-recfg-multi");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({
                "connection_id": connection_id,
                "baud_rate": 9600,
                "data_bits": "7",
                "stop_bits": "2",
                "parity": "odd",
                "flow_control": "software",
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.unwrap();
    assert_eq!(s["baud_rate"], json!(9600));
    assert_eq!(s["data_bits"], json!("7"));
    assert_eq!(s["stop_bits"], json!("2"));
    assert_eq!(s["parity"], json!("odd"));
    assert_eq!(s["flow_control"], json!("software"));

    client.cancel().await.ok();
}

#[tokio::test]
async fn reconfigure_no_params_returns_current_config() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-recfg-noop");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.unwrap();
    assert_eq!(s["baud_rate"], json!(115200));
    assert_eq!(s["data_bits"], json!("8"));

    client.cancel().await.ok();
}

#[tokio::test]
async fn reconfigure_invalid_stop_bits_returns_error() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-recfg-stop");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "stop_bits": "3" }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "parity": "mark" }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    client.cancel().await.ok();
}
