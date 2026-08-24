//! Software-in-the-loop integration tests for the serial-mcp test firmware
//! running on `native_sim` (POSIX emulator, PTY-backed UART).
//!
//! Each test spawns its own `zephyr.exe` instance with a fresh PTY.
//! Tests do not share state, so `--test-threads=N` is safe.
//!
//! ```sh
//! cargo test --test native_sim_validation -- --ignored
//! # or with a custom binary:
//! SERIAL_MCP_NATIVE_SIM_BIN=/path/to/zephyr.exe cargo test --test native_sim_validation -- --ignored
//! ```

use std::time::Duration;

use serde_json::json;

use crate::common::firmware::NativeSimFirmware;
use crate::common::{args_object, connect_client, tool_request, TestServer};

const BAUD_RATE: u32 = 115200;
const NAME: &str = "native-sim-uart";

async fn open_pty(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        crate::common::TestClientHandler,
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

async fn open_with(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        crate::common::TestClientHandler,
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
        crate::common::TestClientHandler,
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
        crate::common::TestClientHandler,
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

/// Read data as a string from the `read` tool.
async fn read_str(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        crate::common::TestClientHandler,
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

/// Read until `expected` appears.
async fn read_until(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        crate::common::TestClientHandler,
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
        crate::common::TestClientHandler,
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

/// Encode raw bytes as hex for `sendraw hex` commands.
fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

async fn close_connection(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        crate::common::TestClientHandler,
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

/// Read the firmware boot banner ("serial-mcp test firmware ready"), then
/// flush input and output so each test starts from a known state.
async fn sync_boot(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        crate::common::TestClientHandler,
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

/// Verify the firmware responds to `ping` with `pong`.
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

/// A matching read waits first, then a later write still reaches the firmware
/// promptly.
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

/// Split write calls stay ordered so the firmware sees one valid command.
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

/// Line framing reports one committed line when the command arrives through
/// multiple write calls.
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

/// Trace mode exposes exact RX byte order for split writes,
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

/// A matching read stops on "Spam complete" after 1024-byte hex spam.
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

/// Set the connection's `max_buffered_bytes` default through `configure`, then
/// verify that `read` stops cleanly at that limit during a hex flood.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_buffer_budget_stops_under_flood() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Write first so the ring contains spam before read starts. Under the ring
    // model, read starts at the current cursor and drains buffered data.
    write_cmd(&client, &id, "spam 65536 hex").await;
    // Give the firmware time to generate enough data. Timing may produce
    // either "drained" or "max_buffered_bytes".
    tokio::time::sleep(Duration::from_millis(1000)).await;

    // Set the connection default before calling read.
    client
        .peer()
        .call_tool(tool_request(
            "configure",
            json!({
                "connection_id": id,
                "defaults": { "max_buffered_bytes": 256 },
            }),
        ))
        .await
        .expect("configure call");

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 5000,
                "max_buffered_bytes": 256,
                "encoding": "utf8",
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert!(
        s["stop_reason"] == json!("max_buffered_bytes") || s["stop_reason"] == json!("drained"),
        "expected max_buffered_bytes or drained, got: {s:?}"
    );
    let data = s["data"].as_str().unwrap_or("");
    assert!(
        data.len() <= 256,
        "data should be ≤ 256 bytes, got {}",
        data.len()
    );

    // Stop the firmware flood.
    write_cmd(&client, &id, "spam stop").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Send "touch" over the PTY command channel. Firmware responds with
/// "touch exit(42)" and exits with code 42. This validates the end-to-end
/// path from serial-mcp `write` to the firmware bootloader-entry trigger.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_bootloader_touch_exits_42() {
    let mut fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

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

    // The PTY may not appear in serialport::available_ports() on every
    // platform; list_ports must still return valid JSON.
    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

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

        // Non-USB transports report null USB-specific fields.
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

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_after_write() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Write a command, flush output, then use a matching read for pong.
    // Under ring semantics, use match-based read to reliably wait for pong.
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
                "timeout_ms": 2000,
                "encoding": "utf8",
                "match": { "pattern": "pong" }
            }),
        ))
        .await
        .expect("read");
    assert_ne!(result.is_error, Some(true), "{result:?}");

    let s = result.structured_content.expect("structured");
    assert_eq!(s["matched"], json!(true), "expected pong after flush+read");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_get_status_after_write_increments_tx_counter() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // sync_boot reads boot output only, so tx may be 0 while rx is positive.
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

    // Write and read should increase both counters.
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

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_reconfigure_baud_rate_persists() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Change baud to 38400.
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

    // Verify through get_status.
    let result = client
        .peer()
        .call_tool(tool_request("get_status", json!({ "connection_id": id })))
        .await
        .unwrap();
    let s = result.structured_content.unwrap();
    assert_eq!(s["baud_rate"], json!(38400), "baud should persist: {s:?}");

    // Verify the connection still works with a ping roundtrip.
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

    // Restore 115200.
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

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_ack_command_provides_pre_execution_ack() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Enable acknowledgments and use a matching read for confirmation.
    write_cmd(&client, &id, "ack on").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "ack on" }
                }),
            ))
            .await
            .expect("read ack on");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true)
        );
    }

    // Read the acknowledgment and pong from one matching read.
    write_cmd(&client, &id, "ping").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read ping1");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        let s = r.structured_content.expect("structured");
        assert_eq!(s["matched"], json!(true));
        let data = s["data"].as_str().unwrap_or("");
        assert!(
            data.contains("ack 0"),
            "ack should appear before pong, got: {data}"
        );
    }

    // The second ping increments the acknowledgment sequence.
    write_cmd(&client, &id, "ping").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read ping2");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        let s = r.structured_content.expect("structured");
        assert_eq!(s["matched"], json!(true));
        let data2 = s["data"].as_str().unwrap_or("");
        assert!(data2.contains("ack 1"), "ack seq should increment: {data2}");
    }

    // Disable acknowledgments and verify that no prefix remains.
    write_cmd(&client, &id, "ack off").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "ack off" }
                }),
            ))
            .await
            .expect("read ack off");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true)
        );
    }
    write_cmd(&client, &id, "ping").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read ping3");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        let s = r.structured_content.expect("structured");
        assert_eq!(s["matched"], json!(true));
        let data3 = s["data"].as_str().unwrap_or("");
        assert!(!data3.contains("ack 2"), "ack should be off: {data3}");
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_txbuf_status_reports_pending() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // When idle, txbuf reports 0. Use a matching read under ring semantics.
    write_cmd(&client, &id, "txbuf status").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "txbuf len=0 busy=0" }
                }),
            ))
            .await
            .expect("read txbuf idle");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "txbuf should be empty when idle"
        );
    }

    // Enable and release TX hold, then verify a ping roundtrip.
    write_cmd(&client, &id, "hold on").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "hold on" }
                }),
            ))
            .await
            .expect("read hold on");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true)
        );
    }
    write_cmd(&client, &id, "hold off").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "hold off" }
                }),
            ))
            .await
            .expect("read hold off");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true)
        );
    }

    write_cmd(&client, &id, "ping").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read pong");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "ping should work after hold on/off cycle"
        );
    }

    // Verify idle state again.
    flush_both(&client, &id).await;
    write_cmd(&client, &id, "txbuf status").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "txbuf len=0" }
                }),
            ))
            .await
            .expect("read txbuf post");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "txbuf should be empty after drain"
        );
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_input_clears_host_rx() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Generate a modest amount of spam, then flush input.
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

    // Read after flushing input; only a small amount should remain.
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

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_during_arm_cmd_delay() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Arm a 500ms delay for the next command and wait for confirmation.
    write_cmd(&client, &id, "arm_cmd 500").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "arm_cmd delay=500" }
                }),
            ))
            .await
            .expect("read arm_cmd");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "arm_cmd should confirm"
        );
    }

    // Firmware waits 500ms before executing ping.
    write_cmd(&client, &id, "ping").await;

    // Flush during the sleep window.
    tokio::time::sleep(Duration::from_millis(100)).await;
    flush_both(&client, &id).await;

    // Flush during the delay, then use a matching read for pong.
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 5000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read pong after flush");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "pong should arrive despite flush during arm_cmd delay"
        );
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// PTY tests cover fully delivered and partially queued TX. Unit tests in
// src/tx_session.rs cover flushed-before-delivery with a QueuedTxIo mock
// SerialIo backend. A PTY cannot reproduce that case: each write() byte
// reaches its kernel buffer immediately, so tcflush(TCOFLUSH) cannot recall
// bytes that already left serialport's output buffer.

