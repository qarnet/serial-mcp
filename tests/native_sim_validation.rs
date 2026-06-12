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

    // Write a command, flush, then read — the pong should arrive.
    write_cmd(&client, &id, "ping").await;

    client
        .peer()
        .call_tool(tool_request("flush", json!({ "connection_id": id })))
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
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": id }),
        ))
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
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": id }),
        ))
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
        .call_tool(tool_request(
            "get_status",
            json!({ "connection_id": id }),
        ))
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
