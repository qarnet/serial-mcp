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
async fn list_tools_returns_all_twelve_tools() {
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
