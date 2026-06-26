//! Software-in-the-loop integration tests for the serial-mcp test firmware
//! running on `native_sim` (POSIX emulator, PTY-backed UART).
//!
//! Each test spawns its own `zephyr.exe` instance with a fresh PTY.
//! No shared state — `--test-threads=N` is safe.
//!
//! ```sh
//! cargo test --test native_sim_validation -- --ignored
//! # or with a custom binary:
//! SERIAL_MCP_NATIVE_SIM_BIN=/path/to/zephyr.exe cargo test --test native_sim_validation -- --ignored
//! ```

use std::time::Duration;

use anyhow::Context;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

mod common;
use common::{args_object, connect_client, next_notification, tool_request, TestServer};

// ── Firmware process management ──────────────────────────────────────────────

fn zephyr_bin() -> std::path::PathBuf {
    common::firmware::ensure_plain_firmware_built()
        .expect("plain native_sim firmware available for validation tests")
}

/// A running native_sim firmware instance with a known PTY path.
/// Spawns `zephyr.exe`, parses the PTY path from stdout, and
/// drains remaining output in a background task. Kills the
/// process on drop.
struct NativeSimFirmware {
    child: tokio::process::Child,
    pty_path: String,
    _stdout_drain: tokio::task::JoinHandle<()>,
}

impl NativeSimFirmware {
    /// Spawn `zephyr.exe`, parse the PTY path from its stdout.
    async fn spawn() -> anyhow::Result<Self> {
        let bin = zephyr_bin();
        let mut child = Command::new(&bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("Failed to spawn {}", bin.display()))?;

        let stdout = child.stdout.take().context("stdout not piped")?;
        let mut reader = BufReader::new(stdout).lines();

        // Read until we find the PTY path line:
        //   uart connected to pseudotty: /dev/pts/N
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
                Ok(Ok(None)) => break, // stdout closed
                Ok(Err(e)) => {
                    anyhow::bail!("Error reading zephyr stdout: {e}");
                }
                Err(_elapsed) => continue, // timeout, poll again
            }
        }

        let pty_path = pty_path
            .ok_or_else(|| anyhow::anyhow!("zephyr.exe did not print PTY path within 5s"))?;

        // Drain remaining stdout in background so the pipe buffer doesn't fill.
        let drain = tokio::spawn(async move {
            while let Ok(Some(_line)) = reader.next_line().await {
                // drain
            }
        });

        Ok(Self {
            child,
            pty_path,
            _stdout_drain: drain,
        })
    }

    fn pty_path(&self) -> &str {
        &self.pty_path
    }

    /// Check whether the firmware process has exited, and return its exit code.
    fn try_exit_code(&mut self) -> Option<i32> {
        self.child.try_wait().ok().flatten().and_then(|s| s.code())
    }
}

impl Drop for NativeSimFirmware {
    fn drop(&mut self) {
        // start_kill sends SIGKILL, best-effort cleanup.
        self.child.start_kill().ok();
    }
}

// ── MCP helper functions ─────────────────────────────────────────────────────

const BAUD_RATE: u32 = 115200;
const NAME: &str = "native-sim-uart";

async fn open_pty(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    pty_path: &str,
) -> String {
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": pty_path,
                "name": NAME,
                "baud_rate": BAUD_RATE,
            }),
        ))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    let s = result.structured_content.expect("structured open");
    assert_eq!(s["name"], json!(NAME));
    s["connection_id"]
        .as_str()
        .expect("connection_id")
        .to_string()
}

#[cfg(unix)]
async fn open_with(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    pty_path: &str,
    extra_fields: serde_json::Value,
) -> String {
    let mut body = json!({
        "port": pty_path,
        "name": NAME,
        "baud_rate": BAUD_RATE,
    });
    if let serde_json::Value::Object(ref mut map) = body {
        if let serde_json::Value::Object(extra) = extra_fields {
            for (k, v) in extra {
                map.insert(k, v);
            }
        }
    }
    let result = client
        .peer()
        .call_tool(tool_request("open", body))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    let s = result.structured_content.expect("structured open");
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

async fn write_raw(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    data: &str,
) {
    let result = client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": data }),
        ))
        .await
        .expect("write call");
    assert_ne!(result.is_error, Some(true), "write failed: {result:?}");
}

#[cfg(unix)]
async fn write_preset(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    data: &str,
    extra_fields: serde_json::Value,
) {
    let mut body = json!({
        "connection_id": connection_id,
        "data": data,
        "protocol": { "type": "at_command" },
    });
    if let serde_json::Value::Object(ref mut map) = body {
        if let serde_json::Value::Object(extra) = extra_fields {
            for (k, v) in extra {
                map.insert(k, v);
            }
        }
    }
    let result = client
        .peer()
        .call_tool(tool_request("write", body))
        .await
        .expect("write call");
    assert_ne!(result.is_error, Some(true), "write failed: {result:?}");
}

