//! MCP protocol compatibility matrix: modern `2026-07-28` discovery /
//! stateless lifecycle vs legacy `2025-11-25` initialize / session lifecycle.
//!
//! Two proof layers:
//!
//! - **Typed matrix** (in-process [`TestServer`]): real `rmcp` clients driven
//!   through `serve_with_lifecycle` (discover vs initialize) asserting the
//!   negotiated version, tool/resource/prompt surface, hardware-free tool
//!   execution, and the capability views.
//! - **Raw wire** (spawned real binary + `reqwest`): hand-built JSON-RPC
//!   POSTs with explicit `MCP-Protocol-Version`, SEP-2243 `Mcp-Method` /
//!   `Mcp-Name`, and `Mcp-Session-Id` headers, asserting exact HTTP status,
//!   JSON-RPC error codes, `resultType` presence, and response-ID echo
//!   without typed rmcp result deserialization.
//!
//! Wire facts are pinned to rmcp 3.0.1 behavior (see
//! `crates/rmcp/src/transport/streamable_http_server/tower.rs` and
//! `crates/rmcp/src/handler/server.rs` in the pinned SDK source).

mod common;

use std::collections::BTreeSet;
use std::future::Future;

use anyhow::Result;
use common::TestServer;
use rmcp::model::{PaginatedRequestParams, ReadResourceRequestParams};
use rmcp::service::RoleClient;
use serde_json::{json, Value};

/// Modern `2026-07-28` per-request `_meta` carried by every raw modern
/// request (SEP-2575 client context; `clientInfo` optional but included).
fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "serial-mcp-test", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// Expected modern capability wire shape (common set only).
fn common_capabilities_json() -> Value {
    json!({
        "completions": {},
        "prompts": {},
        "resources": {},
        "tools": {},
    })
}

// =============================================================================
// Typed matrix
// =============================================================================

/// Run an async assertion body against a typed modern (discover lifecycle)
/// client on a fresh in-process server.
async fn typed_modern<F, Fut>(run: F) -> Result<()>
where
    F: FnOnce(rmcp::service::RunningService<RoleClient, common::TestClientHandler>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let server = TestServer::start().await;
    let (client, _rx) = common::connect_modern_client(&server).await?;
    run(client).await
}

/// Run an async assertion body against a typed legacy (initialize
/// lifecycle) client on a fresh in-process server.
async fn typed_legacy<F, Fut>(run: F) -> Result<()>
where
    F: FnOnce(rmcp::service::RunningService<RoleClient, common::LegacyClientHandler>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let server = TestServer::start().await;
    let (client, _rx) = common::connect_legacy_client(&server).await?;
    run(client).await
}

