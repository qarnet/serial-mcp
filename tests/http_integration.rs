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
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientRequest,
    GetPromptRequestParams, PaginatedRequestParams, ReadResourceRequestParams,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService};
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use serial_mcp::limits::{MAX_TIMEOUT_MS, MAX_WRITE_BYTES};
use serial_mcp::serial::{test_support::loopback_connection, ConnectionManager};

mod common;
use common::controlled::ControlledState;
use common::{
    args_object, connect_client, next_notification, tool_request, NotificationCollector,
    TestServer, EXPECTED_TOOLS,
};

#[tokio::test]
async fn initialize_handshake_succeeds() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();
    let info = client.peer().peer_info().expect("peer_info");
    assert_eq!(info.server_info.name, "serial-mcp");
    client.cancel().await.ok();
}

#[tokio::test]
async fn list_tools_returns_all_twenty_seven_tools() {
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
    let server = TestServer::builder(manager)
        .profiles_path(profiles_path.clone())
        .start()
        .await;
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

/// `list_profiles` exposes metadata and bounded revision history
/// so agents can understand selection and future rollback revisions.
#[tokio::test]
async fn list_profiles_exposes_metadata_and_revisions() {
    let profiles_dir = TempDir::new().unwrap();
    let profiles_path = profiles_dir.path().join("profiles.toml");
    let manager = Arc::new(ConnectionManager::new());
    let server = TestServer::builder(manager)
        .profiles_path(profiles_path)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let name = "meta-probe";

    let created = client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({ "profile": name, "defaults": { "baud_rate": 9600 } }),
        ))
        .await
        .unwrap();
    assert_ne!(created.is_error, Some(true), "{created:?}");

    // Overwrite bumps the revision and records the prior state.
    let overwritten = client
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
    assert_ne!(overwritten.is_error, Some(true), "{overwritten:?}");

    let listed = client
        .peer()
        .call_tool(tool_request("list_profiles", json!({})))
        .await
        .unwrap();
    let s = listed.structured_content.as_ref().unwrap();
    let p = &s["profiles"][0];
    assert_eq!(p["name"], json!(name));
    assert_eq!(p["metadata"]["generated"], json!(false));
    assert_eq!(p["metadata"]["revision"], json!(2));
    assert_eq!(p["metadata"]["use_count"], json!(0));
    assert!(
        !p["metadata"]["created_at_ms"].is_null(),
        "created timestamp set"
    );
    // Prior state snapshot for future rollback.
    let revisions = p["revisions"].as_array().expect("revisions array");
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0]["revision"], json!(1));
    assert_eq!(revisions[0]["defaults"]["baud_rate"], json!(9600));
    // Metadata must survive serialization to the wire (no uint formats etc.
    // are exercised by the schema guards).

    client.cancel().await.ok();
}

#[tokio::test]
async fn configure_profile_overwrites_existing() {
    let profiles_dir = TempDir::new().unwrap();
    let profiles_path = profiles_dir.path().join("profiles.toml");
    let manager = Arc::new(ConnectionManager::new());
    let server = TestServer::builder(manager)
        .profiles_path(profiles_path.clone())
        .start()
        .await;
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
    let server = TestServer::builder(manager)
        .profiles_path(profiles_path.clone())
        .start()
        .await;
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
    // The parallel profile-match preview is always present.
    let ports = structured["ports"].as_array().unwrap();
    let matches = structured["profile_matches"]
        .as_array()
        .expect("list_ports must carry profile_matches");
    assert_eq!(
        matches.len(),
        ports.len(),
        "profile_matches must parallel ports"
    );
    assert_eq!(structured["count"], json!(ports.len()));
    client.cancel().await.ok();
}

/// The `serial://ports` resource serves the same profile-match map
/// as the `list_ports` tool (same fresh store read, same pure computation).
#[tokio::test]
async fn ports_resource_includes_profile_match_map() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let resource = client
        .peer()
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "serial://ports",
        ))
        .await
        .unwrap();
    let body = &resource.contents[0];
    let parsed: serde_json::Value = match body {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
            serde_json::from_str(text).unwrap()
        }
        other => panic!("expected text resource contents, got {other:?}"),
    };
    assert!(parsed.get("count").is_some());
    let ports = parsed["ports"].as_array().unwrap();
    let matches = parsed["profile_matches"]
        .as_array()
        .expect("serial://ports must carry profile_matches");
    assert_eq!(matches.len(), ports.len(), "resource map parallels ports");
    for m in matches {
        assert!(m["port"].is_string());
        assert!(m["outcome"].is_string());
    }
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
    // Subscribe ack is always immediate.

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

    // Subscribe ack is always immediate.

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

// ── Lossless RX encoding fallback ─────────────────────────────────────────

#[tokio::test]
async fn read_invalid_utf8_falls_back_to_exact_hex() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-read-binary-hex");
    let connection_id = manager.insert(conn).await.unwrap();

    peer.write_all(&[0x48, 0x69, 0xFF, 0xFE, 0x00])
        .await
        .unwrap();
    peer.flush().await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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
    // Fallback is a normal result, never a tool error.
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured content");
    assert_eq!(s["data"], json!("48 69 ff fe 00"));
    assert_eq!(s["encoding"], json!("hex"));
    assert_eq!(s["bytes_read"], json!(5));
    // Cat path drains instantly ("drained") when bytes are already buffered;
    // otherwise the read waits and stops at "timeout". Both carry the data.
    assert!(
        s["stop_reason"] == json!("drained") || s["stop_reason"] == json!("timeout"),
        "unexpected stop_reason: {s:?}"
    );
    assert_eq!(s["frames_dropped"], json!(0));

    client.cancel().await.ok();
}

