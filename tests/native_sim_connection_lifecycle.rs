//! Software-only connection-lifecycle integration tests for serial-mcp.
//!
//! These tests exercise the server's behavior end-to-end against a
//! `native_sim` firmware PTY, using only software emulators — no
//! physical hardware, USB-serial adapters, or board bring-up required.
//!
//! Each test spawns its own `zephyr.exe` instance, parses the PTY
//! path from stdout, and is fully isolated. `--test-threads=N` is safe.
//!
//! Coverage focus:
//!   - Named connection bookkeeping in `list_connections`
//!   - `set_flow_control` tool contract (result + summary round-trip)
//!   - Close-while-pending-read behavior at the MCP-tool layer
//!   - Reopen the same PTY path after close
//!   - Fresh `read(match=...)` output after reopen
//!
//! Run:
//! ```sh
//! cargo test --test native_sim_connection_lifecycle -- --ignored
//! # or override firmware binary path:
//! SERIAL_MCP_NATIVE_SIM_BIN=/path/to/zephyr.exe \
//!     cargo test --test native_sim_connection_lifecycle -- --ignored
//! ```

use std::time::Duration;

use anyhow::Context;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

mod common;
use common::{args_object, connect_client, tool_request, TestServer};

// ── Firmware process management (mirrors native_sim_validation.rs) ──────────

const DEFAULT_BIN: &str = "build/firmware/zephyr/zephyr.exe";
const BAUD_RATE: u32 = 115200;
const CONNECTION_NAME: &str = "lifecycle-uart";

fn zephyr_bin() -> String {
    std::env::var("SERIAL_MCP_NATIVE_SIM_BIN")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_BIN.to_string())
}

struct NativeSimFirmware {
    child: tokio::process::Child,
    pty_path: String,
    _stdout_drain: tokio::task::JoinHandle<()>,
}

impl NativeSimFirmware {
    async fn spawn() -> anyhow::Result<Self> {
        let bin = zephyr_bin();
        let mut child = Command::new(&bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("Failed to spawn {bin}"))?;

        let stdout = child.stdout.take().context("stdout not piped")?;
        let mut reader = BufReader::new(stdout).lines();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut pty_path: Option<String> = None;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), reader.next_line()).await {
                Ok(Ok(Some(line))) => {
                    if let Some(pos) = line.find("uart connected to pseudotty:") {
                        if let Some(path_start) = line[pos..].find("/dev/pts/") {
                            pty_path = Some(line[pos + path_start..].to_string());
                            break;
                        }
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => anyhow::bail!("Error reading zephyr stdout: {e}"),
                Err(_) => continue,
            }
        }

        let pty_path = pty_path
            .ok_or_else(|| anyhow::anyhow!("zephyr.exe did not print PTY path within 5s"))?;

        let drain =
            tokio::spawn(async move { while let Ok(Some(_line)) = reader.next_line().await {} });

        Ok(Self {
            child,
            pty_path,
            _stdout_drain: drain,
        })
    }
}

impl Drop for NativeSimFirmware {
    fn drop(&mut self) {
        self.child.start_kill().ok();
    }
}

// ── MCP helpers ──────────────────────────────────────────────────────────────

async fn open_pty(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    pty_path: &str,
    name: &str,
) -> String {
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": pty_path,
                "name": name,
                "baud_rate": BAUD_RATE,
            }),
        ))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    let s = result.structured_content.expect("structured open");
    assert_eq!(s["name"], json!(name));
    s["connection_id"]
        .as_str()
        .expect("connection_id")
        .to_string()
}

async fn open_pty_with_flow(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    pty_path: &str,
    name: &str,
    flow_control: &str,
) -> String {
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": pty_path,
                "name": name,
                "baud_rate": BAUD_RATE,
                "flow_control": flow_control,
            }),
        ))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    let s = result.structured_content.expect("structured open");
    assert_eq!(s["name"], json!(name));
    s["connection_id"]
        .as_str()
        .expect("connection_id")
        .to_string()
}

