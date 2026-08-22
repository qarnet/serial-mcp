//! Shared helpers that resolve the on-disk path of the
//! `native_sim` test firmware binary used by integration tests, plus the
//! [`NativeSimFirmware`] process harness that spawns it.
//!
//! Three resolution rules, in order:
//!
//! 1. Environment override: `SERIAL_MCP_NATIVE_SIM_BIN`. When set to a
//!    non-empty string, the path is taken verbatim. CI artifacts and
//!    release builds can use this to avoid re-invoking `west`.
//!
//! 2. Workspace default: `<CARGO_MANIFEST_DIR>/build/native_sim/firmware/zephyr/zephyr.exe`.
//!
//! 3. Auto-build: if the expected binary is missing, the test process
//!    invokes the repo's `fw-build-native` helper, which produces a
//!    pristine build with `compile_commands.json` for the LSP.
//!
//! These helpers are synchronous. Spawning the firmware itself is owned by
//! [`NativeSimFirmware::spawn`] (async, cross-platform Tokio process
//! I/O), which builds the binary on demand, discovers the PTY path from
//! stdout, drains the remaining output in a background task, and kills
//! the child on drop.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// `native_sim` build.
pub const PLAIN_VARIANT: &str = "native_sim";

/// Env override for the firmware binary path.
pub const PLAIN_BIN_ENV: &str = "SERIAL_MCP_NATIVE_SIM_BIN";

/// Standard Zephyr executable name produced by `west build`.
pub const ZEPHYR_EXE: &str = "zephyr.exe";

/// Default build tree for a given variant (e.g. `build/native_sim`).
pub fn default_build_dir(variant: &str) -> PathBuf {
    super::workspace_root().join("build").join(variant)
}

/// Default firmware binary location.
///
/// `build/native_sim/firmware/zephyr/zephyr.exe`
pub fn default_firmware_bin(variant: &str) -> PathBuf {
    default_build_dir(variant)
        .join("firmware")
        .join("zephyr")
        .join(ZEPHYR_EXE)
}