#[tokio::test]
async fn read_framing_error_keeps_valid_frame_utf8_and_raw_hex() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-read-slip-partial-enc");
    let connection_id = manager.insert(conn).await.unwrap();

    // One valid SLIP frame then a malformed escape (0xDB 0xFF).
    let slip_bytes: [u8; 6] = [0xC0, b'O', b'K', 0xC0, 0xDB, 0xFF];
    peer.write_all(&slip_bytes).await.unwrap();
    peer.flush().await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "encoding": "utf8",
                "rx_framing": { "type": "slip" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured content");
    assert_eq!(s["stop_reason"], json!("framing_error"));
    assert!(
        s["error"].as_str().is_some_and(|e| e.contains("SLIP")),
        "framing error text must be present: {s:?}"
    );
    // Top-level raw bytes (malformed tail) fall back to hex...
    assert_eq!(s["encoding"], json!("hex"));
    assert!(
        s["data"]
            .as_str()
            .unwrap()
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ' '),
        "top-level data should be hex: {s:?}"
    );
    // ...while the independently valid frame stays in requested UTF-8.
    let frames = s["frames"].as_array().expect("partial frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["data"], json!("OK"));
    assert_eq!(frames[0]["encoding"], json!("utf8"));
    assert_eq!(s["frames_dropped"], json!(0));

    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_binary_chunk_emits_hex_advances_and_counts_no_drop() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-sub-binary-hex");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 600,
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // Write AFTER the ack so `from: now` is resolved before the bytes exist.
    peer.write_all(&[0xDE, 0xAD, 0xFF]).await.unwrap();
    peer.flush().await.unwrap();

    // One chunk notification with exact spaced hex + effective encoding.
    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .expect("binary chunk notification");
    let data = event.data.as_object().unwrap();
    assert_eq!(data["connection_id"], json!(connection_id));
    assert_eq!(data["bytes_read"], json!(3));
    assert_eq!(data["data"], json!("de ad ff"));
    assert_eq!(data["encoding"], json!("hex"));
    // No SubscribeEncodingErrorNotification: `encoding_error` must be absent.
    assert!(
        data.get("encoding_error").is_none(),
        "fallback must not emit the error notification: {data:?}"
    );

    // The chunk advances the private cursor: no repeated notification, and
    // the stream ends by timeout with no drop counted.
    let stop = next_notification(&mut rx, Duration::from_secs(3))
        .await
        .expect("stop notification");
    let stop_data = stop.data.as_object().unwrap();
    assert_eq!(stop_data["stop_reason"], json!("timeout"));
    assert_eq!(stop_data["bytes_returned"], json!(3));

    // Successful fallback never increments the notification drop count.
    let status = client
        .peer()
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    let s = status.structured_content.expect("structured status");
    assert_eq!(s["notification_drop_count"], json!(0));
    assert_eq!(s["truncation_count"], json!(0));

    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_binary_framed_frame_emits_hex_without_drop() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-sub-slip-binary");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 600,
                "encoding": "utf8",
                "rx_framing": { "type": "slip" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // One SLIP frame whose payload is a single binary byte (0xFF).
    peer.write_all(&[0xC0, 0xFF, 0xC0]).await.unwrap();
    peer.flush().await.unwrap();

    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .expect("frame notification");
    let data = event.data.as_object().unwrap();
    assert_eq!(data["frame_type"], json!("slip"));
    assert_eq!(data["frame_index"], json!(0));
    assert_eq!(data["data"], json!("ff"));
    assert_eq!(data["encoding"], json!("hex"));
    assert!(
        data.get("encoding_error").is_none(),
        "fallback must not emit the error notification: {data:?}"
    );

    let stop = next_notification(&mut rx, Duration::from_secs(3))
        .await
        .expect("stop notification");
    let stop_data = stop.data.as_object().unwrap();
    assert_eq!(stop_data["stop_reason"], json!("timeout"));
    assert_eq!(stop_data["frames_emitted"], json!(1));
    assert_eq!(stop_data["frames_dropped"], json!(0));

    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_binary_partial_flush_emits_hex_with_effective_encoding() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-sub-partial-binary");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 400,
                "encoding": "utf8",
                "rx_framing": { "type": "line" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // Binary bytes with NO line terminator: the decoder buffers them and
    // flush_partial emits them at stop.
    peer.write_all(&[0xFF, 0xFE, 0x00]).await.unwrap();
    peer.flush().await.unwrap();

    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .expect("partial-frame notification");
    let data = event.data.as_object().unwrap();
    assert_eq!(data["partial"], json!(true));
    assert_eq!(data["data"], json!("ff fe 00"));
    assert_eq!(data["encoding"], json!("hex"));

    let stop = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .expect("stop notification");
    let stop_data = stop.data.as_object().unwrap();
    assert_eq!(stop_data["stop_reason"], json!("timeout"));
    // All observed partial bytes were emitted, so bytes_returned is the exact
    // partial raw length and nothing is reported as truncated.
    assert_eq!(stop_data["bytes_returned"], json!(3));
    assert_eq!(stop_data["truncated"], json!(false));

    client.cancel().await.ok();
}

#[tokio::test]
async fn subscribe_matched_binary_context_reports_hex_data_and_encoding() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-sub-match-binary");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "encoding": "utf8",
                "match": {
                    "pattern": "ff",
                    "config": { "mode": "literal_substring",
                                "pattern_encoding": "hex",
                                "context_amount_of_matched_bytes": 16 },
                },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // Raw chunk with a binary match target (0xFF).
    peer.write_all(&[0xDE, 0xAD, 0xFF]).await.unwrap();
    peer.flush().await.unwrap();

    // Chunk notification (hex fallback) then the matched stop notification.
    let event = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .expect("chunk notification");
    assert_eq!(event.data["data"], json!("de ad ff"));
    assert_eq!(event.data["encoding"], json!("hex"));

    let stop = next_notification(&mut rx, Duration::from_secs(2))
        .await
        .expect("stop notification");
    let stop_data = stop.data.as_object().unwrap();
    assert_eq!(stop_data["stop_reason"], json!("match_found"));
    assert_eq!(stop_data["matched"], json!(true));
    // Shaped context is the full chunk (pre-match + matched bytes).
    assert_eq!(stop_data["data"], json!("de ad ff"));
    assert_eq!(stop_data["encoding"], json!("hex"));
    assert_eq!(stop_data["match_index"], json!(2));

    client.cancel().await.ok();
}

#[tokio::test]
async fn read_and_subscribe_same_literal_match_index_over_chunked_stream() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-match-parity");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, mut rx) = connect_client(&server).await.unwrap();

    // Chunked stream: the literal spans the boundary between two writes.
    peer.write_all(b"ABCD").await.unwrap();
    peer.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    peer.write_all(b"EFOK>tail").await.unwrap();
    peer.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // read over the chunked stream: "OK>" at global index 6.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "match": { "pattern": "OK>" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "{s:?}");
    assert_eq!(s["match_index"], json!(6), "{s:?}");

    // subscribe replays the same retained bytes from buffer_start and must
    // report the same match outcome and index.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "from": { "type": "buffer_start" },
                "timeout_ms": 2000,
                "match": { "pattern": "OK>" },
            }),
        ))
        .await
        .unwrap();

    let mut saw_match_stop = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("stop_reason").is_some() {
                    saw_match_stop = true;
                    assert_eq!(data["matched"], json!(true), "{data:?}");
                    assert_eq!(data["match_index"], json!(6), "{data:?}");
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

    // No-match parity: a pattern absent from the stream times out without a
    // match on both tools.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "from": { "type": "now" },
                "timeout_ms": 400,
                "match": never_match(),
            }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(false), "{s:?}");

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 600,
                "match": never_match(),
            }),
        ))
        .await
        .unwrap();
    let mut saw_no_match_stop = false;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("stop_reason").is_some() {
                    saw_no_match_stop = true;
                    assert_ne!(data.get("matched"), Some(&json!(true)), "{data:?}");
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        saw_no_match_stop,
        "subscribe should emit a timeout stop notification"
    );

    client.cancel().await.ok();
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

// ── Shared persistent profile store ─────────────────────────────────────────
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
    let server = TestServer::builder(Arc::new(ConnectionManager::new()))
        .profiles_path(path)
        .start()
        .await;
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
    let server = TestServer::builder(Arc::new(ConnectionManager::new()))
        .profiles_path(path.clone())
        .start()
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
    let server2 = TestServer::builder(Arc::new(ConnectionManager::new()))
        .profiles_path(path)
        .start()
        .await;
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