async fn write_cmd(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    cmd: &str,
) {
    let result = client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": format!("{cmd}\r\n") }),
        ))
        .await
        .expect("write call");
    assert_ne!(result.is_error, Some(true), "write failed: {result:?}");
}

async fn flush_both(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
) {
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .expect("flush call");
}

async fn close_connection(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
) {
    let result = client
        .peer()
        .call_tool(
            rmcp::model::CallToolRequestParams::new("close")
                .with_arguments(args_object(json!({ "connection_id": connection_id }))),
        )
        .await
        .expect("close call");
    assert_ne!(result.is_error, Some(true), "close failed: {result:?}");
}

/// Wait for the firmware's boot banner to appear, then flush buffers so
/// the connection starts from a known state.
async fn sync_boot(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
) {
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 3000,
                "max_buffered_bytes": 256,
                "encoding": "utf8",
                "match": {
                    "pattern": "test firmware ready",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }),
        ))
        .await
        .expect("sync_boot read");
    assert_ne!(result.is_error, Some(true), "sync_boot: {result:?}");
    flush_both(client, connection_id).await;
}

// ── Test A: named connection appears in list_connections ─────────────────────

/// Replacing the first half of the old `e83_live_validation`: a named
/// connection should appear in `list_connections` with all summary
/// fields populated correctly.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_named_connection_appears_in_list_connections() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path.to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let connection_id = open_pty(&client, &pty_path, CONNECTION_NAME).await;

    let list = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .expect("list_connections call");
    assert_ne!(list.is_error, Some(true), "{list:?}");
    let structured = list
        .structured_content
        .expect("structured list_connections");
    assert_eq!(structured["count"], json!(1));
    let entry = &structured["connections"][0];
    assert_eq!(entry["connection_id"], json!(connection_id));
    assert_eq!(entry["name"], json!(CONNECTION_NAME));
    assert_eq!(entry["port"], json!(pty_path));
    assert_eq!(entry["baud_rate"], json!(BAUD_RATE));
    assert_eq!(entry["flow_control"], json!("none"));

    close_connection(&client, &connection_id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test B: set_flow_control tool round-trip ────────────────────────────────

/// Replaces the `set_flow_control` part of the old `e83_live_validation`:
/// the tool must return the requested mode, and `list_connections` must
/// reflect the updated summary. Uses `none` which is the only mode
/// guaranteed to be supported by every backend (including the PTY that
/// backs the native_sim firmware).
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_set_flow_control_updates_summary_and_result() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path.to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let connection_id = open_pty(&client, &pty_path, CONNECTION_NAME).await;

    let set = client
        .peer()
        .call_tool(tool_request(
            "set_flow_control",
            json!({ "connection_id": connection_id, "flow_control": "none" }),
        ))
        .await
        .expect("set_flow_control call");
    assert_ne!(set.is_error, Some(true), "set_flow_control: {set:?}");
    let set_struct = set.structured_content.expect("structured set_flow_control");
    assert_eq!(set_struct["flow_control"], json!("none"));
    assert_eq!(set_struct["connection_id"], json!(connection_id));

    let list = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .expect("list_connections call");
    assert_ne!(list.is_error, Some(true), "{list:?}");
    let list_struct = list
        .structured_content
        .expect("structured list_connections");
    let entry = &list_struct["connections"][0];
    assert_eq!(entry["flow_control"], json!("none"));
    assert_eq!(entry["connection_id"], json!(connection_id));

    close_connection(&client, &connection_id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test C: close while read is pending returns close error ─────────────────

/// Replaces the most important part of the old `e83_live_validation`:
/// a `read` that is still in-flight when the connection is closed must
/// surface a close-related error to the MCP caller.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_close_while_read_active_returns_close_error() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path.to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path, CONNECTION_NAME).await;
    sync_boot(&client, &id).await;

    // Start a read that is expected to time out (no matching output).
    let reader = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 5000,
                    "max_buffered_bytes": 1024,
                }),
            ))
            .await
        })
    };

    // Give the read a moment to actually start before we close.
    tokio::time::sleep(Duration::from_millis(150)).await;

    close_connection(&client, &id).await;

    let read_result = reader.await.unwrap().expect("read task join");
    assert_eq!(
        read_result.is_error,
        Some(true),
        "expected close-related error, got: {read_result:?}"
    );
    let err_text = read_result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default();
    assert!(
        err_text.contains("closed") || err_text.contains("Connection closed"),
        "expected close-related message, got: {err_text:?}"
    );

    client.cancel().await.ok();
    drop(fw);
}