/// Path to the `native_sim` firmware binary.
pub fn plain_firmware_bin() -> PathBuf {
    firmware_bin_for_variant(PLAIN_VARIANT, PLAIN_BIN_ENV)
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

/// Build the `native_sim` firmware if it is not already on disk.
///
/// Returns the resolved path (identical to [`plain_firmware_bin`]).
/// Existing artifacts are reused; otherwise the repository helper builds the
/// firmware.
pub fn ensure_plain_firmware_built() -> Result<PathBuf> {
    ensure_firmware_built(PLAIN_VARIANT, PLAIN_BIN_ENV, "fw-build-native")
}

fn ensure_firmware_built(variant: &str, env_var: &str, helper: &str) -> Result<PathBuf> {
    // Record entry into the auto-build path. This does not serialize `west`;
    // native_sim suites run with `--test-threads=1`, and callers reuse an
    // artifact that already exists.
    static BUILT: OnceLock<()> = OnceLock::new();
    let bin = firmware_bin_for_variant(variant, env_var);
    if bin.is_file() {
        return Ok(bin);
    }
    BUILT.get_or_init(|| ());
    run_helper(helper, &bin)
}

fn run_helper(helper: &str, bin: &Path) -> Result<PathBuf> {
    let root = super::workspace_root();
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
    let status = std::process::Command::new(helper)
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

/// A running `native_sim` firmware instance with a known PTY path.
///
/// Spawns the firmware binary, parses the PTY path from stdout, and
/// drains remaining output in a background task. Kills the process on
/// drop. Spawning, discovery, and cleanup are all owned here — callers
/// await [`NativeSimFirmware::spawn`] and use [`pty_path`](Self::pty_path).
pub struct NativeSimFirmware {
    child: tokio::process::Child,
    pty_path: String,
    stdout_drain: Option<tokio::task::JoinHandle<()>>,
}

/// Observable result from explicit native firmware cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeSimShutdownReport {
    /// `true` when the stdout drain needed bounded task abort fallback after
    /// the child had already been killed and waited.
    pub stdout_drain_aborted: bool,
}

impl NativeSimFirmware {
    /// Spawn the firmware, parse the PTY path from its stdout.
    ///
    /// Builds the firmware first via [`ensure_plain_firmware_built`] if
    /// the binary is missing, then reads stdout until the PTY path line
    /// (`uart connected to pseudotty: /dev/pts/N`) appears, with a
    /// five-second discovery deadline. Uses only cross-platform Tokio
    /// process/I/O APIs so the harness compiles on every platform.
    pub async fn spawn() -> anyhow::Result<Self> {
        let bin = ensure_plain_firmware_built()
            .expect("plain native_sim firmware available for native_sim tests");
        let mut child = Command::new(&bin)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("Failed to spawn {}", bin.display()))?;

        let stdout = child.stdout.take().context("stdout not piped")?;
        let mut reader = BufReader::new(stdout).lines();

        // Read until we find the PTY path line:
        //   uart connected to pseudotty: /dev/pts/N
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut pty_path: Option<String> = None;

        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), reader.next_line()).await {
                Ok(Ok(Some(line))) => {
                    if let Some(pos) = line.find("uart connected to pseudotty:") {
                        if let Some(path_start) = line[pos..].find("/dev/pts/") {
                            pty_path = Some(line[pos + path_start..].to_string());
                            break;
                        }
                    }
                }
                Ok(Ok(None)) => break, // stdout closed
                Ok(Err(e)) => {
                    anyhow::bail!("Error reading zephyr stdout: {e}");
                }
                Err(_elapsed) => continue, // timeout, poll again
            }
        }

        let pty_path = pty_path
            .ok_or_else(|| anyhow::anyhow!("zephyr.exe did not print PTY path within 5s"))?;

        // Drain remaining stdout in background so the pipe buffer doesn't fill.
        let drain = tokio::spawn(async move {
            while let Ok(Some(_line)) = reader.next_line().await {
                // drain
            }
        });

        Ok(Self {
            child,
            pty_path,
            stdout_drain: Some(drain),
        })
    }

    /// The PTY slave path the firmware's UART is connected to.
    pub fn pty_path(&self) -> &str {
        &self.pty_path
    }

    /// Check whether the firmware process has exited, and return its exit code.
    pub fn try_exit_code(&mut self) -> Option<i32> {
        self.child.try_wait().ok().flatten().and_then(|s| s.code())
    }

    /// Kill and wait for the firmware child, then join its stdout drain within
    /// bounded time. [`Drop`] remains a best-effort fallback for existing
    /// callers that do not use this explicit path.
    pub async fn shutdown_and_join(mut self) -> Result<NativeSimShutdownReport> {
        const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

        let mut errors = Vec::new();
        let needs_wait = match self.child.try_wait() {
            Ok(Some(_)) => false,
            Ok(None) => {
                if let Err(error) = self.child.start_kill() {
                    match self.child.try_wait() {
                        Ok(Some(_)) => false,
                        Ok(None) => {
                            errors.push(format!("kill native_sim child: {error}"));
                            true
                        }
                        Err(check_error) => {
                            errors.push(format!(
                                "kill native_sim child: {error}; recheck child state: {check_error}"
                            ));
                            true
                        }
                    }
                } else {
                    true
                }
            }
            Err(error) => {
                errors.push(format!("check native_sim child state: {error}"));
                if let Err(kill_error) = self.child.start_kill() {
                    errors.push(format!(
                        "kill native_sim child after state-check failure: {kill_error}"
                    ));
                }
                true
            }
        };

        if needs_wait {
            match tokio::time::timeout(SHUTDOWN_TIMEOUT, self.child.wait()).await {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => errors.push(format!("wait for native_sim child: {error}")),
                Err(_) => errors.push(format!(
                    "timed out after {} ms waiting for native_sim child",
                    SHUTDOWN_TIMEOUT.as_millis()
                )),
            }
        }

        let mut stdout_drain = self
            .stdout_drain
            .take()
            .context("native_sim stdout drain task was already taken")?;
        let stdout_drain_aborted =
            match tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut stdout_drain).await {
                Ok(Ok(())) => false,
                Ok(Err(error)) => {
                    errors.push(format!("join native_sim stdout drain: {error}"));
                    false
                }
                Err(_) => {
                    stdout_drain.abort();
                    match tokio::time::timeout(SHUTDOWN_TIMEOUT, &mut stdout_drain).await {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) if error.is_cancelled() => {}
                        Ok(Err(error)) => {
                            errors.push(format!("join aborted native_sim stdout drain: {error}"))
                        }
                        Err(_) => errors.push(format!(
                            "timed out after {} ms joining aborted native_sim stdout drain",
                            SHUTDOWN_TIMEOUT.as_millis()
                        )),
                    }
                    true
                }
            };

        if errors.is_empty() {
            Ok(NativeSimShutdownReport {
                stdout_drain_aborted,
            })
        } else {
            anyhow::bail!("native_sim explicit shutdown failed: {}", errors.join("; "))
        }
    }
}

impl Drop for NativeSimFirmware {
    fn drop(&mut self) {
        // start_kill sends SIGKILL, best-effort cleanup.
        self.child.start_kill().ok();
        if let Some(stdout_drain) = self.stdout_drain.take() {
            stdout_drain.abort();
        }
    }
}
