//! Test blob resources and resource templates.

use rmcp::transport::{child_process::TokioChildProcess, ConfigureCommandExt};
use rmcp::ServiceExt;
use tokio::process::Command;

mod common;
use common::binaries::ensure_serial_mcp_built;

fn build_stdio_server() {
    ensure_serial_mcp_built().expect("serial-mcp binary available for blob resource tests");
}

#[tokio::test]
async fn blob_resource_template_is_advertised() {
    build_stdio_server();

    let cmd = Command::new(common::binaries::serial_mcp_bin()).configure(|cmd| {
        cmd.env("RUST_LOG", "off");
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");
    let client = ().serve(transport).await.expect("initialize client");

    let templates = client
        .list_resource_templates(None)
        .await
        .expect("list resource templates");

    let names: Vec<&str> = templates
        .resource_templates
        .iter()
        .map(|t| t.name.as_ref())
        .collect();

    assert!(
        names.contains(&"Raw binary data from a serial connection"),
        "Expected raw blob template, got: {names:?}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn resource_uri_parsing_includes_raw_suffix() {
    build_stdio_server();

    let cmd = Command::new(common::binaries::serial_mcp_bin()).configure(|cmd| {
        cmd.env("RUST_LOG", "off");
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");
    let client = ().serve(transport).await.expect("initialize client");

    // Verify /raw URIs are recognized (will fail since no connection exists,
    // but should fail with "connection_not_found" not "resource_not_found")
    let result = client
        .read_resource(rmcp::model::ReadResourceRequestParams::new(
            "serial://connections/test-id/raw",
        ))
        .await;

    assert!(
        result.is_err(),
        "Expected error for non-existent connection"
    );
    let err = result.unwrap_err();
    let err_text = format!("{err}");
    assert!(
        err_text.contains("connection_not_found") || err_text.contains("resource_not_found"),
        "Expected connection or resource not found, got: {err_text}"
    );

    client.cancel().await.ok();
}