/// Read unstructured data string via the `read` tool.
async fn read_str(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    timeout_ms: u64,
) -> String {
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": timeout_ms,
                "encoding": "utf8",
            }),
        ))
        .await
        .expect("read call");
    if result.is_error == Some(true) {
        return String::new();
    }
    result
        .structured_content
        .and_then(|s| s["data"].as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Read until `expected` substring is found.
async fn read_until(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    expected: &str,
    timeout_ms: u64,
) -> bool {
    let data = read_str(client, connection_id, timeout_ms).await;
    data.contains(expected)
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

/// Read until the firmware's boot banner ("serial-mcp test firmware ready")
/// then flush. Ensures we start from a clean, known state.
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

// ── Test 1: ping roundtrip ───────────────────────────────────────────────────

/// Verify the firmware is alive: `ping` → `pong`.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_ping_roundtrip() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;

    sync_boot(&client, &id).await;
    write_cmd(&client, &id, "ping").await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
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

    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected pong: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 2: pending read then write ping ─────────────────────────────────────

/// read with match waits first, then a later write still reaches the
/// firmware promptly.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_pending_read_then_write_ping_roundtrip() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 128,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    let start = tokio::time::Instant::now();
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    let elapsed = start.elapsed();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected pong: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));
    assert!(
        elapsed < Duration::from_secs(1),
        "write+response took too long with pending read: {elapsed:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 3: split writes preserve command order ──────────────────────────────

/// Split write calls must stay ordered so the firmware still sees one
/// valid command.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_split_writes_preserve_command_order() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 128,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_raw(&client, &id, "pi").await;
    write_raw(&client, &id, "ng").await;
    write_raw(&client, &id, "\r\n").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected pong: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 4: framing reports single split command ─────────────────────────────

/// Framing mode should report one committed line even when the command
/// arrives through multiple write calls.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_framing_reports_single_split_command() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    write_cmd(&client, &id, "framing on").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_raw(&client, &id, "pi").await;
    write_raw(&client, &id, "ng").await;
    write_raw(&client, &id, "\r\n").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.contains("LINE len=4 data=\"ping\""),
        "expected one framed ping line, got: {data:?}"
    );
    assert!(
        data.contains("pong"),
        "expected pong after framed line: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 5: trace reports exact split byte sequence ──────────────────────────

/// Trace mode should expose exact RX byte order for split writes,
/// including CRLF terminator bytes.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_trace_reports_exact_split_byte_sequence() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    write_cmd(&client, &id, "trace on").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 2048,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "pong",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_raw(&client, &id, "pi").await;
    write_raw(&client, &id, "ng").await;
    write_raw(&client, &id, "\r\n").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let data = s["data"].as_str().unwrap_or("");
    for expected in [
        "RX[0]=0x70",
        "RX[1]=0x69",
        "RX[2]=0x6e",
        "RX[3]=0x67",
        "RX[4]=0x0d",
        "RX[5]=0x0a",
    ] {
        assert!(
            data.contains(expected),
            "missing trace {expected} in {data:?}"
        );
    }
    assert!(
        data.contains("pong"),
        "expected pong after traced bytes: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 6: read match on spam completion ────────────────────────────────────

/// read(match=...) stops on "Spam complete" after a 1024-byte hex spam.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_match_on_spam_complete() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 5000,
                    "max_buffered_bytes": 8192,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "Spam complete",
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    write_cmd(&client, &id, "spam 1024 hex").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected match: {s:?}");
    assert_eq!(s["stop_reason"], json!("match_found"));
    assert_eq!(s["name"], json!(NAME));
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.contains("Spam complete"),
        "data should contain stop phrase: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 7: subscribe match stops on spam completion ─────────────────────────

/// subscribe(match=...) self-stops with match_found when "Spam complete"
/// appears mid-stream.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_subscribe_match_stops_on_spam_complete() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe with match on the completion phrase.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 8192,
                "match": {
                    "pattern": "Spam complete",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8",
                        "context_amount_of_matched_bytes": 64
                    }
                }
            }),
        ))
        .await
        .unwrap();

    write_cmd(&client, &id, "spam 1024 hex").await;

    let mut found_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(3)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("matched").and_then(|v| v.as_bool()) == Some(true) {
                    found_match_stop = true;
                    assert_eq!(data["stop_reason"], json!("match_found"), "{data:?}");
                    assert!(
                        data["match_index"].as_u64().is_some(),
                        "match_index present"
                    );
                    let shaped = data.get("data").and_then(|v| v.as_str()).unwrap_or("");
                    assert!(
                        shaped.contains("Spam complete"),
                        "shaped payload should contain stop phrase: {shaped:?}"
                    );
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        found_match_stop,
        "subscribe should have emitted match_found stop notification"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 8: subscribe silence timeout after spam ends ────────────────────────

/// subscribe(no_new_rx_timeout_ms=500) stops with no_new_rx_timeout once
/// the spam finishes and the board goes silent.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_subscribe_silence_timeout_after_spam() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Run a small spam first to produce data, then let the firmware go silent.
    write_cmd(&client, &id, "spam 512 hex").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    flush_both(&client, &id).await;

    // Now subscribe with silence timeout — firmware is quiet.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 1024,
                "no_new_rx_timeout_ms": 500
            }),
        ))
        .await
        .unwrap();

    let event = next_notification(&mut rx, Duration::from_secs(5))
        .await
        .expect("subscribe should emit stop notification");

    let data = event.data.as_object().unwrap();
    assert_eq!(
        data["stop_reason"],
        json!("no_new_rx_timeout"),
        "expected silence timeout stop: {data:?}"
    );
    assert_ne!(data.get("matched").and_then(|v| v.as_bool()), Some(true));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 9: buffer budget under hex flood ────────────────────────────────────

/// read(max_buffered_bytes=256) stops cleanly with max_buffered_bytes
/// while a hex flood is in progress.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_buffer_budget_stops_under_flood() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 5000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(50)).await;
    write_cmd(&client, &id, "spam 65536 hex").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["stop_reason"], json!("max_buffered_bytes"), "{s:?}");
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.len() <= 256,
        "data should be ≤ 256 bytes, got {}",
        data.len()
    );

    // Stop the flood.
    write_cmd(&client, &id, "spam stop").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 10: subscribe wall-clock timeout stops under active flood ───────────

/// subscribe(timeout_ms=800) stops on wall-clock timeout even while spam
/// data is actively flowing.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_subscribe_timeout_stops_under_flood() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe with 800ms wall-clock timeout.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 16384,
                "timeout_ms": 800,
            }),
        ))
        .await
        .unwrap();

    write_cmd(&client, &id, "spam 1000000 hex").await;

    let mut total_bytes: u64 = 0;
    let mut stop_reason = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(3)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if let Some(n) = data.get("bytes_read").and_then(|v| v.as_u64()) {
                    total_bytes += n;
                }
                if let Some(reason) = data.get("stop_reason").and_then(|v| v.as_str()) {
                    stop_reason = reason.to_string();
                    break;
                }
            }
            Err(_) => break,
        }
    }

    assert_eq!(
        stop_reason, "timeout",
        "expected timeout stop: {stop_reason:?}"
    );
    assert!(
        total_bytes > 0,
        "should have received some bytes before timeout"
    );

    // Stop the flood so the process is clean.
    write_cmd(&client, &id, "spam stop").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 11: close while subscribe active ────────────────────────────────────

/// Close the connection while a subscribe is streaming hex flood.
/// Connection cleanup should be clean; list_connections returns 0.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_close_while_subscribe_active() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 4096,
            }),
        ))
        .await
        .unwrap();

    write_cmd(&client, &id, "spam 1000000 hex delay=5").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    write_cmd(&client, &id, "spam stop").await;
    tokio::time::sleep(Duration::from_millis(50)).await;

    close_connection(&client, &id).await;

    let list = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .expect("list_connections");
    assert_ne!(list.is_error, Some(true), "{list:?}");
    let s = list.structured_content.expect("structured");
    assert_eq!(
        s["count"],
        json!(0),
        "expected 0 connections after close: {s:?}"
    );

    client.cancel().await.ok();
    drop(fw);
}

