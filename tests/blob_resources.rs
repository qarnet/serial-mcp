//! Test blob resources and resource templates.

use rmcp::transport::{child_process::TokioChildProcess, ConfigureCommandExt};
use rmcp::ServiceExt;
use tempfile::TempDir;
use tokio::process::Command;

mod common;

/// Build a stdio server command with profile storage isolated from user
/// configuration through a temporary `--profiles-path`.
fn isolated_stdio_command() -> (tokio::process::Command, TempDir) {
    let profiles_dir = TempDir::new().expect("temp dir for isolated blob profile store");
    let profiles_path = profiles_dir.path().join("profiles.toml");
    let cmd = Command::new(common::binaries::serial_mcp_bin()).configure(|cmd| {
        cmd.env("RUST_LOG", "off");
        cmd.arg("--profiles-path").arg(&profiles_path);
    });
    (cmd, profiles_dir)
}

#[tokio::test]
async fn blob_resource_template_is_advertised() {
    common::binaries::ensure_serial_mcp_built()
        .expect("serial-mcp binary available for blob resource tests");

    let (cmd, _profiles_dir) = isolated_stdio_command();

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
    common::binaries::ensure_serial_mcp_built()
        .expect("serial-mcp binary available for blob resource tests");

    let (cmd, _profiles_dir) = isolated_stdio_command();

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");
    let client = ().serve(transport).await.expect("initialize client");

    // With no connection, a /raw URI may return either "connection_not_found"
    // or "resource_not_found".
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
