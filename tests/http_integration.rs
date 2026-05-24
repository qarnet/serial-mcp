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

use serial_mcp_server::serial::{test_support::loopback_connection, ConnectionManager};

mod common;
use common::{args_object, connect_client, next_notification, tool_request, TestServer};

const EXPECTED_TOOLS: &[&str] = &[
    "list_ports",
    "open",
    "close",
    "write",
    "read",
    "flush",
    "set_dtr_rts",
    "send_break",
    "wait_for",
    "subscribe",
    "unsubscribe",
];

#[tokio::test]
async fn initialize_handshake_succeeds() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let info = client.peer().peer_info().expect("peer_info");
    assert_eq!(info.server_info.name, "serial-mcp-server");
    client.cancel().await.ok();
}

#[tokio::test]
async fn list_tools_returns_all_eleven_tools() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
async fn list_resource_templates_returns_connection_template() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
async fn list_prompts_returns_diagnose_and_interactive() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .read_resource(ReadResourceRequestParams::new("serial://does-not-exist"))
        .await;
    assert!(result.is_err(), "expected resource_not_found error");
    client.cancel().await.ok();
}

#[tokio::test]
async fn read_unknown_connection_yields_not_found() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
async fn wait_for_returns_match_index_over_http() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-waitfor");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Peer side will dribble bytes in as if it were a device.
    let writer = tokio::spawn(async move {
        peer.write_all(b"noise OK> rest").await.unwrap();
        peer.flush().await.unwrap();
    });

    let result = client
        .peer()
        .call_tool(tool_request(
            "wait_for",
            json!({
                "connection_id": connection_id,
                "pattern": "OK>",
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();
    writer.await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["matched"], json!(true));
    assert_eq!(structured["match_index"], json!(6));
    client.cancel().await.ok();
}