// ── Test 12: bootloader touch command → exit(42) ──────────────────────────────

/// Send the "touch" command over the PTY command channel. Firmware
/// should respond with "touch exit(42)" and then call exit(42).
/// This validates the end-to-end path that a bootloader-entry
/// trigger (sent via serial-mcp `write`) causes the expected
/// firmware-side behaviour.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_bootloader_touch_exits_42() {
    let mut fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Send the "touch" command
    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": id,
                "data": "touch\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .expect("write touch command");

    // Give firmware time to process and call exit(42)
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if let Some(code) = fw.try_exit_code() {
            assert_eq!(
                code, 42,
                "expected exit(42) from touch command, got code {code}"
            );
            client.cancel().await.ok();
            return;
        }
    }

    client.cancel().await.ok();
    panic!("firmware did not exit within 2s after touch command");
}

// ── Test 13: list_ports returns valid JSON with an opened PTY ──────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_list_ports_after_open() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let result = client
        .peer()
        .call_tool(tool_request("list_ports", json!({})))
        .await
        .expect("list_ports");
    assert_ne!(result.is_error, Some(true), "{result:?}");

    let s = result.structured_content.expect("structured");
    let ports = s["ports"].as_array().expect("ports is array");
    assert!(
        !ports.is_empty(),
        "expected at least one port in list: {s:?}"
    );

    // The PTY might not be enumerated by serialport::available_ports()
    // on all platforms, but list_ports must return valid JSON.
    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 14: list_ports returns rich device identity ──────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_list_ports_includes_identity_fields() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let result = client
        .peer()
        .call_tool(tool_request("list_ports", json!({})))
        .await
        .expect("list_ports");
    assert_ne!(result.is_error, Some(true), "{result:?}");

    let s = result.structured_content.expect("structured");
    let ports = s["ports"].as_array().expect("ports is array");

    for port in ports {
        // Every port must have at least these fields.
        assert!(port["name"].is_string(), "port missing name: {port:?}");
        assert!(
            port["display_name"].is_string(),
            "port missing display_name: {port:?}"
        );
        assert!(
            port["transport"].is_string(),
            "port missing transport: {port:?}"
        );
        let transport = port["transport"].as_str().unwrap();
        assert!(
            matches!(transport, "usb" | "pci" | "bluetooth" | "unknown"),
            "unexpected transport '{transport}' in {port:?}"
        );

        // USB-specific fields should be null for non-USB transports.
        if transport != "usb" {
            assert!(
                port["vid"].is_null(),
                "non-USB port should have null vid: {port:?}"
            );
            assert!(
                port["pid"].is_null(),
                "non-USB port should have null pid: {port:?}"
            );
            assert!(
                port["serial_number"].is_null(),
                "non-USB port should have null serial_number: {port:?}"
            );
        }
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 15: flush preserves data integrity ────────────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_after_write() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Write a command, flush output, then read — the pong should arrive.
    write_cmd(&client, &id, "ping").await;

    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": id, "target": "output" }),
        ))
        .await
        .expect("flush");

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 1000,
            }),
        ))
        .await
        .expect("read");
    assert_ne!(result.is_error, Some(true), "{result:?}");

    let s = result.structured_content.expect("structured");
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.contains("pong"),
        "expected pong after flush+read, got: {data}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 16: unsubscribe followed by re-subscribe ──────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_unsubscribe_then_resubscribe() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 1024,
            }),
        ))
        .await
        .expect("subscribe");

    // Unsubscribe
    let result = client
        .peer()
        .call_tool(tool_request("unsubscribe", json!({ "connection_id": id })))
        .await
        .expect("unsubscribe");
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // Re-subscribe — should succeed
    let resub = client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 1024,
            }),
        ))
        .await
        .expect("re-subscribe");
    assert_ne!(resub.is_error, Some(true), "{resub:?}");

    client.cancel().await.ok();
    drop(rx);
    drop(fw);
}

