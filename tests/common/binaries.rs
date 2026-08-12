//! Shared helpers that resolve the on-disk path of the `serial-mcp`
//! binary used by integration tests that spawn it as a child process.
//!
//! Two resolution rules, in order:
//!
//! 1. Environment override: `SERIAL_MCP_BIN`. When set to a non-empty
//!    string, the path is taken verbatim. This lets CI and developers
//!    point at a pre-built artifact, or a release-mode build, without
//!    re-invoking cargo.
//!
//! 2. Workspace default: `<CARGO_MANIFEST_DIR>/target/debug/serial-mcp`.
//!    If the file is missing, the test process invokes
//!    `cargo build --bin serial-mcp` from the workspace root and reuses
//!    the result. Cargo's own target-directory lock serializes concurrent
//!    build attempts.
//!
//! These helpers are synchronous. Spawning the binary remains the caller's
//! responsibility.

use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result};

const SERIAL_MCP_BIN_NAME: &str = "serial-mcp";

/// Environment variable that overrides the binary location for tests.
pub const SERIAL_MCP_BIN_ENV: &str = "SERIAL_MCP_BIN";

/// Default path to the debug `serial-mcp` binary inside the workspace.
pub fn default_serial_mcp_bin() -> PathBuf {
    super::workspace_root()
        .join("target")
        .join("debug")
        .join(format!(
            "{}{}",
            SERIAL_MCP_BIN_NAME,
            std::env::consts::EXE_SUFFIX
        ))
}

/// Resolve the path to the `serial-mcp` binary the tests should spawn.
///
/// Honors `SERIAL_MCP_BIN` if set, otherwise returns the workspace
/// default. Does **not** trigger a build — call [`ensure_serial_mcp_built`]
/// separately if you need the artifact to exist on disk.
pub fn serial_mcp_bin() -> PathBuf {
    if let Ok(value) = std::env::var(SERIAL_MCP_BIN_ENV) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_serial_mcp_bin()
}

/// Build the `serial-mcp` binary if it is not already on disk.
///
/// Returns the resolved path (identical to [`serial_mcp_bin`]). The
/// Existing artifacts are reused; otherwise cargo builds the binary.
pub fn ensure_serial_mcp_built() -> Result<PathBuf> {
    static BUILT: OnceLock<()> = OnceLock::new();
    let bin = serial_mcp_bin();
    if bin.is_file() {
        return Ok(bin);
    }
    // Mark that this process entered the auto-build path. Cargo's target-dir
    // lock serializes concurrent cargo processes; every caller still checks
    // its own build result.
    BUILT.get_or_init(|| ());
    run_cargo_build(&bin)?;
    Ok(bin)
}

fn run_cargo_build(bin: &std::path::Path) -> Result<PathBuf> {
    let root = super::workspace_root();
    eprintln!(
        "tests/common/binaries: building {bin_name} (binary missing at {bin})",
        bin_name = SERIAL_MCP_BIN_NAME,
        bin = bin.display()
    );
    let status = Command::new("cargo")
        .args(["build", "--bin", SERIAL_MCP_BIN_NAME])
        .current_dir(root)
        .status()
        .with_context(|| format!("failed to spawn cargo build for {SERIAL_MCP_BIN_NAME}"))?;
    if !status.success() {
        anyhow::bail!(
            "cargo build --bin {SERIAL_MCP_BIN_NAME} exited with {status} from {}",
            root.display()
        );
    }
    if !bin.is_file() {
        anyhow::bail!(
            "cargo build succeeded but {SERIAL_MCP_BIN_NAME} is still missing at {}",
            bin.display()
        );
    }
    Ok(bin.to_path_buf())
}