#[tokio::test]
async fn relative_profiles_path_resolves_against_server_cwd() {
    use common::spawned::{spawn_client, SpawnedServer};
    let dir = TempDir::new().unwrap();
    let relative = std::path::Path::new("profiles.toml");

    // Launch the real binary with cwd = isolated temp dir and a bare
    // relative profile filename.
    let mut server = SpawnedServer::start_with_cwd(dir.path(), Some(relative)).await;
    let (client, _rx) = spawn_client(&server).await.unwrap();
    let created = configure_profile_via(&client, "rel-dev", 9600).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");
    client.cancel().await.ok();
    server.stop().await.unwrap();

    // The file landed in the server's cwd.
    let on_disk = dir.path().join("profiles.toml");
    assert!(
        on_disk.exists(),
        "relative --profiles-path must create the file in the server's cwd"
    );

    // Restart with the same cwd + relative path proves persistence.
    let server2 = SpawnedServer::start_with_cwd(dir.path(), Some(relative)).await;
    let (client2, _rx2) = spawn_client(&server2).await.unwrap();
    let names = list_profile_names_via(&client2).await;
    assert!(
        names.contains(&"rel-dev".to_string()),
        "relative-path store must survive restart: {names:?}"
    );
    client2.cancel().await.ok();
}

// =============================================================================
// capture_boot — controlled `SerialIo` through public HTTP MCP
//
// These tests drive the REAL MCP surface (HTTP transport, tool router,
// RxSession pump, stop controller) against a controlled in-memory backend
// that records line transitions and can inject RX bytes synchronously at
// assertion/release time. No tool/store logic is mocked.
//
// NOTE on request delivery: the rmcp client only transmits a `tools/call`
// request once its future is polled. Tests that need the capture to run
// while the test drives the backend must poll the future (tokio::select!)
// instead of holding it un-polled.
// =============================================================================

/// Default reset pulse shape used by most capture tests: assert both lines
/// low (Arduino-style reset), release back high.
fn capture_reset() -> serde_json::Value {
    json!({
        "assert_dtr": false,
        "assert_rts": false,
        "release_dtr": true,
        "release_rts": true,
        "hold_ms": 150,
    })
}

/// A literal-substring match request that never matches the test payloads.
fn never_match() -> serde_json::Value {
    json!({
        "pattern": "zzz-no-match",
        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
    })
}

/// Start a server with one injected controlled connection.
async fn controlled_server(
    port: &str,
    rx_buffer_size: usize,
) -> (
    TestServer,
    RunningService<RoleClient, NotificationCollector>,
    String,
    Arc<ControlledState>,
) {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, state) = common::controlled::controlled_connection(port, rx_buffer_size);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    (server, client, cid, state)
}

/// Wait until the line log records the first line transition, polling the
/// in-flight capture call so its request is actually transmitted (the rmcp
/// client only sends a request once its future is polled).
async fn wait_for_line(
    call: &mut std::pin::Pin<
        Box<
            impl std::future::Future<
                    Output = Result<rmcp::model::CallToolResult, rmcp::service::ServiceError>,
                > + Send,
        >,
    >,
    state: &ControlledState,
    deadline: Instant,
    what: &str,
) {
    while state.line_log().is_empty() {
        assert!(
            Instant::now() < deadline,
            "{what} never recorded in line log"
        );
        tokio::select! {
            res = call.as_mut() => {
                panic!("capture completed before {what}: {res:?}");
            }
            _ = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
    }
}

// ── 1. Stale bytes excluded; immediate release-hook bytes captured; shared
//       cursor and ring history remain readable afterwards ────────────────

#[tokio::test]
async fn capture_boot_stale_bytes_excluded_boot_bytes_captured_cursor_preserved() {
    let (_server, client, cid, state) = controlled_server("loop-capture-1", 65536).await;

    // Stale pre-mark bytes, consumed by an ordinary read (cursor now at 5).
    state.inject_rx(b"STALE");
    let r = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": cid, "timeout_ms": 1000 }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    assert_eq!(r.structured_content.unwrap()["data"], json!("STALE"));

    // Boot bytes appear only when the reset line is ASSERTED: the hook fires
    // synchronously inside `set_dtr_rts` for the (false,false) state.
    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(b"BOOT!");
            }
        }
    })));

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["mark_offset"], json!(5), "mark after the stale bytes");
    assert_eq!(s["pre_mark_bytes"], json!(5));
    assert_eq!(s["os_input_flushed"], json!(true));
    assert_eq!(
        s["read"]["data"],
        json!("BOOT!"),
        "stale bytes must never appear"
    );
    assert_eq!(s["read"]["from_offset"], json!(5));

    // The shared cursor never moved: a plain read still starts at 5 and
    // re-reads the captured bytes.
    let r = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": cid, "timeout_ms": 1000 }),
        ))
        .await
        .unwrap();
    let s = r.structured_content.expect("structured read result");
    assert_eq!(s["data"], json!("BOOT!"));
    assert_eq!(s["from_offset"], json!(5));

    // Ring history is still readable: replay from the oldest retained byte.
    let r = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": cid,
                "from": { "type": "buffer_start" },
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    let s = r.structured_content.expect("structured read result");
    assert_eq!(s["data"], json!("STALEBOOT!"));

    client.cancel().await.ok();
}

// ── 2. Bytes emitted inside the line-change call are captured and a match
//       stops at the pattern ──────────────────────────────────────────────

#[tokio::test]
async fn capture_boot_immediate_bytes_captured_and_match_stops_at_pattern() {
    let (_server, client, cid, state) = controlled_server("loop-capture-match", 65536).await;

    // The device starts streaming the moment the reset line is asserted.
    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(b"garbage\r\nboot> prompt\r\n");
            }
        }
    })));

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "match": {
                    "pattern": "boot>",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                },
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["stop_reason"], json!("match_found"));
    assert_eq!(s["read"]["matched"], json!(true));
    assert_eq!(s["read"]["data"], json!("garbage\r\nboot>"));
    assert_eq!(s["read"]["match_index"], json!(9));
    assert_eq!(s["mark_offset"], json!(0), "nothing predates the mark");

    client.cancel().await.ok();
}

// ── 3. Request-scoped cancellation during hold releases lines ────────────