// ── Phase 1 gap-fill: get_status on PTY ─────────────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_get_status_after_write_increments_tx_counter() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // sync_boot only reads boot output — tx may be 0, rx > 0
    let result = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": id })))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.unwrap();
    assert_eq!(s["is_open"], json!(true));
    let rx0 = s["rx_bytes"].as_u64().unwrap();
    assert!(rx0 > 0, "rx after boot: {s:?}");

    // Write + read should increase both counters
    write_cmd(&client, &id, "ping").await;
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
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
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    let result = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": id })))
        .await
        .unwrap();
    let s = result.structured_content.unwrap();
    let tx1 = s["tx_bytes"].as_u64().unwrap();
    let rx1 = s["rx_bytes"].as_u64().unwrap();
    assert!(tx1 > 0, "tx after ping: {s:?}");
    assert!(rx1 > 0, "rx after ping: {s:?}");
    assert!(
        !s["last_activity_ms"].is_null(),
        "last_activity after I/O: {s:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Phase 1 gap-fill: reconfigure on PTY ─────────────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_reconfigure_baud_rate_persists() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Change baud to 38400
    let result = client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": id, "baud_rate": 38400 }),
        ))
        .await
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.unwrap();
    assert_eq!(s["baud_rate"], json!(38400), "{s:?}");

    // Verify via get_status
    let result = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": id })))
        .await
        .unwrap();
    let s = result.structured_content.unwrap();
    assert_eq!(s["baud_rate"], json!(38400), "baud should persist: {s:?}");

    // Connection still works — ping roundtrip
    write_cmd(&client, &id, "ping").await;
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
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
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");

    // Reconfigure back to 115200
    client
        .peer()
        .call_tool(tool_request(
            "reconfigure",
            json!({ "connection_id": id, "baud_rate": 115200 }),
        ))
        .await
        .unwrap();

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 19: ack command provides pre-execution acknowledgment ───────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_ack_command_provides_pre_execution_ack() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Enable acks.
    write_cmd(&client, &id, "ack on").await;
    read_until(&client, &id, "ack on", 2000).await;

    // Write ping, read everything (ack + pong arrive together).
    write_cmd(&client, &id, "ping").await;
    let data = read_str(&client, &id, 2000).await;
    assert!(
        data.contains("ack 0"),
        "ack should appear before pong, got: {data}"
    );
    assert!(data.contains("pong"), "pong should follow ack, got: {data}");

    // Second ping — ack increments.
    write_cmd(&client, &id, "ping").await;
    let data2 = read_str(&client, &id, 2000).await;
    assert!(data2.contains("ack 1"), "ack seq should increment: {data2}");
    assert!(data2.contains("pong"), "second pong: {data2}");

    // Disable acks, verify no more ack prefix.
    write_cmd(&client, &id, "ack off").await;
    read_until(&client, &id, "ack off", 2000).await;
    write_cmd(&client, &id, "ping").await;
    let data3 = read_str(&client, &id, 2000).await;
    assert!(!data3.contains("ack 2"), "ack should be off: {data3}");
    assert!(data3.contains("pong"), "pong without ack: {data3}");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 20: txbuf status reports pending TX ──────────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_txbuf_status_reports_pending() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Idle: txbuf should show 0.
    write_cmd(&client, &id, "txbuf status").await;
    let idle = read_until(&client, &id, "txbuf len=0 busy=0", 2000).await;
    assert!(idle, "txbuf should be empty when idle");

    // Enable TX hold, then release. Verify pong still works roundtrip.
    write_cmd(&client, &id, "hold on").await;
    read_until(&client, &id, "hold on", 2000).await;
    write_cmd(&client, &id, "hold off").await;
    read_until(&client, &id, "hold off", 2000).await;

    write_cmd(&client, &id, "ping").await;
    let pong = read_until(&client, &id, "pong", 2000).await;
    assert!(pong, "ping should work after hold on/off cycle");

    // Verify idle again.
    flush_both(&client, &id).await;
    write_cmd(&client, &id, "txbuf status").await;
    let post = read_until(&client, &id, "txbuf len=0", 2000).await;
    assert!(post, "txbuf should be empty after drain");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 21: flush(input) clears host RX ─────────────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_input_clears_host_rx() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Spam a modest amount, then flush input.
    write_cmd(&client, &id, "spam 2000 hex").await;
    read_until(&client, &id, "spam start", 2000).await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let flush = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": id, "target": "input" }),
        ))
        .await
        .unwrap();
    assert_ne!(flush.is_error, Some(true), "flush input failed: {flush:?}");

    write_cmd(&client, &id, "spam stop").await;
    read_until(&client, &id, "Spam stopped", 2000).await;

    // Read after flush input — should be minimal.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 500,
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();
    assert_ne!(
        result.is_error,
        Some(true),
        "read after flush input: {result:?}"
    );
    let s = result.structured_content.expect("structured");
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.len() < 500,
        "expected few bytes after flush input, got len={}",
        data.len()
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 22: flush during arm_cmd delay ──────────────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_during_arm_cmd_delay() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Arm a 500ms delay for the next command.
    write_cmd(&client, &id, "arm_cmd 500").await;
    let armed = read_until(&client, &id, "arm_cmd delay=500", 2000).await;
    assert!(armed, "arm_cmd should confirm");

    // Write ping — it will sleep 500ms before executing.
    write_cmd(&client, &id, "ping").await;

    // Flush during the sleep window.
    tokio::time::sleep(Duration::from_millis(100)).await;
    flush_both(&client, &id).await;

    // Pong should arrive after the delay, despite the flush.
    let pong = read_until(&client, &id, "pong", 5000).await;
    assert!(
        pong,
        "pong should arrive despite flush during arm_cmd delay"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── TX flush semantics: fully delivered / partially queued
//
// These tests cover two of the three TX flush cases:
//   1. fully delivered TX  — covered below;
//   2. partially queued TX — covered below;
//   3. flushed-before-delivery — covered in src/tx_session.rs unit tests via
//      a QueuedTxIo mock SerialIo backend (a PTY cannot reproduce this case
//      because it delivers every write() byte into its kernel buffer
//      immediately, so the host's tcflush(TCOFLUSH) cannot recall bytes that
//      have already left serialport's output buffer).

/// Case 1: a fully-delivered command (ping → pong) is unaffected by a
/// subsequent flush(output). Proves flush(output) does not retroactively
/// disturb already-consumed bytes or later writes.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_output_after_full_delivery_is_safe() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // First ping fully delivered: write, then wait for pong so firmware has
    // consumed the command and replied.
    write_cmd(&client, &id, "ping").await;
    let pong1 = read_until(&client, &id, "pong", 2000).await;
    assert!(pong1, "first ping should produce pong");

    // Now flush output. With the first command already consumed, this must
    // not affect any already-delivered bytes.
    let flush = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": id, "target": "output" }),
        ))
        .await
        .expect("flush call");
    assert_ne!(flush.is_error, Some(true), "flush output: {flush:?}");

    // Second ping must still arrive — proves flush(output) did not break the
    // stream or drop a later, independent write.
    write_cmd(&client, &id, "ping").await;
    let pong2 = read_until(&client, &id, "pong", 2000).await;
    assert!(pong2, "second ping after flush should still produce pong");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Case 2: a command written without a line terminator is held in firmware's
