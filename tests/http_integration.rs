//! HTTP integration tests use real `rmcp` clients over in-process `axum`
//! transport. They cover the MCP surface without opening an OS serial port;
//! tests needing serial I/O inject in-memory loopback connections.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, CancelledNotificationParam, ClientRequest,
    GetPromptRequestParams, PaginatedRequestParams, ReadResourceRequestParams, Role,
};
use rmcp::service::{PeerRequestOptions, RoleClient, RunningService};
use serde_json::json;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use serial_mcp::limits::{MAX_TIMEOUT_MS, MAX_WRITE_BYTES};
use serial_mcp::serial::{test_support::loopback_connection, ConnectionManager};

mod common;
use common::controlled::ControlledState;
use common::{args_object, connect_client, tool_request, TestServer, EXPECTED_TOOLS};

#[tokio::test]
async fn initialize_handshake_succeeds() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();
    let info = client.peer().peer_info().expect("peer_info");
    assert_eq!(
        info.server_info.as_ref().expect("server_info present").name,
        "serial-mcp"
    );
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

    // List resources with default pagination.
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

    // List resource templates with default pagination.
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
    // The new profile must appear in list_profiles.
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
}

/// `list_profiles` exposes profile metadata and retained revision snapshots.
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

    // Overwriting increments the revision and retains prior defaults.
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
    // The retained revision contains the prior defaults.
    let revisions = p["revisions"].as_array().expect("revisions array");
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0]["revision"], json!(1));
    assert_eq!(revisions[0]["defaults"]["baud_rate"], json!(9600));
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
    // Overwrite with a higher baud rate.
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
    // An overwrite without the flag must fail.
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
}

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
    // The tool always returns profile matches alongside ports.
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

/// The `serial://ports` resource exposes the same profile-match map as
/// `list_ports`.
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
    assert!(matches!(first.role, Role::User));
    let rendered = serde_json::to_string(&first.content).unwrap();
    assert!(rendered.contains("/dev/ttyUSB7"));
    client.cancel().await.ok();
}

/// The rendered `diagnose_port` prompt uses `read` with `match` for pattern
/// waits. `max_buffered_bytes` is not a per-call field, and `wait_for` is not a
/// tool.
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
    // The prompt uses read with timeout and match.
    assert!(
        rendered.contains("read(connection_id"),
        "prompt must drive the current read flow: {rendered}"
    );
    // max_buffered_bytes is a connection default, not a per-call read field.
    assert!(
        !rendered.contains("max_buffered_bytes"),
        "prompt must not use the removed per-call max_buffered_bytes: {rendered}"
    );
    // wait_for is not a tool; read(match=...) is the pattern-wait flow.
    assert!(
        !rendered.contains("wait_for"),
        "prompt must not reference the removed wait_for tool: {rendered}"
    );
    client.cancel().await.ok();
}

/// `tools/list` descriptions for `read`, `transact`, and `flush` must advertise
/// tagged-object `ReadFrom` values, not string shorthand.
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
    // Read description must include all tagged forms, including an absolute
    // offset object.
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
    // Descriptions must not advertise bare-string forms.
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

/// The `read` and `transact` input schemas describe `from` with tagged objects,
/// such as `{"type":"now"}` and `{"type":"offset","offset":N}`. They must
/// not advertise bare strings.
#[tokio::test]
async fn read_tool_input_schema_uses_tagged_readfrom_examples() {
    let server = common::spawned::SpawnedServer::start().await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let result = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();

    for name in ["read", "transact"] {
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
    // Encoding fallback is a normal tool result, not a tool error.
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured content");
    assert_eq!(s["data"], json!("48 69 ff fe 00"));
    assert_eq!(s["encoding"], json!("hex"));
    assert_eq!(s["bytes_read"], json!(5));
    // Buffered input may stop as "drained"; otherwise this call stops at
    // "timeout". Both normal results retain the data.
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

    // A valid SLIP frame is followed by malformed escape bytes (0xDB 0xFF).
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
    // Raw bytes include the malformed tail, so they fall back to hex.
    assert_eq!(s["encoding"], json!("hex"));
    assert!(
        s["data"]
            .as_str()
            .unwrap()
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == ' '),
        "top-level data should be hex: {s:?}"
    );
    // The valid frame is encoded independently and remains UTF-8.
    let frames = s["frames"].as_array().expect("partial frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["data"], json!("OK"));
    assert_eq!(frames[0]["encoding"], json!("utf8"));
    assert_eq!(s["frames_dropped"], json!(0));

    client.cancel().await.ok();
}
#[tokio::test]
async fn read_match_index_over_chunked_stream_preserved() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-match-parity");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // The literal spans the boundary between two writes.
    peer.write_all(b"ABCD").await.unwrap();
    peer.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    peer.write_all(b"EFOK>tail").await.unwrap();
    peer.flush().await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    // The match is at global index 6.
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

    // An absent pattern times out without a match.
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

    client.cancel().await.ok();
}

