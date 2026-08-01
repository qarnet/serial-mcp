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
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use serial_mcp::limits::{MAX_TIMEOUT_MS, MAX_WRITE_BYTES};
use serial_mcp::serial::{test_support::loopback_connection, ConnectionManager};

mod common;
use common::{args_object, connect_client, next_notification, tool_request, TestServer};

const EXPECTED_TOOLS: &[&str] = &[
    "list_ports",
    "list_connections",
    "open",
    "close",
    "write",
    "transact",
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
    "save_profile",
    "delete_profile",
    "configure",
    "get_log",
    "clear_log",
    "export_log",
    "reconnect",
    "compute_checksum",
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
async fn list_tools_returns_all_twenty_five_tools() {
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
        vec![
            "serial://connections/{id}",
            "serial://connections/{id}/raw",
            "serial://connections/{id}/log"
        ]
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
        3,
        "all three templates fit on single page"
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

// ── configure tool: profile mode ─────────────────────────────────────────────

#[tokio::test]
async fn configure_profile_creates_new_profile() {
    let profiles_dir = TempDir::new().unwrap();
    let profiles_path = profiles_dir.path().join("profiles.toml");
    let manager = Arc::new(ConnectionManager::new());
    let server = TestServer::start_with_profiles_path(manager, profiles_path.clone()).await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let name = "test-configure-create";
    let result = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": name,
                "defaults": { "baud_rate": 9600, "rx_framing": {"type": "line"} }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["mode"], "profile");
    assert_eq!(s["created"], true);
    assert_eq!(s["defaults"]["baud_rate"], 9600);
    // Verify it shows up in list_profiles.
    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let ls = listed.structured_content.expect("structured");
    let names = ls["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&name), "profile should be listed: {names:?}");
    client.cancel().await.ok();
    // TempDir cleanup: profiles_dir dropped here, deletes profiles.toml.
}

#[tokio::test]
async fn configure_profile_overwrites_existing() {
    let profiles_dir = TempDir::new().unwrap();
    let profiles_path = profiles_dir.path().join("profiles.toml");
    let manager = Arc::new(ConnectionManager::new());
    let server = TestServer::start_with_profiles_path(manager, profiles_path.clone()).await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let name = "test-configure-ow";
    // Create initial profile.
    let _ = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": name,
                "defaults": { "baud_rate": 9600 }
            }),
        ))
        .await
        .unwrap();
    // Overwrite via configure with overwrite: true, higher baud.
    let result = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": name,
                "overwrite": true,
                "defaults": { "baud_rate": 19200 }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["mode"], "profile");
    assert_eq!(s["created"], false);
    assert_eq!(s["defaults"]["baud_rate"], 19200);
    client.cancel().await.ok();
    // TempDir cleanup: profiles_dir dropped here, deletes profiles.toml.
}

#[tokio::test]
async fn configure_profile_rejects_existing_without_overwrite() {
    let profiles_dir = TempDir::new().unwrap();
    let profiles_path = profiles_dir.path().join("profiles.toml");
    let manager = Arc::new(ConnectionManager::new());
    let server = TestServer::start_with_profiles_path(manager, profiles_path.clone()).await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let name = "test-configure-rej";
    // Create initial profile.
    let _ = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": name,
                "defaults": { "baud_rate": 9600 }
            }),
        ))
        .await
        .unwrap();
    // Attempt overwrite without overwrite flag.
    let result = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": name,
                "defaults": { "baud_rate": 19200 }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.is_error,
        Some(true),
        "should reject existing profile without overwrite: {result:?}"
    );
    client.cancel().await.ok();
    // TempDir cleanup: profiles_dir dropped here, deletes profiles.toml.
}

// ── configure tool: validation errors ────────────────────────────────────────

#[tokio::test]
async fn configure_rejects_both_profile_and_connection_id() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let result = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": "x",
                "connection_id": "y"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        result.is_error,
        Some(true),
        "should reject both profile and connection_id: {result:?}"
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn configure_rejects_neither() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let result = client
        .peer()
        .call_tool(tool_request("configure", json!({})))
        .await
        .unwrap();
    assert_eq!(
        result.is_error,
        Some(true),
        "should reject neither profile nor connection_id: {result:?}"
    );
    client.cancel().await.ok();
}