#[tokio::test]
async fn typed_modern_lifecycle_selects_exact_version() {
    typed_modern(|client| async move {
        let info = client.peer_info().expect("modern peer info");
        assert_eq!(
            info.protocol_version,
            rmcp::model::ProtocolVersion::V_2026_07_28
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_legacy_lifecycle_selects_exact_version() {
    typed_legacy(|client| async move {
        let info = client.peer_info().expect("legacy peer info");
        assert_eq!(
            info.protocol_version,
            rmcp::model::ProtocolVersion::V_2025_11_25
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_modern_tools_list_returns_exact_twenty_five_tools() {
    typed_modern(|client| async move {
        let result = client
            .peer()
            .list_tools(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        let names: BTreeSet<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
        let expected: BTreeSet<&str> = common::EXPECTED_TOOLS.iter().copied().collect();
        assert_eq!(
            names, expected,
            "modern tools/list must match EXPECTED_TOOLS"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_legacy_tools_list_returns_exact_twenty_five_tools() {
    typed_legacy(|client| async move {
        let result = client
            .peer()
            .list_tools(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        let names: BTreeSet<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
        let expected: BTreeSet<&str> = common::EXPECTED_TOOLS.iter().copied().collect();
        assert_eq!(
            names, expected,
            "legacy tools/list must match EXPECTED_TOOLS"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_modern_resources_and_templates_and_prompts() {
    typed_modern(|client| async move {
        let resources = client
            .peer()
            .list_resources(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_eq!(resources.resources.len(), 2, "two static resources");
        let templates = client
            .peer()
            .list_resource_templates(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_eq!(templates.resource_templates.len(), 3, "three templates");
        let prompts = client
            .peer()
            .list_prompts(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_eq!(prompts.prompts.len(), 2, "two prompts");
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_legacy_resources_and_templates_and_prompts() {
    typed_legacy(|client| async move {
        let resources = client
            .peer()
            .list_resources(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_eq!(resources.resources.len(), 2, "two static resources");
        let templates = client
            .peer()
            .list_resource_templates(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_eq!(templates.resource_templates.len(), 3, "three templates");
        let prompts = client
            .peer()
            .list_prompts(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_eq!(prompts.prompts.len(), 2, "two prompts");
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_modern_compute_checksum_succeeds_without_hardware() {
    typed_modern(|client| async move {
        let result = client
            .peer()
            .call_tool(common::tool_request(
                "compute_checksum",
                json!({"data": "$GPGGA,1", "algorithm": "xor"}),
            ))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let structured = result.structured_content.expect("structured content");
        assert_eq!(structured["checksum"], 111);
        assert_eq!(structured["checksum_hex"], "6F");
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_legacy_compute_checksum_succeeds_without_hardware() {
    typed_legacy(|client| async move {
        let result = client
            .peer()
            .call_tool(common::tool_request(
                "compute_checksum",
                json!({"data": "$GPGGA,1", "algorithm": "xor"}),
            ))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(false), "{result:?}");
        let structured = result.structured_content.expect("structured content");
        assert_eq!(structured["checksum"], 111);
        assert_eq!(structured["checksum_hex"], "6F");
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_modern_read_serial_ports_resource_succeeds() {
    typed_modern(|client| async move {
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("serial://ports"))
            .await
            .unwrap();
        assert!(!result.contents.is_empty());
        match &result.contents[0] {
            rmcp::model::ResourceContents::TextResourceContents { uri, mime_type, .. } => {
                assert_eq!(uri.as_str(), "serial://ports");
                assert_eq!(mime_type.as_deref(), Some("application/json"));
            }
            other => panic!("expected text resource contents, got {other:?}"),
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_legacy_read_serial_ports_resource_succeeds() {
    typed_legacy(|client| async move {
        let result = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("serial://ports"))
            .await
            .unwrap();
        assert!(!result.contents.is_empty());
        match &result.contents[0] {
            rmcp::model::ResourceContents::TextResourceContents { uri, mime_type, .. } => {
                assert_eq!(uri.as_str(), "serial://ports");
                assert_eq!(mime_type.as_deref(), Some("application/json"));
            }
            other => panic!("expected text resource contents, got {other:?}"),
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_modern_capabilities_are_exact_common_set() {
    typed_modern(|client| async move {
        let caps = client
            .peer_info()
            .expect("modern peer info")
            .capabilities
            .clone();
        assert_eq!(
            serde_json::to_value(caps).unwrap(),
            common_capabilities_json(),
            "modern capabilities"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_legacy_capabilities_are_exact_common_set() {
    typed_legacy(|client| async move {
        let caps = client
            .peer_info()
            .expect("legacy peer info")
            .capabilities
            .clone();
        assert_eq!(
            serde_json::to_value(caps).unwrap(),
            common_capabilities_json(),
            "legacy capabilities"
        );
        Ok(())
    })
    .await
    .unwrap();
}

// =============================================================================
// Raw wire helpers
// =============================================================================

/// A raw HTTP response plus the parsed JSON-RPC payload (direct JSON body or
/// first non-empty SSE `data:` event).
#[derive(Debug)]
struct RawWire {
    status: u16,
    content_type: String,
    /// `Mcp-Session-Id` response header, when present.
    session_id: Option<String>,
    /// First non-empty JSON-RPC payload, when the response carried one.
    json: Option<Value>,
}

/// Parse a response body that is either direct JSON or an SSE stream of
/// `data:` events. Returns the first non-empty JSON value found.
fn parse_raw_body(content_type: &str, body: &str) -> Option<Value> {
    if content_type.starts_with("application/json") {
        return serde_json::from_str(body).ok();
    }
    for event in body.split("\n\n") {
        let mut data_lines = Vec::new();
        for line in event.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
            }
        }
        let data = data_lines.join("\n");
        if !data.trim().is_empty() {
            if let Ok(value) = serde_json::from_str(&data) {
                return Some(value);
            }
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
async fn raw_post(
    url: &str,
    id: Option<u64>,
    method: &str,
    params: Value,
    protocol_version: Option<&str>,
    session_id: Option<&str>,
    mcp_method: Option<&str>,
    mcp_name: Option<&str>,
) -> RawWire {
    let client = reqwest::Client::new();
    let mut req = client
        .post(url)
        .header("Accept", "application/json, text/event-stream")
        .header("Content-Type", "application/json");
    if let Some(v) = protocol_version {
        req = req.header("MCP-Protocol-Version", v);
    }
    if let Some(v) = session_id {
        req = req.header("Mcp-Session-Id", v);
    }
    if let Some(v) = mcp_method {
        req = req.header("Mcp-Method", v);
    }
    if let Some(v) = mcp_name {
        req = req.header("Mcp-Name", v);
    }
    let body = match id {
        Some(id) => json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        // A JSON-RPC notification carries no id; an id would make the
        // server treat it as a request and answer -32601.
        None => json!({"jsonrpc": "2.0", "method": method, "params": params}),
    };
    let resp = req.json(&body).send().await.expect("raw POST");
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap_or("?").to_string())
        .unwrap_or_default();
    let session_id = resp
        .headers()
        .get("mcp-session-id")
        .map(|v| v.to_str().unwrap_or("?").to_string());
    let text = resp.text().await.expect("raw response body");
    let json = parse_raw_body(&content_type, &text);
    RawWire {
        status,
        content_type,
        session_id,
        json,
    }
}

/// Modern request: `_meta` + matching `MCP-Protocol-Version` header plus the
/// SEP-2243 `Mcp-Method` / `Mcp-Name` headers rmcp requires for 2026-07-28.
async fn raw_modern(
    url: &str,
    id: u64,
    method: &str,
    params: Value,
    name: Option<&str>,
) -> RawWire {
    let mut params = params;
    params["_meta"] = modern_meta();
    raw_post(
        url,
        Some(id),
        method,
        params,
        Some("2026-07-28"),
        None,
        Some(method),
        name,
    )
    .await
}

/// Establish a legacy session: initialize with `2025-11-25`, capture
/// `Mcp-Session-Id`, send `notifications/initialized`. Returns the session
/// id and the initialize raw wire.
async fn raw_legacy_session(url: &str) -> (String, RawWire) {
    let init = raw_post(
        url,
        Some(400),
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "serial-mcp-test", "version": "1"},
        }),
        Some("2025-11-25"),
        None,
        None,
        None,
    )
    .await;
    let session = init
        .session_id
        .clone()
        .expect("initialize must return Mcp-Session-Id");
    let initialized = raw_post(
        url,
        None,
        "notifications/initialized",
        json!({}),
        Some("2025-11-25"),
        Some(&session),
        None,
        None,
    )
    .await;
    assert_eq!(
        initialized.status, 202,
        "notifications/initialized -> 202 Accepted"
    );
    assert!(
        initialized.json.is_none(),
        "notification response carries no JSON payload"
    );
    (session, init)
}

/// Legacy request: session header + `MCP-Protocol-Version: 2025-11-25`.
async fn raw_legacy(url: &str, session: &str, id: u64, method: &str, params: Value) -> RawWire {
    raw_post(
        url,
        Some(id),
        method,
        params,
        Some("2025-11-25"),
        Some(session),
        None,
        None,
    )
    .await
}

// =============================================================================
// Raw wire assertions
// =============================================================================

#[tokio::test]
async fn raw_discover_succeeds_without_session_and_lists_versions_modern_first() {
    let server = common::spawned::SpawnedServer::start().await;
    let raw = raw_modern(&server.url, 1, "server/discover", json!({}), None).await;
    assert_eq!(raw.status, 200, "discover without session id succeeds");
    assert!(
        raw.content_type.starts_with("text/event-stream"),
        "discover result arrives over SSE: {}",
        raw.content_type
    );
    assert!(raw.session_id.is_none(), "no session header required");
    let json = raw.json.expect("discover JSON");
    assert_eq!(json["id"], 1, "response id echoes request id");
    let result = json["result"].clone();
    assert_eq!(result["resultType"], "complete");
    assert_eq!(
        result["supportedVersions"],
        json!(["2026-07-28", "2025-11-25"]),
        "supportedVersions exactly modern then legacy"
    );
    assert_eq!(result["capabilities"], common_capabilities_json());
    // Cache policy (`ttlMs` / `cacheScope`) is Phase 4 scope; no cache
    // assertion belongs in the Phase 2 discovery acceptance.
}

#[tokio::test]
async fn raw_modern_surface_includes_result_type_complete() {
    let server = common::spawned::SpawnedServer::start().await;

    // tools/list
    let raw = raw_modern(&server.url, 2, "tools/list", json!({}), None).await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 2);
    assert_eq!(json["result"]["resultType"], "complete");
    let names: BTreeSet<&str> = json["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    let expected: BTreeSet<&str> = common::EXPECTED_TOOLS.iter().copied().collect();
    assert_eq!(names, expected);

    // resources/list
    let raw = raw_modern(&server.url, 3, "resources/list", json!({}), None).await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert_eq!(json["result"]["resources"].as_array().unwrap().len(), 2);

    // resources/read(serial://ports)
    let raw = raw_modern(
        &server.url,
        4,
        "resources/read",
        json!({"uri": "serial://ports"}),
        Some("serial://ports"),
    )
    .await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert_eq!(json["result"]["contents"][0]["uri"], "serial://ports");
    assert_eq!(
        json["result"]["contents"][0]["mimeType"],
        "application/json"
    );

    // prompts/list
    let raw = raw_modern(&server.url, 5, "prompts/list", json!({}), None).await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert_eq!(json["result"]["prompts"].as_array().unwrap().len(), 2);

    // prompts/get
    let raw = raw_modern(
        &server.url,
        6,
        "prompts/get",
        json!({"name": "diagnose_port", "arguments": {"port": "/dev/ttyUSB7"}}),
        Some("diagnose_port"),
    )
    .await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert!(
        !json["result"]["messages"].as_array().unwrap().is_empty(),
        "prompt messages present"
    );

    // completion/complete
    let raw = raw_modern(
        &server.url,
        7,
        "completion/complete",
        json!({
            "ref": {"type": "ref/resource", "uri": "serial://ports"},
            "argument": {"name": "port", "value": ""},
        }),
        None,
    )
    .await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert!(
        json["result"]["completion"]["values"].is_array(),
        "completion values array"
    );

    // tools/call(compute_checksum)
    let raw = raw_modern(
        &server.url,
        8,
        "tools/call",
        json!({"name": "compute_checksum", "arguments": {"data": "$GPGGA,1", "algorithm": "xor"}}),
        Some("compute_checksum"),
    )
    .await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert_eq!(json["result"]["structuredContent"]["checksum"], 111);
    assert_eq!(json["result"]["structuredContent"]["checksum_hex"], "6F");
}

#[tokio::test]
async fn raw_legacy_responses_omit_result_type() {
    let server = common::spawned::SpawnedServer::start().await;
    let (session, init) = raw_legacy_session(&server.url).await;
    assert_eq!(init.status, 200);
    let init_json = init.json.clone().unwrap();
    assert_eq!(init_json["id"], 400);
    assert_eq!(init_json["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        init_json["result"]["capabilities"],
        common_capabilities_json()
    );

    let list = raw_legacy(&server.url, &session, 10, "tools/list", json!({})).await;
    assert_eq!(list.status, 200);
    assert!(
        list.content_type.starts_with("text/event-stream"),
        "legacy responses arrive over SSE: {}",
        list.content_type
    );
    let json = list.json.unwrap();
    assert_eq!(json["id"], 10);
    assert!(
        json["result"].get("resultType").is_none(),
        "legacy responses must omit resultType: {json}"
    );
    assert_eq!(
        json["result"]["tools"].as_array().unwrap().len(),
        common::EXPECTED_TOOLS.len()
    );

    let call = raw_legacy(
        &server.url,
        &session,
        11,
        "tools/call",
        json!({"name": "compute_checksum", "arguments": {"data": "$GPGGA,1", "algorithm": "xor"}}),
    )
    .await;
    assert_eq!(call.status, 200);
    let json = call.json.unwrap();
    assert!(json["result"].get("resultType").is_none());
    assert_eq!(json["result"]["structuredContent"]["checksum"], 111);
}

#[tokio::test]
async fn raw_modern_unknown_resource_is_invalid_params_with_uri() {
    let server = common::spawned::SpawnedServer::start().await;
    let raw = raw_modern(
        &server.url,
        12,
        "resources/read",
        json!({"uri": "serial://does-not-exist"}),
        Some("serial://does-not-exist"),
    )
    .await;
    assert_eq!(raw.status, 400, "modern unknown resource -> HTTP 400");
    assert!(
        raw.content_type.starts_with("application/json"),
        "modern errors are direct JSON: {}",
        raw.content_type
    );
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 12, "request id echoed");
    assert_eq!(json["error"]["code"], -32602, "INVALID_PARAMS for modern");
    assert_eq!(
        json["error"]["data"]["uri"], "serial://does-not-exist",
        "error data carries the requested URI"
    );
}

#[tokio::test]
async fn raw_legacy_unknown_resource_keeps_resource_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    let (session, _init) = raw_legacy_session(&server.url).await;
    let raw = raw_legacy(
        &server.url,
        &session,
        13,
        "resources/read",
        json!({"uri": "serial://does-not-exist"}),
    )
    .await;
    assert_eq!(raw.status, 200, "legacy error stays inside an SSE 200");
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 13);
    assert_eq!(
        json["error"]["code"], -32002,
        "legacy keeps RESOURCE_NOT_FOUND"
    );
    assert_eq!(json["error"]["data"]["uri"], "serial://does-not-exist");
}

#[tokio::test]
async fn raw_modern_missing_required_meta_returns_400() {
    let server = common::spawned::SpawnedServer::start().await;

    // server/discover without _meta: invalid params, no header can save it.
    let raw = raw_post(
        &server.url,
        Some(14),
        "server/discover",
        json!({}),
        Some("2026-07-28"),
        None,
        Some("server/discover"),
        None,
    )
    .await;
    assert_eq!(raw.status, 400);
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 14);
    assert_eq!(json["error"]["code"], -32602);

    // _meta present but missing the required clientCapabilities key.
    let raw = raw_post(
        &server.url,
        Some(15),
        "tools/list",
        json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                "io.modelcontextprotocol/clientInfo": {"name": "serial-mcp-test", "version": "1"},
            }
        }),
        Some("2026-07-28"),
        None,
        Some("tools/list"),
        None,
    )
    .await;
    assert_eq!(raw.status, 400);
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 15);
    assert_eq!(json["error"]["code"], -32602);
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("clientCapabilities"),
        "error names the missing key: {json}"
    );
}

#[tokio::test]
async fn raw_modern_header_meta_version_mismatch_returns_400() {
    let server = common::spawned::SpawnedServer::start().await;
    let mut params = json!({});
    params["_meta"] = modern_meta();
    let raw = raw_post(
        &server.url,
        Some(16),
        "tools/list",
        params,
        Some("2025-11-25"), // header disagrees with _meta 2026-07-28
        None,
        Some("tools/list"),
        None,
    )
    .await;
    assert_eq!(raw.status, 400);
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 16);
    assert_eq!(json["error"]["code"], -32020, "HEADER_MISMATCH");
}

#[tokio::test]
async fn raw_modern_unsupported_version_returns_400_with_supported_list() {
    let server = common::spawned::SpawnedServer::start().await;
    let mut params = json!({});
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": "2027-01-01",
        "io.modelcontextprotocol/clientInfo": {"name": "serial-mcp-test", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let raw = raw_post(
        &server.url,
        Some(17),
        "tools/list",
        params,
        Some("2027-01-01"),
        None,
        Some("tools/list"),
        None,
    )
    .await;
    assert_eq!(raw.status, 400);
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 17);
    assert_eq!(
        json["error"]["code"], -32022,
        "UNSUPPORTED_PROTOCOL_VERSION"
    );
    assert_eq!(json["error"]["data"]["requested"], "2027-01-01");
    assert_eq!(
        json["error"]["data"]["supported"],
        json!(["2026-07-28", "2025-11-25"]),
        "supported list comes from the product slice"
    );
}

#[tokio::test]
async fn raw_modern_routing_rejects_legacy_only_methods() {
    let server = common::spawned::SpawnedServer::start().await;

    for (id, method, params, name) in [
        (18, "ping", json!({}), None),
        (19, "logging/setLevel", json!({"level": "error"}), None),
        (
            20,
            "resources/subscribe",
            json!({"uri": "serial://ports"}),
            Some("serial://ports"),
        ),
        (
            21,
            "resources/unsubscribe",
            json!({"uri": "serial://ports"}),
            Some("serial://ports"),
        ),
    ] {
        let raw = raw_modern(&server.url, id, method, params, name).await;
        assert_eq!(raw.status, 404, "{method} -> HTTP 404");
        assert!(
            raw.content_type.starts_with("application/json"),
            "{method} modern error is direct JSON: {}",
            raw.content_type
        );
        let json = raw.json.unwrap();
        assert_eq!(json["id"], id, "{method} id echo");
        assert_eq!(
            json["error"]["code"], -32601,
            "{method} -> METHOD_NOT_FOUND"
        );
    }
}

#[tokio::test]
async fn raw_modern_initialize_is_rejected_with_method_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    // The server only allows `initialize` for the legacy `2025-11-25`
    // lifecycle; a modern `2026-07-28` initialize is rejected in
    // `SerialHandler::initialize` with METHOD_NOT_FOUND before any peer
    // bookkeeping. rmcp routes the stateless (discover-lifecycle) request
    // through `serve_negotiated_request_directly`, which maps the handler's
    // `-32601` to HTTP 404 with a direct JSON body — the same modern
    // routing semantics as ping/logging/setLevel/subscribe. No session is
    // established, so no `Mcp-Session-Id` header appears.
    let mut params = json!({
        "protocolVersion": "2026-07-28",
        "capabilities": {},
        "clientInfo": {"name": "serial-mcp-test", "version": "1"},
    });
    params["_meta"] = modern_meta();
    let raw = raw_post(
        &server.url,
        Some(22),
        "initialize",
        params,
        Some("2026-07-28"),
        None,
        None,
        None,
    )
    .await;
    assert_eq!(raw.status, 404, "modern initialize -> HTTP 404");
    assert!(
        raw.content_type.starts_with("application/json"),
        "modern initialize error is direct JSON: {}",
        raw.content_type
    );
    assert!(
        raw.session_id.is_none(),
        "rejected initialize establishes no session header"
    );
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 22, "request id echoed");
    assert_eq!(
        json["error"]["code"], -32601,
        "modern initialize -> METHOD_NOT_FOUND"
    );
    assert_eq!(json["error"]["message"], "initialize");
}

#[tokio::test]
async fn raw_legacy_ping_succeeds_and_subscription_methods_are_method_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    let (session, _init) = raw_legacy_session(&server.url).await;

    let ping = raw_legacy(&server.url, &session, 23, "ping", json!({})).await;
    assert_eq!(ping.status, 200);
    let json = ping.json.unwrap();
    assert_eq!(json["id"], 23);
    assert_eq!(json["result"], json!({}), "legacy ping -> empty result");

    for (id, method, params) in [
        (24, "resources/subscribe", json!({"uri": "serial://ports"})),
        (
            25,
            "resources/unsubscribe",
            json!({"uri": "serial://ports"}),
        ),
    ] {
        let raw = raw_legacy(&server.url, &session, id, method, params).await;
        assert_eq!(raw.status, 200, "{method} legacy error stays in SSE 200");
        let json = raw.json.unwrap();
        assert_eq!(json["id"], id);
        assert_eq!(
            json["error"]["code"], -32601,
            "{method} -> METHOD_NOT_FOUND"
        );
    }
}

#[tokio::test]
async fn raw_listen_is_method_not_found_for_both_protocols() {
    let server = common::spawned::SpawnedServer::start().await;

    let modern = raw_modern(
        &server.url,
        26,
        "subscriptions/listen",
        json!({"notifications": {"resourceSubscriptions": []}}),
        None,
    )
    .await;
    assert_eq!(modern.status, 404, "modern listen -> HTTP 404");
    assert!(
        modern.content_type.starts_with("application/json"),
        "modern listen error is direct JSON: {}",
        modern.content_type
    );
    let json = modern.json.unwrap();
    assert_eq!(json["id"], 26);
    assert_eq!(
        json["error"]["code"], -32601,
        "modern listen METHOD_NOT_FOUND"
    );

    let (session, _init) = raw_legacy_session(&server.url).await;
    let legacy = raw_legacy(
        &server.url,
        &session,
        27,
        "subscriptions/listen",
        json!({"notifications": {"resourceSubscriptions": []}}),
    )
    .await;
    assert_eq!(legacy.status, 200, "legacy listen error stays in SSE 200");
    assert!(
        legacy.content_type.starts_with("text/event-stream"),
        "legacy listen error arrives over SSE: {}",
        legacy.content_type
    );
    let json = legacy.json.unwrap();
    assert_eq!(json["id"], 27);
    assert_eq!(
        json["error"]["code"], -32601,
        "legacy listen METHOD_NOT_FOUND"
    );
}
