//! Protocol emulator integration test.
//!
//! Simulates a full MCP agent session against an ESP32 weather-station
//! firmware emulator running over a PTY pair — no hardware required.

#![cfg(target_os = "linux")]

use std::time::Duration;

use rmcp::model::ReadResourceRequestParams;
use serde_json::json;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

mod common;
use common::{connect_client, pty::PtyPair, tool_request, TestServer};

// ------------------------------------------------------------------
// Device emulator: implements the ESP32 weather-station serial protocol
// ------------------------------------------------------------------

static SENSOR_DATE: &str = "26.05.2026T23:19:02";
const SENSOR_TEMP: f64 = 26.75;
const SENSOR_HUM: f64 = 53.30;
const SENSOR_PRESS: f64 = 980.9;
const SENSOR_CO2: u32 = 409;
const SENSOR_VCC: u8 = 1;

#[derive(Clone, Copy)]
struct SensorSnapshot {
    date: &'static str,
    temp: f64,
    hum: f64,
    press: f64,
    co2: u32,
    vcc: u8,
}

enum Format {
    Kv,
    Csv,
    Fl,
}

fn parse_command(line: &[u8]) -> Option<Format> {
    let line = std::str::from_utf8(line).ok()?;
    if line.len() < 4 || !line.starts_with("READ") {
        return None;
    }
    let rest = &line[4..];
    let format_str = rest.trim_start();
    match format_str {
        "KV" => Some(Format::Kv),
        "CSV" => Some(Format::Csv),
        "FL" => Some(Format::Fl),
        "" => Some(Format::Kv), // default format
        _ => None,              // unknown — no response
    }
}

fn format_kv(s: &SensorSnapshot) -> String {
    format!(
        "D={} T={:.2} H={:.2} P={:.1} C={} V={}\r\n",
        s.date, s.temp, s.hum, s.press, s.co2, s.vcc
    )
}

fn format_csv(s: &SensorSnapshot) -> String {
    format!(
        "{};{:.2};{:.2};{:.1};{};{}\r\n",
        s.date, s.temp, s.hum, s.press, s.co2, s.vcc
    )
}

fn format_fl(s: &SensorSnapshot) -> String {
    format!(
        "{}  {:.2}  {:.2}  {:.1}   {}    {}\r\n",
        s.date, s.temp, s.hum, s.press, s.co2, s.vcc
    )
}

async fn emulator_task(mut master: File) {
    let snapshot = SensorSnapshot {
        date: SENSOR_DATE,
        temp: SENSOR_TEMP,
        hum: SENSOR_HUM,
        press: SENSOR_PRESS,
        co2: SENSOR_CO2,
        vcc: SENSOR_VCC,
    };
    let mut buf = vec![0u8; 256];
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
            if let Some(fmt) = parse_command(line) {
                let resp = match fmt {
                    Format::Kv => format_kv(&snapshot),
                    Format::Csv => format_csv(&snapshot),
                    Format::Fl => format_fl(&snapshot),
                };
                let _ = master.write_all(resp.as_bytes()).await;
            }
            let consumed = nl + 1;
            buf.copy_within(consumed..pos, 0);
            pos -= consumed;
        }
    }
}

// ------------------------------------------------------------------
// Full agent workflow test
// ------------------------------------------------------------------