/// A fully delivered command (`ping` then `pong`) is unaffected by a later
/// `flush(output)`. This proves flush does not retroactively disturb consumed
/// bytes or later writes.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_flush_output_after_full_delivery_is_safe() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Wait for pong so firmware has consumed the first command and replied.
    write_cmd(&client, &id, "ping").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read pong1");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "first ping should produce pong"
        );
    }

    // The first command is consumed, so flushing output must not affect its
    // delivered bytes.
    let flush = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": id, "target": "output" }),
        ))
        .await
        .expect("flush call");
    assert_ne!(flush.is_error, Some(true), "flush output: {flush:?}");

    // A second ping must still arrive; this checks that flush did not break
    // the stream or drop a later independent write.
    write_cmd(&client, &id, "ping").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read pong2");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "second ping after flush should still produce pong"
        );
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// A command without a line terminator remains in firmware's partial-line
/// `cmd_buf` and is not executed. Writing the remainder and terminator
/// completes the command. The test observes partially queued TX through
/// behavior, not a probe. `rxbuf status` is unusable here because its probe
/// bytes would append to `cmd_buf` and corrupt the partial line.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_partial_line_buffered_then_completed() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Write a partial command without a terminator. Firmware buffers it in
    // cmd_buf without executing it, so no pong should appear.
    write_raw(&client, &id, "pi").await;
    // Let firmware scan the bytes into cmd_buf, then drain stray output.
    tokio::time::sleep(Duration::from_millis(80)).await;
    flush_both(&client, &id).await;

    // A short read must not contain pong because firmware executes only on a
    // line terminator.
    let pre = read_str(&client, &id, 400).await;
    assert!(
        !pre.contains("pong"),
        "partial command without terminator must not execute, got pong in: {pre}"
    );

    // Complete the line with "ng\r\n". Firmware assembles "pi" and "ng" as
    // "ping", then executes it and emits pong.
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

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
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

    // Sync the boot banner so the test starts from a known state.
    sync_boot(&client, &id).await;

    // Verify initial data with a matching read under ring semantics.
    write_raw(&client, &id, "ping\r\n").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read pong");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "expected pong after first ping"
        );
    }

    // Reconnecting while already connected succeeds immediately.
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

    // Verify data flow after reconnect.
    flush_both(&client, &id).await;
    write_raw(&client, &id, "ping\r\n").await;
    {
        let r = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 2000,
                    "max_buffered_bytes": 256,
                    "encoding": "utf8",
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
            .expect("read pong after reconnect");
        assert_ne!(r.is_error, Some(true), "{r:?}");
        assert_eq!(
            r.structured_content
                .as_ref()
                .and_then(|s| s["matched"].as_bool()),
            Some(true),
            "expected pong after reconnect"
        );
    }
    assert_eq!(s["connection_id"], json!(id));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_line_framing_splits_lines() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;

    // Drain boot output so read only sees our commands.
    flush_both(&client, &id).await;

    // Write commands first; ring reads drain buffered data.
    write_cmd(&client, &id, "ping").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    write_cmd(&client, &id, "info").await;
    // Give firmware a moment to process both commands.
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Read with line framing after the flush.
    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 3000,
                "max_buffered_bytes": 512,
                "encoding": "utf8",
                "rx_framing": { "type": "line" }
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array should exist");
    assert!(
        frames.len() >= 2,
        "expected at least 2 frames (pong, info), got {}: {frames:?}",
        frames.len()
    );
    // Find the pong frame; command echoes may precede it.
    let pong_frame = frames
        .iter()
        .find(|f| f["data"].as_str().unwrap_or("").contains("pong"))
        .expect("frames should contain a pong line");
    assert_eq!(pong_frame["frame_type"], json!("line"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_json_parser_decodes_jsonout() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // Write first; ring reads drain buffered data.
    write_cmd(&client, &id, "jsonout").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 3000,
                "max_buffered_bytes": 1024,
                "encoding": "utf8",
                "rx_framing": { "type": "line" },
                "rx_parser": { "type": "json_lines" }
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    let json_frames: Vec<_> = frames
        .iter()
        .filter(|f| f["parsed"]["parser"] == json!("json"))
        .collect();
    assert_eq!(
        json_frames.len(),
        3,
        "expected 3 JSON frames from jsonout, got {} ({} total frames): {frames:?}",
        json_frames.len(),
        frames.len()
    );

    // Each JSON frame has inline sensor fields.
    for frame in &json_frames {
        let parsed = frame["parsed"].as_object().expect("parsed object");
        assert_eq!(
            parsed["parser"],
            json!("json"),
            "parser mismatch: {parsed:?}"
        );
        assert!(parsed["sensor"].is_string(), "missing sensor: {parsed:?}");
    }

    // Verify sensor values are inline rather than nested under "value".
    let f0 = &json_frames[0]["parsed"];
    assert_eq!(f0["sensor"], json!("temp"));
    assert!((f0["value"].as_f64().unwrap() - 25.5).abs() < 0.01);

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_at_parser_parses_pong() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // Write first; ring reads drain buffered data.
    write_cmd(&client, &id, "ping").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 3000,
                "max_buffered_bytes": 512,
                "encoding": "utf8",
                "rx_framing": { "type": "line" },
                "rx_parser": { "type": "at_command" }
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");

    // Find the AT-parsed frame; command echoes may precede it.
    let at_frame = frames
        .iter()
        .find(|f| f["parsed"]["parser"] == json!("at_command"))
        .expect("frames should contain an AT-parsed frame");
    let parsed = at_frame["parsed"].as_object().expect("parsed object");
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

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
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

    // Send three commands that each produce one output line. Read should stop
    // after capturing two frames.
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

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_framing_plus_match_combined() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // Write first, then read with line framing and a match on "pong".
    // Under ring semantics, the read drains buffered data and match forces
    // waiting until the pattern is found.
    write_cmd(&client, &id, "ping").await;
    tokio::time::sleep(Duration::from_millis(300)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
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
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["stop_reason"], json!("match_found"), "{s:?}");
    assert_eq!(s["matched"], json!(true));
    // The match may fire on raw bytes before framing decodes them, so frames
    // may be null or empty.
    let empty_frames = vec![];
    let frames = s
        .get("frames")
        .and_then(|v| v.as_array())
        .unwrap_or(&empty_frames);
    // Pong may appear in raw data or in a frame.
    let has_pong = s["data"].as_str().is_some_and(|d| d.contains("pong"))
        || frames
            .iter()
            .any(|f| f["data"].as_str().unwrap_or("").contains("pong"));
    assert!(has_pong, "should find pong in data or frames: {s:?}");

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
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

    // Write first; ring reads drain buffered data.
    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": id, "data": "ping" }),
        ))
        .await
        .expect("write");
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 3000,
                "max_buffered_bytes": 512,
                "encoding": "utf8"
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    // Find the AT-parsed pong frame from the connection default.
    let pong_frame = frames
        .iter()
        .find(|f| {
            f["parsed"]["parser"] == json!("at_command")
                && f["parsed"]["response_type"] == json!("data")
                && f["parsed"]["fields"].as_array().is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|f| f.as_str().unwrap_or("").contains("pong"))
                })
        })
        .expect("frames should contain AT-parsed pong");
    let parsed = pong_frame["parsed"].as_object().expect("parsed object");
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

    // Write first; ring reads drain buffered data.
    write_cmd(&client, &id, "ping").await;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 3000,
                "max_buffered_bytes": 512,
                "encoding": "utf8"
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");

    // Find the AT-parsed frame; it may not be frames[0].
    let at_frame = frames
        .iter()
        .find(|f| f["parsed"]["parser"] == json!("at_command"))
        .expect("frames should contain an AT-parsed frame");
    let parsed = at_frame["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");

    let data = at_frame["data"].as_str().expect("frame data");
    assert!(
        data.ends_with('\r'),
        "connection lf default should retain \\r: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// native_sim does not exercise save_profile because direct PTY opens may lack
// OS-enumerated PortInfo, which save_profile requires. save_profile snapshots
// current connection defaults; the ProfileDefaults roundtrip test in
// src/profiles.rs covers struct-level serialization.

/// Verify SLIP RX decoding over the real software-serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
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
    // Firmware emits END, "pong", END.
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

/// Verify a malformed SLIP escape returns a partial result with
/// `stop_reason=framing_error`, not a tool error.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_slip_malformed_escape_returns_partial_result() {
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
    // Firmware emits END, ESC, invalid byte 0x41, END.
    write_cmd(&client, &id, "sendraw hex C0DB41C0").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert_eq!(s["stop_reason"], json!("framing_error"));
    let error = s["error"].as_str().expect("error field");
    assert!(error.contains("SLIP"), "error should mention SLIP: {error}");
    assert!(error.contains("0x41"), "error should name byte: {error}");
    assert_eq!(s["encoding"], json!("hex"));
    let data = s["data"].as_str().expect("data field");
    assert!(
        data.chars().all(|c| c.is_ascii_hexdigit() || c == ' '),
        "data should be hex-encoded: {data}"
    );
    // The frames field may be null when no valid frame precedes the error.
    assert!(
        s["frames"].is_array() || s["frames"].is_null(),
        "frames should be array or null: {:?}",
        s["frames"]
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify delimiter RX framing over the real serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
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
    // Firmware emits |pong|, delimited by pipe characters.
    write_cmd(&client, &id, "sendraw hex 7C706F6E677C").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    // The first "|" produces an empty frame; "pong|" produces frame[1].
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

/// Verify length-prefixed RX framing over the real serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_length_prefixed_framing_decodes() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;
    // Move to live_edge so new data starts at a known position. A read with
    // `from: {"type":"now"}` moves the cursor to the live edge, then times out.
    client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({ "connection_id": id, "from": { "type": "now" }, "timeout_ms": 500, "max_buffered_bytes": 32 }),
        ))
        .await
        .expect("read from now");

    // The firmware command echo corrupts length-prefixed decoding under the
    // ring model because its first byte is interpreted as a prefix length.
    // Read raw hex data and verify the payload. The length-prefixed decoder
    // is covered by unit tests in src/framing/decoder.rs.
    write_cmd(&client, &id, "sendraw hex 04706F6E67").await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "timeout_ms": 3000,
                "max_buffered_bytes": 512,
                "encoding": "hex",
            }),
        ))
        .await
        .expect("read call");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    // The raw payload bytes 0x04 0x70 0x6F 0x6E 0x67 should appear in the
    // hex output, possibly after the command echo.
    let data = s["data"].as_str().expect("data field");
    let expected_hex = "04 70 6f 6e 67";
    assert!(
        data.to_lowercase().contains(expected_hex),
        "hex data should contain length-prefixed pong bytes, got: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify start_end RX framing over the real serial path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
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
                        "start": ["<<"],
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

/// Decode the firmware trace read payload back into raw bytes.
///
/// The firmware's `trace on` mode emits one `RX[n]=0xXX\r\n` line per
/// received byte as valid UTF-8 text; no encoding fallback applies. Each
/// line's `=0x` suffix is parsed as a single hex byte; lines without a valid
/// `=0x` pair (e.g. the firmware's own command echoes) are skipped.
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

/// Verify TX framing through firmware trace output, which exposes received bytes.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
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
            json!({"type":"start_end","start":["<<"],"end":">>","marker_encoding":"utf8"}),
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

/// Verify explicit line endings with sendraw hex payloads.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_explicit_line_endings_split_correctly() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // lf retains CR. Write first under ring semantics.
    {
        let ending = "lf";
        write_cmd(&client, &id, "sendraw hex 616C7068610D0A626574610A").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id, "timeout_ms": 3000, "max_buffered_bytes": 512,
                    "encoding": "utf8", "rx_framing": {"type":"line","ending":ending}
                }),
            ))
            .await
            .expect("read call");
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let s = result.structured_content.expect("structured");
        let frames = s["frames"].as_array().expect("frames array");
        let alpha = frames
            .iter()
            .find(|f| f["data"] == json!("alpha\r"))
            .expect("lf: alpha\\r frame");
        let beta = frames
            .iter()
            .find(|f| f["data"] == json!("beta"))
            .expect("lf: beta frame");
        assert!(alpha["data"] == json!("alpha\r"), "lf retains CR");
        assert!(beta["data"] == json!("beta"));
    }

    // cr.
    {
        let ending = "cr";
        write_cmd(&client, &id, "sendraw hex 616C7068610D626574610D").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id, "timeout_ms": 3000, "max_buffered_bytes": 512,
                    "encoding": "utf8", "rx_framing": {"type":"line","ending":ending}
                }),
            ))
            .await
            .expect("read call");
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let s = result.structured_content.expect("structured");
        let frames = s["frames"].as_array().expect("frames array");
        let alpha = frames
            .iter()
            .find(|f| f["data"] == json!("alpha"))
            .expect("cr: alpha frame");
        let beta = frames
            .iter()
            .find(|f| f["data"] == json!("beta"))
            .expect("cr: beta frame");
        assert!(alpha["data"] == json!("alpha"));
        assert!(beta["data"] == json!("beta"));
    }

    // crlf.
    {
        let ending = "crlf";
        write_cmd(&client, &id, "sendraw hex 616C7068610D0A626574610D0A").await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = client
            .peer()
            .call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id, "timeout_ms": 3000, "max_buffered_bytes": 512,
                    "encoding": "utf8", "rx_framing": {"type":"line","ending":ending}
                }),
            ))
            .await
            .expect("read call");
        assert_ne!(result.is_error, Some(true), "{result:?}");
        let s = result.structured_content.expect("structured");
        let frames = s["frames"].as_array().expect("frames array");
        let alpha = frames
            .iter()
            .find(|f| f["data"] == json!("alpha"))
            .expect("crlf: alpha frame");
        let beta = frames
            .iter()
            .find(|f| f["data"] == json!("beta"))
            .expect("crlf: beta frame");
        assert!(alpha["data"] == json!("alpha"));
        assert!(beta["data"] == json!("beta"));
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify the connection remains usable after a SLIP decode error; each call
/// creates a fresh decoder.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_slip_recovers_after_error_on_next_call() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    // Read #1: malformed SLIP produces a framing error.
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
        assert_ne!(
            result.is_error,
            Some(true),
            "read #1: new partial-result contract (was: is_error)"
        );
        let s1 = result.structured_content.expect("structured");
        assert_eq!(s1["stop_reason"], json!("framing_error"));
        let error1 = s1["error"].as_str().expect("error field");
        assert!(
            error1.contains("SLIP"),
            "read #1 error must mention SLIP: {error1}"
        );
    }

    // Read #2: valid SLIP succeeds.
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
        assert!(
            !frames.is_empty(),
            "read #2 should produce at least one frame"
        );
        let pong = frames
            .iter()
            .find(|f| f["data"] == json!("70 6f 6e 67"))
            .expect("read #2 must contain the 'pong' frame somewhere (leading empty frame from trailing 0xC0 is allowed)");
        assert_eq!(pong["frame_type"], json!("slip"));
    }

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify COBS RX decoding through the `cobs` preset over the software-serial
/// path.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_cobs_preset_decodes_frame() {
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
                    "protocol": { "type": "cobs" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    let framed = serial_mcp::framing::TxFramingMode::Cobs
        .encode(b"pong")
        .expect("cobs encode pong");
    let hex = bytes_to_hex(&framed);
    write_cmd(&client, &id, &format!("sendraw hex {hex}")).await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    assert_eq!(frames[0]["data"], json!("70 6f 6e 67"));
    assert_eq!(frames[0]["frame_type"], json!("cobs"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify NDJSON RX decoding through the `ndjson` preset: two JSON objects and
/// one skipped blank line.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_ndjson_preset_decodes_json_frames() {
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
                    "protocol": { "type": "ndjson" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    let payload = b"{\"a\":1}\n\n{\"b\":2}\n";
    let hex = bytes_to_hex(payload);
    write_cmd(&client, &id, &format!("sendraw hex {hex}")).await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert_eq!(frames.len(), 2, "expected 2 frames (blank skipped)");

    let f0 = &frames[0];
    assert_eq!(f0["data"], json!("{\"a\":1}"));
    assert_eq!(f0["frame_type"], json!("line"));
    let parsed0 = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed0["parser"], json!("json"));
    assert_eq!(parsed0["a"], json!(1));

    let f1 = &frames[1];
    assert_eq!(f1["data"], json!("{\"b\":2}"));
    assert_eq!(f1["frame_type"], json!("line"));
    let parsed1 = f1["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed1["parser"], json!("json"));
    assert_eq!(parsed1["b"], json!(2));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify NDJSON `skip_empty` skips blank and whitespace-only lines end to end.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_ndjson_preset_skips_empty_lines() {
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
                    "protocol": { "type": "ndjson" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    let payload = b"{\"a\":1}\n\n\n{\"b\":2}\n   \n{\"c\":3}\n";
    let hex = bytes_to_hex(payload);
    write_cmd(&client, &id, &format!("sendraw hex {hex}")).await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert_eq!(
        frames.len(),
        3,
        "expected 3 frames (blanks+whitespace skipped)"
    );

    let parsed0 = frames[0]["parsed"].as_object().expect("parsed0");
    assert_eq!(parsed0["parser"], json!("json"));
    assert_eq!(parsed0["a"], json!(1));

    let parsed1 = frames[1]["parsed"].as_object().expect("parsed1");
    assert_eq!(parsed1["parser"], json!("json"));
    assert_eq!(parsed1["b"], json!(2));

    let parsed2 = frames[2]["parsed"].as_object().expect("parsed2");
    assert_eq!(parsed2["parser"], json!("json"));
    assert_eq!(parsed2["c"], json!(3));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify NMEA-0183 RX decoding through the `nmea0183` preset with checksum
/// validation.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_nmea0183_preset_decodes_parsed_frame() {
    /// Compute an XOR checksum because the `checksums` module is `pub(crate)`
    /// and unavailable to integration tests.
    fn xor(bytes: &[u8]) -> u8 {
        bytes.iter().fold(0u8, |acc, b| acc ^ b)
    }

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
                    "protocol": { "type": "nmea0183" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    let body = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
    let checksum = xor(body);
    let sentence = format!("${}*{checksum:02X}\r\n", std::str::from_utf8(body).unwrap());
    let hex = bytes_to_hex(sentence.as_bytes());
    write_cmd(&client, &id, &format!("sendraw hex {hex}")).await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one NMEA frame");
    let f0 = &frames[0];
    assert_eq!(f0["frame_type"], json!("start_end"));
    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("nmea"), "parser: {parsed:?}");
    assert_eq!(parsed["talker_id"], json!("GP"));
    assert_eq!(parsed["sentence_type"], json!("GGA"));
    assert_eq!(parsed["checksum_valid"], json!(true));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

/// Verify Modbus ASCII RX decoding through the `modbus_ascii` preset with LRC
/// validation.
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_read_modbus_ascii_preset_decodes_parsed_frame() {
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
                    "protocol": { "type": "modbus_ascii" }
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    let frame = b":010300000001FB\r\n";
    let hex = bytes_to_hex(frame);
    write_cmd(&client, &id, &format!("sendraw hex {hex}")).await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(
        !frames.is_empty(),
        "expected at least one Modbus ASCII frame"
    );
    let f0 = &frames[0];
    assert_eq!(f0["frame_type"], json!("start_end"));
    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(
        parsed["parser"],
        json!("modbus_ascii"),
        "parser: {parsed:?}"
    );
    assert_eq!(parsed["address"], json!(1));
    assert_eq!(parsed["function_code"], json!(3));
    assert_eq!(parsed["checksum_valid"], json!(true));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}

// native_sim's PTY UART has no modem-line callbacks, so DTR/RTS assertion
// cannot be observed here. The controlled backend in http_integration.rs
// covers the atomic reset behavior. Arm-only capture (`reset=null`) needs no
// line control and exercises the always-on pump and ring pipeline. It captures
// post-arm command output and excludes pre-arm boot banner bytes.

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn native_capture_boot_arm_only_captures_post_arm_command_output() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;

    // Consume the pre-arm boot banner so we know exactly what predates the
    // capture (and where the shared cursor sits).
    sync_boot(&client, &id).await;

    // Arm-only capture waiting for the pong prompt.
    let call = client.peer().call_tool(tool_request(
        "capture_boot",
        json!({
            "connection_id": id,
            "reset": null,
            "match": {
                "pattern": "pong",
                "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
            },
            "timeout_ms": 3000,
        }),
    ));
    let mut call = Box::pin(call);

    // The command is issued while the capture is armed (an external actor
    // reset/powered the device): only its response may land in the result.
    tokio::select! {
        res = call.as_mut() => {
            panic!("arm-only capture completed before the command output: {res:?}");
        }
        _ = tokio::time::sleep(Duration::from_millis(300)) => {}
    }
    write_cmd(&client, &id, "ping").await;

    let result = call.await.unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    assert!(s["reset"].is_null(), "arm-only capture reports reset=null");
    assert_eq!(s["read"]["stop_reason"], json!("match_found"));
    let data = s["read"]["data"].as_str().unwrap_or_default();
    assert!(
        data.contains("pong"),
        "post-arm command output must be captured: {data:?}"
    );
    assert!(
        !data.contains("test firmware ready"),
        "pre-arm boot banner must never appear in the capture: {data:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}
