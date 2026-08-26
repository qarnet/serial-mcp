//! Standalone historical MCP client interoperability fixture.
//!
//! This fixture compiles against the exact pinned `rmcp 1.7.0`, the
//! pre-migration SDK resolved by `origin/main`, to prove that the current
//! serial-mcp server interoperates with a real historical client over both
//! transports:
//!
//! ```text
//! rmcp-1-client http <server-url>
//! rmcp-1-client stdio <absolute-serial-mcp-binary>
//! ```
//!
//! This package remains standalone. It does not depend on serial-mcp library
//! internals or import current test constants. Every expected value below is
//! an independent fixture-local constant, providing implementation evidence
//! rather than mirroring current code.
//!
//! Both modes run the same public-behavior verifier. It checks peer info,
//! negotiated `2025-11-25`, server identity, the exact 25-tool surface,
//! resources, templates, prompts, and one real `compute_checksum` call. The
//! client then shuts down cleanly. A verification failure still attempts
//! client cancellation before returning the error. On success, one JSON line
//! is printed to stdout:
//!
//! ```json
//! {"mode":"http|stdio","protocolVersion":"2025-11-25","tools":25,"status":"ok"}
//! ```
//!
//! Runtime validation does not panic. Every failure is a contextual error that
//! exits nonzero.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rmcp::model::{CallToolRequestParams, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use serde_json::{Map, Value};

/// Defines the CLI contract. Invalid or missing arguments print this to stderr
/// and exit nonzero.
const USAGE: &str = "\
usage: rmcp-1-client http <server-url>
       rmcp-1-client stdio <absolute-serial-mcp-binary>";

/// Maximum duration for one complete mode, including initialize, verification,
/// and shutdown.
const MODE_TIMEOUT: Duration = Duration::from_secs(30);

/// Independent fixture-local expected tool names. The verifier compares sets,
/// so order is not part of the contract.
const EXPECTED_TOOLS: [&str; 25] = [
    "list_ports",
    "list_connections",
    "open",
    "close",
    "write",
    "transact",
    "read",
    "capture_boot",
    "flush",
    "set_dtr_rts",
    "set_flow_control",
    "send_break",
    "get_status",
    "reconfigure",
    "list_profiles",
    "open_profile",
    "save_profile",
    "delete_profile",
    "configure",
    "rollback_profile",
    "get_log",
    "clear_log",
    "export_log",
    "reconnect",
    "compute_checksum",
];

/// Independent fixture-local expected static resource URIs.
const EXPECTED_RESOURCES: [&str; 2] = ["serial://ports", "serial://connections"];

/// Independent fixture-local expected resource templates for connection detail,
/// raw, and log resources.
const EXPECTED_RESOURCE_TEMPLATES: [&str; 3] = [
    "serial://connections/{id}",
    "serial://connections/{id}/raw",
    "serial://connections/{id}/log",
];

/// Independent fixture-local expected prompt names.
const EXPECTED_PROMPTS: [&str; 2] = ["diagnose_port", "interactive_terminal"];

