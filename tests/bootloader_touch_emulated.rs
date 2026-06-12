//! Integration test for the 1200-baud touch → bootloader entry flow
//! using native_sim USB CDC-ACM via USB/IP (port 3241).
//!
//! **Architecture:**
//! - zephyr.exe opens a TCP socket on port 3241 — no privileges needed
//! - `usbip --tcp-port 3241 attach` writes to vhci_hcd sysfs — needs root
//! - This test supports two privilege paths:
//!
//!   **Path A — usbip-native-sim wrappers (NixOS, preferred):**
//!   Uses `sudo -n usbip-native-sim-attach <busid>` with a NOPASSWD
//!   sudoers entry for the resolved nix-store path. The wrapper handles
//!   `--tcp-port 3241`.
//!
//!   **Path B — udev rule (rootless, one-time setup):**
//!   ```nix
//!   users.groups.usbip = {};
//!   users.users.<you>.extraGroups = ["usbip"];
//!   services.udev.extraRules = ''
//!     SUBSYSTEM=="platform", DRIVER=="vhci_hcd", GROUP="usbip", MODE="0660"
//!   '';
//!   ```
//!   Then `sudo nixos-rebuild switch`, log out/in.
//!
//! - Kernel modules loaded once at boot; `vhci_hcd` must be available.
//!
//! **Port:** Our CMakeLists.txt overrides Zephyr's hardcoded 3240 → 3241.
//!
//! **Environment:**
//! - `SERIAL_MCP_NATIVE_SIM_USB_BIN` — path to USB-enabled zephyr.exe
//!   (default: `build/native_sim_usb/firmware/zephyr/zephyr.exe`)
//! - `USBIP_NATIVE_SIM_ATTACH_CMD` — override path to attach wrapper
//!   (default: resolved path of `usbip-native-sim-attach`)
//! - `USBIP_NATIVE_SIM_DETACH_CMD` — override path to detach wrapper
//!   (default: resolved path of `usbip-native-sim-detach`)
//!
//! **Running:**
//! ```sh
//! cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1
//! ```

use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

mod common;
use common::{args_object, connect_client, tool_request, TestServer};

// ── Constants ───────────────────────────────────────────────────────────────

const TOUCH_BAUD: u32 = 1200;
const BOOTLOADER_EXIT_CODE: i32 = 42;

fn zephyr_bin() -> std::path::PathBuf {
    common::firmware::ensure_usb_firmware_built()
        .expect("USB native_sim firmware available for bootloader tests")
}

// ── Firmware process management ─────────────────────────────────────────────

struct UsbFirmware {
    child: tokio::process::Child,
    _stdout_drain: tokio::task::JoinHandle<()>,
    _stderr_drain: tokio::task::JoinHandle<()>,
}

impl UsbFirmware {
    /// Spawn zephyr.exe (no privileges needed — standard TCP socket on port 3241).
    async fn spawn() -> anyhow::Result<Self> {
        let bin = zephyr_bin();
        let mut child = Command::new(&bin)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("Failed to spawn {}", bin.display()))?;

        let stdout = child.stdout.take().context("stdout not piped")?;
        let stderr = child.stderr.take().context("stderr not piped")?;

        let mut stdout_reader = BufReader::new(stdout).lines();

        // Read until we find the PTY path line:
        //   uart connected to pseudotty: /dev/pts/N
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut pty_path: Option<String> = None;

        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), stdout_reader.next_line()).await
            {
                Ok(Ok(Some(line))) => {
                    if let Some(pos) = line.find("uart connected to pseudotty:") {
                        if let Some(path_start) = line[pos..].find("/dev/pts/") {
                            pty_path = Some(line[pos + path_start..].to_string());
                            break;
                        }
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(_)) => continue,
                Err(_elapsed) => continue,
            }
        }

        let _pty_path = pty_path
            .ok_or_else(|| anyhow::anyhow!("zephyr.exe did not print PTY path within 10s"))?;

        // Drain remaining stdout/stderr.
        let stdout_drain =
            tokio::spawn(async move { while let Ok(Some(_)) = stdout_reader.next_line().await {} });
        let mut stderr_reader = BufReader::new(stderr).lines();
        let stderr_drain =
            tokio::spawn(async move { while let Ok(Some(_)) = stderr_reader.next_line().await {} });

        Ok(Self {
            child,
            _stdout_drain: stdout_drain,
            _stderr_drain: stderr_drain,
        })
    }

    /// Verify the firmware exited with the bootloader magic exit code.
    async fn verify_bootloader_exit(mut self) -> anyhow::Result<()> {
        tokio::time::sleep(Duration::from_millis(250)).await;
        match self.child.try_wait() {
            Ok(Some(status)) => {
                let code = status.code().unwrap_or(-1);
                anyhow::ensure!(
                    code == BOOTLOADER_EXIT_CODE,
                    "Expected exit code {BOOTLOADER_EXIT_CODE}, got {code}"
                );
                Ok(())
            }
            Ok(None) => {
                self.child.start_kill().ok();
                let _ = self.child.wait().await;
                anyhow::bail!("zephyr.exe did not exit after bootloader touch");
            }
            Err(e) => {
                anyhow::bail!("Failed to check zephyr.exe status: {e}");
            }
        }
    }
}

