//! Standalone historical MCP client interoperability fixture.
//!
//! Compiled against the exact pinned `rmcp 1.7.0` (the pre-migration SDK that
//! `origin/main` resolved before the rmcp 3 migration) to prove the CURRENT
//! serial-mcp server still interoperates with a real historical client
//! implementation over both transports:
//!
//! ```text
//! rmcp-1-client http <server-url>
//! rmcp-1-client stdio <absolute-serial-mcp-binary>
//! ```
//!
//! This package is deliberately standalone: it does not depend on serial-mcp
//! library internals and does not import current test constants. Every
//! expected value below is an independent fixture-local constant — the whole
//! point is independent implementation evidence, not a mirror of current
//! code.
//!
//! Both modes run the same public-behavior verifier (peer info, negotiated
//! `2025-11-25`, server identity, exact 25-tool surface, resources,
//! templates, prompts, and one real `compute_checksum` tool call), then shut
//! the client down cleanly. A verification failure still attempts client
//! cancellation before returning the error. On success one JSON line is
//! printed to stdout:
//!
//! ```json
//! {"mode":"http|stdio","protocolVersion":"2025-11-25","tools":25,"status":"ok"}
//! ```
//!
//! Runtime validation never panics: every failure is a contextual error that
//! exits nonzero.

use std::collections::BTreeSet;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use rmcp::model::{CallToolRequestParams, ProtocolVersion};
use rmcp::service::{RoleClient, RunningService, ServiceExt};
use rmcp::transport::{StreamableHttpClientTransport, TokioChildProcess};
use serde_json::{Map, Value};

/// CLI contract. Invalid/missing arguments print this to stderr and exit
/// nonzero.
const USAGE: &str = "\
usage: rmcp-1-client http <server-url>
       rmcp-1-client stdio <absolute-serial-mcp-binary>";

/// How long one complete mode (initialize + verify + shutdown) may take.
const MODE_TIMEOUT: Duration = Duration::from_secs(30);

/// Exact 25-tool surface of the current server, as an independent
/// fixture-local constant (set-compared; order is not the contract).
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

/// Exact static resource URIs of the current server.
const EXPECTED_RESOURCES: [&str; 2] = ["serial://ports", "serial://connections"];

/// Exact resource template URIs of the current server: connection detail,
/// raw, and log templates.
const EXPECTED_RESOURCE_TEMPLATES: [&str; 3] = [
    "serial://connections/{id}",
    "serial://connections/{id}/raw",
    "serial://connections/{id}/log",
];

/// Exact prompt names of the current server.
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

/// HTTP mode: rmcp 1.7.0's reqwest Streamable HTTP client transport against the
/// passed URL (`/mcp` on the current server).
async fn http_mode(url: &str) -> Result<()> {
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = ().serve(transport).await.context("http mode: initialize/session failed")?;
    verify_and_shutdown(client, "http").await
}

/// Stdio mode: spawn the current serial-mcp binary with an isolated temporary
/// profile path and `RUST_LOG=off`, then drive it through rmcp 1.7.0's
/// `TokioChildProcess` transport. The temporary directory stays alive through
/// client shutdown (it is dropped only after the child has been closed).
async fn stdio_mode(binary: &str) -> Result<()> {
    let temp_dir = tempfile::tempdir().context("stdio mode: create temp directory")?;
    let profiles_path = temp_dir.path().join("profiles.toml");
    let mut command = tokio::process::Command::new(binary);
    command.arg("--profiles-path").arg(&profiles_path);
    command.env("RUST_LOG", "off");
    let child =
        TokioChildProcess::new(command).context("stdio mode: spawn serial-mcp child process")?;
    let client = ().serve(child).await.context("stdio mode: initialize/session failed")?;
    // temp_dir lives here across the whole verify+shutdown, so the child can
    // never observe a missing profile path.
    verify_and_shutdown(client, "stdio").await
}

/// Run the shared public-behavior verifier, then shut the client down
/// cleanly. On verification failure the client is still cancelled (best
/// effort) before the error is returned; stdio child cleanup rides on the
/// client shutdown (`Transport::close` -> `graceful_shutdown`).
async fn verify_and_shutdown(client: RunningService<RoleClient, ()>, mode: &str) -> Result<()> {
    let verified = verify(&client).await;
    if let Err(error) = &verified {
        // Best-effort cancellation before returning the error: the child /
        // HTTP session must not be left running on a failed check.
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

/// The shared public-behavior verifier for the compatibility contract.
/// Uses the all-page rmcp 1.7.0 helpers (`list_all_*`) so pagination cannot
/// hide items. Returns a descriptive error on the first failed check.
async fn verify(client: &RunningService<RoleClient, ()>) -> std::result::Result<(), String> {
    // 1. Peer info exists after initialize.
    let peer_info = client
        .peer_info()
        .ok_or_else(|| "peer info missing after initialize".to_string())?;

    // 2. Negotiated protocol is exactly V_2025_11_25.
    if peer_info.protocol_version != ProtocolVersion::V_2025_11_25 {
        return Err(format!(
            "negotiated protocol {:?}, expected {:?}",
            peer_info.protocol_version,
            ProtocolVersion::V_2025_11_25
        ));
    }

    // 3. Server implementation name is `serial-mcp`.
    if peer_info.server_info.name != "serial-mcp" {
        return Err(format!(
            "server implementation name {:?}, expected \"serial-mcp\"",
            peer_info.server_info.name
        ));
    }

    // 4. Exact 25-tool surface. Set equality alone cannot catch duplicates,
    //    so the RAW count is asserted first (exact 25, not "at least the
    //    expected set"), then the name set must equal the fixture-local
    //    constant exactly.
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

    // 5. Static resource URIs are exactly serial://ports + serial://connections
    //    (raw count 2 first, then exact set equality).
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

    // 6. Resource template URIs are exactly the connection detail/raw/log
    //    templates (raw count 3 first, then exact set equality).
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

    // 7. Prompt names are exactly diagnose_port + interactive_terminal (raw
    //    count 2 first, then exact set equality).
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

    // 8. compute_checksum with the standard fixture arguments succeeds and
    //    the structured result carries integer checksum=111 and string
    //    checksum_hex="6F". A tool-error result must never pass merely
    //    because it carries fields: explicitly require is_error == Some(false)
    //    before accepting the structured content.
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