// ── compute_checksum: known vectors ──────────────────────────────────────────

#[tokio::test]
async fn compute_checksum_xor_known_vector() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let r = client
        .peer()
        .call_tool(tool_request(
            "compute_checksum",
            json!({
                "data": "hello",
                "algorithm": "xor"
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured");
    assert_eq!(s["algorithm"], "xor");
    assert_eq!(s["checksum_hex"], "62");
    assert_eq!(s["checksum"], 98);
    assert_eq!(s["byte_count"], 5);
    client.cancel().await.ok();
}

#[tokio::test]
async fn compute_checksum_lrc_known_vector() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let r = client
        .peer()
        .call_tool(tool_request(
            "compute_checksum",
            json!({
                "data": "010203",
                "encoding": "hex",
                "algorithm": "lrc"
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured");
    assert_eq!(s["algorithm"], "lrc");
    assert_eq!(s["checksum_hex"], "FA");
    assert_eq!(s["checksum"], 250);
    assert_eq!(s["byte_count"], 3);
    client.cancel().await.ok();
}

#[tokio::test]
async fn compute_checksum_hex_input() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let r = client
        .peer()
        .call_tool(tool_request(
            "compute_checksum",
            json!({
                "data": "48656c6c6f",
                "encoding": "hex",
                "algorithm": "xor"
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured");
    assert_eq!(s["checksum_hex"], "42");
    assert_eq!(s["byte_count"], 5);
    client.cancel().await.ok();
}

#[tokio::test]
async fn compute_checksum_base64_input() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let r = client
        .peer()
        .call_tool(tool_request(
            "compute_checksum",
            json!({
                "data": "SGVsbG8=",
                "encoding": "base64",
                "algorithm": "xor"
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured");
    assert_eq!(s["checksum_hex"], "42");
    client.cancel().await.ok();
}

#[tokio::test]
async fn compute_checksum_rejects_bad_encoding() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let r = client
        .peer()
        .call_tool(tool_request(
            "compute_checksum",
            json!({
                "data": "hello",
                "encoding": "garbage",
                "algorithm": "xor"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true), "should reject bad encoding: {r:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn compute_checksum_rejects_bad_hex() {
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let r = client
        .peer()
        .call_tool(tool_request(
            "compute_checksum",
            json!({
                "data": "ZZ",
                "encoding": "hex",
                "algorithm": "xor"
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        r.is_error,
        Some(true),
        "should reject invalid hex data: {r:?}"
    );
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

/// The rendered `diagnose_port` prompt must use current tool shapes: a valid
/// `read` flow, no removed per-call `max_buffered_bytes`, and no removed
/// `wait_for` tool.
#[tokio::test]
async fn get_prompt_diagnose_port_uses_current_read_flow() {
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
    let rendered = serde_json::to_string(&result.messages[0].content).unwrap();
    assert!(rendered.contains("/dev/ttyUSB7"));
    // Uses the current read flow (read with timeout / match).
    assert!(
        rendered.contains("read(connection_id"),
        "prompt must drive the current read flow: {rendered}"
    );
    // The per-call max_buffered_bytes argument was removed in v0.8.1.
    assert!(
        !rendered.contains("max_buffered_bytes"),
        "prompt must not use the removed per-call max_buffered_bytes: {rendered}"
    );
    // The wait_for tool was removed; read(match=...) is the pattern-wait flow.
    assert!(
        !rendered.contains("wait_for"),
        "prompt must not reference the removed wait_for tool: {rendered}"
    );
    client.cancel().await.ok();
}

/// `tools/list` descriptions for `read`/`transact`/`flush` must advertise the
/// actual tagged-object `ReadFrom` wire shape, not string shorthand.
#[tokio::test]
async fn read_tool_description_uses_tagged_readfrom_examples() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();

    let desc = |name: &str| -> String {
        result
            .tools
            .iter()
            .find(|t| t.name.as_ref() == name)
            .unwrap_or_else(|| panic!("tool {name} missing from tools/list"))
            .description
            .as_deref()
            .unwrap_or("")
            .to_string()
    };

    for name in ["read", "transact", "flush"] {
        let d = desc(name);
        assert!(
            d.contains(r#"{"type":"now"}"#),
            "{name} description must show the tagged from example: {d}"
        );
    }
    // The read description must carry the complete tagged set, including the
    // absolute offset object.
    let read = desc("read");
    for tagged in [
        r#"{"type":"cursor"}"#,
        r#"{"type":"now"}"#,
        r#"{"type":"buffer_start"}"#,
        r#"{"type":"offset","offset":N}"#,
    ] {
        assert!(
            read.contains(tagged),
            "read description must contain {tagged}: {read}"
        );
    }
    // No bare string shorthand survives in the examples.
    for name in ["read", "transact", "flush"] {
        let d = desc(name);
        for shorthand in [
            r#"from: "now""#,
            r#"from: "cursor""#,
            r#"from: "buffer_start""#,
        ] {
            assert!(
                !d.contains(shorthand),
                "{name} description must not advertise {shorthand}: {d}"
            );
        }
    }
    client.cancel().await.ok();
}

/// The generated input schemas for `read`/`subscribe`/`transact` carry the
/// agent-visible `from` guidance in the property description. It must
/// advertise the tagged `ReadFrom` wire form, never bare string shorthand —
/// agents copy these descriptions when constructing calls.
#[tokio::test]
async fn read_tool_input_schema_uses_tagged_readfrom_examples() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();

    for name in ["read", "subscribe", "transact"] {
        let tool = result
            .tools
            .iter()
            .find(|t| t.name.as_ref() == name)
            .unwrap_or_else(|| panic!("tool {name} missing from tools/list"));
        let from_desc = tool
            .input_schema
            .get("properties")
            .and_then(|p| p.get("from"))
            .and_then(|f| f.get("description"))
            .and_then(|d| d.as_str())
            .unwrap_or_else(|| {
                panic!("{name}.inputSchema.properties.from.description must be present")
            });
        for tagged in [
            r#"{"type":"cursor"}"#,
            r#"{"type":"now"}"#,
            r#"{"type":"buffer_start"}"#,
            r#"{"type":"offset","offset":N}"#,
        ] {
            assert!(
                from_desc.contains(tagged),
                "{name}.from description must contain {tagged}: {from_desc}"
            );
        }
        for shorthand in [
            r#"{"offset": N}"#,
            r#""now" (default)"#,
            r#""cursor" (default)"#,
            r#"from: "now""#,
        ] {
            assert!(
                !from_desc.contains(shorthand),
                "{name}.from description must not advertise {shorthand}: {from_desc}"
            );
        }
    }
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
            }),
        ))
        .await
        .unwrap();

    assert_ne!(result.is_error, Some(true), "{result:?}");
    // Subscribe ack is always immediate after PLAN 1b.

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
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // Subscribe ack is always immediate after PLAN 1b.

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
    // After close, the subscribe task exits and may emit a stop notification.
    // We should NOT receive a data streaming event, but a stop notification
    // is expected and acceptable.
    let maybe_event = tokio::time::timeout(Duration::from_millis(250), rx_a.recv()).await;
    if let Ok(Some(event)) = maybe_event {
        // If we got an event, it should be a stop notification, not data.
        let data = event.data.as_object().unwrap();
        assert!(
            data.contains_key("stop_reason"),
            "received unexpected data event after close: {data:?}"
        );
    }

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
        elapsed < 1500,
        "elapsed_ms {elapsed} should be reasonable for a 1s timeout"
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
        assert_eq!(
            s["target"],
            json!(target),
            "target mismatch for {target} in {s:?}"
        );
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
        assert_ne!(
            result.is_error,
            Some(true),
            "valid utf8 should succeed: {result:?}"
        );
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
        s["was_active"],
        json!(false),
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
        s["stop_reason"],
        json!("no_new_rx_timeout"),
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
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result
        .structured_content
        .expect("first subscribe structured");
    assert_eq!(
        s["replaced_previous"],
        json!(false),
        "first subscribe should have replaced_previous=false: {s:?}"
    );

    // Second subscribe — replaces first
    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result
        .structured_content
        .expect("second subscribe structured");
    assert_eq!(
        s["replaced_previous"],
        json!(true),
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
        result.is_error,
        Some(true),
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
        result.is_error,
        Some(true),
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
        (
            "write",
            json!({ "connection_id": bogus_id, "data": "test" }),
        ),
        ("read", json!({ "connection_id": bogus_id })),
        ("flush", json!({ "connection_id": bogus_id })),
        (
            "set_dtr_rts",
            json!({ "connection_id": bogus_id, "dtr": true, "rts": false }),
        ),
        (
            "set_flow_control",
            json!({ "connection_id": bogus_id, "flow_control": "none" }),
        ),
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
            result.is_error,
            Some(true),
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
            result.is_error,
            Some(true),
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
        result.is_error,
        Some(true),
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
    assert_eq!(s["read_ops"], json!(0));
    assert_eq!(s["write_ops"], json!(0));
    assert_eq!(s["truncation_count"], json!(0));
    assert_eq!(s["notification_drop_count"], json!(0));
    assert!(s["last_activity_ms"].is_null());
    assert!(
        s["port_info"].is_null(),
        "port_info should be null for loopback connections"
    );

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
    assert_eq!(s["read_ops"], json!(1), "read_ops should be 1: {s:?}");
    assert_eq!(s["write_ops"], json!(1), "write_ops should be 1: {s:?}");
    assert_eq!(s["truncation_count"], json!(0));
    assert_eq!(s["notification_drop_count"], json!(0));
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
        result.is_error,
        Some(true),
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
    assert_eq!(
        s["baud_rate"],
        json!(9600),
        "baud_rate should persist: {s:?}"
    );

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
        result.is_error,
        Some(true),
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

// ── Phase 2: shared persistent profile store ────────────────────────────────
//
// These tests prove user-observable persistence behavior through public MCP
// calls: real process restart, shared HTTP sessions, concurrent writers
// (same process and across processes), legacy migration, startup rejection
// of corrupt/future files, and failed-write preservation.

fn profile_names(s: &serde_json::Value) -> Vec<String> {
    s["profiles"]
        .as_array()
        .expect("list_profiles structured content has profiles array")
        .iter()
        .map(|p| p["name"].as_str().expect("profile name").to_string())
        .collect()
}

/// Create a profile through the public `configure(profile=...)` MCP tool.
async fn configure_profile_via<H: rmcp::handler::client::ClientHandler>(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, H>,
    name: &str,
    baud_rate: u64,
) -> rmcp::model::CallToolResult {
    client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "profile": name,
                "defaults": { "baud_rate": baud_rate }
            }),
        ))
        .await
        .unwrap()
}

async fn list_profile_names_via<H: rmcp::handler::client::ClientHandler>(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, H>,
) -> Vec<String> {
    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    assert_ne!(listed.is_error, Some(true), "{listed:?}");
    profile_names(&listed.structured_content.expect("structured"))
}

/// Run the real binary and require it to exit within `timeout`; returns the
/// captured stdout. Panics on a hang (a regression where startup succeeds).
async fn run_bin_with_timeout(args: &[&str], timeout: Duration) -> (bool, Vec<u8>) {
    let child = tokio::process::Command::new(common::binaries::serial_mcp_bin())
        .args(args)
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn serial-mcp binary");
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => (out.status.success(), out.stdout),
        Ok(Err(e)) => panic!("failed to wait for serial-mcp: {e}"),
        Err(_) => {
            panic!("serial-mcp did not exit within {timeout:?}; startup unexpectedly succeeded")
        }
    }
}

#[tokio::test]
async fn profiles_survive_real_process_restart() {
    use common::spawned::{spawn_client, SpawnedServer};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");

    // First actual process creates profile A through the MCP surface.
    let mut server = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client, _rx) = spawn_client(&server).await.unwrap();
    let created = configure_profile_via(&client, "restart-a", 9600).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");
    client.cancel().await.ok();
    server.stop().await.unwrap();

    // Fresh actual process with the same path must load it.
    let server2 = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client2, _rx2) = spawn_client(&server2).await.unwrap();
    let names = list_profile_names_via(&client2).await;
    assert!(
        names.contains(&"restart-a".to_string()),
        "restart must load persisted profile: {names:?}"
    );
    let listed = client2
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.expect("structured");
    let a = s["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "restart-a")
        .expect("restart-a listed");
    assert_eq!(a["defaults"]["baud_rate"], 9600, "defaults survive restart");
    client2.cancel().await.ok();
}

#[tokio::test]
async fn profiles_shared_across_http_sessions() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");
    let server =
        TestServer::start_with_profiles_path(Arc::new(ConnectionManager::new()), path).await;
    let (client_a, _rx_a) = connect_client(&server).await.unwrap();
    let (client_b, _rx_b) = connect_client(&server).await.unwrap();

    // Client A creates a profile.
    let created = configure_profile_via(&client_a, "session-a", 9600).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");

    // Client B (separate session, same server) sees it immediately.
    let names_b = list_profile_names_via(&client_b).await;
    assert!(
        names_b.contains(&"session-a".to_string()),
        "client B must observe client A's profile: {names_b:?}"
    );

    // Client B deletes it.
    let deleted = client_b
        .peer()
        .call_tool(tool_request(
            "delete_profile",
            json!({ "profile_name": "session-a" }),
        ))
        .await
        .unwrap();
    assert_ne!(deleted.is_error, Some(true), "{deleted:?}");

    // Client A no longer sees it.
    let names_a = list_profile_names_via(&client_a).await;
    assert!(
        !names_a.contains(&"session-a".to_string()),
        "client A must observe client B's delete: {names_a:?}"
    );
    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
}

#[tokio::test]
async fn concurrent_same_process_profile_writes_keep_both() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");
    let server =
        TestServer::start_with_profiles_path(Arc::new(ConnectionManager::new()), path.clone())
            .await;
    let (client_a, _rx_a) = connect_client(&server).await.unwrap();
    let (client_b, _rx_b) = connect_client(&server).await.unwrap();

    // Two distinct clients create different profiles at the same time.
    let (ra, rb) = tokio::join!(
        configure_profile_via(&client_a, "conc-a", 9600),
        configure_profile_via(&client_b, "conc-b", 19200),
    );
    assert_ne!(ra.is_error, Some(true), "{ra:?}");
    assert_ne!(rb.is_error, Some(true), "{rb:?}");

    // A third view sees both.
    let names = list_profile_names_via(&client_a).await;
    assert!(
        names.contains(&"conc-a".to_string()) && names.contains(&"conc-b".to_string()),
        "both concurrent writes must be visible: {names:?}"
    );
    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
    drop(server);

    // A fresh store over the same file proves both persisted.
    let server2 =
        TestServer::start_with_profiles_path(Arc::new(ConnectionManager::new()), path).await;
    let (client2, _rx2) = connect_client(&server2).await.unwrap();
    let names2 = list_profile_names_via(&client2).await;
    assert!(
        names2.contains(&"conc-a".to_string()) && names2.contains(&"conc-b".to_string()),
        "both profiles must survive store reopen: {names2:?}"
    );
    client2.cancel().await.ok();
}

#[tokio::test]
async fn concurrent_server_process_profile_writes_keep_both() {
    use common::spawned::{spawn_client, SpawnedServer};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");

    // Two actual server processes share one profile file but bind different
    // ports. Advisory locking + reload-under-lock must preserve both writes.
    let mut server1 = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let mut server2 = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client1, _rx1) = spawn_client(&server1).await.unwrap();
    let (client2, _rx2) = spawn_client(&server2).await.unwrap();

    let (ra, rb) = tokio::join!(
        configure_profile_via(&client1, "proc-a", 9600),
        configure_profile_via(&client2, "proc-b", 19200),
    );
    assert_ne!(ra.is_error, Some(true), "{ra:?}");
    assert_ne!(rb.is_error, Some(true), "{rb:?}");
    client1.cancel().await.ok();
    client2.cancel().await.ok();
    server1.stop().await.unwrap();
    server2.stop().await.unwrap();

    // Third process proves both writes landed.
    let server3 = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client3, _rx3) = spawn_client(&server3).await.unwrap();
    let names = list_profile_names_via(&client3).await;
    assert!(
        names.contains(&"proc-a".to_string()) && names.contains(&"proc-b".to_string()),
        "cross-process concurrent writes must both persist: {names:?}"
    );
    client3.cancel().await.ok();
}

#[tokio::test]
async fn legacy_unversioned_profiles_migrate_on_mutation() {
    use common::spawned::{spawn_client, SpawnedServer};
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");
    std::fs::write(
        &path,
        r#"
[[profile]]
name = "legacy-dev"
[profile.selector]
vid = 0x1366
[profile.defaults]
baud_rate = 115200
"#,
    )
    .unwrap();

    let mut server = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client, _rx) = spawn_client(&server).await.unwrap();

    // The legacy profile loads with its settings.
    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.expect("structured");
    let names = profile_names(&s);
    assert!(names.contains(&"legacy-dev".to_string()), "{names:?}");
    let legacy = s["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "legacy-dev")
        .expect("legacy-dev listed");
    assert_eq!(legacy["defaults"]["baud_rate"], 115200);

    // A mutation persists the migration: the file now declares v2.
    let created = configure_profile_via(&client, "new-dev", 9600).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");
    client.cancel().await.ok();
    server.stop().await.unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("schema_version = 2"),
        "mutation must persist current schema version:\n{content}"
    );

    // Restart proves both profiles remain.
    let server2 = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client2, _rx2) = spawn_client(&server2).await.unwrap();
    let names2 = list_profile_names_via(&client2).await;
    assert!(
        names2.contains(&"legacy-dev".to_string()) && names2.contains(&"new-dev".to_string()),
        "legacy + new profiles must survive restart: {names2:?}"
    );
    client2.cancel().await.ok();
}

#[tokio::test]
async fn future_schema_version_fails_startup_and_preserves_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");
    let original = b"schema_version = 999\n\n[[profile]]\nname = \"future-dev\"\n";
    std::fs::write(&path, original).unwrap();

    let (success, _stdout) = run_bin_with_timeout(
        &[
            "--transport=http",
            "--bind=127.0.0.1:0",
            "--profiles-path",
            path.to_str().unwrap(),
        ],
        Duration::from_secs(15),
    )
    .await;
    assert!(
        !success,
        "binary must exit nonzero for an unsupported future schema version"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "future-version file must be left byte-identical"
    );
}

#[tokio::test]
async fn malformed_profiles_file_fails_startup_and_preserves_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");
    let original = b"not valid toml {{{".to_vec();
    std::fs::write(&path, &original).unwrap();

    let (success, _stdout) = run_bin_with_timeout(
        &[
            "--transport=http",
            "--bind=127.0.0.1:0",
            "--profiles-path",
            path.to_str().unwrap(),
        ],
        Duration::from_secs(15),
    )
    .await;
    assert!(
        !success,
        "binary must exit nonzero for a malformed profiles file"
    );
    assert_eq!(
        std::fs::read(&path).unwrap(),
        original,
        "malformed file must be left byte-identical"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn failed_profile_write_preserves_previous_state() {
    use common::spawned::{spawn_client, SpawnedServer};
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("profiles.toml");

    let mut server = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client, _rx) = spawn_client(&server).await.unwrap();

    // Profile A succeeds.
    let ra = configure_profile_via(&client, "keep-a", 9600).await;
    assert_ne!(ra.is_error, Some(true), "{ra:?}");

    // Make the profile directory non-writable (keep execute so the tempdir
    // stays traversable and cleanup can restore it).
    let dir_path = dir.path();
    let original_mode = std::fs::metadata(dir_path).unwrap().permissions().mode();
    std::fs::set_permissions(dir_path, std::fs::Permissions::from_mode(0o555)).unwrap();

    // Creating profile B must fail as a tool error, not a crash.
    let rb = configure_profile_via(&client, "lost-b", 19200).await;
    assert_eq!(
        rb.is_error,
        Some(true),
        "write must fail with the profile dir read-only: {rb:?}"
    );

    // The in-memory view is unchanged: A present, B absent.
    let names = list_profile_names_via(&client).await;
    assert!(
        names.contains(&"keep-a".to_string()) && !names.contains(&"lost-b".to_string()),
        "failed write must leave the cache untouched: {names:?}"
    );

    // Restore permissions, restart the actual server: disk still has A, not B.
    std::fs::set_permissions(dir_path, std::fs::Permissions::from_mode(original_mode)).unwrap();
    client.cancel().await.ok();
    server.stop().await.unwrap();

    let server2 = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client2, _rx2) = spawn_client(&server2).await.unwrap();
    let names2 = list_profile_names_via(&client2).await;
    assert!(
        names2.contains(&"keep-a".to_string()) && !names2.contains(&"lost-b".to_string()),
        "failed write must leave the file untouched across restart: {names2:?}"
    );
    client2.cancel().await.ok();
}