#[tokio::test]
async fn validation_limits_return_tool_errors_over_http() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-validation");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let cases = [tool_request(
        "send_break",
        json!({ "connection_id": connection_id, "duration_ms": MAX_TIMEOUT_MS + 1 }),
    )];

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
    // Timeout is a normal tool result, not a tool error.
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

#[tokio::test]
async fn write_with_invalid_encoding_returns_tool_error() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-write-enc");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

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

#[tokio::test]
async fn send_break_cancellation_stops_gracefully() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, _peer) = loopback_connection("loop-break-cancel");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Start a long break and cancel the client before it completes.
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

    client.cancel().await.ok();

    // The call may return a tool result, transport cancellation, or the local
    // timeout. The request must not hang.
    match result {
        Ok(Ok(call_result)) => {
            // A completed call may be a normal result or a tool error.
            assert!(
                call_result.is_error == Some(true) || call_result.is_error == Some(false),
                "break completed: {call_result:?}"
            );
        }
        _ => {
            // Local timeout or transport cancellation; the request was already
            // cancelled.
        }
    }
}

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

    // list_connections has no connection_id; it should still succeed.
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

#[tokio::test]
async fn get_status_returns_config_and_counters() {
    let manager = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("loop-get-status");
    let connection_id = manager.insert(conn).await.unwrap();

    let server = TestServer::start_with(manager).await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Initial counters are zero.
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
    assert!(s["last_activity_ms"].is_null());
    assert!(
        s["port_info"].is_null(),
        "port_info should be null for loopback connections"
    );

    // A write increments the TX counter.
    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": "hello" }),
        ))
        .await
        .unwrap();

    peer.write_all(b"world").await.unwrap();

    // A read increments the RX counter.
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

    // Status reflects the I/O counters.
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

    // Start with the default configuration.
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

    // Change the baud rate to 9600.
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

    // get_status reports the changed configuration.
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

    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "baud_rate": 0 }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "data_bits": "9" }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": connection_id, "flow_control": "bogus" }),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true), "{result:?}");

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

// Profile persistence through public MCP calls covers real process restart,
// shared HTTP sessions, same- and cross-process concurrency, legacy migration,
// corrupt/future-file startup rejection, and failed-write preservation.

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