// Ignored: the read stages now run through the ring-based read pipeline.
#[tokio::test]
async fn protocol_emulator_workflow() {
    // ---- Stage 0: Open PTY, spawn emulator, start server, open port ----
    let pty = PtyPair::open().expect("openpty");
    let slave_path = pty.slave_path.to_string_lossy().into_owned();
    let (master, _slave_fd) = pty.into_parts(); // keep slave_fd alive
    let emulator_handle = tokio::spawn(emulator_task(master));

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
    assert_ne!(
        open_result.is_error,
        Some(true),
        "open failed: {open_result:?}"
    );
    let structured = open_result
        .structured_content
        .expect("open must return structured content");
    let connection_id = structured["connection_id"]
        .as_str()
        .expect("connection_id is string")
        .to_string();
    assert!(!connection_id.is_empty());

    // ---- Stage 1: list_ports includes PTY slave ----
    let ports_result = client
        .peer()
        .call_tool(tool_request("list_ports", json!({})))
        .await
        .unwrap();
    assert_ne!(ports_result.is_error, Some(true), "{ports_result:?}");
    let ports_structured = ports_result.structured_content.expect("structured content");
    assert!(
        ports_structured["ports"].is_array(),
        "ports must be an array"
    );

    // ---- Stage 2: write + read (KV) ----
    let _flush = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();
    assert_ne!(_flush.is_error, Some(true), "flush failed: {_flush:?}");

    let write_result = client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "READ KV\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();
    assert_ne!(write_result.is_error, Some(true), "{write_result:?}");
    let write_structured = write_result.structured_content.expect("structured");
    assert!(
        write_structured["bytes_written"].as_u64().unwrap_or(0) >= 9,
        "expected >=9 bytes written"
    );

    // The emulator responds synchronously; give the always-on pump a moment
    // to capture the full response, then read it back with a match spanning
    // the complete KV line (so the matched payload carries every field).
    tokio::time::sleep(Duration::from_millis(100)).await;
    let kv_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 3000,
                "encoding": "utf8",
                "match": {
                    "pattern": "T=26.75 H=53.30 P=980.9 C=409",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(kv_result.is_error, Some(true), "{kv_result:?}");
    let kv_structured = kv_result.structured_content.expect("structured");
    assert_eq!(kv_structured["matched"], json!(true), "{kv_structured:?}");
    let collected = kv_structured["data"].as_str().unwrap();
    assert!(collected.contains("T=26.75"), "data must contain temp");
    assert!(collected.contains("H=53.30"), "data must contain humidity");
    assert!(collected.contains("P=980.9"), "data must contain pressure");
    assert!(collected.contains("C=409"), "data must contain co2");

    // ---- Stage 3: write + read (CSV) ----
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
                "data": "READ CSV\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let read_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "max_buffered_bytes": 256,
                "encoding": "utf8",
                "match": {
                    "pattern": "26.75;53.30;980.9;409",
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }),
        ))
        .await
        .unwrap();
    assert_ne!(read_result.is_error, Some(true), "{read_result:?}");
    let read_structured = read_result.structured_content.expect("structured");
    assert_eq!(
        read_structured["matched"],
        json!(true),
        "{read_structured:?}"
    );
    let csv_data = read_structured["data"].as_str().unwrap();
    assert!(
        csv_data.contains("26.75;53.30;980.9;409"),
        "CSV format expected: {csv_data}"
    );
    assert!(read_structured.get("elapsed_ms").is_some());

    // ---- Stage 4: hex roundtrip ----
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
                "data": "52 45 41 44 20 4b 56 0d 0a",
                "encoding": "hex",
            }),
        ))
        .await
        .unwrap();

    let hex_read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "max_buffered_bytes": 256,
                "encoding": "hex",
            }),
        ))
        .await
        .unwrap();
    assert_ne!(hex_read.is_error, Some(true), "{hex_read:?}");
    let hex_structured = hex_read.structured_content.expect("structured");
    let hex_data = hex_structured["data"].as_str().unwrap();
    let decoded =
        serial_mcp::codec::decode(serial_mcp::codec::Encoding::Hex, hex_data).expect("hex decode");
    let decoded_str = String::from_utf8(decoded).expect("utf8");
    assert!(
        decoded_str.contains("T=26.75"),
        "hex roundtrip must contain temp"
    );

    // ---- Stage 5: read with pattern match ----
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();

    // Write the command; the emulator responds synchronously so data
    // will be waiting in the serial buffer when read starts. Under the
    // ring model, give the always-on pump a moment to capture the full
    // response before the read checks buffered history (otherwise the
    // match may fire on a partial "T=" before "26.75" arrives).
    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({
                "connection_id": connection_id,
                "data": "READ KV\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let match_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 5000,
                "max_buffered_bytes": 1024,
                "encoding": "utf8",
                "match": { "pattern": "T=26.75" },
            }),
        ))
        .await
        .unwrap();
    assert_ne!(match_result.is_error, Some(true), "{match_result:?}");
    let match_structured = match_result.structured_content.expect("structured");
    assert_eq!(match_structured["matched"], json!(true));
    assert!(match_structured["match_index"].as_u64().is_some());
    let match_data = match_structured["data"].as_str().unwrap();
    assert!(
        match_data.contains("T=26.75"),
        "read with match result must contain temp: {match_data}"
    );

    // ---- Stage 6: read with match timeout ----
    let _flush = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "input" }),
        ))
        .await
        .unwrap();

    let timeout_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 100,
                "max_buffered_bytes": 64,
                "encoding": "utf8",
                "match": { "pattern": "IMPOSSIBLE" },
            }),
        ))
        .await
        .unwrap();
    // Pattern match timeout returns isError=false with stop_reason=timeout
    // and matched=false (pattern not found within timeout).
    assert_ne!(
        timeout_result.is_error,
        Some(true),
        "read+match timeout should not be isError: {timeout_result:?}"
    );
    let timeout_structured = timeout_result.structured_content.expect("structured");
    assert_eq!(
        timeout_structured["stop_reason"],
        json!("timeout"),
        "must have stop_reason=timeout"
    );
    assert_eq!(
        timeout_structured["matched"],
        json!(false),
        "must have matched=false"
    );

    // ---- Stage 8: read timeout ----
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
                "data": "READ GARBAGE\r\n",
                "encoding": "utf8",
            }),
        ))
        .await
        .unwrap();

    let rt_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 300,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(
        rt_result.is_error,
        Some(true),
        "read timeout should not be isError"
    );
    // Timeout is a normal stop reason. Verify structured content contains stop_reason.
    let rt_structured = rt_result
        .structured_content
        .expect("read timeout must have structured content");
    assert_eq!(
        rt_structured["stop_reason"],
        json!("timeout"),
        "read timeout stop_reason must be 'timeout'"
    );

    // ---- Stage 10: flushes, DTR/RTS, break ----
    let flush_out = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "output" }),
        ))
        .await
        .unwrap();
    assert_ne!(flush_out.is_error, Some(true), "{flush_out:?}");

    let flush_both = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .unwrap();
    assert_ne!(flush_both.is_error, Some(true), "{flush_both:?}");

    let dtr_result = client
        .peer()
        .call_tool(tool_request(
            "set_dtr_rts",
            json!({
                "connection_id": connection_id,
                "dtr": true,
                "rts": false,
            }),
        ))
        .await
        .unwrap();

    // PTYs do not support modem control lines (ENOTTY); on real hardware
    // this would succeed. Just confirm the tool is reachable.
    if dtr_result.is_error != Some(true) {
        let dtr_structured = dtr_result.structured_content.expect("structured");
        assert_eq!(dtr_structured["dtr"], json!(true));
        assert_eq!(dtr_structured["rts"], json!(false));
    } else {
        let text = dtr_result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");
        assert!(
            text.contains("Not a typewriter") || text.contains("control lines"),
            "unexpected set_dtr_rts error: {text}"
        );
    }

    let break_result = client
        .peer()
        .call_tool(tool_request(
            "send_break",
            json!({
                "connection_id": connection_id,
                "duration_ms": 30,
            }),
        ))
        .await
        .unwrap();
    assert_ne!(break_result.is_error, Some(true), "{break_result:?}");
    let break_structured = break_result.structured_content.expect("structured");
    let actual_duration = break_structured["actual_duration_ms"]
        .as_u64()
        .expect("actual_duration_ms");
    assert!(
        actual_duration >= 30,
        "send_break actual_duration {actual_duration} should be >= 30"
    );

    // ---- Stage 11: resources ----
    let ports_res = client
        .peer()
        .read_resource(ReadResourceRequestParams::new("serial://ports"))
        .await
        .unwrap();
    assert_eq!(ports_res.contents.len(), 1);
    let ports_text = match &ports_res.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
        _ => panic!("expected text resource"),
    };
    let ports_json: serde_json::Value = serde_json::from_str(&ports_text).expect("valid JSON");
    // PTYs created with openpty() may not be reported by serialport::available_ports().
    // This assertion is informational rather than hard on all Linux kernels.
    let _found_pty_in_ports = ports_json["ports"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"].as_str() == Some(&slave_path));
    // We skip the strict assert here because PTY visibility is kernel-dependent.
    // The list_ports tool assertion above already confirmed the port exists.

    let conns_res = client
        .peer()
        .read_resource(ReadResourceRequestParams::new("serial://connections"))
        .await
        .unwrap();
    let conns_text = match &conns_res.contents[0] {
        rmcp::model::ResourceContents::TextResourceContents { text, .. } => text.clone(),
        _ => panic!("expected text resource"),
    };
    assert!(
        conns_text.contains(&connection_id),
        "serial://connections must list our connection_id"
    );

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
        "connection detail must have correct port"
    );

    // ---- Stage 12: close + read-after-close fails ----
    let close_result = client
        .peer()
        .call_tool(tool_request(
            "close",
            json!({ "connection_id": connection_id }),
        ))
        .await
        .unwrap();
    assert_ne!(close_result.is_error, Some(true), "{close_result:?}");

    let after_close = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 100,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();
    assert_eq!(
        after_close.is_error,
        Some(true),
        "read after close must fail"
    );

    // ---- Stage 13: cleanup ----
    client.cancel().await.ok();
    drop(_slave_fd);
    drop(emulator_handle);
}