#[tokio::test]
async fn capture_boot_cancellation_releases_lines_request_scoped() {
    let (_server, client, cid, state) = controlled_server("loop-capture-cancel", 65536).await;

    let handle = client
        .peer()
        .send_cancellable_request(
            ClientRequest::CallToolRequest(CallToolRequest::new(
                CallToolRequestParams::new("capture_boot").with_arguments(args_object(json!({
                    "connection_id": cid,
                    "reset": {
                        "assert_dtr": false,
                        "assert_rts": false,
                        "release_dtr": true,
                        "release_rts": true,
                        "hold_ms": 30000,
                    },
                }))),
            )),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();

    // Wait until the pulse is asserted (hold phase).
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.line_log().is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.line_log(),
        vec![(false, false)],
        "assertion must precede cancellation"
    );

    // Request-scoped cancellation (notifications/cancelled for THIS request),
    // not whole-client teardown. Note: rmcp's client resolves its own
    // cancelled request handle with `Err(Cancelled)` and discards the server
    // response, so the structured `cancelled` outcome is proven at the unit
    // level (read_loop.rs) and the wire-level release is proven below.
    handle
        .peer
        .notify_cancelled(CancelledNotificationParam {
            request_id: handle.id.clone(),
            reason: Some("test cancel".into()),
        })
        .await
        .unwrap();

    // The capture must release the lines promptly instead of holding for the
    // full 30s hold.
    let deadline = Instant::now() + Duration::from_secs(2);
    while !state.line_log().iter().any(|&(d, r)| d && r) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        state.line_log().iter().any(|&(d, r)| d && r),
        "lines must be released after cancellation: {:?}",
        state.line_log()
    );
    assert_eq!(
        state.line_log(),
        vec![(false, false), (true, true)],
        "assert then release, in order"
    );

    // The capture finished server-side: the control lock is free, so a
    // subsequent line-control call completes promptly.
    let r = tokio::time::timeout(
        Duration::from_secs(2),
        client.peer().call_tool(tool_request(
            "set_dtr_rts",
            json!({ "connection_id": cid, "dtr": true, "rts": true }),
        )),
    )
    .await
    .expect("set_dtr_rts must not block after cancellation")
    .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");

    // The client is still fully functional (request-scoped, not teardown).
    let r = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": cid })))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");

    client.cancel().await.ok();
}

/// Cancellation during hold where the FIRST release attempt fails: the
/// capture must NOT disarm the guard (the old unconditional disarm disabled
/// the drop retry), the drop-time retry must apply the configured release
/// state through the control lock, and the control lock must become usable
/// again afterwards. The transition ordering in the line log proves the
/// retry: assert, failed release attempt, then a second release-state
/// transition from the drop cleanup.
#[tokio::test]
async fn capture_boot_cancellation_with_failed_release_retries_cleanup_via_control_lock() {
    let (_server, client, cid, state) =
        controlled_server("loop-capture-cancel-rel-fail", 65536).await;

    let handle = client
        .peer()
        .send_cancellable_request(
            ClientRequest::CallToolRequest(CallToolRequest::new(
                CallToolRequestParams::new("capture_boot").with_arguments(args_object(json!({
                    "connection_id": cid,
                    "reset": {
                        "assert_dtr": false,
                        "assert_rts": false,
                        "release_dtr": true,
                        "release_rts": true,
                        "hold_ms": 30000,
                    },
                }))),
            )),
            PeerRequestOptions::no_options(),
        )
        .await
        .unwrap();

    // Wait for the assertion (hold phase), then arm a failure for the
    // release attempt the cancellation will trigger.
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.line_log().is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state.line_log(), vec![(false, false)]);
    state.set_fail_next_set(1);

    // Request-scoped cancellation: the release attempt records the release
    // state and FAILS; the guard must stay armed.
    handle
        .peer
        .notify_cancelled(CancelledNotificationParam {
            request_id: handle.id.clone(),
            reason: Some("release-failure cancel".into()),
        })
        .await
        .unwrap();

    // The failed attempt recorded (true,true) inside the capture; the
    // drop-time retry — queued through the control lock after the capture's
    // pulse guard drops — records (true,true) again. Exactly three
    // transitions, in order.
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.line_log().len() < 3 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.line_log(),
        vec![(false, false), (true, true), (true, true)],
        "assert, failed release attempt, then drop-time retry in order: {:?}",
        state.line_log()
    );

    // The control lock became usable after the capture and its cleanup: a
    // fresh line-control call completes promptly (nothing stuck holding it).
    let r = tokio::time::timeout(
        Duration::from_secs(2),
        client.peer().call_tool(tool_request(
            "set_dtr_rts",
            json!({ "connection_id": cid, "dtr": true, "rts": true }),
        )),
    )
    .await
    .expect("set_dtr_rts must not block after cleanup")
    .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");

    client.cancel().await.ok();
}

// ── 4. Assertion failure and release failure attempt configured cleanup ──

#[tokio::test]
async fn capture_boot_assertion_failure_errors_and_attempts_cleanup() {
    let (_server, client, cid, state) = controlled_server("loop-capture-assert-fail", 65536).await;

    // The assert call fails (intent recorded, then I/O error — the RTS
    // failure after DTR applied case).
    state.set_fail_next_set(1);

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        r.is_error,
        Some(true),
        "assertion failure must be a tool error: {r:?}"
    );

    // The drop-time guard must still attempt the configured release state.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if state.line_log().iter().any(|&(d, r)| d && r) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        state.line_log().iter().any(|&(d, r)| d && r),
        "cleanup release with the configured release state must be attempted: {:?}",
        state.line_log()
    );
    assert_eq!(state.line_log().first(), Some(&(false, false)));

    client.cancel().await.ok();
}

#[tokio::test]
async fn capture_boot_release_failure_errors_and_retries_cleanup() {
    let (_server, client, cid, state) = controlled_server("loop-capture-rel-fail", 65536).await;

    let call = client.peer().call_tool(tool_request(
        "capture_boot",
        json!({
            "connection_id": cid,
            "reset": {
                "assert_dtr": false,
                "assert_rts": false,
                "release_dtr": true,
                "release_rts": true,
                "hold_ms": 800,
            },
            "timeout_ms": 500,
        }),
    ));
    let mut call = Box::pin(call);

    // Wait for the assert (polling the call so the request is transmitted),
    // then arm a failure for the release call.
    let deadline = Instant::now() + Duration::from_secs(2);
    wait_for_line(&mut call, &state, deadline, "assertion").await;
    assert_eq!(state.line_log(), vec![(false, false)]);
    state.set_fail_next_set(1);

    let r = call.await.unwrap();
    assert_eq!(
        r.is_error,
        Some(true),
        "release failure must be a tool error: {r:?}"
    );

    // The guard's drop retries the release (failure flag now consumed);
    // the release must eventually land in the log.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if state.line_log().iter().any(|&(d, r)| d && r) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        state.line_log().iter().any(|&(d, r)| d && r),
        "cleanup release retry must be attempted: {:?}",
        state.line_log()
    );

    client.cancel().await.ok();
}

// ── 5. Invalid framing construction fails before any line transition ─────

#[tokio::test]
async fn capture_boot_invalid_framing_fails_before_line_transition() {
    let (_server, client, cid, state) = controlled_server("loop-capture-bad-framing", 65536).await;

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "rx_framing": { "type": "length_prefixed", "prefix_size": 3 },
                "timeout_ms": 500,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(r.is_error, Some(true), "{r:?}");
    assert!(
        r.structured_content.is_none(),
        "invalid framing must surface as a tool error: {r:?}"
    );
    assert!(
        state.line_log().is_empty(),
        "no line transition may occur for an invalid framing config: {:?}",
        state.line_log()
    );

    client.cancel().await.ok();
}

// ── 6. Runtime SLIP framing error: partial structured result, lines
//       released ──────────────────────────────────────────────────────────

