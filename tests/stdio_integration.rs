//! Layer 5 — STDIO transport integration tests.
//!
//! These tests spawn the `serial-mcp` binary as a child process,
//! connect via stdin/stdout pipes using rmcp's `TokioChildProcess` transport,
//! and assert the MCP surface works identically to the HTTP variant.

use std::time::Duration;

use rmcp::{
    model::{CallToolRequestParams, PaginatedRequestParams},
    transport::{child_process::TokioChildProcess, ConfigureCommandExt},
    ServiceExt,
};
use tokio::process::Command;

const EXPECTED_TOOLS: &[&str] = &[
    "list_ports",
    "get_version",
    "open",
    "close",
    "write",
    "read",
    "read_line",
    "flush",
    "set_dtr_rts",
    "send_break",
    "wait_for",
    "subscribe",
    "unsubscribe",
];

fn build_stdio_server() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let output = std::process::Command::new("cargo")
            .args(["build", "--bin", "serial-mcp"])
            .output()
            .expect("cargo build");
        if !output.status.success() {
            panic!(
                "cargo build --bin serial-mcp failed:\nstderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    });
}

async fn start_stdio_client() -> rmcp::service::RunningService<rmcp::service::RoleClient, ()> {
    build_stdio_server();

    let cmd = Command::new(
        std::env::current_dir()
            .unwrap()
            .join("target/debug/serial-mcp"),
    )
    .configure(|cmd| {
        cmd.env("RUST_LOG", "off");
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");

    ().serve(transport).await.expect("initialize client")
}

#[tokio::test]
async fn stdio_initialize_handshake_succeeds() {
    let client = start_stdio_client().await;
    let info = client.peer_info();
    assert!(info.is_some(), "no peer_info returned");
    assert_eq!(info.unwrap().server_info.name, "serial-mcp");
    client.cancel().await.ok();
}

#[tokio::test]
async fn stdio_list_tools_returns_all_thirteen_tools() {
    let client = start_stdio_client().await;

    let result = client
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();

    for expected in EXPECTED_TOOLS {
        assert!(
            names.contains(expected),
            "tool {expected} missing; got {names:?}"
        );
    }
    assert_eq!(names.len(), EXPECTED_TOOLS.len(), "got {names:?}");
    client.cancel().await.ok();
}

#[tokio::test]
async fn stdio_list_resources_returns_statics_and_templates() {
    let client = start_stdio_client().await;

    let resources = client
        .list_resources(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    assert_eq!(resources.resources.len(), 2, "expected 2 static resources");

    let templates = client
        .list_resource_templates(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    assert_eq!(
        templates.resource_templates.len(),
        2,
        "expected 2 resource templates (connection + raw)"
    );

    client.cancel().await.ok();
}

#[tokio::test]
#[ignore = "requires hardware loopback device"]
async fn stdio_full_connection_lifecycle_with_hardware() {
    let port = std::env::var("SERIAL_MCP_TEST_PORT").unwrap_or_else(|_| "/dev/ttyACM0".to_string());

    build_stdio_server();

    let cmd = Command::new(
        std::env::current_dir()
            .unwrap()
            .join("target/debug/serial-mcp"),
    )
    .configure(|cmd| {
        cmd.env("RUST_LOG", "off");
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");

    let client = ().serve(transport).await.expect("initialize client");

    // Open the hardware port
    let open = client
        .call_tool(
            CallToolRequestParams::new("open").with_arguments(
                serde_json::json!({
                    "port": &port,
                    "baud_rate": 115200,
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(open.is_error, Some(true), "open failed: {open:?}");
    let structured = open.structured_content.expect("structured");
    let conn_id = structured["connection_id"].as_str().unwrap();

    // Write test data
    let write = client
        .call_tool(
            CallToolRequestParams::new("write").with_arguments(
                serde_json::json!({
                    "connection_id": conn_id,
                    "data": "hello-stdio",
                    "encoding": "utf8",
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(write.is_error, Some(true), "write failed: {write:?}");

    // Give time for loopback
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Read back the data
    let read = client
        .call_tool(
            CallToolRequestParams::new("read").with_arguments(
                serde_json::json!({
                    "connection_id": conn_id,
                    "timeout_ms": 1000,
                    "max_bytes": 64,
                    "encoding": "utf8",
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(read.is_error, Some(true), "read failed: {read:?}");
    let structured = read.structured_content.expect("structured");
    let data = structured["data"].as_str().unwrap();
    assert!(
        data.contains("hello-stdio"),
        "expected 'hello-stdio' in read data, got {data:?}"
    );

    // Close the connection
    let close = client
        .call_tool(
            CallToolRequestParams::new("close").with_arguments(
                serde_json::json!({
                    "connection_id": conn_id,
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert_ne!(close.is_error, Some(true), "close failed: {close:?}");

    client.cancel().await.ok();
}
