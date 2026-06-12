//! Shared helpers that resolve the on-disk path of the
//! `native_sim` test firmware binaries used by integration tests.
//!
//! Three resolution rules, in order:
//!
//! 1. Environment override: `SERIAL_MCP_NATIVE_SIM_BIN` (plain) and
//!    `SERIAL_MCP_NATIVE_SIM_USB_BIN` (USB variant). When set to a
//!    non-empty string, the path is taken verbatim. CI artifacts and
//!    release builds can use these to avoid re-invoking `west`.
//!
//! 2. Workspace default: `<CARGO_MANIFEST_DIR>/build/<variant>/firmware/zephyr/zephyr.exe`.
//!    Each variant owns its own build tree (`native_sim` and
//!    `native_sim_usb`) so the plain and USB builds can never
//!    contaminate each other's Kconfig / devicetree state.
//!
//! 3. Auto-build: if the expected binary is missing, the test process
//!    invokes the repo's `fw-build-native` / `fw-build-native-usb`
//!    helper, which produces a pristine build with
//!    `compile_commands.json` for the LSP. Build is guarded by a
//!    process-global `OnceLock` so concurrent test threads share a
//!    single `west` invocation.
//!
//! These helpers are intentionally synchronous: they are called from
//! preludes and from `Once`-guarded setup blocks, not from inside
//! async test bodies. Spawning the firmware itself remains the
//! responsibility of the caller.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use anyhow::{Context, Result};

/// Plain `native_sim` build (no USB).
pub const PLAIN_VARIANT: &str = "native_sim";
/// USB-enabled `native_sim` build (Tier 2 1200-baud touch tests).
pub const USB_VARIANT: &str = "native_sim_usb";

/// Env override for the plain firmware binary path.
pub const PLAIN_BIN_ENV: &str = "SERIAL_MCP_NATIVE_SIM_BIN";
/// Env override for the USB firmware binary path.
pub const USB_BIN_ENV: &str = "SERIAL_MCP_NATIVE_SIM_USB_BIN";

/// Standard Zephyr executable name produced by `west build`.
pub const ZEPHYR_EXE: &str = "zephyr.exe";

/// Absolute path to the `serial-mcp` workspace root.
///
/// Resolved at first call from the `CARGO_MANIFEST_DIR` env var that
/// cargo always exports while running tests.
pub fn workspace_root() -> &'static PathBuf {
    static WORKSPACE_ROOT: OnceLock<PathBuf> = OnceLock::new();
    WORKSPACE_ROOT.get_or_init(|| {
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        debug_assert!(
            manifest.join("Cargo.toml").is_file(),
            "CARGO_MANIFEST_DIR does not point at a Cargo workspace root: {}",
            manifest.display()
        );
        manifest
    })
}

/// Default build tree for a given variant (e.g. `build/native_sim`).
pub fn default_build_dir(variant: &str) -> PathBuf {
    workspace_root().join("build").join(variant)
}

/// Default firmware binary location for a given variant.
///
/// Plain: `build/native_sim/firmware/zephyr/zephyr.exe`
/// USB:   `build/native_sim_usb/firmware/zephyr/zephyr.exe`
pub fn default_firmware_bin(variant: &str) -> PathBuf {
    default_build_dir(variant)
        .join("firmware")
        .join("zephyr")
        .join(ZEPHYR_EXE)
}

/// Path to the plain `native_sim` firmware binary.
pub fn plain_firmware_bin() -> PathBuf {
    firmware_bin_for_variant(PLAIN_VARIANT, PLAIN_BIN_ENV)
}

/// Path to the USB `native_sim` firmware binary.
pub fn usb_firmware_bin() -> PathBuf {
    firmware_bin_for_variant(USB_VARIANT, USB_BIN_ENV)
}

fn firmware_bin_for_variant(variant: &str, env_var: &str) -> PathBuf {
    if let Ok(value) = std::env::var(env_var) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    default_firmware_bin(variant)
}

/// Build the plain `native_sim` firmware if it is not already on disk.
///
/// Returns the resolved path (identical to [`plain_firmware_bin`]).
/// The build runs at most once per test process; subsequent calls just
/// re-check the artifact and return the path.
pub fn ensure_plain_firmware_built() -> Result<PathBuf> {
    ensure_firmware_built(PLAIN_VARIANT, PLAIN_BIN_ENV, "fw-build-native")
}

/// Build the USB `native_sim` firmware if it is not already on disk.
///
/// Returns the resolved path (identical to [`usb_firmware_bin`]).
pub fn ensure_usb_firmware_built() -> Result<PathBuf> {
    ensure_firmware_built(USB_VARIANT, USB_BIN_ENV, "fw-build-native-usb")
}

fn ensure_firmware_built(variant: &str, env_var: &str, helper: &str) -> Result<PathBuf> {
    // A process-global flag is enough — concurrent callers asking for
    // the same variant race on the same `west build`; the artifact
    // check after the build absorbs that race. The two variants share
    // one flag intentionally: only one `west` invocation runs at a
    // time inside the test process, and tests asking for both variants
    // pay the second build cost once.
    static BUILT: OnceLock<()> = OnceLock::new();
    let bin = firmware_bin_for_variant(variant, env_var);
    if bin.is_file() {
        return Ok(bin);
    }
    BUILT.get_or_init(|| ());
    run_helper(helper, &bin)
}

fn run_helper(helper: &str, bin: &Path) -> Result<PathBuf> {
    let root = workspace_root();
    eprintln!(
        "tests/common/firmware: building {variant} firmware via {helper} (binary missing at {bin})",
        variant = bin
            .components()
            .filter_map(|c| c.as_os_str().to_str())
            .nth_back(3)
            .unwrap_or("?"),
        helper = helper,
        bin = bin.display()
    );
    let status = Command::new(helper)
        .current_dir(root)
        .status()
        .with_context(|| format!("failed to spawn {helper} from {}", root.display()))?;
    if !status.success() {
        anyhow::bail!("{helper} exited with {status} from {}", root.display());
    }
    if !bin.is_file() {
        anyhow::bail!(
            "{helper} succeeded but firmware is still missing at {}",
            bin.display()
        );
    }
    Ok(bin.to_path_buf())
}