/// Run the binary with a bounded startup check and return status plus stdout.
/// A timeout panics because these callers expect startup to fail.
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

    // First process creates profile A through MCP.
    let mut server = SpawnedServer::start_with_profiles_path(Some(&path)).await;
    let (client, _rx) = spawn_client(&server).await.unwrap();
    let created = configure_profile_via(&client, "restart-a", 9600).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");
    client.cancel().await.ok();
    server.stop().await.unwrap();

    // A fresh process with the same path must load it.
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

    // Client B uses a separate session on the same server.
    let names_b = list_profile_names_via(&client_b).await;
    assert!(
        names_b.contains(&"session-a".to_string()),
        "client B must observe client A's profile: {names_b:?}"
    );

    // Client B deletes the profile.
    let deleted = client_b
        .peer()
        .call_tool(tool_request(
            "delete_profile",
            json!({ "profile_name": "session-a" }),
        ))
        .await
        .unwrap();
    assert_ne!(deleted.is_error, Some(true), "{deleted:?}");

    // Client A must no longer see the profile.
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

    // Two clients create different profiles concurrently.
    let (ra, rb) = tokio::join!(
        configure_profile_via(&client_a, "conc-a", 9600),
        configure_profile_via(&client_b, "conc-b", 19200),
    );
    assert_ne!(ra.is_error, Some(true), "{ra:?}");
    assert_ne!(rb.is_error, Some(true), "{rb:?}");

    // Another view sees both profiles.
    let names = list_profile_names_via(&client_a).await;
    assert!(
        names.contains(&"conc-a".to_string()) && names.contains(&"conc-b".to_string()),
        "both concurrent writes must be visible: {names:?}"
    );
    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
    drop(server);

    // A fresh store over the same file sees both persisted profiles.
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

    // Independent server processes share one profile file. Advisory locking and
    // reload-under-lock must preserve both writes.
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

    // A third process confirms both writes persisted.
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

    // A mutation persists the migration as schema version 2.
    let created = configure_profile_via(&client, "new-dev", 9600).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");
    client.cancel().await.ok();
    server.stop().await.unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(
        content.contains("schema_version = 2"),
        "mutation must persist current schema version:\n{content}"
    );

    // Restart confirms both profiles remain.
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

    // Make the profile directory non-writable while keeping it traversable for
    // cleanup.
    let dir_path = dir.path();
    let original_mode = std::fs::metadata(dir_path).unwrap().permissions().mode();
    std::fs::set_permissions(dir_path, std::fs::Permissions::from_mode(0o555)).unwrap();

    // Creating profile B must return a tool error, not crash the server.
    let rb = configure_profile_via(&client, "lost-b", 19200).await;
    assert_eq!(
        rb.is_error,
        Some(true),
        "write must fail with the profile dir read-only: {rb:?}"
    );

    // The in-memory view remains unchanged: A is present and B is absent.
    let names = list_profile_names_via(&client).await;
    assert!(
        names.contains(&"keep-a".to_string()) && !names.contains(&"lost-b".to_string()),
        "failed write must leave the cache untouched: {names:?}"
    );

    // Restore permissions and restart; disk must still contain A, not B.
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

    // Launch the binary in an isolated cwd with a relative profile filename.
    let mut server = SpawnedServer::start_with_cwd(dir.path(), Some(relative)).await;
    let (client, _rx) = spawn_client(&server).await.unwrap();
    let created = configure_profile_via(&client, "rel-dev", 9600).await;
    assert_ne!(created.is_error, Some(true), "{created:?}");
    client.cancel().await.ok();
    server.stop().await.unwrap();

    // The file is created in the server's cwd.
    let on_disk = dir.path().join("profiles.toml");
    assert!(
        on_disk.exists(),
        "relative --profiles-path must create the file in the server's cwd"
    );

    // Restart with the same cwd and relative path confirms persistence.
    let server2 = SpawnedServer::start_with_cwd(dir.path(), Some(relative)).await;
    let (client2, _rx2) = spawn_client(&server2).await.unwrap();
    let names = list_profile_names_via(&client2).await;
    assert!(
        names.contains(&"rel-dev".to_string()),
        "relative-path store must survive restart: {names:?}"
    );
    client2.cancel().await.ok();
}

// capture_boot tests use the public HTTP MCP surface, tool router, RX pump, and
// stop controller with a controlled in-memory backend. They cover post-mark
// scope, private cursor reads, line transitions, and pump-gate ordering. The
// backend injects RX bytes during assertion or release; no tool or store logic
// is mocked.
//
// rmcp sends a tools/call request only after its future is polled. Tests that
// drive the backend during capture poll the future with tokio::select!.

/// Default reset pulse: assert both lines low, then release both high.
fn capture_reset() -> serde_json::Value {
    json!({
        "assert_dtr": false,
        "assert_rts": false,
        "release_dtr": true,
        "release_rts": true,
        "hold_ms": 150,
    })
}

/// A literal-substring match request that never matches test payloads.
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
    RunningService<RoleClient, common::TestClientHandler>,
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

/// Wait for the first line transition while polling the in-flight capture call.
/// rmcp sends the request only after its future is polled.
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

#[tokio::test]
async fn capture_boot_stale_bytes_excluded_boot_bytes_captured_cursor_preserved() {
    let (_server, client, cid, state) = controlled_server("loop-capture-1", 65536).await;

    // Consume five stale bytes with an ordinary read; the shared cursor reaches
    // offset 5 before capture starts.
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

    // Inject boot bytes synchronously when the reset line is asserted at
    // (false, false).
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

    // Capture uses a private cursor, so a plain read still starts at 5 and
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

    // Ring history remains readable from the oldest retained byte.
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

#[tokio::test]
async fn capture_boot_immediate_bytes_captured_and_match_stops_at_pattern() {
    let (_server, client, cid, state) = controlled_server("loop-capture-match", 65536).await;

    // The device starts streaming when the reset line is asserted.
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

    // Wait until assertion starts the hold phase.
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.line_log().is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        state.line_log(),
        vec![(false, false)],
        "assertion must precede cancellation"
    );

    // Cancel this request through notifications/cancelled, not whole-client
    // teardown. rmcp resolves the cancelled handle with Err(Cancelled) and
    // discards the server response; read_loop.rs proves the structured
    // cancelled result, while this test proves wire-level line release.
    handle
        .peer
        .notify_cancelled(CancelledNotificationParam::new(
            Some(handle.id.clone()),
            Some("test cancel".into()),
        ))
        .await
        .unwrap();

    // Capture must release lines promptly instead of waiting for the 30-second
    // hold to finish.
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

    // Server-side capture cleanup frees the control lock for a new call.
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

    // Request-scoped cancellation leaves the client usable.
    let r = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": cid })))
        .await
        .unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");

    client.cancel().await.ok();
}