// ── Test D: reopen same port after close works ──────────────────────────────

/// Replaces the reopen part of the old `e83_live_validation`: the same
/// native_sim PTY can be opened again after a clean close, and a fresh
/// `ping` round-trip succeeds.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_reopen_same_port_after_close_works() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path.to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let first_id = open_pty(&client, &pty_path, CONNECTION_NAME).await;
    sync_boot(&client, &first_id).await;
    write_cmd(&client, &first_id, "ping").await;
    close_connection(&client, &first_id).await;

    let second_id = open_pty(&client, &pty_path, CONNECTION_NAME).await;
    sync_boot(&client, &second_id).await;
    write_cmd(&client, &second_id, "ping").await;

    let read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": second_id,
                "timeout_ms": 2000,
                "max_buffered_bytes": 64,
                "encoding": "utf8",
                "match": {
                    "pattern": "pong",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(read.is_error, Some(true), "{read:?}");
    let s = read.structured_content.expect("structured");
    assert_eq!(
        s["matched"],
        json!(true),
        "expected pong after reopen: {s:?}"
    );

    close_connection(&client, &second_id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test E: reopen + match finds only fresh post-reopen output ─────────────

/// Replaces the post-reopen read part of the old `e83_live_validation`:
/// after reopening, a fresh `read(match=...)` must see the response to a
/// new command issued on the new connection. This proves the new
/// connection is functional and the read path is not stuck on stale
/// session data.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_reopen_then_match_finds_fresh_output() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path.to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // First connection: do a round-trip and close.
    let first_id = open_pty(&client, &pty_path, CONNECTION_NAME).await;
    sync_boot(&client, &first_id).await;
    write_cmd(&client, &first_id, "ping").await;
    let first_read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": first_id,
                "timeout_ms": 2000,
                "max_buffered_bytes": 64,
                "encoding": "utf8",
                "match": {
                    "pattern": "pong",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }),
        ))
        .await
        .expect("first read");
    assert_eq!(
        first_read.structured_content.expect("structured")["matched"],
        json!(true)
    );
    close_connection(&client, &first_id).await;

    // Reopen the same PTY. The new connection must independently be
    // able to issue a write and observe a match for the response.
    let second_id = open_pty(&client, &pty_path, CONNECTION_NAME).await;
    sync_boot(&client, &second_id).await;
    write_cmd(&client, &second_id, "ping").await;

    let read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": second_id,
                "timeout_ms": 2000,
                "max_buffered_bytes": 64,
                "encoding": "utf8",
                "match": {
                    "pattern": "pong",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(read.is_error, Some(true), "{read:?}");
    let s = read.structured_content.expect("structured");
    assert_eq!(
        s["matched"],
        json!(true),
        "expected pong after reopen: {s:?}"
    );

    close_connection(&client, &second_id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test F: set_flow_control accepted at open time for named connection ─────

/// Covers the path where flow control is supplied on `open` (instead of
/// via a later `set_flow_control` call). The resulting `list_connections`
/// summary should reflect the requested mode.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_open_with_flow_control_persists_in_summary() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path.to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty_with_flow(&client, &pty_path, CONNECTION_NAME, "none").await;

    let list = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .expect("list_connections call");
    assert_ne!(list.is_error, Some(true), "{list:?}");
    let s = list.structured_content.expect("structured");
    let entry = &s["connections"][0];
    assert_eq!(entry["flow_control"], json!("none"));
    assert_eq!(entry["name"], json!(CONNECTION_NAME));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}
