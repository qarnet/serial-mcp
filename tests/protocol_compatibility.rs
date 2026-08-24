//! MCP protocol compatibility matrix indexed by exact protocol version.
//! It covers the `2026-07-28` discovery/stateless lifecycle and the
//! `2025-11-25` initialize/session lifecycle.
//!
//! The tests use two independent proof layers:
//!
//! - The typed layer uses an in-process [`TestServer`] and real `rmcp` clients
//!   through `serve_with_lifecycle`. It checks the negotiated version,
//!   tool/resource/prompt surface, hardware-free tool execution, and
//!   capability views.
//! - The raw-wire layer uses the spawned binary and `reqwest` to send
//!   hand-built JSON-RPC POSTs with explicit `MCP-Protocol-Version`, SEP-2243
//!   `Mcp-Method`/`Mcp-Name`, and `Mcp-Session-Id` headers. It checks exact HTTP
//!   statuses, JSON-RPC error codes, `resultType` presence, and response-ID
//!   echo without typed rmcp result deserialization.
//!
//! Raw expected values never derive from production `src/mcp_protocol.rs`.
//! Keeping the layers independent prevents implementation and expectation from
//! failing together. The coverage-lock test compares the independent
//! `TestProtocol::ALL` list with the raw `server/discover` `supportedVersions`
//! output.

mod common;

use std::collections::BTreeSet;
use std::future::Future;

use anyhow::Result;
use base64::Engine as _;
use common::{TestProtocol, TestServer};
use rmcp::model::{PaginatedRequestParams, ReadResourceRequestParams};
use rmcp::service::RoleClient;
use serde_json::{json, Value};

/// Builds the exact `2026-07-28` per-request `_meta` for raw modern requests.
/// This represents SEP-2575 client context. The protocol treats `clientInfo`
/// as optional, but this raw test fixture includes it.
fn meta_2026_07_28() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientInfo": {"name": "serial-mcp-test", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

/// Returns the expected common capability wire shape for `2025-11-25`.
fn capabilities_2025_11_25_json() -> Value {
    json!({
        "completions": {},
        "prompts": {},
        "resources": {},
        "tools": {},
    })
}

/// Returns the expected `2026-07-28` capability shape: the common set plus
/// `resources.subscribe`, without list-change flags.
fn capabilities_2026_07_28_json() -> Value {
    json!({
        "completions": {},
        "prompts": {},
        "resources": {"subscribe": true},
        "tools": {},
    })
}

/// Runs an async assertion body against a typed client for one exact protocol
/// version on a fresh in-process server. The common
/// [`common::VersionedClientHandler`] serves both lifecycle modes, so every
/// case uses the same return type.
async fn typed_protocol<F, Fut>(protocol: TestProtocol, run: F) -> Result<()>
where
    F: FnOnce(rmcp::service::RunningService<RoleClient, common::VersionedClientHandler>) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let server = TestServer::start().await;
    let (client, _rx) = common::connect_protocol_client(&server, protocol).await?;
    run(client).await
}