impl Drop for UsbFirmware {
    fn drop(&mut self) {
        self.child.start_kill().ok();
    }
}

// ── USB/IP helpers ──────────────────────────────────────────────────────────

/// Resolve the real path of a command (follows symlinks).
/// Needed because `sudo` matches against the resolved path in sudoers.
fn resolve_cmd(name: &str) -> Option<String> {
    // Env var override takes priority.
    let env_key = format!(
        "USBIP_NATIVE_SIM_{}_CMD",
        name.replace("usbip-native-sim-", "")
            .replace('-', "_")
            .to_uppercase()
    );
    if let Ok(val) = std::env::var(&env_key) {
        if !val.is_empty() {
            return Some(val);
        }
    }

    let which = std::process::Command::new("which")
        .arg(name)
        .output()
        .ok()?;
    if !which.status.success() {
        return None;
    }
    let symlink = String::from_utf8_lossy(&which.stdout).trim().to_string();
    if symlink.is_empty() {
        return None;
    }

    // Resolve symlinks so sudo can match the nix-store path.
    let resolved = std::process::Command::new("readlink")
        .arg("-f")
        .arg(&symlink)
        .output()
        .ok()?;
    if resolved.status.success() {
        let real = String::from_utf8_lossy(&resolved.stdout).trim().to_string();
        if !real.is_empty() {
            return Some(real);
        }
    }

    Some(symlink)
}

/// Attach the native_sim USB device via USB/IP.
///
/// Strategy:
/// 1. If `usbip-native-sim-attach` wrapper exists → `sudo -n <wrapper> <busid>`
/// 2. Fall back to raw `usbip --tcp-port 3241 attach -r 127.0.0.1 -b <busid>`
///    (requires udev rule for rootless, or a separate sudoers entry).
async fn usbip_attach() -> anyhow::Result<String> {
    // List remote devices (no privileges needed).
    let list_output = Command::new("usbip")
        .args(["--tcp-port", "3241", "list", "-r", "127.0.0.1"])
        .output()
        .await
        .context("usbip list failed — is vhci_hcd loaded?")?;

    if !list_output.status.success() {
        let stderr = String::from_utf8_lossy(&list_output.stderr);
        anyhow::bail!("usbip list failed: {stderr}");
    }

    let list_str = String::from_utf8_lossy(&list_output.stdout);
    let bus_id = list_str
        .lines()
        .find(|l| l.contains("2fe3") || l.contains("Nordic"))
        .and_then(|l| l.split_whitespace().next())
        .map(|s| s.trim_end_matches(':'))
        .unwrap_or("1-1");

    // Try native-sim attach wrapper first (NixOS sudoers path).
    if let Some(attach_cmd) = resolve_cmd("usbip-native-sim-attach") {
        let output = Command::new("sudo")
            .args(["-n", &attach_cmd, bus_id])
            .output()
            .await
            .context("sudo usbip-native-sim-attach failed — check sudoers NOPASSWD entry")?;

        if output.status.success() {
            return Ok(bus_id.to_string());
        }

        let stderr = String::from_utf8_lossy(&output.stderr);
        // If sudo failed with "password required", fall through to raw usbip.
        if !stderr.contains("password is required") {
            anyhow::bail!("usbip-native-sim-attach failed: {stderr}");
        }
        // Else: fall through to raw usbip below.
    }

    // Fallback: raw usbip (needs udev rule or regular sudo).
    let output = Command::new("usbip")
        .args([
            "--tcp-port",
            "3241",
            "attach",
            "-r",
            "127.0.0.1",
            "-b",
            bus_id,
        ])
        .output()
        .await
        .with_context(|| {
            "usbip attach failed.\n\
             On NixOS, either:\n  \
             - Add NOPASSWD sudoers for usbip-native-sim-attach, or\n  \
             - Add a udev rule to make vhci_hcd writable:\n    \
             SUBSYSTEM==\"platform\", DRIVER==\"vhci_hcd\", GROUP=\"usbip\", MODE=\"0660\""
                .to_string()
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("usbip attach failed: {stderr}");
    }

    Ok(bus_id.to_string())
}

/// Detach USB/IP device.
async fn usbip_detach(port: &str) {
    // Try native-sim detach wrapper first.
    if let Some(detach_cmd) = resolve_cmd("usbip-native-sim-detach") {
        let _ = Command::new("sudo")
            .args(["-n", &detach_cmd, port])
            .output()
            .await;
        return;
    }

    // Fallback: raw usbip.
    let _ = Command::new("usbip")
        .args(["--tcp-port", "3241", "detach", "-p", port])
        .output()
        .await;
}

/// Find the newly created /dev/ttyACM device by recording what existed
/// before attach and diffing after.
async fn find_tty_acm(before: &str) -> anyhow::Result<String> {
    // Poll for a new /dev/ttyACM device that wasn't present before.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let output = Command::new("sh")
            .arg("-c")
            .arg("ls /dev/ttyACM* 2>/dev/null")
            .output()
            .await
            .context("ls /dev/ttyACM*")?;

        let after = String::from_utf8_lossy(&output.stdout).to_string();
        for dev in after.lines() {
            if !before.contains(dev) {
                return Ok(dev.to_string());
            }
        }
    }

    anyhow::bail!("No new /dev/ttyACM device appeared after usbip attach");
}