/// A failed first release leaves cleanup armed. Drop retries the configured
/// release state through the control lock, which remains usable afterwards.
/// The line log must show assertion, failed release, then retry.
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

    // Wait for assertion, then fail the release attempt triggered by
    // cancellation.
    let deadline = Instant::now() + Duration::from_secs(2);
    while state.line_log().is_empty() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(state.line_log(), vec![(false, false)]);
    state.set_fail_next_set(1);

    // Request-scoped cancellation records the release state and fails; cleanup
    // must remain armed.
    handle
        .peer
        .notify_cancelled(CancelledNotificationParam::new(
            Some(handle.id.clone()),
            Some("release-failure cancel".into()),
        ))
        .await
        .unwrap();

    // Failed release and drop-time retry both record (true, true). Expect
    // assertion, failed release, then retry in that order.
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

    // Cleanup must leave the control lock usable for a fresh line-control call.
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

#[tokio::test]
async fn capture_boot_assertion_failure_errors_and_attempts_cleanup() {
    let (_server, client, cid, state) = controlled_server("loop-capture-assert-fail", 65536).await;

    // The assertion records its state, then fails with an I/O error.
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

    // Drop cleanup must still attempt the configured release state.
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

    // Poll the call until assertion is recorded, then fail the release call.
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

    // Drop cleanup retries release after the failure flag is consumed.
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

#[tokio::test]
async fn capture_boot_invalid_framing_fails_before_line_transition() {
    let (_server, client, cid, state) = controlled_server("loop-capture-bad-framing", 65536).await;

    // Invalid framing construction must return a tool error before line changes.
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

// Runtime SLIP errors return partial structured data and still release lines.

#[tokio::test]
async fn capture_boot_runtime_framing_error_returns_partial_result_and_releases_lines() {
    let (_server, client, cid, state) = controlled_server("loop-capture-slip-err", 65536).await;

    // A valid SLIP frame is followed by malformed escape bytes (0xDB 0xFF).
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
    // Runtime framing errors are structured results, not tool errors.
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let s = r.structured_content.expect("structured capture result");
    assert_eq!(s["read"]["stop_reason"], json!("framing_error"));
    assert!(
        s["read"]["error"]
            .as_str()
            .is_some_and(|e| e.contains("SLIP")),
        "framing error text must be present: {s:?}"
    );
    // The valid frame survives. Frame encoding is independent, so "OK" stays
    // UTF-8 while the raw malformed tail falls back to hex.
    let frames = s["read"]["frames"].as_array().expect("partial frames");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0]["data"], json!("OK"));
    assert_eq!(frames[0]["encoding"], json!("utf8"));
    assert_eq!(s["read"]["encoding"], json!("hex"));
    assert_eq!(s["read"]["frames_dropped"], json!(0));
    // Lines are released after the decode error.
    assert_eq!(state.line_log(), vec![(false, false), (true, true)]);

    client.cancel().await.ok();
}

// Preset/parser decoding and binary encoding use the current read behavior.

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

    // Request base64 encoding for the next capture.
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

// Capture stops on silence after a banner and on wall timeout during output.

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

    // Keep feeding bytes so the device never goes silent.
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

// Disconnect returns a partial capture with `connection_closed`.

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

    // Close the connection after assertion, while capture is active.
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

// Ring wrap reports `bytes_lost` while preserving the atomic mark.

