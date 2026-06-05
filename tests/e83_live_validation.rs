use std::time::Duration;

use serde_json::json;

mod common;
use common::{args_object, connect_client, tool_request, TestServer};

const PORT_ENV: &str = "SERIAL_MCP_E83_PORT";
const DEFAULT_PORT: &str = "/dev/ttyUSB0";
const BAUD_RATE: u32 = 115200;
const NAME: &str = "e83-uart";

fn e83_port() -> String {
    std::env::var(PORT_ENV)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_PORT.to_string())
}

async fn open_named_port(
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
            json!({
                "port": port,
                "name": NAME,
                "baud_rate": BAUD_RATE,
                "flow_control": "none"
            }),
        ))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");

    let structured = result.structured_content.expect("structured open result");
    assert_eq!(structured["name"], json!(NAME));
    structured["connection_id"]
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
            json!({
                "connection_id": connection_id,
                "data": format!("{cmd}\r\n")
            }),
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
    let result = client
        .peer()
        .call_tool(tool_request(
            "flush",
            json!({ "connection_id": connection_id, "target": "both" }),
        ))
        .await
        .expect("flush call");
    assert_ne!(result.is_error, Some(true), "flush failed: {result:?}");
}

#[tokio::test]
#[ignore = "requires live E83 board on /dev/ttyUSB0"]
async fn e83_live_validation() {
    let port = e83_port();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let connection_id = open_named_port(&client, &port).await;

    let list_connections = client
        .peer()
        .call_tool(tool_request("list_connections", json!({})))
        .await
        .expect("list_connections call");
    assert_ne!(
        list_connections.is_error,
        Some(true),
        "{list_connections:?}"
    );
    let structured = list_connections
        .structured_content
        .expect("structured list_connections result");
    assert!(
        structured["connections"]
            .as_array()
            .expect("connections array")
            .iter()
            .any(|entry| {
                entry["connection_id"] == json!(connection_id)
                    && entry["name"] == json!(NAME)
                    && entry["port"] == json!(port)
                    && entry["baud_rate"] == json!(BAUD_RATE)
                    && entry["flow_control"] == json!("none")
            }),
        "named connection missing from list_connections: {structured:?}"
    );

    let set_flow = client
        .peer()
        .call_tool(tool_request(
            "set_flow_control",
            json!({ "connection_id": connection_id, "flow_control": "none" }),
        ))
        .await
        .expect("set_flow_control call");
    assert_ne!(set_flow.is_error, Some(true), "{set_flow:?}");

    write_cmd(&client, &connection_id, "audio stop").await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    flush_both(&client, &connection_id).await;

    let reader = {
        let peer = client.peer().clone();
        let id = connection_id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "timeout_ms": 5000,
                    "max_buffered_bytes": 1024
                }),
            ))
            .await
        })
    };

    tokio::time::sleep(Duration::from_millis(150)).await;

    let close_result = client
        .peer()
        .call_tool(
            rmcp::model::CallToolRequestParams::new("close").with_arguments(args_object(json!({
                "connection_id": connection_id,
            }))),
        )
        .await
        .expect("close call");
    assert_ne!(close_result.is_error, Some(true), "{close_result:?}");

    let read_after_close = reader.await.unwrap().expect("read task join");
    assert_eq!(
        read_after_close.is_error,
        Some(true),
        "{read_after_close:?}"
    );
    let close_err = read_after_close
        .content
        .first()
        .and_then(|content| content.as_text())
        .map(|text| text.text.clone())
        .unwrap_or_default();
    assert!(
        close_err.contains("Connection closed") || close_err.contains("closed"),
        "expected close-related read error, got: {close_err}"
    );

    let reopened = open_named_port(&client, &port).await;
    flush_both(&client, &reopened).await;
    write_cmd(&client, &reopened, "audio i2s-test").await;

    let read_match_result = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": reopened,
                "timeout_ms": 3000,
                "max_buffered_bytes": 4096,
                "encoding": "utf8",
                "match": { "pattern": "Queued TX" },
            }),
        ))
        .await
        .expect("read+match call");
    assert_ne!(
        read_match_result.is_error,
        Some(true),
        "{read_match_result:?}"
    );
    let read_match_structured = read_match_result
        .structured_content
        .expect("read+match structured");
    assert_eq!(
        read_match_structured["matched"],
        json!(true),
        "{read_match_structured:?}"
    );
    assert_eq!(read_match_structured["name"], json!(NAME));

    let post_reopen_read = client
        .peer()
        .call_tool(tool_request(
            "read",
            json!({
                "connection_id": reopened,
                "timeout_ms": 2500,
                "max_buffered_bytes": 2048
            }),
        ))
        .await
        .expect("post-reopen read call");
    assert_ne!(
        post_reopen_read.is_error,
        Some(true),
        "{post_reopen_read:?}"
    );
    let read_structured = post_reopen_read
        .structured_content
        .expect("post-reopen structured read");
    let read_data = read_structured["data"].as_str().unwrap_or("");
    assert!(
        read_data.contains("Queued TX") || read_data.contains("supply_next_buffers"),
        "expected fresh I2S debug data after reopen, got: {read_data:?}"
    );

    write_cmd(&client, &reopened, "audio stop").await;
    tokio::time::sleep(Duration::from_millis(150)).await;

    let _ = client
        .peer()
        .call_tool(
            rmcp::model::CallToolRequestParams::new("close").with_arguments(args_object(json!({
                "connection_id": reopened,
            }))),
        )
        .await;
    client.cancel().await.ok();
}