#[tokio::main]
async fn main() -> std::process::ExitCode {
    match run().await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rmcp-1-client: {error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mode = args
        .first()
        .map(String::as_str)
        .ok_or_else(|| anyhow!("{USAGE}"))?;
    if args.len() != 2 {
        return Err(anyhow!("{USAGE}"));
    }
    match mode {
        "http" => {
            let url = args[1].as_str();
            tokio::time::timeout(MODE_TIMEOUT, http_mode(url))
                .await
                .map_err(|_| anyhow!("http mode timed out after 30s"))??;
        }
        "stdio" => {
            let binary = args[1].as_str();
            tokio::time::timeout(MODE_TIMEOUT, stdio_mode(binary))
                .await
                .map_err(|_| anyhow!("stdio mode timed out after 30s"))??;
        }
        _ => return Err(anyhow!("{USAGE}")),
    }
    Ok(())
}

/// Runs HTTP mode through rmcp 1.7.0's reqwest Streamable HTTP client transport
/// against the passed server URL (`/mcp` on the current server).
async fn http_mode(url: &str) -> Result<()> {
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = ().serve(transport).await.context("http mode: initialize/session failed")?;
    verify_and_shutdown(client, "http").await
}

/// Runs stdio mode by spawning the current serial-mcp binary with an isolated
/// temporary profile path and `RUST_LOG=off`, then drives it through rmcp
/// 1.7.0's `TokioChildProcess` transport. The temporary directory stays alive
/// through client shutdown and is dropped only after the child closes.
async fn stdio_mode(binary: &str) -> Result<()> {
    let temp_dir = tempfile::tempdir().context("stdio mode: create temp directory")?;
    let profiles_path = temp_dir.path().join("profiles.toml");
    let mut command = tokio::process::Command::new(binary);
    command.arg("--profiles-path").arg(&profiles_path);
    command.env("RUST_LOG", "off");
    let child =
        TokioChildProcess::new(command).context("stdio mode: spawn serial-mcp child process")?;
    let client = ().serve(child).await.context("stdio mode: initialize/session failed")?;
    // Keep temp_dir alive through verification and shutdown so the child never
    // observes a missing profile path.
    verify_and_shutdown(client, "stdio").await
}

/// Verifies public behavior, then shuts the client down cleanly. On
/// verification failure, it still attempts cancellation before returning the
/// error. Stdio child cleanup follows client shutdown through
/// `Transport::close` and `graceful_shutdown`.
async fn verify_and_shutdown(client: RunningService<RoleClient, ()>, mode: &str) -> Result<()> {
    let verified = verify(&client).await;
    if let Err(error) = &verified {
        // Cancel before returning the error so a child or HTTP session is not
        // left running after a failed check.
        let _ = client.cancel().await;
        return Err(anyhow!("{error}"));
    }
    client
        .cancel()
        .await
        .context("clean client shutdown failed")?;
    let summary = serde_json::json!({
        "mode": mode,
        "protocolVersion": "2025-11-25",
        "tools": 25,
        "status": "ok",
    });
    println!("{summary}");
    Ok(())
}

/// Verifies the public compatibility contract with rmcp 1.7.0. The all-page
/// helpers (`list_all_*`) prevent pagination from hiding items. The verifier
/// returns a descriptive error on the first failed check.
async fn verify(client: &RunningService<RoleClient, ()>) -> std::result::Result<(), String> {
    let peer_info = client
        .peer_info()
        .ok_or_else(|| "peer info missing after initialize".to_string())?;

    if peer_info.protocol_version != ProtocolVersion::V_2025_11_25 {
        return Err(format!(
            "negotiated protocol {:?}, expected {:?}",
            peer_info.protocol_version,
            ProtocolVersion::V_2025_11_25
        ));
    }

    if peer_info.server_info.name != "serial-mcp" {
        return Err(format!(
            "server implementation name {:?}, expected \"serial-mcp\"",
            peer_info.server_info.name
        ));
    }

    // Check the raw tool count before set equality so duplicate entries cannot
    // satisfy the expected set. Then compare names with the fixture-local
    // constant.
    let tools = client
        .list_all_tools()
        .await
        .map_err(|e| format!("list_all_tools failed: {e}"))?;
    if tools.len() != EXPECTED_TOOLS.len() {
        return Err(format!(
            "tool count mismatch: got {} tool(s), expected {}",
            tools.len(),
            EXPECTED_TOOLS.len()
        ));
    }
    let tool_names: BTreeSet<&str> = tools.iter().map(|t| t.name.as_ref()).collect();
    let expected_tools: BTreeSet<&str> = EXPECTED_TOOLS.into_iter().collect();
    if tool_names != expected_tools {
        return Err(format!(
            "tool name set mismatch: got {} tool(s) {tool_names:?}, expected {}",
            tool_names.len(),
            EXPECTED_TOOLS.len()
        ));
    }

    // Check the raw resource count before comparing the exact URI set.
    let resources = client
        .list_all_resources()
        .await
        .map_err(|e| format!("list_all_resources failed: {e}"))?;
    if resources.len() != EXPECTED_RESOURCES.len() {
        return Err(format!(
            "resource count mismatch: got {} resource(s), expected {}",
            resources.len(),
            EXPECTED_RESOURCES.len()
        ));
    }
    let resource_uris: BTreeSet<&str> = resources.iter().map(|r| r.uri.as_str()).collect();
    let expected_resources: BTreeSet<&str> = EXPECTED_RESOURCES.into_iter().collect();
    if resource_uris != expected_resources {
        return Err(format!(
            "resource URI set mismatch: got {} resource(s) {resource_uris:?}, expected {}",
            resource_uris.len(),
            expected_resources.len()
        ));
    }

    // Check the raw template count before comparing the exact URI-template set.
    let templates = client
        .list_all_resource_templates()
        .await
        .map_err(|e| format!("list_all_resource_templates failed: {e}"))?;
    if templates.len() != EXPECTED_RESOURCE_TEMPLATES.len() {
        return Err(format!(
            "resource template count mismatch: got {} template(s), expected {}",
            templates.len(),
            EXPECTED_RESOURCE_TEMPLATES.len()
        ));
    }
    let template_uris: BTreeSet<&str> = templates.iter().map(|t| t.uri_template.as_str()).collect();
    let expected_templates: BTreeSet<&str> = EXPECTED_RESOURCE_TEMPLATES.into_iter().collect();
    if template_uris != expected_templates {
        return Err(format!(
            "resource template URI set mismatch: got {} template(s) {template_uris:?}, expected {}",
            template_uris.len(),
            expected_templates.len()
        ));
    }

    // Check the raw prompt count before comparing the exact name set.
    let prompts = client
        .list_all_prompts()
        .await
        .map_err(|e| format!("list_all_prompts failed: {e}"))?;
    if prompts.len() != EXPECTED_PROMPTS.len() {
        return Err(format!(
            "prompt count mismatch: got {} prompt(s), expected {}",
            prompts.len(),
            EXPECTED_PROMPTS.len()
        ));
    }
    let prompt_names: BTreeSet<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    let expected_prompts: BTreeSet<&str> = EXPECTED_PROMPTS.into_iter().collect();
    if prompt_names != expected_prompts {
        return Err(format!(
            "prompt name set mismatch: got {} prompt(s) {prompt_names:?}, expected {}",
            prompt_names.len(),
            expected_prompts.len()
        ));
    }

    // Require a successful compute_checksum result before reading structured
    // fields. The fixture expects checksum=111 and checksum_hex="6F".
    let mut arguments = Map::new();
    arguments.insert("algorithm".to_string(), Value::String("xor".to_string()));
    arguments.insert("data".to_string(), Value::String("$GPGGA,1".to_string()));
    arguments.insert("encoding".to_string(), Value::String("utf8".to_string()));
    let call = client
        .call_tool(CallToolRequestParams::new("compute_checksum").with_arguments(arguments))
        .await
        .map_err(|e| format!("compute_checksum call failed: {e}"))?;
    if call.is_error != Some(false) {
        return Err(format!(
            "compute_checksum result is an error: is_error={:?}",
            call.is_error
        ));
    }
    let structured = call
        .structured_content
        .ok_or_else(|| "compute_checksum result carries no structured_content".to_string())?;
    let checksum = structured
        .get("checksum")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("structured result lacks integer checksum: {structured}"))?;
    let checksum_hex = structured
        .get("checksum_hex")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("structured result lacks string checksum_hex: {structured}"))?;
    if checksum != 111 || checksum_hex != "6F" {
        return Err(format!(
            "compute_checksum result mismatch: checksum={checksum} checksum_hex={checksum_hex:?}, \
             expected 111 / \"6F\""
        ));
    }

    Ok(())
}
