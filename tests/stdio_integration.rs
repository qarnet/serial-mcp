//! Layer 5 — STDIO transport integration tests.
//!
//! These tests spawn the `serial-mcp` binary as a child process,
//! connect via stdin/stdout pipes using rmcp's `TokioChildProcess` transport,
//! and assert the MCP surface works identically to the HTTP variant.

use rmcp::{
    model::PaginatedRequestParams,
    transport::{child_process::TokioChildProcess, ConfigureCommandExt},
    ClientLifecycleMode, ClientServiceExt, ServiceExt,
};
use serde_json::json;
use tempfile::TempDir;
use tokio::process::Command;

mod common;
use common::EXPECTED_TOOLS;

/// Start a stdio server child with an isolated temporary `--profiles-path`
/// so the test never touches the user's actual default profile config.
/// Returns the running client plus the tempdir that must stay alive for
/// the client's lifetime.
async fn start_stdio_client() -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    TempDir,
) {
    common::binaries::ensure_serial_mcp_built()
        .expect("serial-mcp binary available for stdio tests");

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

/// Start a stdio server child and connect an explicit MODERN `2026-07-28`
/// discover-lifecycle client (self-contained per-request `_meta`).
async fn start_stdio_modern_client() -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, ()>,
    TempDir,
) {
    common::binaries::ensure_serial_mcp_built()
        .expect("serial-mcp binary available for stdio tests");

    let profiles_dir = TempDir::new().expect("temp dir for isolated stdio profile store");
    let profiles_path = profiles_dir.path().join("profiles.toml");

    let cmd = Command::new(common::binaries::serial_mcp_bin()).configure(|cmd| {
        cmd.env("RUST_LOG", "off");
        cmd.arg("--profiles-path").arg(&profiles_path);
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");

    let client = ()
        .serve_with_lifecycle(
            transport,
            ClientLifecycleMode::Discover {
                preferred_versions: vec![rmcp::model::ProtocolVersion::V_2026_07_28],
            },
        )
        .await
        .expect("modern discover client");
    (client, profiles_dir)
}

/// Start a stdio server child and connect an explicit LEGACY `2025-11-25`
/// initialize-lifecycle client.
async fn start_stdio_legacy_client() -> (
    rmcp::service::RunningService<rmcp::service::RoleClient, common::LegacyClientHandler>,
    TempDir,
) {
    common::binaries::ensure_serial_mcp_built()
        .expect("serial-mcp binary available for stdio tests");

    let profiles_dir = TempDir::new().expect("temp dir for isolated stdio profile store");
    let profiles_path = profiles_dir.path().join("profiles.toml");

    let cmd = Command::new(common::binaries::serial_mcp_bin()).configure(|cmd| {
        cmd.env("RUST_LOG", "off");
        cmd.arg("--profiles-path").arg(&profiles_path);
    });

    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");

    let client = common::LegacyClientHandler
        .serve_with_lifecycle(transport, ClientLifecycleMode::Initialize)
        .await
        .expect("legacy initialize client");
    (client, profiles_dir)
}

#[tokio::test]
async fn stdio_initialize_handshake_succeeds() {
    let (client, _profiles_dir) = start_stdio_client().await;
    let info = client.peer_info();
    assert!(info.is_some(), "no peer_info returned");
    assert_eq!(
        info.unwrap()
            .server_info
            .as_ref()
            .expect("server_info present")
            .name,
        "serial-mcp"
    );
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

#[tokio::test]
async fn stdio_modern_discovery_lifecycle_selects_2026_07_28() {
    let (client, _profiles_dir) = start_stdio_modern_client().await;
    let info = client.peer_info().expect("modern peer info");
    assert_eq!(
        info.protocol_version,
        rmcp::model::ProtocolVersion::V_2026_07_28,
        "modern discover lifecycle must negotiate 2026-07-28"
    );

    let result = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.len(), EXPECTED_TOOLS.len(), "got {names:?}");
    for expected in EXPECTED_TOOLS {
        assert!(names.contains(expected), "tool {expected} missing");
    }

    let checksum = client
        .peer()
        .call_tool(common::tool_request(
            "compute_checksum",
            json!({"data": "$GPGGA,1", "algorithm": "xor"}),
        ))
        .await
        .unwrap();
    assert_eq!(checksum.is_error, Some(false), "{checksum:?}");
    assert_eq!(
        checksum.structured_content.expect("structured content")["checksum"],
        111
    );
    client.cancel().await.ok();
}

#[tokio::test]
async fn stdio_listener_cancellation_completes_cleanly() {
    // Modern `subscriptions/listen` over stdio: the acknowledgment carries
    // the accepted filter, cancellation completes with a clean `Cancelled`
    // end state, and the server keeps serving afterwards (no hang, no
    // protocol error, child stays alive).
    let (client, _profiles_dir) = start_stdio_modern_client().await;

    let mut subscription = client
        .peer()
        .listen(
            rmcp::model::SubscriptionFilter::builder()
                .resource_subscription("serial://ports")
                .build(),
        )
        .await
        .expect("modern listen over stdio");
    assert_eq!(
        subscription
            .acknowledged()
            .resource_subscriptions
            .as_deref(),
        Some(&["serial://ports".to_string()][..]),
        "stdio listen acknowledgment carries the accepted filter"
    );

    subscription
        .cancel()
        .await
        .expect("cancelling the stdio listener");
    assert_eq!(
        subscription.end(),
        Some(&rmcp::service::SubscriptionEnd::Cancelled),
        "stdio listener cancellation completes with Cancelled"
    );

    // The server is still healthy after the cancelled listener.
    let info = client.peer_info().expect("modern peer info after cancel");
    assert_eq!(
        info.protocol_version,
        rmcp::model::ProtocolVersion::V_2026_07_28
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn stdio_legacy_initialize_lifecycle_selects_2025_11_25() {
    let (client, _profiles_dir) = start_stdio_legacy_client().await;
    let info = client.peer_info().expect("legacy peer info");
    assert_eq!(
        info.protocol_version,
        rmcp::model::ProtocolVersion::V_2025_11_25,
        "legacy initialize lifecycle must negotiate 2025-11-25"
    );

    let result = client
        .peer()
        .list_tools(Some(PaginatedRequestParams::default()))
        .await
        .unwrap();
    let names: Vec<&str> = result.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names.len(), EXPECTED_TOOLS.len(), "got {names:?}");
    for expected in EXPECTED_TOOLS {
        assert!(names.contains(expected), "tool {expected} missing");
    }

    let checksum = client
        .peer()
        .call_tool(common::tool_request(
            "compute_checksum",
            json!({"data": "$GPGGA,1", "algorithm": "xor"}),
        ))
        .await
        .unwrap();
    assert_eq!(checksum.is_error, Some(false), "{checksum:?}");
    assert_eq!(
        checksum.structured_content.expect("structured content")["checksum"],
        111
    );
    client.cancel().await.ok();
}

// ── Version flag tests ────────────────────────────────────────────────
//
// These tests exercise the CLI --version / -V / version surface, which
// prints a single line and exits before the MCP handshake. They use
// std::process::Command directly, not the rmcp transport.

fn run_bin(args: &[&str]) -> (std::process::Output, String) {
    common::binaries::ensure_serial_mcp_built().expect("serial-mcp binary available");
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

// ── Capture CLI surface ─────────────────────────────────────────────────────

#[test]
fn stdio_help_documents_capture_options() {
    let (out, stdout) = run_bin(&["--help"]);
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}",
        out.status
    );
    for opt in [
        "--capture-dir",
        "--capture-max-file-bytes",
        "--capture-max-total-bytes",
        "--capture-max-files",
    ] {
        assert!(
            stdout.contains(opt),
            "help must document {opt}, got: {stdout:?}"
        );
    }
}

#[test]
fn stdio_capture_dir_consumes_version_as_value() {
    // `--capture-dir --version`: `--version` is the VALUE of --capture-dir
    // (not a version request). The value is not an absolute path, so the
    // process must exit with a startup error — never print the version.
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args(["--capture-dir", "--version"])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(
        !stdout.starts_with("serial-mcp "),
        "must not print version when --version is the value of --capture-dir, got: {stdout:?}"
    );
    assert!(
        !out.status.success(),
        "expected startup error for non-absolute capture dir, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("capture") || stderr.contains("absolute"),
        "stderr must explain the capture dir error: {stderr:?}"
    );
}

#[test]
fn stdio_capture_quota_without_root_rejects_startup() {
    let (out, _stdout) = run_bin(&["--capture-max-files=5"]);
    assert!(
        !out.status.success(),
        "quota option without --capture-dir must reject startup, got {:?}",
        out.status
    );
    let (out, _stdout) = run_bin(&["--capture-max-file-bytes=1024"]);
    assert!(
        !out.status.success(),
        "per-file quota without root must reject"
    );
    let (out, _stdout) = run_bin(&["--capture-max-total-bytes=1024"]);
    assert!(
        !out.status.success(),
        "total quota without root must reject"
    );
}

#[test]
fn stdio_capture_relative_root_rejects_startup() {
    // A relative --capture-dir must fail startup (absolute required), even
    // when the directory exists relative to the child's cwd.
    let cwd = std::env::current_dir().expect("cwd");
    let rel = cwd.join("target/capture-cli-rel-test");
    std::fs::create_dir_all(&rel).unwrap();
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args(["--capture-dir", "target/capture-cli-rel-test"])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    std::fs::remove_dir_all(&rel).ok();
    assert!(
        !out.status.success(),
        "relative capture dir must reject startup, got {:?}",
        out.status
    );
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        stderr.contains("absolute"),
        "stderr must demand an absolute dir: {stderr:?}"
    );
}

#[test]
fn stdio_capture_missing_root_rejects_startup() {
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args(["--capture-dir", "/definitely/not/a/real/dir"])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    assert!(
        !out.status.success(),
        "missing capture dir must reject startup, got {:?}",
        out.status
    );
}

#[test]
fn stdio_capture_invalid_quota_relation_rejects_startup() {
    let dir = TempDir::new().expect("temp dir for capture root");
    // per-file > total is invalid.
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args([
            "--capture-dir",
            dir.path().to_str().unwrap(),
            "--capture-max-file-bytes=2048",
            "--capture-max-total-bytes=1024",
        ])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    assert!(
        !out.status.success(),
        "per-file > total must reject startup, got {:?}",
        out.status
    );
    // Zero limits are invalid too.
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args([
            "--capture-dir",
            dir.path().to_str().unwrap(),
            "--capture-max-files=0",
        ])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    assert!(
        !out.status.success(),
        "zero limits must reject startup, got {:?}",
        out.status
    );
}

#[test]
fn stdio_capture_file_root_rejects_startup() {
    let dir = TempDir::new().expect("temp dir");
    let file = dir.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args(["--capture-dir", file.to_str().unwrap()])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    assert!(
        !out.status.success(),
        "non-directory capture root must reject startup, got {:?}",
        out.status
    );
}

#[cfg(unix)]
#[test]
fn stdio_capture_symlink_root_rejects_startup() {
    let dir = TempDir::new().expect("temp dir");
    let real = dir.path().join("real");
    std::fs::create_dir(&real).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let out = std::process::Command::new(common::binaries::serial_mcp_bin())
        .args(["--capture-dir", link.to_str().unwrap()])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn serial-mcp");
    assert!(
        !out.status.success(),
        "symlink capture root must reject startup, got {:?}",
        out.status
    );
}

#[tokio::test]
async fn stdio_server_starts_with_capture_dir() {
    // A valid absolute --capture-dir must start the stdio server and the
    // handshake must succeed.
    common::binaries::ensure_serial_mcp_built()
        .expect("serial-mcp binary available for stdio tests");
    let capture_dir = TempDir::new().expect("temp capture dir");
    let profiles_dir = TempDir::new().expect("temp profiles dir");
    let profiles_path = profiles_dir.path().join("profiles.toml");
    let cmd = Command::new(common::binaries::serial_mcp_bin()).configure(|cmd| {
        cmd.env("RUST_LOG", "off");
        cmd.arg("--profiles-path").arg(&profiles_path);
        cmd.arg("--capture-dir").arg(capture_dir.path());
    });
    let transport = TokioChildProcess::new(cmd).expect("spawn stdio server");
    let client = ().serve(transport).await.expect("initialize client");
    let info = client.peer_info().expect("peer info");
    assert_eq!(
        info.server_info.as_ref().expect("server_info present").name,
        "serial-mcp"
    );
    client.cancel().await.ok();
}