#[tokio::test]
async fn typed_protocol_lifecycle_selects_exact_version() {
    for protocol in TestProtocol::ALL {
        typed_protocol(protocol, |client| async move {
            let info = client.peer_info().expect("peer info");
            assert_eq!(
                info.protocol_version,
                protocol.version(),
                "case {protocol:?} must negotiate exact version {}",
                protocol.version()
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn typed_protocol_tools_list_returns_exact_twenty_five_tools() {
    for protocol in TestProtocol::ALL {
        typed_protocol(protocol, |client| async move {
            let result = client
                .peer()
                .list_tools(Some(PaginatedRequestParams::default()))
                .await
                .unwrap();
            let names: BTreeSet<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
            let expected: BTreeSet<&str> = common::EXPECTED_TOOLS.iter().copied().collect();
            assert_eq!(
                names, expected,
                "case {protocol:?} tools/list must match EXPECTED_TOOLS"
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn typed_protocol_resources_and_templates_and_prompts() {
    for protocol in TestProtocol::ALL {
        typed_protocol(protocol, |client| async move {
            let resources = client
                .peer()
                .list_resources(Some(PaginatedRequestParams::default()))
                .await
                .unwrap();
            assert_eq!(
                resources.resources.len(),
                2,
                "case {protocol:?}: two static resources"
            );
            let templates = client
                .peer()
                .list_resource_templates(Some(PaginatedRequestParams::default()))
                .await
                .unwrap();
            assert_eq!(
                templates.resource_templates.len(),
                3,
                "case {protocol:?}: three templates"
            );
            let prompts = client
                .peer()
                .list_prompts(Some(PaginatedRequestParams::default()))
                .await
                .unwrap();
            assert_eq!(prompts.prompts.len(), 2, "case {protocol:?}: two prompts");
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn typed_protocol_compute_checksum_succeeds_without_hardware() {
    for protocol in TestProtocol::ALL {
        typed_protocol(protocol, |client| async move {
            let result = client
                .peer()
                .call_tool(common::tool_request(
                    "compute_checksum",
                    json!({"data": "$GPGGA,1", "algorithm": "xor"}),
                ))
                .await
                .unwrap();
            assert_eq!(
                result.is_error,
                Some(false),
                "case {protocol:?}: {result:?}"
            );
            let structured = result.structured_content.expect("structured content");
            assert_eq!(structured["checksum"], 111, "case {protocol:?}");
            assert_eq!(structured["checksum_hex"], "6F", "case {protocol:?}");
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn typed_protocol_read_serial_ports_resource_succeeds() {
    for protocol in TestProtocol::ALL {
        typed_protocol(protocol, |client| async move {
            let result = client
                .peer()
                .read_resource(ReadResourceRequestParams::new("serial://ports"))
                .await
                .unwrap();
            assert!(!result.contents.is_empty(), "case {protocol:?}");
            match &result.contents[0] {
                rmcp::model::ResourceContents::TextResourceContents { uri, mime_type, .. } => {
                    assert_eq!(uri.as_str(), "serial://ports", "case {protocol:?}");
                    assert_eq!(
                        mime_type.as_deref(),
                        Some("application/json"),
                        "case {protocol:?}"
                    );
                }
                other => {
                    panic!("case {protocol:?}: expected text resource contents, got {other:?}")
                }
            }
            Ok(())
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn typed_2026_07_28_capabilities_advertise_resource_subscriptions() {
    typed_protocol(TestProtocol::V2026_07_28, |client| async move {
        let caps = client
            .peer_info()
            .expect("2026-07-28 peer info")
            .capabilities
            .clone();
        assert_eq!(
            serde_json::to_value(caps).unwrap(),
            capabilities_2026_07_28_json(),
            "2026-07-28 capabilities advertise resources.subscribe"
        );
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_2025_11_25_capabilities_keep_subscription_disabled() {
    typed_protocol(TestProtocol::V2025_11_25, |client| async move {
        let caps = client
            .peer_info()
            .expect("2025-11-25 peer info")
            .capabilities
            .clone();
        assert_eq!(
            serde_json::to_value(caps).unwrap(),
            capabilities_2025_11_25_json(),
            "2025-11-25 capabilities keep resource subscriptions disabled"
        );
        Ok(())
    })
    .await
    .unwrap();
}

// SEP-2549 cache fields follow the exact protocol version. For `2026-07-28`,
// every cacheable list or resource-read family carries
// `ttlMs: 0` and `cacheScope: "private"`. Legacy `2025-11-25` peers omit both
// fields. rmcp strips `resultType` for legacy peers but does not strip cache
// fields, so the server omits them itself. Tool calls, prompts/get, and
// completion have no applicable cache fields.

/// Asserts that a typed `2026-07-28` list or read result carries SEP-2549
/// cache fields.
fn assert_2026_07_28_cache_fields(
    ttl_ms: Option<u64>,
    cache_scope: Option<rmcp::model::CacheScope>,
) {
    assert_eq!(ttl_ms, Some(0), "2026-07-28 result must carry ttlMs: 0");
    assert_eq!(
        cache_scope,
        Some(rmcp::model::CacheScope::Private),
        "2026-07-28 result must carry cacheScope: private"
    );
}

/// Asserts that a typed `2025-11-25` list or read result carries neither cache
/// field.
fn assert_2025_11_25_no_cache_fields(
    ttl_ms: Option<u64>,
    cache_scope: Option<rmcp::model::CacheScope>,
) {
    assert_eq!(ttl_ms, None, "2025-11-25 result must omit ttlMs");
    assert_eq!(cache_scope, None, "2025-11-25 result must omit cacheScope");
}

#[tokio::test]
async fn typed_2026_07_28_cache_fields_on_every_cacheable_family() {
    typed_protocol(TestProtocol::V2026_07_28, |client| async move {
        let tools = client
            .peer()
            .list_tools(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2026_07_28_cache_fields(tools.ttl_ms, tools.cache_scope);

        let resources = client
            .peer()
            .list_resources(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2026_07_28_cache_fields(resources.ttl_ms, resources.cache_scope);

        let templates = client
            .peer()
            .list_resource_templates(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2026_07_28_cache_fields(templates.ttl_ms, templates.cache_scope);

        let prompts = client
            .peer()
            .list_prompts(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2026_07_28_cache_fields(prompts.ttl_ms, prompts.cache_scope);

        // Each supported resources/read URI kind carries these fields.
        for uri in ["serial://ports", "serial://connections"] {
            let read = client
                .peer()
                .read_resource(ReadResourceRequestParams::new(uri))
                .await
                .unwrap();
            assert_2026_07_28_cache_fields(read.ttl_ms, read.cache_scope);
        }
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_2025_11_25_cache_fields_absent_on_every_cacheable_family() {
    typed_protocol(TestProtocol::V2025_11_25, |client| async move {
        let tools = client
            .peer()
            .list_tools(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2025_11_25_no_cache_fields(tools.ttl_ms, tools.cache_scope);

        let resources = client
            .peer()
            .list_resources(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2025_11_25_no_cache_fields(resources.ttl_ms, resources.cache_scope);

        let templates = client
            .peer()
            .list_resource_templates(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2025_11_25_no_cache_fields(templates.ttl_ms, templates.cache_scope);

        let prompts = client
            .peer()
            .list_prompts(Some(PaginatedRequestParams::default()))
            .await
            .unwrap();
        assert_2025_11_25_no_cache_fields(prompts.ttl_ms, prompts.cache_scope);

        let read = client
            .peer()
            .read_resource(ReadResourceRequestParams::new("serial://ports"))
            .await
            .unwrap();
        assert_2025_11_25_no_cache_fields(read.ttl_ms, read.cache_scope);
        Ok(())
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn typed_2026_07_28_list_cursor_pages_are_honored() {
    // The explicit tools/list and prompts/list handlers use the same
    // `paginate` helper as resources. A cursor past the single-page catalog
    // yields an empty page without a next cursor.
    typed_protocol(TestProtocol::V2026_07_28, |client| async move {
        let cursor = base64::engine::general_purpose::STANDARD.encode("999".as_bytes());
        let tools = client
            .peer()
            .list_tools(Some(PaginatedRequestParams::with_cursor(
                PaginatedRequestParams::default(),
                Some(cursor.clone()),
            )))
            .await
            .unwrap();
        assert!(
            tools.tools.is_empty(),
            "cursor past end -> empty tools page"
        );
        assert!(tools.next_cursor.is_none(), "no next cursor after the end");

        let prompts = client
            .peer()
            .list_prompts(Some(PaginatedRequestParams::with_cursor(
                PaginatedRequestParams::default(),
                Some(cursor),
            )))
            .await
            .unwrap();
        assert!(
            prompts.prompts.is_empty(),
            "cursor past end -> empty prompts page"
        );
        assert!(
            prompts.next_cursor.is_none(),
            "no next cursor after the end"
        );
        Ok(())
    })
    .await
    .unwrap();
}

/// Stores a raw HTTP response and its parsed JSON-RPC payload. The payload may
/// come from a direct JSON body or the first non-empty SSE `data:` event.
#[derive(Debug)]
struct RawWire {
    status: u16,
    content_type: String,
    /// `Mcp-Session-Id` response header when present.
    session_id: Option<String>,
    /// First non-empty JSON-RPC payload when the response carried one.
    json: Option<Value>,
}

/// Parses a direct JSON response body or an SSE stream of `data:` events and
/// returns the first non-empty JSON value.
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
        // JSON-RPC notifications have no id. Including one would make the
        // server treat this message as a request and return -32601.
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

/// Builds a `2026-07-28` request with matching `_meta` and
/// `MCP-Protocol-Version` values plus the SEP-2243 `Mcp-Method`/`Mcp-Name`
/// headers required by rmcp.
async fn raw_2026_07_28(
    url: &str,
    id: u64,
    method: &str,
    params: Value,
    name: Option<&str>,
) -> RawWire {
    let mut params = params;
    params["_meta"] = meta_2026_07_28();
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

/// Starts a `2025-11-25` session, captures `Mcp-Session-Id`, and sends
/// `notifications/initialized`. Returns the session id and initialize response.
async fn raw_2025_11_25_session(url: &str) -> (String, RawWire) {
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

/// Builds a `2025-11-25` request with the session header and matching
/// `MCP-Protocol-Version` value.
async fn raw_2025_11_25(url: &str, session: &str, id: u64, method: &str, params: Value) -> RawWire {
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

#[tokio::test]
async fn raw_discover_succeeds_without_session_and_lists_2026_07_28_first() {
    let server = common::spawned::SpawnedServer::start().await;
    let raw = raw_2026_07_28(&server.url, 1, "server/discover", json!({}), None).await;
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
        "supportedVersions exactly 2026-07-28 then 2025-11-25"
    );
    assert_eq!(result["capabilities"], capabilities_2026_07_28_json());
    // `server/discover` is not a cacheable list or read family. It carries
    // only rmcp's required zero/private cache fields, not server-added fields.
    // The version-indexed cache tests below cover the cacheable families.
}

#[tokio::test]
async fn coverage_matrix_matches_exact_supported_versions_on_the_wire() {
    // Keep the independent test-case list `TestProtocol::ALL` aligned with the
    // exact `supportedVersions` returned by raw `server/discover`. A missing,
    // extra, or reordered version fails this test, so every future production
    // policy row requires an explicit test case. The expected list remains
    // independent from `src/mcp_protocol.rs` internals.
    let server = common::spawned::SpawnedServer::start().await;
    let raw = raw_2026_07_28(&server.url, 50, "server/discover", json!({}), None).await;
    assert_eq!(raw.status, 200, "discover without session id succeeds");
    let json = raw.json.expect("discover JSON");
    let expected: Vec<String> = TestProtocol::ALL
        .iter()
        .map(|p| p.version().as_str().to_string())
        .collect();
    let wire: Vec<String> = json["result"]["supportedVersions"]
        .as_array()
        .expect("supportedVersions array")
        .iter()
        .map(|v| v.as_str().expect("version string").to_string())
        .collect();
    assert_eq!(
        wire, expected,
        "server/discover supportedVersions must equal TestProtocol::ALL exactly, in order"
    );
}

#[tokio::test]
async fn raw_2026_07_28_surface_includes_result_type_complete() {
    let server = common::spawned::SpawnedServer::start().await;

    let raw = raw_2026_07_28(&server.url, 2, "tools/list", json!({}), None).await;
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

    let raw = raw_2026_07_28(&server.url, 3, "resources/list", json!({}), None).await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert_eq!(json["result"]["resources"].as_array().unwrap().len(), 2);

    let raw = raw_2026_07_28(
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

    let raw = raw_2026_07_28(&server.url, 5, "prompts/list", json!({}), None).await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert_eq!(json["result"]["resultType"], "complete");
    assert_eq!(json["result"]["prompts"].as_array().unwrap().len(), 2);

    let raw = raw_2026_07_28(
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

    let raw = raw_2026_07_28(
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

    let raw = raw_2026_07_28(
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
async fn raw_2025_11_25_responses_omit_result_type() {
    let server = common::spawned::SpawnedServer::start().await;
    let (session, init) = raw_2025_11_25_session(&server.url).await;
    assert_eq!(init.status, 200);
    let init_json = init.json.clone().unwrap();
    assert_eq!(init_json["id"], 400);
    assert_eq!(init_json["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        init_json["result"]["capabilities"],
        capabilities_2025_11_25_json()
    );

    let list = raw_2025_11_25(&server.url, &session, 10, "tools/list", json!({})).await;
    assert_eq!(list.status, 200);
    assert!(
        list.content_type.starts_with("text/event-stream"),
        "2025-11-25 responses arrive over SSE: {}",
        list.content_type
    );
    let json = list.json.unwrap();
    assert_eq!(json["id"], 10);
    assert!(
        json["result"].get("resultType").is_none(),
        "2025-11-25 responses must omit resultType: {json}"
    );
    assert_eq!(
        json["result"]["tools"].as_array().unwrap().len(),
        common::EXPECTED_TOOLS.len()
    );

    let call = raw_2025_11_25(
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
async fn raw_2026_07_28_cache_fields_present_2025_11_25_absent() {
    let server = common::spawned::SpawnedServer::start().await;

    // Modern list-family responses carry `ttlMs: 0` and private `cacheScope`.
    for (id, method, params, name) in [
        (30, "tools/list", json!({}), None),
        (31, "resources/list", json!({}), None),
        (32, "resources/templates/list", json!({}), None),
        (33, "prompts/list", json!({}), None),
    ] {
        let raw = raw_2026_07_28(&server.url, id, method, params, name).await;
        assert_eq!(raw.status, 200, "{method} 2026-07-28 -> 200");
        let json = raw.json.unwrap();
        assert_eq!(json["result"]["ttlMs"], 0, "{method} 2026-07-28 ttlMs");
        assert_eq!(
            json["result"]["cacheScope"], "private",
            "{method} 2026-07-28 cacheScope"
        );
    }

    // Modern resources/read responses carry the same fields for both static
    // URI kinds.
    for (id, uri) in [(34u64, "serial://ports"), (35u64, "serial://connections")] {
        let raw = raw_2026_07_28(
            &server.url,
            id,
            "resources/read",
            json!({"uri": uri}),
            Some(uri),
        )
        .await;
        assert_eq!(raw.status, 200, "2026-07-28 read {uri} -> 200");
        let json = raw.json.unwrap();
        assert_eq!(json["result"]["ttlMs"], 0, "2026-07-28 read {uri} ttlMs");
        assert_eq!(
            json["result"]["cacheScope"], "private",
            "2026-07-28 read {uri} cacheScope"
        );
    }

    // Legacy responses must omit both cache fields.
    let (session, _init) = raw_2025_11_25_session(&server.url).await;
    for (id, method, params) in [
        (36, "tools/list", json!({})),
        (37, "resources/list", json!({})),
        (38, "resources/templates/list", json!({})),
        (39, "prompts/list", json!({})),
    ] {
        let raw = raw_2025_11_25(&server.url, &session, id, method, params).await;
        assert_eq!(raw.status, 200, "{method} 2025-11-25 -> 200");
        let json = raw.json.unwrap();
        assert!(
            json["result"].get("ttlMs").is_none(),
            "{method} 2025-11-25 must omit ttlMs: {json}"
        );
        assert!(
            json["result"].get("cacheScope").is_none(),
            "{method} 2025-11-25 must omit cacheScope: {json}"
        );
    }

    let raw = raw_2025_11_25(
        &server.url,
        &session,
        40,
        "resources/read",
        json!({"uri": "serial://ports"}),
    )
    .await;
    assert_eq!(raw.status, 200, "2025-11-25 read -> 200");
    let json = raw.json.unwrap();
    assert!(
        json["result"].get("ttlMs").is_none(),
        "2025-11-25 read must omit ttlMs: {json}"
    );
    assert!(
        json["result"].get("cacheScope").is_none(),
        "2025-11-25 read must omit cacheScope: {json}"
    );
}

#[tokio::test]
async fn raw_2026_07_28_list_cursor_pages_are_honored() {
    let server = common::spawned::SpawnedServer::start().await;
    // The manual tools/list and prompts/list handlers use the same `paginate`
    // helper as resources. A cursor past the single-page catalog yields an
    // empty page without a next cursor.
    let cursor = base64::engine::general_purpose::STANDARD.encode("999".as_bytes());
    let raw = raw_2026_07_28(
        &server.url,
        41,
        "tools/list",
        json!({"cursor": cursor.clone()}),
        None,
    )
    .await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert!(
        json["result"]["tools"].as_array().unwrap().is_empty(),
        "cursor past end -> empty tools page"
    );
    assert!(
        json["result"].get("nextCursor").is_none(),
        "no next cursor after the end"
    );

    let raw = raw_2026_07_28(
        &server.url,
        42,
        "prompts/list",
        json!({"cursor": cursor}),
        None,
    )
    .await;
    assert_eq!(raw.status, 200);
    let json = raw.json.unwrap();
    assert!(
        json["result"]["prompts"].as_array().unwrap().is_empty(),
        "cursor past end -> empty prompts page"
    );
    assert!(
        json["result"].get("nextCursor").is_none(),
        "no next cursor after the end"
    );
}

#[tokio::test]
async fn raw_2026_07_28_unknown_resource_is_invalid_params_with_uri() {
    let server = common::spawned::SpawnedServer::start().await;
    let raw = raw_2026_07_28(
        &server.url,
        12,
        "resources/read",
        json!({"uri": "serial://does-not-exist"}),
        Some("serial://does-not-exist"),
    )
    .await;
    assert_eq!(raw.status, 400, "2026-07-28 unknown resource -> HTTP 400");
    assert!(
        raw.content_type.starts_with("application/json"),
        "2026-07-28 errors are direct JSON: {}",
        raw.content_type
    );
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 12, "request id echoed");
    assert_eq!(
        json["error"]["code"], -32602,
        "INVALID_PARAMS for 2026-07-28"
    );
    assert_eq!(
        json["error"]["data"]["uri"], "serial://does-not-exist",
        "error data carries the requested URI"
    );
}

#[tokio::test]
async fn raw_2025_11_25_unknown_resource_keeps_resource_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    let (session, _init) = raw_2025_11_25_session(&server.url).await;
    let raw = raw_2025_11_25(
        &server.url,
        &session,
        13,
        "resources/read",
        json!({"uri": "serial://does-not-exist"}),
    )
    .await;
    assert_eq!(raw.status, 200, "2025-11-25 error stays inside an SSE 200");
    let json = raw.json.unwrap();
    assert_eq!(json["id"], 13);
    assert_eq!(
        json["error"]["code"], -32002,
        "2025-11-25 keeps RESOURCE_NOT_FOUND"
    );
    assert_eq!(json["error"]["data"]["uri"], "serial://does-not-exist");
}

#[tokio::test]
async fn raw_2026_07_28_missing_required_meta_returns_400() {
    let server = common::spawned::SpawnedServer::start().await;

    // `server/discover` without `_meta` must fail; headers cannot replace it.
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

    // An `_meta` object without `clientCapabilities` must also fail.
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
async fn raw_2026_07_28_header_meta_version_mismatch_returns_400() {
    let server = common::spawned::SpawnedServer::start().await;
    let mut params = json!({});
    params["_meta"] = meta_2026_07_28();
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
async fn raw_strict_metadata_option_rejects_modern_request_missing_header() {
    // Check the opt-in `stateless_protocol_metadata_required` seam on the
    // shipped HTTP binary (see `common::spawned`). A modern `2026-07-28`
    // request with complete per-request `_meta` routes through stateless
    // handling before the missing `MCP-Protocol-Version` header causes HTTP
    // 400 and JSON-RPC `-32020` (HEADER_MISMATCH). The missing `result`
    // confirms that no tool ran.
    //
    // This isolates the strict option's reachable behavior under mixed
    // routing. rmcp classifies requests missing both signals as legacy and
    // rejects them earlier with HTTP 422 and no JSON-RPC body. This test does
    // not assert that rmcp-owned path.
    let server = common::spawned::SpawnedServer::start().await;
    let mut params = json!({});
    params["_meta"] = meta_2026_07_28();
    let raw = raw_post(
        &server.url,
        Some(26),
        "tools/list",
        params,
        None, // no MCP-Protocol-Version header
        None,
        Some("tools/list"), // correct Mcp-Method for a modern request
        None,
    )
    .await;
    assert_eq!(
        raw.status, 400,
        "modern stateless request without protocol header -> HTTP 400"
    );
    assert!(
        raw.content_type.starts_with("application/json"),
        "rejection is direct JSON: {}",
        raw.content_type
    );
    let json = raw.json.expect("JSON-RPC error body");
    assert_eq!(json["id"], 26, "request id echoed");
    assert_eq!(
        json["error"]["code"], -32020,
        "missing MCP-Protocol-Version header -> HEADER_MISMATCH before dispatch"
    );
    assert!(
        json.get("result").is_none(),
        "no result: rejected before tool dispatch"
    );
}

#[tokio::test]
async fn raw_2026_07_28_unsupported_version_returns_400_with_supported_list() {
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
async fn raw_2026_07_28_routing_rejects_legacy_only_methods() {
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
        let raw = raw_2026_07_28(&server.url, id, method, params, name).await;
        assert_eq!(raw.status, 404, "{method} -> HTTP 404");
        assert!(
            raw.content_type.starts_with("application/json"),
            "{method} 2026-07-28 error is direct JSON: {}",
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
async fn raw_2026_07_28_initialize_is_rejected_with_method_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    // The server allows `initialize` only for the legacy `2025-11-25`
    // lifecycle. `SerialHandler::initialize` rejects a modern `2026-07-28`
    // initialize with `-32601` before peer bookkeeping. rmcp routes this
    // stateless request through `serve_negotiated_request_directly` and maps
    // the error to HTTP 404 with a direct JSON body, matching modern routing
    // for ping, logging/setLevel, and resource subscription methods. No
    // session is established, so no `Mcp-Session-Id` header appears.
    let mut params = json!({
        "protocolVersion": "2026-07-28",
        "capabilities": {},
        "clientInfo": {"name": "serial-mcp-test", "version": "1"},
    });
    params["_meta"] = meta_2026_07_28();
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
    assert_eq!(raw.status, 404, "2026-07-28 initialize -> HTTP 404");
    assert!(
        raw.content_type.starts_with("application/json"),
        "2026-07-28 initialize error is direct JSON: {}",
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
        "2026-07-28 initialize -> METHOD_NOT_FOUND"
    );
    assert_eq!(json["error"]["message"], "initialize");
}

#[tokio::test]
async fn raw_2025_11_25_ping_succeeds_and_subscription_methods_are_method_not_found() {
    let server = common::spawned::SpawnedServer::start().await;
    let (session, _init) = raw_2025_11_25_session(&server.url).await;

    let ping = raw_2025_11_25(&server.url, &session, 23, "ping", json!({})).await;
    assert_eq!(ping.status, 200);
    let json = ping.json.unwrap();
    assert_eq!(json["id"], 23);
    assert_eq!(json["result"], json!({}), "2025-11-25 ping -> empty result");

    for (id, method, params) in [
        (24, "resources/subscribe", json!({"uri": "serial://ports"})),
        (
            25,
            "resources/unsubscribe",
            json!({"uri": "serial://ports"}),
        ),
    ] {
        let raw = raw_2025_11_25(&server.url, &session, id, method, params).await;
        assert_eq!(
            raw.status, 200,
            "{method} 2025-11-25 error stays in SSE 200"
        );
        let json = raw.json.unwrap();
        assert_eq!(json["id"], id);
        assert_eq!(
            json["error"]["code"], -32601,
            "{method} -> METHOD_NOT_FOUND"
        );
    }
}

#[tokio::test]
async fn raw_2025_11_25_listen_stays_method_not_found() {
    // Modern `subscriptions/listen` coverage lives in
    // tests/resource_subscriptions.rs. A raw modern listen is a long-lived
    // SSE stream that completes only on cancellation, so typed clients drive
    // it. Legacy `2025-11-25` must not see that surface: rmcp gates the method
    // and the server returns `-32601` inside an SSE 200 response.
    let server = common::spawned::SpawnedServer::start().await;
    let (session, _init) = raw_2025_11_25_session(&server.url).await;
    let legacy = raw_2025_11_25(
        &server.url,
        &session,
        27,
        "subscriptions/listen",
        json!({"notifications": {"resourceSubscriptions": []}}),
    )
    .await;
    assert_eq!(
        legacy.status, 200,
        "2025-11-25 listen error stays in SSE 200"
    );
    assert!(
        legacy.content_type.starts_with("text/event-stream"),
        "2025-11-25 listen error arrives over SSE: {}",
        legacy.content_type
    );
    let json = legacy.json.unwrap();
    assert_eq!(json["id"], 27);
    assert_eq!(
        json["error"]["code"], -32601,
        "2025-11-25 listen METHOD_NOT_FOUND"
    );
}