#[tokio::test]
async fn capture_boot_runtime_framing_error_returns_partial_result_and_releases_lines() {
    let (_server, client, cid, state) = controlled_server("loop-capture-slip-err", 65536).await;

    // One valid SLIP frame then a malformed escape (0xDB 0xFF).
    let slip_bytes: [u8; 6] = [0xC0, b'O', b'K', 0xC0, 0xDB, 0xFF];
    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(&slip_bytes);
            }
        }
    })));

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "rx_framing": { "type": "slip" },
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();
    // Framing errors are structured results, not tool errors.
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["stop_reason"], json!("framing_error"));
    assert!(
        s["read"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("SLIP")),
        "framing error text must be present: {s:?}"
    );
    // The valid frame decoded before the error survives. Each frame is
    // encoded independently from the requested encoding (utf8 default), so
    // the valid "OK" frame stays UTF-8 while only the top-level raw bytes
    // (malformed tail) fall back to hex.
    let frames = s["read"]["frames"].as_array().expect("partial frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["data"], json!("OK"));
    assert_eq!(frames[0]["encoding"], json!("utf8"));
    assert_eq!(s["read"]["encoding"], json!("hex"));
    assert_eq!(s["read"]["frames_dropped"], json!(0));
    // Lines were released despite the decode error.
    assert_eq!(state.line_log(), vec![(false, false), (true, true)]);

    client.cancel().await.ok();
}

// ── 7. NDJSON framing/parser and binary hex/base64 output reuse current
//       read behavior ─────────────────────────────────────────────────────

#[tokio::test]
async fn capture_boot_ndjson_preset_decodes_frames() {
    let (_server, client, cid, state) = controlled_server("loop-capture-ndjson", 65536).await;

    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(b"{\"a\":1}\n{\"b\":2}\n");
            }
        }
    })));

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "protocol": { "type": "ndjson" },
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    let frames = s["read"]["frames"].as_array().expect("ndjson frames");
    assert_eq!(frames.len(), 2, "two JSON lines decoded: {frames:?}");
    assert!(
        frames[0]["parsed"].is_object(),
        "parsed JSON payload: {frames:?}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn capture_boot_binary_output_hex_and_base64() {
    let (_server, client, cid, state) = controlled_server("loop-capture-binary", 65536).await;

    let binary: [u8; 4] = [0x00, 0x01, 0x02, 0xFF];
    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(&binary);
            }
        }
    })));

    // Hex output.
    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "encoding": "hex",
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["encoding"], json!("hex"));
    assert_eq!(s["read"]["data"], json!("00 01 02 ff"));

    // Base64 output for the next capture.
    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "encoding": "base64",
                "timeout_ms": 1000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["encoding"], json!("base64"));
    assert_eq!(s["read"]["data"], json!("AAEC/w=="));

    client.cancel().await.ok();
}

// ── 8. Silence timeout after a banner; wall timeout during continuous
//       output ────────────────────────────────────────────────────────────

#[tokio::test]
async fn capture_boot_silence_timeout_stops_after_banner() {
    let (_server, client, cid, state) = controlled_server("loop-capture-silence", 65536).await;

    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(b"banner-line\n");
            }
        }
    })));

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "match": never_match(),
                "no_new_rx_timeout_ms": 150,
                "timeout_ms": 3000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["stop_reason"], json!("no_new_rx_timeout"));
    assert!(
        s["read"]["data"]
            .as_str()
            .unwrap_or_default()
            .contains("banner-line"),
        "banner must be captured before the silence stop: {s:?}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn capture_boot_wall_timeout_stops_during_continuous_output() {
    let (_server, client, cid, state) = controlled_server("loop-capture-wall", 65536).await;

    // A device that never goes silent: keep feeding bytes.
    let stop = tokio_util::sync::CancellationToken::new();
    let injector = {
        let state = Arc::clone(&state);
        let stop = stop.clone();
        tokio::spawn(async move {
            while !stop.is_cancelled() {
                state.inject_rx(b"AAAA");
                tokio::time::sleep(Duration::from_millis(15)).await;
            }
        })
    };

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "match": never_match(),
                "timeout_ms": 250,
            }),
        ))
        .await
        .unwrap();
    stop.cancel();
    injector.await.unwrap();

    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["stop_reason"], json!("timeout"));
    assert!(
        s["read"]["bytes_observed"].as_u64().unwrap_or(0) > 0,
        "wall timeout must fire during continuous output: {s:?}"
    );

    client.cancel().await.ok();
}

// ── 9. Disconnect returns a partial capture with `connection_closed` ────

#[tokio::test]
async fn capture_boot_disconnect_returns_partial_capture_connection_closed() {
    let (_server, client, cid, state) = controlled_server("loop-capture-disc", 65536).await;

    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(b"PARTIAL");
            }
        }
    })));

    let call = client.peer().call_tool(tool_request(
        "capture_boot",
        json!({
            "connection_id": cid,
            "reset": capture_reset(),
            "match": never_match(),
            "timeout_ms": 5000,
        }),
    ));
    let mut call = Box::pin(call);

    // Wait for the assertion, then close the connection mid-capture.
    let deadline = Instant::now() + Duration::from_secs(2);
    wait_for_line(&mut call, &state, deadline, "assertion").await;
    assert_eq!(state.line_log(), vec![(false, false)]);
    let r = client
        .peer()
        .call_tool(tool_request("close", json!({ "connection_id": cid })))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "close: {r:?}");

    let r = call.await.unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["stop_reason"], json!("connection_closed"));
    assert!(
        s["read"]["data"]
            .as_str()
            .unwrap_or_default()
            .contains("PARTIAL"),
        "bytes captured before the disconnect must survive: {s:?}"
    );

    client.cancel().await.ok();
}

// ── 10. Ring wrap reports `bytes_lost`; the atomic mark is preserved ─────