#[tokio::test]
async fn capture_boot_ring_wrap_reports_bytes_lost_and_preserves_mark() {
    // Tiny ring forces a wrap while capture is in flight.
    let (_server, client, cid, state) = controlled_server("loop-capture-wrap", 16).await;

    // 32 bytes into a 16-byte ring force a wrap. The read reports bytes_lost
    // and starts at the retained offset; mark_offset keeps the original
    // boundary.
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

// Concurrent `set_dtr_rts` cannot interleave inside the reset pulse.

#[tokio::test]
async fn capture_boot_concurrent_set_dtr_rts_cannot_interleave_inside_pulse() {
    let (server, client, cid, state) = controlled_server("loop-capture-lock", 65536).await;
    // A second session on the same server issues the concurrent line-control
    // call.
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

    // Capture holds the control lock after assertion.
    let deadline = Instant::now() + Duration::from_secs(2);
    wait_for_line(&mut call, &state, deadline, "assertion").await;
    assert_eq!(state.line_log(), vec![(false, false)]);

    // Concurrent set_dtr_rts must wait for assertion, hold, and release.
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

    // Expect assertion, release, then the user's set.
    assert_eq!(
        state.line_log(),
        vec![(false, false), (true, true), (true, true)],
        "no line-control call may interleave between assert and release"
    );

    client.cancel().await.ok();
    client_b.cancel().await.ok();
}

// Arm-only capture has no reset config and never touches lines.

#[tokio::test]
async fn capture_boot_arm_only_does_not_touch_lines() {
    let (_server, client, cid, state) = controlled_server("loop-capture-arm", 65536).await;

    // Bytes that predate capture must never appear in the result.
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

    // Poll the request, then let capture reach gate acquisition and mark before
    // injecting external bytes. This takes at most one pump cycle, about 100 ms.
    tokio::select! {
        res = call.as_mut() => {
            panic!("arm-only capture completed before external bytes: {res:?}");
        }
        _ = tokio::time::sleep(Duration::from_millis(300)) => {}
    }

    // Inject device bytes after capture reaches its post-mark read phase.
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

// Capture schemas must not emit non-standard unsigned-integer formats.

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

// export_log tests cover disabled ordering, safe names, atomic no-clobber
// commits, symlinks, quotas, concurrency, snapshots, and recovery.

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

/// Loopback connection seeded with `events` rx_data entries and its automatic
/// `open` event.
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

/// Run one `export_log` call through the public MCP boundary.
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

/// Join text content blocks; tool error text lives in `content`.
fn tool_error_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| c.as_text())
        .map(|t| t.text.clone())
        .collect::<Vec<_>>()
        .join("\n")
}

/// ConnectionConfig with enabled logging and capacity 0; every event is evicted
/// immediately, so exports are empty.
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
    }
}

/// Failed exports leave only the advisory lock in the capture root. Temp files
/// are transient and `NamedTempFile` removes them on failure.
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
    let server = TestServer::start_with(manager).await; // persistent store disabled by default
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

    // Disabled storage is checked before connection lookup: a bogus ID still
    // yields the disabled error, not connection_not_found.
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

    // Use get_log as the reference snapshot.
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
    // A normal Unix commit has no durability warning.
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
    // Capacity 0 evicts every event, so the snapshot is empty.
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

    // A second zero-byte export consumes another file slot.
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
        // Invalid names return tool errors with messages.
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
    // The symlink and its outside target remain untouched.
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
    // The committed file is a complete snapshot; the loser changed nothing.
    let raw = std::fs::read(root.path().join("race.jsonl")).unwrap();
    assert!(raw.ends_with(b"\n"));
    assert!(!raw.is_empty());

    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
}

#[tokio::test]
async fn export_log_per_file_quota_failure_creates_no_file() {
    let root = TempDir::new().unwrap();
    // The tiny per-file quota rejects this snapshot.
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

    // Server A commits one file and reports its exact committed byte count.
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

    // A fresh store scans the same root. Its total quota equals A's committed
    // size, so the identical second snapshot exceeds the total quota.
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
    // The failed attempt commits no file.
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
    // Independent stores share a root but not a process-local mutex. The root
    // advisory lock serializes scan and commit, so the file-count quota permits
    // at most one concurrent export.
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
    // The loser sees the winner's committed file during its cross-store scan.
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

    // A bad filename fails before writing.
    let result = client
        .peer()
        .call_tool(tool_request(
            "export_log",
            export_call(&cid, "bad/name.jsonl"),
        ))
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(true));

    // The connection still answers get_log, and a valid export succeeds.
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

    // Events recorded after export must not appear in the committed file; the
    // snapshot captured the buffer once.
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
    // A real binary with HTTP transport accepts --capture-dir and still serves
    // the initialized catalog.
    let root = TempDir::new().unwrap();
    let server = common::spawned::SpawnedServer::start_with_capture_dir(root.path()).await;
    let (client, _rx) = common::spawned::spawn_client(&server).await.unwrap();

    let info = client.peer_info().expect("peer info");
    assert_eq!(
        info.server_info.as_ref().expect("server_info present").name,
        "serial-mcp"
    );
    let tools = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    assert_eq!(tools.tools.len(), 25);

    client.cancel().await.ok();
}
