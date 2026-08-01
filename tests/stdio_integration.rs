//! Layer 5 — STDIO transport integration tests.
//!
//! These tests spawn the `serial-mcp` binary as a child process,
//! connect via stdin/stdout pipes using rmcp's `TokioChildProcess` transport,
//! and assert the MCP surface works identically to the HTTP variant.

use rmcp::{
    model::PaginatedRequestParams,
    transport::{child_process::TokioChildProcess, ConfigureCommandExt},
    ServiceExt,
};
use tempfile::TempDir;
use tokio::process::Command;

mod common;
use common::binaries::ensure_serial_mcp_built;

const EXPECTED_TOOLS: &[&str] = &[
    "list_ports",
    "list_connections",
    "open",
    "close",
    "write",
    "transact",
    "read",
    "flush",
    "set_dtr_rts",
    "set_flow_control",
    "send_break",
    "subscribe",
    "unsubscribe",
    "get_status",
    "reconfigure",
    "list_profiles",
    "open_profile",
    "save_profile",
    "delete_profile",
    "configure",
    "get_log",
    "clear_log",
    "export_log",
    "reconnect",
    "compute_checksum",
];

fn build_stdio_server() {
    ensure_serial_mcp_built().expect("serial-mcp binary available for stdio tests");
}

/// Start a stdio server child with an isolated temporary `--profiles-path`
/// so the test never touches the user's actual default profile config.
/// Returns the running client plus the tempdir that must stay alive for
/// the client's lifetime.
async fn start_stdio_client() -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    TempDir,
) {
    build_stdio_server();

    let profiles_dir = TempDir::new().expect("temp dir for isolated stdio profile store");
    let profiles_path = profiles_dir.path().join("profiles.toml");

    let cmd = Command::new(common::binaries::serial_mcp_bin()).configure(|cmd| {
        cmd.env("RUST_LOG", "off");
        cmd.arg("--profiles-path").arg(&profiles_path);
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");

    let client = ().serve(transport).await.expect("initialize client");
    (client, profiles_dir)
}

#[tokio::test]
async fn stdio_initialize_handshake_succeeds() {
    let (client, _profiles_dir) = start_stdio_client().await;
    let info = client.peer_info();
    assert!(info.is_some(), "no peer_info returned");
    assert_eq!(info.unwrap().server_info.name, "serial-mcp");
    client.cancel().await.ok();
}

#[tokio::test]
async fn stdio_list_tools_returns_all_twenty_five_tools() {
    let (client, _profiles_dir) = start_stdio_client().await;

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
    let (client, _profiles_dir) = start_stdio_client().await;

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
        3,
        "expected 3 resource templates (connection + raw + log)"
    );

    client.cancel().await.ok();
}

// ── Version flag tests ────────────────────────────────────────────────
//
// These tests exercise the CLI --version / -V / version surface, which
// prints a single line and exits before the MCP handshake. They use
// std::process::Command directly, not the rmcp transport.

fn run_bin(args: &[&str]) -> (std::process::Output, String) {
    ensure_serial_mcp_built().expect("serial-mcp binary available");
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args(args)
        .output()
        .expect("spawn serial-mcp");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    (out, stdout)
}

#[test]
fn stdio_version_flag_prints_version_string() {
    let (out, stdout) = run_bin(&["--version"]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    // e.g. serial-mcp 0.7.1 (abc1234, x86_64-unknown-linux-gnu)
    assert!(
        regex::Regex::new(r"^serial-mcp \d+\.\d+\.\d+ \(.+, .+\)\n$")
            .unwrap()
            .is_match(&stdout),
        "unexpected version output: {stdout:?}"
    );
}

#[test]
fn stdio_version_short_flag_matches_long() {
    let (out_long, stdout_long) = run_bin(&["--version"]);
    assert!(out_long.status.success());
    let (out_short, stdout_short) = run_bin(&["-V"]);
    assert!(out_short.status.success());
    assert_eq!(
        stdout_long, stdout_short,
        "-V and --version output must be identical"
    );
}

#[test]
fn stdio_version_subcommand_matches_flag() {
    let (out_flag, stdout_flag) = run_bin(&["--version"]);
    assert!(out_flag.status.success());
    let (out_sub, stdout_sub) = run_bin(&["version"]);
    assert!(out_sub.status.success());
    assert_eq!(
        stdout_flag, stdout_sub,
        "version subcommand and --version output must be identical"
    );
}

#[test]
fn stdio_version_flag_takes_precedence_over_other_args() {
    let (out, stdout) = run_bin(&["--version", "--transport=http"]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert!(
        regex::Regex::new(r"^serial-mcp \d+\.\d+\.\d+ \(.+, .+\)\n$")
            .unwrap()
            .is_match(&stdout),
        "should print version and ignore trailing args, got: {stdout:?}"
    );
}

#[test]
fn stdio_version_flag_not_consumed_as_option_value() {
    // `--bind --version`: `--version` is the value of `--bind`, not a
    // version request. The bind parse fails on the bogus value and the
    // process exits non-zero — it must NOT print the version string.
    let (out, stdout) = run_bin(&["--bind", "--version"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit (argument error), got {:?}",
        out.status
    );
    assert!(
        !stdout.contains("serial-mcp"),
        "must not print version when --version is the value of --bind, got: {stdout:?}"
    );
}

#[test]
fn stdio_version_flag_not_consumed_as_equal_form_option_value() {
    // `--bind=--version`: value embedded via `=`, so the next token IS a
    // flag position. `--version` after it should still print version.
    let (out, stdout) = run_bin(&["--bind=0.0.0.0:8000", "--version"]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert!(
        regex::Regex::new(r"^serial-mcp \d+\.\d+\.\d+ \(.+, .+\)\n$")
            .unwrap()
            .is_match(&stdout),
        "should print version (--bind=value does not consume next token), got: {stdout:?}"
    );
}

#[test]
fn stdio_version_flag_not_recognized_after_double_dash() {
    // `-- --version`: everything after `--` is positional, not a flag.
    // The process should error on the unexpected positional `--version`,
    // not print the version.
    let (out, stdout) = run_bin(&["--", "--version"]);
    assert!(
        !out.status.success(),
        "expected non-zero exit (unexpected positional), got {:?}",
        out.status
    );
    assert!(
        !stdout.contains("serial-mcp"),
        "must not print version for `-- --version`, got: {stdout:?}"
    );
}

#[test]
fn stdio_help_documents_profiles_path() {
    let (out, stdout) = run_bin(&["--help"]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    assert!(
        stdout.contains("--profiles-path"),
        "help must document --profiles-path, got: {stdout:?}"
    );
}

#[test]
fn stdio_profiles_path_consumes_version_as_value() {
    // `--profiles-path --version`: `--version` is the value of the option,
    // not a version request — same rule as `--bind --version`. The binary
    // must NOT print the version string. stdin is closed, so a stdio
    // server that did start exits on EOF.
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args(["--profiles-path", "--version"])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !stdout.starts_with("serial-mcp "),
        "must not print version when --version is the value of --profiles-path, got: {stdout:?}"
    );
}