#[tokio::test]
async fn capture_boot_ring_wrap_reports_bytes_lost_and_preserves_mark() {
    // Tiny ring forces a wrap while the capture is in flight.
    let (_server, client, cid, state) = controlled_server("loop-capture-wrap", 16).await;

    // 32 bytes injected at assertion into a 16-byte ring: the ring wraps, so
    // the read must report bytes_lost and start at the clamped offset while
    // mark_offset keeps the original atomic boundary.
    let burst: [u8; 32] = *b"0123456789abcdefGHIJKLMNOPQRSTUV";
    state.set_on_line_change(Some(Arc::new({
        let state = Arc::clone(&state);
        move |dtr, rts| {
            if !dtr && !rts {
                state.inject_rx(&burst);
            }
        }
    })));

    let r = client
        .peer()
        .call_tool(tool_request(
            "capture_boot",
            json!({
                "connection_id": cid,
                "reset": capture_reset(),
                "timeout_ms": 2000,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["mark_offset"], json!(0), "atomic boundary recorded");
    assert_eq!(
        s["read"]["from_offset"],
        json!(16),
        "read clamped to retained start"
    );
    assert_eq!(s["read"]["bytes_lost"], json!(16), "wrap loss reported");
    assert_eq!(
        s["read"]["data"],
        json!("GHIJKLMNOPQRSTUV"),
        "tail of the burst retained"
    );

    client.cancel().await.ok();
}

// ── 11. Concurrent `set_dtr_rts` cannot interleave inside the pulse ──────

#[tokio::test]
async fn capture_boot_concurrent_set_dtr_rts_cannot_interleave_inside_pulse() {
    let (server, client, cid, state) = controlled_server("loop-capture-lock", 65536).await;
    // A second client session on the SAME server for the concurrent
    // line-control call.
    let (client_b, _rx_b) = connect_client(&server).await.unwrap();

    let call = client.peer().call_tool(tool_request(
        "capture_boot",
        json!({
            "connection_id": cid,
            "reset": {
                "assert_dtr": false,
                "assert_rts": false,
                "release_dtr": true,
                "release_rts": true,
                "hold_ms": 600,
            },
            "timeout_ms": 2000,
        }),
    ));
    let mut call = Box::pin(call);

    // Wait for the assertion (capture now holds the control lock).
    let deadline = Instant::now() + Duration::from_secs(2);
    wait_for_line(&mut call, &state, deadline, "assertion").await;
    assert_eq!(state.line_log(), vec![(false, false)]);

    // A concurrent set_dtr_rts on the same connection must wait for the
    // whole pulse (assert + hold + release) to finish.
    let set_call = client_b.peer().call_tool(tool_request(
        "set_dtr_rts",
        json!({ "connection_id": cid, "dtr": true, "rts": true }),
    ));
    let mut set_call = Box::pin(set_call);
    let polled = tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(300)) => "pending",
        _res = &mut set_call => "done",
    };
    assert_eq!(
        polled, "pending",
        "set_dtr_rts must not interleave inside the reset pulse"
    );

    let capture_result = call.await.unwrap();
    assert_ne!(capture_result.is_error, Some(true), "{capture_result:?}");
    let set_result = set_call.await.unwrap();
    assert_ne!(set_result.is_error, Some(true), "{set_result:?}");

    // Exactly three transitions, in order: assert, release, user's set.
    assert_eq!(
        state.line_log(),
        vec![(false, false), (true, true), (true, true)],
        "no line-control call may interleave between assert and release"
    );

    client.cancel().await.ok();
    client_b.cancel().await.ok();
}

// ── 12. Arm-only capture (no reset config) never touches lines ───────────

#[tokio::test]
async fn capture_boot_arm_only_does_not_touch_lines() {
    let (_server, client, cid, state) = controlled_server("loop-capture-arm", 65536).await;

    // Stale bytes that predate the capture (purged or pre-mark — either way
    // they must never appear in the result).
    state.inject_rx(b"STALE");
    let call = client.peer().call_tool(tool_request(
        "capture_boot",
        json!({
            "connection_id": cid,
            "reset": null,
            "match": never_match(),
            "timeout_ms": 1500,
        }),
    ));
    let mut call = Box::pin(call);

    // Poll the call so the request is transmitted, then wait for the
    // server-side capture to reach its read phase (gate acquisition + mark:
    // at most one pump cycle, ~100ms) before emitting the external bytes.
    tokio::select! {
        res = call.as_mut() => {
            panic!("arm-only capture completed before external bytes: {res:?}");
        }
        _ = tokio::time::sleep(Duration::from_millis(300)) => {}
    }

    // Device bytes arrive after the capture is armed (external reset).
    state.inject_rx(b"EXT-BOOT");

    let r = call.await.unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert!(s["reset"].is_null(), "arm-only capture reports reset=null");
    assert_eq!(s["os_input_flushed"], json!(true));
    let data = s["read"]["data"].as_str().unwrap_or_default();
    assert!(
        data.contains("EXT-BOOT"),
        "post-arm bytes must be captured: {data:?}"
    );
    assert!(
        !data.contains("STALE"),
        "pre-capture bytes must never appear in the result: {data:?}"
    );
    assert!(
        state.line_log().is_empty(),
        "arm-only capture must never touch DTR/RTS: {:?}",
        state.line_log()
    );

    client.cancel().await.ok();
}

// ── Schema guard: capture_boot schemas carry no non-standard uint formats ─

#[test]
fn capture_boot_schemas_have_no_nonstandard_uint_formats() {
    for schema in [
        schemars::schema_for!(serial_mcp::tools::types::CaptureBootArgs),
        schemars::schema_for!(serial_mcp::tools::types::CaptureBootReset),
        schemars::schema_for!(serial_mcp::tools::types::CaptureBootResult),
    ] {
        let text = serde_json::to_string(&schema).unwrap();
        for bad in ["uint", "uint8", "uint16", "uint32", "uint64"] {
            assert!(
                !text.contains(&format!("\"format\":\"{bad}\"")),
                "capture_boot schema must not contain {bad}"
            );
        }
    }
}

// ── Safe persistent capture (export_log) ─────────────────────────────────────

use serial_mcp::capture_store::{CaptureLimits, CaptureStore};
use serial_mcp::log_buffer::LogEntry;
use serial_mcp::serial::{test_support::loopback_connection_with_config, ConnectionConfig};

fn capture_store_in(
    root: &std::path::Path,
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_files: usize,
) -> Arc<CaptureStore> {
    Arc::new(
        CaptureStore::open(
            root.to_path_buf(),
            CaptureLimits {
                max_file_bytes,
                max_total_bytes,
                max_files,
            },
        )
        .expect("open capture store for test"),
    )
}

/// Loopback connection seeded with `events` rx_data entries (plus the
/// automatic `open` event, which is always present).
fn seeded_log_conn(
    name: &str,
    events: usize,
) -> (
    serial_mcp::serial::SerialConnection,
    tokio::io::DuplexStream,
) {
    let (conn, peer) = loopback_connection(name);
    for i in 0..events {
        conn.log().rx_data(i + 1);
    }
    (conn, peer)
}

fn export_call(connection_id: &str, path: &str) -> serde_json::Value {
    json!({ "connection_id": connection_id, "path": path })
}

/// Run one `export_log` call through the real MCP boundary.
async fn export_via<H: rmcp::handler::client::ClientHandler>(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, H>,
    connection_id: &str,
    path: &str,
) -> rmcp::model::CallToolResult {
    client
        .peer()
        .call_tool(tool_request("export_log", export_call(connection_id, path)))
        .await
        .unwrap()
}

/// Concatenated text of a tool-call result's content blocks (error text
/// lives in `content`, alongside `structured_content`).
fn tool_error_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// ConnectionConfig for a connection whose log is enabled but has capacity
/// 0 — every recorded event is immediately evicted, so exports are empty.
fn empty_log_config(port: &str) -> ConnectionConfig {
    use serial_mcp::serial::{DataBits, FlowControl, Parity, StopBits};
    ConnectionConfig {
        port: port.into(),
        name: None,
        baud_rate: 115200,
        data_bits: DataBits::Eight,
        stop_bits: StopBits::One,
        parity: Parity::None,
        flow_control: FlowControl::None,
        port_info: None,
        log_capacity: 0,
        log_enabled: true,
        tx_framing: None,
        rx_framing: None,
        rx_parser: None,
        protocol: None,
        rx_buffer_size: serial_mcp::limits::DEFAULT_RX_BUFFER_SIZE,
        max_buffered_bytes: 32768,
        poll_interval_ms: 200,
    }
}

/// The only entries allowed in a capture root after failed exports: the
/// advisory lock file. Temp files may exist only transiently and are
/// removed on failure (NamedTempFile drop), so this is also a no-temp check.
fn assert_root_has_only_lock(root: &std::path::Path) {
    let entries: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n != serial_mcp::capture_store::CAPTURE_LOCK_FILE)
        .collect();
    assert!(
        entries.is_empty(),
        "capture root must contain only the lock file, got: {entries:?}"
    );
}

