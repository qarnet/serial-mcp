//! Layer 4 — hardware-in-the-loop tests.
//!
//! Marked `#[ignore]` so they are skipped on regular `cargo test` runs and
//! in CI. They only run when invoked explicitly:
//!
//! ```sh
//! SERIAL_MCP_TEST_PORT=/dev/ttyUSB0 cargo test --test hardware_loopback -- --ignored
//! ```
//!
//! Setup required:
//!
//! 1. A USB-Serial adapter (FTDI / CP2102 / CH340 / similar) plugged into
//!    the test machine.
//! 2. The adapter's TX line jumpered to its own RX line, so every byte the
//!    server writes loops back into the read path. A breadboard jumper or
//!    a paper-clip across pins 2 (RXD) and 3 (TXD) on a DB-9 is enough.
//! 3. `SERIAL_MCP_TEST_PORT` set to the device path
//!    (`/dev/ttyUSB0` on Linux, `COM3` on Windows, `/dev/cu.usbserial-XXX`
//!    on macOS).
//!
//! These tests bypass the PTY harness and talk to a real OS serial port
//! through `tokio_serial::SerialStream`, end to end.

use std::time::Duration;

use rmcp::model::CallToolRequestParams;
use serde_json::json;

mod common;
use common::{args_object, connect_client, tool_request, TestServer};

const PORT_ENV: &str = "SERIAL_MCP_TEST_PORT";

fn require_port() -> String {
    match std::env::var(PORT_ENV) {
        Ok(p) if !p.is_empty() => p,
        _ => panic!(
            "{PORT_ENV} not set. Plug in a USB-Serial adapter with TX-RX jumpered \
             and re-run, e.g. `{PORT_ENV}=/dev/ttyUSB0 cargo test --test hardware_loopback \
             -- --ignored`."
        ),
    }
}

async fn open_port(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    port: &str,
) -> String {
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({ "port": port, "baud_rate": 115200 }),
        ))
        .await
        .expect("open call");
    if result.is_error == Some(true) {
        panic!("could not open {port}: {result:?}");
    }
    result.structured_content.expect("structured")["connection_id"]
        .as_str()
        .expect("connection_id")
        .to_string()
}

#[tokio::test]
#[ignore]
async fn hw_loopback_write_then_read_roundtrip() {
    let port = require_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let connection_id = open_port(&client, &port).await;

    // Flush any startup garbage the adapter may have buffered.
    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .unwrap();

    client
        .peer()
        .call_tool(tool_request(
            "write",
            json!({ "connection_id": connection_id, "data": "loopback-test\r\n" }),
        ))
        .await
        .unwrap();

    let read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 1500,
                "max_buffered_bytes": 64,
            }),
        ))
        .await
        .unwrap();
    let structured = read.structured_content.expect("structured");
    let data = structured["data"].as_str().unwrap();
    assert!(
        data.contains("loopback-test"),
        "TX-RX jumper missing or wrong port? got {data:?}"
    );

    client
        .peer()
        .call_tool(
            CallToolRequestParams::new("close").with_arguments(args_object(json!({
                "connection_id": connection_id,
            }))),
        )
        .await
        .unwrap();
    client.cancel().await.ok();
}

#[tokio::test]
#[ignore]
async fn hw_loopback_read_match_echo() {
    let port = require_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let connection_id = open_port(&client, &port).await;

    client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .unwrap();

    // Kick off read with match first, then send the line that should match.
    let reader = {
        let client = client.peer().clone();
        let id = connection_id.clone();
        tokio::spawn(async move {
            client
                .call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": id,
                        "timeout_ms": 3000,
                        "max_buffered_bytes": 256,
                        "encoding": "utf8",
                        "match": { "pattern": "PROMPT>" },
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
            json!({ "connection_id": connection_id, "data": "noise PROMPT> trailing" }),
        ))
        .await
        .unwrap();

    let read_result = reader.await.unwrap().expect("read+match call");
    let structured = read_result.structured_content.expect("structured");
    assert_eq!(structured["matched"], json!(true), "{structured:?}");

    client
        .peer()
        .call_tool(
            CallToolRequestParams::new("close").with_arguments(args_object(json!({
                "connection_id": connection_id,
            }))),
        )
        .await
        .unwrap();
    client.cancel().await.ok();
}