/// partial-line cmd_buf and is NOT executed. Writing the remainder plus
/// terminator then drives the assembled command to completion. Proves
/// partially-queued TX state is observable in behavior (buffered → resumed),
/// not just in a probe. A probe-via-`rxbuf status` is unusable here because
/// the probe bytes themselves get appended to cmd_buf and corrupt the very
/// partial line being observed, so we assert on behavior instead.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_partial_line_buffered_then_completed() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Write a partial command (no terminator). Firmware should buffer it in
    // cmd_buf without executing — no pong yet.
    write_raw(&client, &id, "pi").await;
    // Settle so the bytes are scanned into cmd_buf, then drain any stray
    // output so the next observation is clean.
    tokio::time::sleep(Duration::from_millis(80)).await;
    flush_both(&client, &id).await;

    // Confirm the partial command did NOT execute: a short read should not
    // contain pong. (Firmware only executes on a line terminator.)
    let pre = read_str(&client, &id, 400).await;
    assert!(
        !pre.contains("pong"),
        "partial command without terminator must not execute, got pong in: {pre}"
    );

    // Complete the line: "ng\r\n". Firmware assembles "pi" + "ng" → "ping"
    // and executes it, emitting pong.
    write_raw(&client, &id, "ng\r\n").await;
    let pong = read_until(&client, &id, "pong", 2000).await;
    assert!(
        pong,
        "completed partial line should assemble to ping and produce pong"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 23: regex match finds pong ────────────────────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_regex_matches_pong() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 128,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "po.g",
                        "config": { "mode": "regex", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(
        s["matched"],
        json!(true),
        "regex po.g should match pong: {s:?}"
    );
    assert_eq!(s["stop_reason"], json!("match_found"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 24: glob match per-line finds pong line ───────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_glob_matches_pong_line() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 128,
                    "encoding": "utf8",
                    "match": {
                        "pattern": "po*",
                        "config": { "mode": "glob", "pattern_encoding": "utf8" }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(
        s["matched"],
        json!(true),
        "glob po* should match pong line: {s:?}"
    );
    assert_eq!(s["stop_reason"], json!("match_found"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 25: reconnect tool works on an open connection ──────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_auto_reconnect_preserves_connection() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": pty_path,
                "name": "reconnect-test",
                "baud_rate": 115200,
                "reconnect_policy": {
                    "enabled": true,
                    "max_attempts": 10,
                    "initial_delay_ms": 200,
                    "max_delay_ms": 1000,
                    "backoff_multiplier": 1.5
                }
            }),
        ))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    let id = result
        .structured_content
        .as_ref()
        .and_then(|s| s["connection_id"].as_str().map(str::to_string))
        .expect("connection_id");

    let status = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": id })))
        .await
        .unwrap();
    let s = status.structured_content.expect("status");
    assert_eq!(s["state"], json!("open"));

    // Initial data test.
    write_raw(&client, &id, "ping\r\n").await;
    let data = read_str(&client, &id, 2000).await;
    assert!(data.contains("pong"), "expected pong after first ping");

    // Reconnect while already connected — succeeds immediately.
    let result = client
        .peer()
        .call_tool(tool_request("reconnect", json!({"connection_id": id})))
        .await
        .expect("reconnect call");
    assert_ne!(
        result.is_error,
        Some(true),
        "reconnect should succeed when already open: {result:?}"
    );

    let status = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": id })))
        .await
        .unwrap();
    let s = status.structured_content.expect("status");
    assert_eq!(
        s["state"],
        json!("open"),
        "expected open after reconnect, got: {s:?}"
    );

    // Verify data flows again.
    flush_both(&client, &id).await;
    write_raw(&client, &id, "ping\r\n").await;
    let data = read_str(&client, &id, 2000).await;
    assert!(data.contains("pong"), "expected pong after reconnect");
    assert_eq!(s["connection_id"], json!(id));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 26: line framing splits multi-line output into frames ─────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_line_framing_splits_lines() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Drain boot output so read only sees our commands.
    flush_both(&client, &id).await;

    // Start a read with line framing before writing to capture all output.
    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "rx_framing": { "type": "line" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;
    // Give firmware time to process first command before sending second.
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "info").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array should exist");
    assert!(
        frames.len() >= 2,
        "expected at least 2 frames (pong, info), got {}: {frames:?}",
        frames.len()
    );
    // First frame should be pong line.
    let f0 = &frames[0];
    assert_eq!(f0["frame_type"], json!("line"));
    assert!(f0["data"].as_str().unwrap().contains("pong"), "f0: {f0:?}");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 27: JSON parser decodes jsonout command output ───────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_json_parser_decodes_jsonout() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 1024,
                    "encoding": "utf8",
                    "rx_framing": {
                        "type": "line"
                    },
                    "rx_parser": { "type": "json_lines" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "jsonout").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert_eq!(
        frames.len(),
        3,
        "expected 3 JSON frames from jsonout, got {}: {frames:?}",
        frames.len()
    );

    // Each frame should have parsed JSON with inline fields.
    // The JSON parser inlines the object fields into the parsed result:
    // { "parser": "json", "sensor": "temp", "value": 25.5, "unit": "C" }
    for frame in frames {
        let parsed = frame["parsed"].as_object().expect("parsed object");
        assert_eq!(
            parsed["parser"],
            json!("json"),
            "parser mismatch: {parsed:?}"
        );
        assert!(parsed["sensor"].is_string(), "missing sensor: {parsed:?}");
    }

    // Verify specific sensor values (inline, not nested under "value" key).
    let f0 = &frames[0]["parsed"];
    assert_eq!(f0["sensor"], json!("temp"));
    assert!((f0["value"].as_f64().unwrap() - 25.5).abs() < 0.01);

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 28: AT command parser treats pong as data line ────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_at_parser_parses_pong() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "rx_framing": {
                        "type": "line"
                    },
                    "rx_parser": { "type": "at_command" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");

    // The pong line should be parsed as AT data.
    let f0 = &frames[0];
    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");
    assert_eq!(
        parsed["response_type"],
        json!("data"),
        "pong should be AT data line: {parsed:?}"
    );
    let fields = parsed["fields"].as_array().expect("fields array");
    assert!(
        fields.iter().any(|f| f.as_str().unwrap().contains("pong")),
        "fields should contain pong: {fields:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 29: subscribe with line framing emits per-frame notifications ────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_subscribe_line_framing_emits_per_frame() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe with line framing, auto-stop after 2 seconds.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 8192,
                "timeout_ms": 2000,
                "encoding": "utf8",
                "rx_framing": {
                    "type": "line"
                }
            }),
        ))
        .await
        .unwrap();

    write_cmd(&client, &id, "ping").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "info").await;

    let mut frame_count = 0;
    let mut saw_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                // Stop notification has stop_reason; frame notifications have frame_index.
                if data.contains_key("stop_reason") {
                    saw_stop = true;
                    assert!(data["frames_emitted"].as_u64().unwrap_or(0) > 0);
                    break;
                }
                if data.contains_key("frame_index") {
                    frame_count += 1;
                    assert!(
                        data.contains_key("frame_type"),
                        "missing frame_type: {data:?}"
                    );
                    // Frames replace raw chunks, so no bytes_read field.
                    assert!(
                        !data.contains_key("bytes_read"),
                        "frame notifications should not have bytes_read: {data:?}"
                    );
                }
            }
            Err(_) => break,
        }
    }
    assert!(saw_stop, "subscribe should emit stop notification");
    assert!(
        frame_count >= 2,
        "expected at least 2 frame notifications (pong, info), got {frame_count}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 30: max_frames stops read after N frames ─────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_framing_max_frames_stops() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // Start read with max_frames=2.
    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "rx_framing": {
                        "type": "line",
                        "max_frames": 2
                    }
                }),
            ))
            .await
        })
    };

    // Send 3 commands that each produce one line of output.
    // read should stop after capturing 2 frames.
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "info").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["stop_reason"], json!("max_frames"));
    let frames = s["frames"].as_array().expect("frames array");
    assert_eq!(
        frames.len(),
        2,
        "max_frames=2 should return exactly 2 frames, got {frames:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 31: framing + match combined returns both ────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_framing_plus_match_combined() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // Read with both framing (line) and match on the word "pong".
    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "rx_framing": { "type": "line" },
                    "match": {
                        "pattern": "pong",
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8"
                        }
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["stop_reason"], json!("match_found"));
    assert_eq!(s["matched"], json!(true));
    // match_frame_index should be set when framing+match combined.
    assert!(
        s["match_frame_index"].as_u64().is_some(),
        "should have match_frame_index: {s:?}"
    );
    // Frames should still be returned (pong line captured before match triggered).
    let frames = s["frames"].as_array().expect("frames array");
    assert!(
        !frames.is_empty(),
        "combined framing+match should return frames"
    );
    let f0 = &frames[0];
    assert_eq!(f0["frame_type"], json!("line"));
    assert!(f0["data"].as_str().unwrap().contains("pong"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 33: subscribe max_frames stops after N frames ────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_subscribe_framing_max_frames_stops() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe with max_frames=2, long timeout.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 8192,
                "timeout_ms": 5000,
                "encoding": "utf8",
                "rx_framing": {
                    "type": "line",
                    "max_frames": 2
                }
            }),
        ))
        .await
        .unwrap();

    // Send 4 commands — each produces one line, stream stops after 2 frames.
    write_cmd(&client, &id, "ping").await;
    write_cmd(&client, &id, "info").await;
    write_cmd(&client, &id, "ping").await;
    write_cmd(&client, &id, "info").await;

    let mut frame_count = 0;
    let mut stop_reason = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if let Some(reason) = data.get("stop_reason") {
                    stop_reason = reason.as_str().unwrap_or("").to_string();
                    break;
                }
                if data.contains_key("frame_index") {
                    frame_count += 1;
                }
            }
            Err(_) => break,
        }
    }
    assert_eq!(
        stop_reason, "max_frames",
        "subscribe with max_frames=2 should stop with max_frames, got {stop_reason}"
    );
    assert_eq!(frame_count, 2, "expected exactly 2 frame notifications");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 34: subscribe + framing + match combined ──────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_subscribe_framing_plus_match_combined() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe with line framing + match on "pong".
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 8192,
                "encoding": "utf8",
                "rx_framing": { "type": "line" },
                "match": {
                    "pattern": "pong",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8"
                    }
                }
            }),
        ))
        .await
        .unwrap();

    write_cmd(&client, &id, "ping").await;

    let mut found_frame = false;
    let mut found_match_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.contains_key("stop_reason") {
                    found_match_stop = true;
                    assert_eq!(data["stop_reason"], json!("match_found"));
                    assert_eq!(data["matched"], json!(true));
                    assert!(
                        data["match_frame_index"].as_u64().is_some(),
                        "should have match_frame_index"
                    );
                    assert!(data["frames_emitted"].as_u64().unwrap_or(0) > 0);
                    break;
                }
                if data.contains_key("frame_index") {
                    found_frame = true;
                }
            }
            Err(_) => break,
        }
    }
    assert!(found_frame, "should have at least one frame notification");
    assert!(
        found_match_stop,
        "subscribe+framing+match should find pong match"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 35: subscribe emits partial frame on timeout ──────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_subscribe_framing_partial_on_timeout() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe with line framing + short timeout.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 8192,
                "timeout_ms": 1500,
                "encoding": "utf8",
                "rx_framing": { "type": "line" }
            }),
        ))
        .await
        .unwrap();

    // Send raw data without line terminator — no \n to trigger frame boundary.
    write_cmd(&client, &id, "sendraw text partial_no_newline").await;
    // Give time for the data to reach the pump.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut saw_partial = false;
    let mut stop_reason = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if let Some(reason) = data.get("stop_reason") {
                    stop_reason = reason.as_str().unwrap_or("").to_string();
                    break;
                }
                if data.get("partial").and_then(|v| v.as_bool()) == Some(true) {
                    saw_partial = true;
                    assert!(data["data"]
                        .as_str()
                        .unwrap()
                        .contains("partial_no_newline"));
                }
            }
            Err(_) => break,
        }
    }
    assert_eq!(
        stop_reason, "timeout",
        "subscribe should timeout after sendraw with no terminator"
    );
    assert!(
        saw_partial,
        "should emit partial frame notification on timeout"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Test 36: subscribe flushes partial frame on close ──────────────────
//
// When the connection is closed via the "close" tool, the pump channel
// closes, the subscribe loop exits, and flush_partial emits any buffered
// data as a partial frame notification. The close handler now waits for
// the subscribe task to finish (join_without_abort) before cleanup.

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_subscribe_framing_partial_on_close() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Subscribe with line framing, no timeout.
    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "max_buffered_bytes": 8192,
                "encoding": "utf8",
                "rx_framing": { "type": "line" }
            }),
        ))
        .await
        .unwrap();

    // Send raw data without line terminator — decoder buffers it as partial.
    write_cmd(&client, &id, "sendraw text before_close").await;
    // Give the pump time to receive the raw data from the firmware.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // Close the connection. The close handler now waits for the subscribe
    // task to finish naturally, allowing flush_partial to emit the partial
    // frame notification before the task is cleaned up.
    close_connection(&client, &id).await;

    let mut saw_partial = false;
    let mut saw_stop = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match next_notification(&mut rx, Duration::from_secs(2)).await {
            Ok(event) => {
                let data = event.data.as_object().unwrap();
                if data.get("partial").and_then(|v| v.as_bool()) == Some(true) {
                    saw_partial = true;
                    assert!(
                        data["data"].as_str().unwrap().contains("before_close"),
                        "partial frame should contain before_close: {data:?}"
                    );
                }
                if data.contains_key("stop_reason") {
                    saw_stop = true;
                    let reason = data["stop_reason"].as_str().unwrap_or("");
                    // Close can produce connection_closed, channel_closed,
                    // or read_error depending on pump exit timing.
                    assert!(
                        reason == "connection_closed"
                            || reason == "channel_closed"
                            || reason == "read_error",
                        "expected close-related stop reason, got {reason}"
                    );
                    break;
                }
            }
            Err(_) => break,
        }
    }
    assert!(
        saw_partial,
        "should emit partial frame notification on close"
    );
    assert!(saw_stop, "should emit stop notification on close");

    client.cancel().await.ok();
    drop(fw);
}