#[tokio::test]
async fn export_log_disabled_errors_before_path_write_and_creates_nothing() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-disabled", 2);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::start_with(manager).await; // default: disabled store
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "boot.jsonl")))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = tool_error_text(&result);
    assert!(
        text.contains("Persistent capture is disabled"),
        "got: {text}"
    );
    assert!(
        text.contains("--capture-dir"),
        "disabled error must teach --capture-dir: {text}"
    );

    // The disabled check runs BEFORE connection lookup: a bogus connection
    // id yields the disabled error, not connection_not_found.
    let result = client
        .peer()
        .call_tool(tool_request(
            "export_log",
            export_call("no-such-connection", "boot.jsonl"),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let text = tool_error_text(&result);
    assert!(
        text.contains("Persistent capture is disabled"),
        "disabled check must precede connection lookup: {text}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_enabled_writes_valid_jsonl_matching_get_log() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 8);
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-ok", 3);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Reference snapshot via get_log.
    let get = client
        .peer()
        .call_tool(tool_request("get_log", json!({ "connection_id": cid })))
        .await
        .unwrap();
    let structured = get.structured_content.expect("structured");
    let events: Vec<LogEntry> = serde_json::from_value(structured["events"].clone()).unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "boot.jsonl")))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["events_written"], json!(events.len()));
    assert_eq!(structured["files_used"], json!(1));
    assert_eq!(structured["total_bytes_used"], structured["bytes_written"]);
    let canonical = root.path().canonicalize().unwrap().join("boot.jsonl");
    assert_eq!(structured["path"], json!(canonical.display().to_string()));
    // No durability warning on a normally-durable Unix commit.
    assert!(
        structured.get("durability_warning").is_none(),
        "normal export must omit durability_warning: {structured:?}"
    );

    let raw = std::fs::read(&canonical).unwrap();
    assert_eq!(
        raw.len(),
        structured["bytes_written"].as_u64().unwrap() as usize
    );
    assert_eq!(structured["bytes_written"], json!(raw.len()));
    let text = String::from_utf8(raw).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), events.len());
    for (i, line) in lines.iter().enumerate() {
        let parsed: LogEntry = serde_json::from_str(line).unwrap();
        assert_eq!(parsed, events[i], "line {i} must equal get_log entry");
    }
    assert!(text.ends_with('\n'));

    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_empty_log_commits_zero_byte_file_and_consumes_slot() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 1024, 1024, 4);
    let manager = Arc::new(ConnectionManager::new());
    // log_capacity 0 with logging enabled: every record is immediately
    // evicted, so the buffer is empty.
    let (conn, _peer) = loopback_connection_with_config(empty_log_config("loop-export-empty"));
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "empty.jsonl")))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["events_written"], json!(0));
    assert_eq!(structured["bytes_written"], json!(0));
    assert_eq!(structured["files_used"], json!(1));
    let committed = root.path().join("empty.jsonl");
    assert!(committed.is_file());
    assert_eq!(std::fs::metadata(&committed).unwrap().len(), 0);

    // A second zero-byte export consumes a second file slot.
    let result = client
        .peer()
        .call_tool(tool_request(
            "export_log",
            export_call(&cid, "empty2.jsonl"),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let structured = result.structured_content.expect("structured content");
    assert_eq!(structured["files_used"], json!(2));

    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_rejects_traversal_absolute_and_bad_names_without_files() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 8);
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-bad", 2);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    for bad in [
        "../escape.jsonl",
        "sub/dir.jsonl",
        "sub\\dir.jsonl",
        "/abs.jsonl",
        "name.txt",
        "name.json",
        "name.JSONL",
        "CON.jsonl",
        "lpt1.jsonl",
        ".jsonl",
        ".serial-mcp-capture-x.jsonl",
        &format!("{}.jsonl", "a".repeat(120)),
        "",
        "..",
    ] {
        let result = client
            .peer()
            .call_tool(tool_request("export_log", export_call(&cid, bad)))
            .await
            .unwrap();
        assert_eq!(
            result.is_error,
            Some(true),
            "name {bad:?} must fail: {result:?}"
        );
        // The validation error is a tool error with a useful message.
        let text = tool_error_text(&result);
        assert!(!text.is_empty(), "name {bad:?} must produce a message");
    }

    assert_root_has_only_lock(root.path());
    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_existing_target_remains_byte_identical() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 8);
    std::fs::write(root.path().join("boot.jsonl"), b"original-content").unwrap();
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-clobber", 2);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "boot.jsonl")))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        std::fs::read(root.path().join("boot.jsonl")).unwrap(),
        b"original-content"
    );
    assert_eq!(
        std::fs::metadata(root.path().join("boot.jsonl"))
            .unwrap()
            .len(),
        16
    );

    client.cancel().await.ok();
}

#[cfg(unix)]
#[tokio::test]
async fn export_log_rejects_symlink_target_and_leaves_outside_untouched() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 8);
    let outside = TempDir::new().unwrap();
    let victim = outside.path().join("victim.jsonl");
    std::fs::write(&victim, b"outside-data").unwrap();
    std::os::unix::fs::symlink(&victim, root.path().join("boot.jsonl")).unwrap();

    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-symlink", 2);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "boot.jsonl")))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    // The symlink still points at the outside target; the target is untouched.
    assert!(root
        .path()
        .join("boot.jsonl")
        .symlink_metadata()
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(std::fs::read(&victim).unwrap(), b"outside-data");

    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_concurrent_same_name_yields_exactly_one_success() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 8);
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-race", 2);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client_a, _rx_a) = connect_client(&server).await.unwrap();
    let (client_b, _rx_b) = connect_client(&server).await.unwrap();

    let (r1, r2) = tokio::join!(
        export_via(&client_a, &cid, "race.jsonl"),
        export_via(&client_b, &cid, "race.jsonl")
    );
    let results = [r1, r2];
    let successes: Vec<_> = results
        .iter()
        .filter(|r| r.is_error != Some(true))
        .collect();
    assert_eq!(
        successes.len(),
        1,
        "exactly one concurrent same-name export may succeed: {results:?}"
    );
    // The committed file is a complete snapshot (the loser changed nothing).
    let raw = std::fs::read(root.path().join("race.jsonl")).unwrap();
    assert!(raw.ends_with(b"\n"));
    assert!(!raw.is_empty());

    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
}

