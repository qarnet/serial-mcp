//! Binary payload integration test for the protocol emulator.
//!
//! Exercises the hex / base64 / utf8 encoding paths with raw binary data
//! that cannot be represented as valid UTF-8, plus large payloads and
//! hex-pattern matching in wait_for.

#![cfg(target_os = "linux")]

use rmcp::model::ReadResourceRequestParams;
use serde_json::json;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod common;
use common::{connect_client, pty::PtyPair, tool_request, TestServer};

// ------------------------------------------------------------------
// Minimal device emulator that speaks three commands:
//   READ BIN       → 256 bytes: 0x00 .. 0xFF
//   READ BIG       → 3 120 bytes (repeating chunk, exercises chunking)
//   READ PATTERN   → 5 bytes: 0xCA 0xFE 0xC0 0xFF 0xEE
// Everything else is silently ignored (no response).
// ------------------------------------------------------------------

async fn emulator_task(mut master: File) {
    let mut buf = vec![0u8; 512];
    let mut pos: usize = 0;
    loop {
        let n = match master.read(&mut buf[pos..]).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        pos += n;
        while let Some(nl) = buf[..pos].iter().position(|&b| b == b'\n') {
            let line = &buf[..nl];
            let line = if line.last() == Some(&b'\r') {
                &line[..line.len() - 1]
            } else {
                line
            };
            let _ = match line {
                b"READ BIN" => {
                    let payload: Vec<u8> = (0u8..=255).collect();
                    master.write_all(&payload).await
                }
                b"READ BIG" => {
                    // 13 * 240 = 3 120 bytes
                    let payload = b"HELLO WORLD! ".repeat(240);
                    master.write_all(&payload).await
                }
                b"READ PATTERN" => master.write_all(b"\xCA\xFE\xC0\xFF\xEE").await,
                _ => Ok(()),
            };
            let consumed = nl + 1;
            buf.copy_within(consumed..pos, 0);
            pos -= consumed;
        }
    }
}

fn assert_tool_ok(result: &rmcp::model::CallToolResult, label: &str) {
    assert_ne!(result.is_error, Some(true), "{label} failed: {result:?}");
}

#[tokio::test]
async fn protocol_emulator_binary_workflow() {
    // ---- Stage 0: setup ----
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();
    let (master, _slave_fd) = pty.into_parts();
    tokio::spawn(emulator_task(master));

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let open_result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({ "port": slave_path, "baud_rate": 115200 }),
        ))
        .await
        .unwrap();
    assert_tool_ok(&open_result, "open");
    let connection_id = open_result.structured_content.expect("structured")["connection_id"]
        .as_str()
        .expect("string")
        .to_string();

    // ---- Stage 1: hex roundtrip of 0x00..0xFF ----
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "READ BIN\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();

    let hex_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "max_bytes": 512,
                "encoding": "hex",
            }),
        ))
        .await
        .unwrap();
    assert_tool_ok(&hex_result, "read hex roundtrip");
    let hex_structured = hex_result.structured_content.expect("structured");
    let hex_str = hex_structured["data"].as_str().unwrap();
    let decoded =
        serial_mcp_server::codec::decode(serial_mcp_server::codec::Encoding::Hex, hex_str)
            .expect("hex decode");
    assert_eq!(decoded.len(), 256, "expected 256 raw bytes");
    let expected: Vec<u8> = (0u8..=255).collect();
    assert_eq!(decoded, expected, "byte values must be 0x00..0xFF");

    // ---- Stage 2: base64 roundtrip of 0x00..0xFF ----
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "READ BIN\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();

    let b64_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "max_bytes": 512,
                "encoding": "base64",
            }),
        ))
        .await
        .unwrap();
    assert_tool_ok(&b64_result, "read base64 roundtrip");
    let b64_structured = b64_result.structured_content.expect("structured");
    let b64_str = b64_structured["data"].as_str().unwrap();
    let decoded =
        serial_mcp_server::codec::decode(serial_mcp_server::codec::Encoding::Base64, b64_str)
            .expect("base64 decode");
    assert_eq!(decoded.len(), 256, "expected 256 raw bytes");
    assert_eq!(
        decoded, expected,
        "base64 roundtrip must match original bytes"
    );

    // ---- Stage 3: utf8 encoding on binary data uses replacement characters ----
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "READ BIN\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();

    let utf8_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "max_bytes": 256,
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();
    assert_tool_ok(&utf8_result, "utf8_encoding_binary_lossy");
    let utf8_structured = utf8_result.structured_content.expect("structured");
    let utf8_str = utf8_structured["data"].as_str().unwrap();
    // Bytes 0x00-0x7F are valid ASCII/UTF-8; bytes 0x80-0xFF are replaced with \u{FFFD}
    assert!(
        utf8_str.contains('\u{FFFD}'),
        "lossy utf8 must contain replacement characters for invalid bytes: {utf8_str}"
    );
    assert_eq!(
        utf8_structured["bytes_read"].as_u64().unwrap(),
        256,
        "lossy utf8 must report 256 bytes read"
    );

    // ---- Stage 4: large payload (>3 KB) via hex read ----
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "READ BIG\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();

    let big_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "max_bytes": 4096,
                "encoding": "hex",
            }),
        ))
        .await
        .unwrap();
    assert_tool_ok(&big_result, "read big payload");
    let big_structured = big_result.structured_content.expect("structured");
    let big_bytes = big_structured["bytes_read"].as_u64().unwrap();
    assert!(
        big_bytes >= 3000,
        "expected >=3000 bytes for big payload, got {big_bytes}"
    );

    // ---- Stage 5: wait_for with a hex pattern in binary data ----
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "READ PATTERN\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();

    let wait_result = client
        .peer()
        .call_tool(tool_request(
            "wait_for",
            json!({
                "connection_id": connection_id,
                "pattern": "cafe c0ffee",
                "pattern_encoding": "hex",
                "response_encoding": "hex",
                "timeout_ms": 3000,
                "max_bytes": 64,
            }),
        ))
        .await
        .unwrap();
    assert_tool_ok(&wait_result, "wait_for hex pattern");
    let wait_structured = wait_result.structured_content.expect("structured");
    assert_eq!(
        wait_structured["matched"],
        json!(true),
        "pattern must match"
    );
    assert!(
        wait_structured["match_index"].as_u64().is_some(),
        "match_index must be present"
    );

    // ---- Stage 6: resource still lists the open connection ----
    let detail_uri = format!("serial://connections/{connection_id}");
    let detail_res = client
        .peer()
        .read_resource(ReadResourceRequestParams::new(&detail_uri))
        .await
        .unwrap();
    let detail_text = match &detail_res.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
        _ => panic!("expected text resource"),
    };
    let detail_json: serde_json::Value = serde_json::from_str(&detail_text).expect("valid JSON");
    assert_eq!(
        detail_json["port"].as_str().unwrap(),
        slave_path,
        "connection detail must match PTY slave path"
    );

    // ---- Stage 7: cleanup ----
    let close_result = client
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_tool_ok(&close_result, "close");
    client.cancel().await.ok();
}
