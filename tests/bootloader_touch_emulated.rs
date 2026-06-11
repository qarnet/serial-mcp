//! Integration test for the 1200-baud touch → bootloader entry flow
//! using native_sim USB CDC-ACM via USB/IP (port 3241).
//!
//! **Architecture:**
//! - zephyr.exe opens a TCP socket on port 3241 — no privileges needed
//! - `usbip --tcp-port 3241 attach` writes to vhci_hcd sysfs — needs root
//! - Kernel modules loaded once at boot; WSL2 bridge already does this
//!
//! **Port:** Our CMakeLists.txt overrides Zephyr's hardcoded 3240 → 3241.
//!
//! **NixOS rootless setup (one-time):** Add a udev rule to make vhci_hcd
//! sysfs files writable by the `usbip` group. Add to configuration.nix:
//! ```nix
//! users.groups.usbip = {};
//! users.users.thomas-workstation.extraGroups = ["usbip"];
//! services.udev.extraRules = ''
//!   SUBSYSTEM=="platform", DRIVER=="vhci_hcd", GROUP="usbip", MODE="0660"
//! '';
//! ```
//! Then `sudo nixos-rebuild switch`, log out/in. No sudo needed after.
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

const USB_FW_BIN: &str = "build/firmware/zephyr/zephyr.exe";
const TOUCH_BAUD: u32 = 1200;
const BOOTLOADER_EXIT_CODE: i32 = 42;

fn zephyr_bin() -> String {
    std::env::var("SERIAL_MCP_NATIVE_SIM_USB_BIN")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| USB_FW_BIN.to_string())
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
            .with_context(|| format!("Failed to spawn {bin}"))?;

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

/// Run `usbip --tcp-port 3241 attach -r 127.0.0.1 -b <bus>`.
/// No sudo needed when udev rule makes vhci_hcd sysfs group-writable.
async fn usbip_attach() -> anyhow::Result<String> {
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

    let output = Command::new("usbip")
        .args([
            "--tcp-port", "3241",
            "attach", "-r", "127.0.0.1", "-b", bus_id,
        ])
        .output()
        .await
        .with_context(|| {
            format!(
                "usbip attach failed.\n\
                 On NixOS, add a udev rule to make vhci_hcd writable:\n  \
                 services.udev.extraRules = ''\n    \
                 SUBSYSTEM==\"platform\", DRIVER==\"vhci_hcd\", GROUP=\"usbip\", MODE=\"0660\"\n  '';"
            )
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("usbip attach failed: {stderr}");
    }

    Ok(bus_id.to_string())
}

/// Detach USB/IP device. No sudo when udev rule is in place.
async fn usbip_detach(port: &str) {
    let _ = Command::new("usbip")
        .args(["--tcp-port", "3241", "detach", "-p", port])
        .output()
        .await;
}

/// Find the newly created /dev/ttyACM device.
async fn find_tty_acm() -> anyhow::Result<String> {
    let output = Command::new("sh")
        .arg("-c")
        .arg("ls /dev/ttyACM* 2>/dev/null | tail -1")
        .output()
        .await
        .context("ls /dev/ttyACM*")?;

    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();

    if path.is_empty() {
        anyhow::bail!("No /dev/ttyACM device found after usbip attach");
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
    Ok(path)
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
/// 2. usbip attach → /dev/ttyACMx appears (needs root for vhci_hcd)
/// 3. Open port at 1200 baud
/// 4. Set DTR true, then false (the "touch")
/// 5. Verify zephyr.exe exits with code 42
/// 6. usbip detach, cleanup
#[tokio::test]
#[ignore = "needs one-time udev rule for rootless usbip: SUBSYSTEM==\"platform\", DRIVER==\"vhci_hcd\", GROUP=\"usbip\", MODE=\"0660\""]
async fn bootloader_touch_via_usbip_exits_with_42() {
    // Kernel modules should already be loaded (WSL2 bridge loads them).
    // If not, load them once: sudo modprobe vhci_hcd usbip-core usbip-host

    // Spawn firmware. Port 3240 must be free (no other USB/IP server).
    let fw = UsbFirmware::spawn()
        .await
        .expect("spawn zephyr.exe — is port 3240 free? Stop usbip-wsl2-attach if running.");

    // Give USB/IP server time to start.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Attach via USB/IP (needs root).
    let bus_id = usbip_attach()
        .await
        .expect("usbip attach — needs root (sudo -n usbip ...)");

    // Find the emulated CDC-ACM device.
    let tty_path = find_tty_acm()
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