// ── Protocol preset e2e tests ─────────────────────────────────────────────<AZ>
// (3 tests — requires native_sim firmware)

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_write_protocol_preset_appends_cr() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "protocol": { "type": "at_command" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_preset(
        &client,
        &id,
        "ping",
        serde_json::Value::Object(Default::default()),
    )
    .await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    let f0 = &frames[0];
    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");
    assert_eq!(parsed["response_type"], json!("data"));
    let fields = parsed["fields"].as_array().expect("fields array");
    assert!(
        fields.iter().any(|f| f.as_str().unwrap().contains("pong")),
        "fields should contain pong: {fields:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_write_explicit_tx_framing_overrides_protocol() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "protocol": { "type": "at_command" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_preset(
        &client,
        &id,
        "ping",
        json!({ "tx_framing": { "type": "line", "ending": "crlf" } }),
    )
    .await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    let f0 = &frames[0];
    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");
    assert_eq!(parsed["response_type"], json!("data"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_explicit_rx_framing_overrides_protocol() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "protocol": { "type": "at_command" },
                    "rx_framing": { "type": "line", "ending": "lf" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    let f0 = &frames[0];

    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");

    let data = f0["data"].as_str().expect("frame data");
    assert!(data.ends_with('\r'), "lf mode should retain \\r: {data:?}");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Connection-default e2e tests (Phase 5 layer 3-4) ────────────────────────

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_open_protocol_default_drives_write_and_read() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_with(
        &client,
        &pty_path,
        json!({ "protocol": { "type": "at_command" } }),
    )
    .await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8"
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": id, "data": "ping" }),
        ))
        .await
        .expect("write");

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    let f0 = &frames[0];
    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");
    assert_eq!(parsed["response_type"], json!("data"));
    let fields = parsed["fields"].as_array().expect("fields array");
    assert!(
        fields.iter().any(|f| f.as_str().unwrap().contains("pong")),
        "fields should contain pong: {fields:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_explicit_rx_framing_beats_connection_default() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_with(
        &client,
        &pty_path,
        json!({
            "protocol": { "type": "at_command" },
            "rx_framing": { "type": "line", "ending": "lf" }
        }),
    )
    .await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8"
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "ping").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    let f0 = &frames[0];

    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");

    let data = f0["data"].as_str().expect("frame data");
    assert!(
        data.ends_with('\r'),
        "connection lf default should retain \\r: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// save_profile snapshots not tested on native_sim — save_profile requires
// port_info (USB identity) which the software PTY does not provide.
// The save_profile wiring (port_ops.rs) copies the four defaults from the
// connection; the struct-level snapshot is exercised by the
// ProfileDefaults roundtrip test in src/profiles.rs tests.

// ── Framing-mode e2e coverage tests ──────────────────────────────────────────

/// Prove SLIP RX decode over the real software-serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_slip_decodes_frame() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "hex",
                    "rx_framing": { "type": "slip" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    // Firmware emits raw bytes: END + "pong" + END
    write_cmd(&client, &id, "sendraw hex C0706F6E67C0").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    assert_eq!(frames[0]["data"], json!("70 6f 6e 67"));
    assert_eq!(frames[0]["frame_type"], json!("slip"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Prove SLIP malformed escape surfaces as is_error on read path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_slip_malformed_escape_surfaces_framing_error() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "rx_framing": { "type": "slip" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    // Firmware emits: END, ESC, invalid byte 0x41, END
    write_cmd(&client, &id, "sendraw hex C0DB41C0").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_eq!(result.is_error, Some(true), "must surface as tool error");
    // rmcp surfaces is_error messages in content[0].text.
    let text: &str = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|rtc| rtc.text.as_str())
        .unwrap_or("");
    assert!(
        text.contains("SLIP framing error"),
        "error should mention SLIP: {text}"
    );
    assert!(text.contains("0x41"), "error should name byte: {text}");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Prove delimiter RX framing over the real serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_delimiter_framing_decodes() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "hex",
                    "rx_framing": {
                        "type": "delimiter",
                        "delimiter": "|",
                        "delimiter_encoding": "utf8"
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    // Firmware emits raw bytes: |pong| (delimited by pipe chars)
    write_cmd(&client, &id, "sendraw hex 7C706F6E677C").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    // First byte is "|" → empty frame[0], then "pong|" → frame[1] = "pong"
    assert!(
        frames.len() >= 2,
        "expected at least 2 frames, got {}",
        frames.len()
    );
    assert_eq!(frames[1]["data"], json!("70 6f 6e 67"));
    assert_eq!(frames[1]["frame_type"], json!("delimiter"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Prove length-prefixed RX framing over the real serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_length_prefixed_framing_decodes() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "rx_framing": {
                        "type": "length_prefixed",
                        "prefix_size": 1,
                        "endianness": "big"
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    // Firmware emits raw bytes: 1-byte length prefix 4, then "pong"
    write_cmd(&client, &id, "sendraw hex 04706F6E67").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    assert_eq!(frames[0]["data"], json!("pong"));
    assert_eq!(frames[0]["frame_type"], json!("length_prefixed"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Prove subscribe emits per-frame notifications with parsed content
/// when rx_parser is configured.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_subscribe_line_framing_with_at_parser_emits_parsed_frames() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, mut rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": id,
                "poll_interval_ms": 50,
                "timeout_ms": 2000,
                "encoding": "utf8",
                "max_buffered_bytes": 8192,
                "rx_framing": { "type": "line" },
                "rx_parser": { "type": "at_command" }
            }),
        ))
        .await
        .unwrap();

    write_cmd(&client, &id, "ping").await;

    let mut saw_parsed = false;
    loop {
        let n = match next_notification(&mut rx, Duration::from_secs(3)).await {
            Ok(n) => n,
            Err(_) => break,
        };
        let obj = n.data.as_object().unwrap();
        if let Some(reason) = obj.get("stop_reason").and_then(|v| v.as_str()) {
            assert_eq!(reason, "timeout", "stop: {obj:?}");
            break;
        }
        if obj.get("frame_index").is_some() {
            if let Some(parsed) = obj.get("parsed") {
                assert_eq!(parsed["parser"], json!("at_command"), "parsed: {parsed:?}");
                assert_eq!(
                    parsed["response_type"],
                    json!("data"),
                    "response_type: {parsed:?}"
                );
                saw_parsed = true;
            }
        }
    }
    assert!(
        saw_parsed,
        "should see at least one parsed frame notification"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// ── Remaining framing e2e coverage ──────────────────────────────────────────

#[cfg(unix)]
fn extract_trace_bytes(data: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for cap in data.lines() {
        if let Some(idx) = cap.find("=0x") {
            let hex_part = &cap[idx + 3..].trim_end();
            if hex_part.len() >= 2 {
                if let Ok(b) = u8::from_str_radix(&hex_part[..2], 16) {
                    bytes.push(b);
                }
            }
        }
    }
    bytes
}

/// Prove start_end RX framing over the real serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_start_end_framing_decodes() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "rx_framing": {
                        "type": "start_end",
                        "start": "<<",
                        "end": ">>",
                        "marker_encoding": "utf8"
                    }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "sendraw hex 3C3C706F6E673E3E").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    assert_eq!(frames[0]["data"], json!("pong"));
    assert_eq!(frames[0]["frame_type"], json!("start_end"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Prove TX framing via firmware's trace on (observes exact received bytes).
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_write_tx_framing_modes_observed_via_trace() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    write_cmd(&client, &id, "trace on").await;
    flush_both(&client, &id).await;

    let modes: &[(&str, serde_json::Value, &[u8])] = &[
        (
            "delimiter",
            json!({"type":"delimiter","delimiter":"|","delimiter_encoding":"utf8"}),
            b"ping|",
        ),
        (
            "length_prefixed",
            json!({"type":"length_prefixed","prefix_size":1,"endianness":"big"}),
            &[0x04, b'p', b'i', b'n', b'g'],
        ),
        (
            "start_end",
            json!({"type":"start_end","start":"<<","end":">>","marker_encoding":"utf8"}),
            b"<<ping>>",
        ),
        (
            "slip",
            json!({"type":"slip"}),
            &[0xC0, b'p', b'i', b'n', b'g', 0xC0],
        ),
    ];

    for (_name, tx_framing, expected) in modes {
        let read_handle = {
            let peer = client.peer().clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": id2,
                        "timeout_ms": 3000,
                        "max_buffered_bytes": 4096,
                        "encoding": "utf8",
                        "match": { "pattern": "pong" }
                    }),
                ))
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        client
            .peer()
            .call_tool(tool_request(
                "write",
                json!({
                    "connection_id": id, "data": "ping", "tx_framing": tx_framing
                }),
            ))
            .await
            .expect("write");
        let result = read_handle.await.unwrap().expect("read task");
        assert_ne!(result.is_error, Some(true), "read error: {result:?}");
        let s = result.structured_content.expect("structured");
        let data = s["data"].as_str().expect("data string");
        let trace_bytes = extract_trace_bytes(data);
        let found = trace_bytes.windows(expected.len()).any(|w| w == *expected);
        assert!(
            found,
            "trace should contain {expected:02x?}, got: {trace_bytes:02x?}",
        );
    }

    write_cmd(&client, &id, "trace off").await;
    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Prove explicit line endings via sendraw hex payloads.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_explicit_line_endings_split_correctly() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // lf: retains CR.
    {
        let ending = "lf";
        let read_handle = {
            let peer = client.peer().clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": id2, "timeout_ms": 3000, "max_buffered_bytes": 512,
                        "encoding": "utf8", "rx_framing": {"type":"line","ending":ending}
                    }),
                ))
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        write_cmd(&client, &id, "sendraw hex 616C7068610D0A626574610A").await;
        let result = read_handle.await.unwrap().expect("read task");
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let s = result.structured_content.expect("structured");
        let frames = s["frames"].as_array().expect("frames array");
        assert_eq!(frames.len(), 2, "lf: 2 frames");
        assert_eq!(frames[0]["data"], json!("alpha\r"), "lf retains CR");
        assert_eq!(frames[1]["data"], json!("beta"));
    }

    // cr.
    {
        let ending = "cr";
        let read_handle = {
            let peer = client.peer().clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": id2, "timeout_ms": 3000, "max_buffered_bytes": 512,
                        "encoding": "utf8", "rx_framing": {"type":"line","ending":ending}
                    }),
                ))
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        write_cmd(&client, &id, "sendraw hex 616C7068610D626574610D").await;
        let result = read_handle.await.unwrap().expect("read task");
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let s = result.structured_content.expect("structured");
        let frames = s["frames"].as_array().expect("frames array");
        assert_eq!(frames.len(), 2, "cr: 2 frames");
        assert_eq!(frames[0]["data"], json!("alpha"));
        assert_eq!(frames[1]["data"], json!("beta"));
    }

    // crlf.
    {
        let ending = "crlf";
        let read_handle = {
            let peer = client.peer().clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": id2, "timeout_ms": 3000, "max_buffered_bytes": 512,
                        "encoding": "utf8", "rx_framing": {"type":"line","ending":ending}
                    }),
                ))
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        write_cmd(&client, &id, "sendraw hex 616C7068610D0A626574610D0A").await;
        let result = read_handle.await.unwrap().expect("read task");
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let s = result.structured_content.expect("structured");
        let frames = s["frames"].as_array().expect("frames array");
        assert_eq!(frames.len(), 2, "crlf: 2 frames");
        assert_eq!(frames[0]["data"], json!("alpha"));
        assert_eq!(frames[1]["data"], json!("beta"));
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Prove connection usable after SLIP decode error (per-call decoder reset).
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_slip_recovers_after_error_on_next_call() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // Read #1: malformed SLIP → error.
    {
        let read_handle = {
            let peer = client.peer().clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": id2, "timeout_ms": 2000, "max_buffered_bytes": 512,
                        "encoding": "utf8", "rx_framing": {"type":"slip"}
                    }),
                ))
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        write_cmd(&client, &id, "sendraw hex C0DB41C0").await;
        let result = read_handle.await.unwrap().expect("read task");
        assert_eq!(result.is_error, Some(true), "read #1 must error");
    }

    // Read #2: valid SLIP → success.
    {
        let read_handle = {
            let peer = client.peer().clone();
            let id2 = id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": id2, "timeout_ms": 2000, "max_buffered_bytes": 512,
                        "encoding": "hex", "rx_framing": {"type":"slip"}
                    }),
                ))
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        write_cmd(&client, &id, "sendraw hex C0706F6E67C0").await;
        let result = read_handle.await.unwrap().expect("read task");
        assert_ne!(result.is_error, Some(true), "read #2 must succeed");
        let s = result.structured_content.expect("structured");
        let frames = s["frames"].as_array().expect("frames array");
        assert!(!frames.is_empty());
        assert_eq!(frames[0]["data"], json!("70 6f 6e 67"));
        assert_eq!(frames[0]["frame_type"], json!("slip"));
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}