// ── MCP helpers ─────────────────────────────────────────────────────────────

async fn open_port(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    port: &str,
    baud_rate: u32,
) -> String {
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": port,
                "baud_rate": baud_rate,
            }),
        ))
        .await
        .expect("open call");
    assert_ne!(result.is_error, Some(true), "open failed: {result:?}");
    let s = result.structured_content.expect("structured open");
    s["connection_id"]
        .as_str()
        .expect("connection_id")
        .to_string()
}

async fn set_dtr(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
    dtr: bool,
) {
    let result = client
        .peer()
        .call_tool(tool_request(
            "set_dtr_rts",
            json!({
                "connection_id": connection_id,
                "dtr": dtr,
                "rts": false,
            }),
        ))
        .await
        .expect("set_dtr_rts call");
    assert_ne!(
        result.is_error,
        Some(true),
        "set_dtr_rts failed: {result:?}"
    );
}

async fn close_connection(
    client: &rmcp::service::RunningService<
        rmcp::service::RoleClient,
        common::NotificationCollector,
    >,
    connection_id: &str,
) {
    let result = client
        .peer()
        .call_tool(
            rmcp::model::CallToolRequestParams::new("close")
                .with_arguments(args_object(json!({ "connection_id": connection_id }))),
        )
        .await
        .expect("close call");
    assert_ne!(result.is_error, Some(true), "close failed: {result:?}");
}

// ── Test: 1200-baud touch triggers bootloader entry ─────────────────────────

/// Full 1200-baud touch flow via USB/IP:
/// 1. Spawn USB firmware (zephyr.exe — no sudo, standard TCP socket)
/// 2. usbip attach → /dev/ttyACMx appears (uses sudo or udev rule)
/// 3. Open port at 1200 baud
/// 4. Set DTR true, then false (the "touch")
/// 5. Verify zephyr.exe exits with code 42
/// 6. usbip detach, cleanup
#[tokio::test]
#[ignore = "needs privileged USB/IP access: either NOPASSWD sudoers for usbip-native-sim-attach, or vhci_hcd udev rule"]
async fn bootloader_touch_via_usbip_exits_with_42() {
    // Snapshot existing /dev/ttyACM devices before attach.
    let before_acm = std::process::Command::new("sh")
        .arg("-c")
        .arg("ls /dev/ttyACM* 2>/dev/null || true")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    // Spawn firmware. Port 3241 must be free (no other USB/IP server).
    let fw = UsbFirmware::spawn()
        .await
        .expect("spawn zephyr.exe — is port 3241 free? Stop usbip-wsl2-attach if running.");

    // Give USB/IP server time to start.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Attach via USB/IP (uses sudo or udev rule).
    let bus_id = usbip_attach()
        .await
        .expect("usbip attach failed — check sudoers or udev rule for vhci_hcd");

    // Find the newly-appeared CDC-ACM device.
    let tty_path = find_tty_acm(&before_acm)
        .await
        .expect("find /dev/ttyACM device after attach");

    // Start MCP server and open the CDC-ACM port at 1200 baud.
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_port(&client, &tty_path, TOUCH_BAUD).await;

    // Give firmware time to detect the baud rate.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Pulse DTR: assert then de-assert.
    set_dtr(&client, &id, true).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    set_dtr(&client, &id, false).await;

    // Close the MCP connection.
    close_connection(&client, &id).await;
    client.cancel().await.ok();

    // Detach USB/IP.
    usbip_detach(&bus_id).await;

    // Verify firmware exited with bootloader magic code.
    fw.verify_bootloader_exit()
        .await
        .expect("bootloader exit code 42");
}