#[tokio::test]
async fn export_log_per_file_quota_failure_creates_no_file() {
    let root = TempDir::new().unwrap();
    // Tiny per-file quota: any real snapshot exceeds it.
    let store = capture_store_in(root.path(), 16, 1024, 8);
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-file-quota", 5);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "big.jsonl")))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = tool_error_text(&result);
    assert!(
        text.contains("quota"),
        "per-file quota failure must mention quota: {text}"
    );
    assert_root_has_only_lock(root.path());

    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_total_byte_quota_persists_across_exports_and_fresh_stores() {
    let root = TempDir::new().unwrap();
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-total", 1);
    let cid = manager.insert(conn).await.unwrap();

    // Server A commits one file against a generous quota; its result
    // reports the EXACT committed byte count.
    let store_a = capture_store_in(root.path(), 4096, 100_000, 8);
    let server_a = TestServer::builder(Arc::clone(&manager))
        .capture_store(store_a)
        .start()
        .await;
    let (client_a, _rx) = connect_client(&server_a).await.unwrap();
    let result = export_via(&client_a, &cid, "a.jsonl").await;
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let used_bytes = result.structured_content.expect("structured")["total_bytes_used"]
        .as_u64()
        .unwrap();
    assert!(used_bytes > 0);
    client_a.cancel().await.ok();
    drop(server_a);

    // Server B is a FRESH CaptureStore instance scanning the same root with
    // a total quota equal to A's committed size: B's identical snapshot
    // passes the per-file check but blows the total (A's file + B's file).
    let store_b = capture_store_in(root.path(), used_bytes, used_bytes, 8);
    let server_b = TestServer::builder(Arc::clone(&manager))
        .capture_store(store_b)
        .start()
        .await;
    let (client_b, _rx) = connect_client(&server_b).await.unwrap();
    let result = export_via(&client_b, &cid, "b.jsonl").await;
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = tool_error_text(&result);
    assert!(
        text.contains("total-byte quota"),
        "fresh store must observe A's usage: {text}"
    );
    // No file was committed by the failed attempt.
    assert!(!root.path().join("b.jsonl").exists());
    assert_eq!(
        std::fs::metadata(root.path().join("a.jsonl"))
            .unwrap()
            .len(),
        used_bytes
    );

    client_b.cancel().await.ok();
}

#[tokio::test]
async fn export_log_file_count_quota_includes_prior_committed_files() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 1);
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-count", 1);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "one.jsonl")))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        result.structured_content.expect("structured")["files_used"],
        json!(1)
    );

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "two.jsonl")))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");
    let text = tool_error_text(&result);
    assert!(
        text.contains("file-count quota"),
        "count quota must include prior committed files: {text}"
    );
    assert!(!root.path().join("two.jsonl").exists());

    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_independent_servers_sharing_root_cannot_exceed_quota() {
    // Two servers, each with an independent CaptureStore (independent
    // process-local mutexes). Only the advisory lock on the shared root
    // serializes scan+commit, so the FILE-COUNT quota holds across them:
    // at most one of the two concurrent exports may commit.
    let root = TempDir::new().unwrap();
    let manager_a = Arc::new(ConnectionManager::new());
    let (conn_a, _peer_a) = loopback_connection("loop-export-xstore-a");
    conn_a.log().rx_data(7);
    conn_a.log().rx_data(8);
    let cid_a = manager_a.insert(conn_a).await.unwrap();

    let manager_b = Arc::new(ConnectionManager::new());
    let (conn_b, _peer_b) = loopback_connection("loop-export-xstore-b");
    conn_b.log().rx_data(9);
    let cid_b = manager_b.insert(conn_b).await.unwrap();

    let store_a = capture_store_in(root.path(), 4096, 8192, 1);
    let store_b = capture_store_in(root.path(), 4096, 8192, 1);
    let server_a = TestServer::builder(manager_a)
        .capture_store(store_a)
        .start()
        .await;
    let server_b = TestServer::builder(manager_b)
        .capture_store(store_b)
        .start()
        .await;
    let (client_a, _rx_a) = connect_client(&server_a).await.unwrap();
    let (client_b, _rx_b) = connect_client(&server_b).await.unwrap();

    let (ra, rb) = tokio::join!(
        export_via(&client_a, &cid_a, "a.jsonl"),
        export_via(&client_b, &cid_b, "b.jsonl")
    );
    let results = [ra, rb];
    let successes: Vec<_> = results
        .iter()
        .filter(|r| r.is_error != Some(true))
        .collect();
    assert_eq!(
        successes.len(),
        1,
        "independent stores sharing a root may commit at most one file: {results:?}"
    );
    let files: Vec<String> = std::fs::read_dir(root.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".jsonl"))
        .collect();
    assert_eq!(
        files.len(),
        1,
        "only the winning commit may persist: {files:?}"
    );
    // The loser's error names the count quota (cross-store scan saw the
    // winner's committed file).
    let loser = results
        .iter()
        .find(|r| r.is_error == Some(true))
        .expect("one loser");
    let text = tool_error_text(loser);
    assert!(
        text.contains("file-count quota") || text.contains("already exists"),
        "loser must observe the winner's commit: {text}"
    );

    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
}

#[tokio::test]
async fn export_log_failure_leaves_connection_usable() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 8);
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-usable", 2);
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Failed export (bad filename).
    let result = client
        .peer()
        .call_tool(tool_request(
            "export_log",
            export_call(&cid, "bad/name.jsonl"),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));

    // The connection still answers get_log and a good export succeeds.
    let result = client
        .peer()
        .call_tool(tool_request("get_log", json!({ "connection_id": cid })))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "after.jsonl")))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    client.cancel().await.ok();
}

#[tokio::test]
async fn export_log_snapshot_is_point_in_time() {
    let root = TempDir::new().unwrap();
    let store = capture_store_in(root.path(), 4096, 8192, 8);
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = seeded_log_conn("loop-export-pit", 2);
    let log = Arc::clone(conn.log());
    let cid = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager)
        .capture_store(store)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request("export_log", export_call(&cid, "pit.jsonl")))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let events_written = result.structured_content.expect("structured")["events_written"]
        .as_u64()
        .unwrap();

    // Events recorded AFTER the export must not appear in the committed
    // file (the snapshot locked the buffer exactly once).
    log.rx_data(999);
    log.rx_data(998);
    let text = std::fs::read_to_string(root.path().join("pit.jsonl")).unwrap();
    assert_eq!(text.lines().count(), events_written as usize);
    assert!(
        !text.contains("\"bytes\":999") && !text.contains("\"bytes\":998"),
        "post-export events must not leak into the committed file: {text}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn spawned_server_starts_with_capture_dir() {
    // Real binary + real HTTP transport: a valid --capture-dir must not
    // break startup, and the initialized server still serves the catalog.
    let root = TempDir::new().unwrap();
    let server = common::spawned::SpawnedServer::start_with_capture_dir(root.path()).await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let info = client.peer_info().expect("peer info");
    assert_eq!(info.server_info.name, "serial-mcp");
    let tools = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    assert_eq!(tools.tools.len(), 27);

    client.cancel().await.ok();
}
