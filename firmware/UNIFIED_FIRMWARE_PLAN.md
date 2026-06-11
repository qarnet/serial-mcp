# Unified Firmware Plan — native_sim + xiao_ble

## Goal

One firmware source tree builds for both `native_sim` (software emulation,
fast CI) and `xiao_ble` (real nRF52840 hardware). Same `ping`/`spam`/`trace`
commands. Same test suite. No code duplication.

## Architecture

```
firmware/
├── CMakeLists.txt              # unchanged — finds Zephyr, builds src/
├── prj.conf                    # common Kconfig for both targets
│
├── src/                        # shared source (both targets)
│   ├── main.c                  # super loop, command dispatch
│   ├── uart_drv.c              # Zephyr UART API (device-agnostic)
│   ├── uart_drv.h
│   ├── command.c               # ping, spam, trace, framing, etc.
│   ├── command.h
│   ├── usb_cdc.c               # USB CDC-ACM init + 1200-baud touch (NEW)
│   └── usb_cdc.h
│
├── boards/
│   ├── native_sim.overlay      # PTY UART (always applied)
│   ├── native_sim.conf         # PTY UART Kconfig
│   ├── native_sim_usb.overlay  # USB CDC-ACM node (only with USB conf)
│   ├── native_sim_usb.conf     # USB device stack + virtual USB controller
│   ├── xiao_ble.overlay        # uart0 nrf-uarte (always applied)
│   ├── xiao_ble.conf           # UF2 output, flash partitions
│   ├── xiao_ble_usb.overlay    # USB CDC-ACM node (only with USB conf)
│   └── xiao_ble_usb.conf       # USB device stack
│
├── pm_static.yml               # xiao_ble flash layout (ignored by native_sim)
│
├── AGENTS.md                   # updated
├── UF2_BOOTLOADER_PLAN.md      # bootloader plan (hardware)
├── UNIFIED_FIRMWARE_PLAN.md    # this file
└── ZEPHYR_EMULATION_RESEARCH.md
```

## Target Differences

| | native_sim | xiao_ble |
|---|---|---|
| Arch | POSIX (Linux executable) | ARM Cortex-M4 |
| UART driver | `zephyr,native-pty-uart` | `nordic,nrf-uarte` |
| UART binding | `/dev/pts/N` or stdin/stdout | PicoProbe via `/dev/ttyACM0` |
| USB CDC-ACM | `zephyr,native-posix-udc` via USB/IP | `nordic,nrf-usbd` via native USB-C |
| GPREGRET | Simulated global variable | Real `NRF_POWER->GPREGRET` register |
| 1200-baud reset | Calls `exit(0)` | Calls `NVIC_SystemReset()` |
| Flash layout | N/A (executable) | 0x27000 app, 0x0 bootloader |
| Build output | `zephyr.exe` | `zephyr.hex` / `zephyr.uf2` |
| pm_static.yml | Ignored | Required (0x27000) |

## UART Strategy — Device-Agnostic Code

The key insight: Zephyr's UART API is the same regardless of driver.
`uart_poll_out()`, `uart_irq_callback_set()`, etc. work identically on
`nrf-uarte` and `native-pty-uart`. The device binding changes per board:

**native_sim.overlay:**
```dts
/ {
    chosen {
        zephyr,console = &uart0;
    };
};

&uart0 {
    status = "okay";
    compatible = "zephyr,native-pty-uart";
    current-speed = <115200>;
};
```

**xiao_ble.overlay:**
```dts
&uart0 {
    status = "okay";
    current-speed = <115200>;
};
```

**uart_drv.c (unchanged logic):**
```c
// Device-agnostic — works with any Zephyr UART driver
#define UART_DEV DEVICE_DT_GET(DT_CHOSEN(zephyr_console))

void uart_init(void) {
    uart_irq_callback_set(UART_DEV, uart_isr);
    uart_irq_rx_enable(UART_DEV);
}
// ...
```

No `#ifdef` needed for the UART layer. The DTS overlay handles the binding.
This is Zephyr's strength.

## USB CDC-ACM — Conditional Compilation

USB CDC-ACM is needed for the 1200-baud touch entry. It's optional — only
active when `CONFIG_USB_DEVICE_STACK=y`. Guarded by `#ifdef`.

### Exact Zephyr APIs (from Zephyr source)

The CDC-ACM driver exposes DTR and baud rate through the USB device-next
message system. Two message types matter:

| Message | Fires when | Read current value with |
|---|---|---|
| `USBD_MSG_CDC_ACM_CONTROL_LINE_STATE` | Host changes DTR/RTS | `uart_line_ctrl_get(msg->dev, UART_LINE_CTRL_DTR, &val)` |
| `USBD_MSG_CDC_ACM_LINE_CODING` | Host changes baud rate | `uart_line_ctrl_get(msg->dev, UART_LINE_CTRL_BAUD_RATE, &val)` |

Reference: Zephyr sample `samples/subsys/usb/cdc_acm/src/main.c` lines 78-92.

### usb_cdc.c (NEW file)

```c
#include "usb_cdc.h"
#include <zephyr/kernel.h>

#ifdef CONFIG_USB_DEVICE_STACK
#include <zephyr/usb/usbd.h>
#include <zephyr/drivers/uart.h>

static struct usbd_context *usb_ctx;
static uint32_t last_baud = 115200;
static bool dtr_active = false;

#ifdef CONFIG_BOARD_NATIVE_SIM
// Simulated GPREGRET register for native_sim
// Real nRF hardware uses NRF_POWER->GPREGRET directly
uint32_t sim_gpregret = 0;
#endif

/* USB device message callback — handles DTR and baud changes from host */
static void usb_msg_cb(struct usbd_context *const ctx,
                       const struct usbd_msg *msg)
{
    if (msg->type == USBD_MSG_CDC_ACM_CONTROL_LINE_STATE) {
        uint32_t dtr = 0;
        uart_line_ctrl_get(msg->dev, UART_LINE_CTRL_DTR, &dtr);
        dtr_active = (dtr != 0);
        /* Check: DTR falling edge while baud is 1200 → bootloader entry */
        if (!dtr_active && last_baud == 1200) {
            do_bootloader_entry();
        }
    }
    if (msg->type == USBD_MSG_CDC_ACM_LINE_CODING) {
        uart_line_ctrl_get(msg->dev, UART_LINE_CTRL_BAUD_RATE, &last_baud);
    }
}

/* Common entry point — target-specific reset */
static void do_bootloader_entry(void)
{
#ifdef CONFIG_BOARD_NATIVE_SIM
    sim_gpregret = 0x57;
    printk("BOOTLOADER_ENTRY gpregret=0x57\n");
    exit(42);  /* magic exit code: 42 = entered UF2 bootloader */
#else
    /* Real nRF52: write GPREGRET magic, then system reset */
    NRF_POWER->GPREGRET = 0x57;
    NVIC_SystemReset();
#endif
}

/* Initialize USB device stack and register message callback */
int usb_cdc_init(void)
{
    usb_ctx = usbd_init_device(usb_msg_cb);
    if (!usb_ctx) {
        printk("USB: init failed\n");
        return -1;
    }
    int err = usbd_enable(usb_ctx);
    if (err) {
        printk("USB: enable failed %d\n", err);
        return err;
    }
    printk("USB: CDC-ACM ready\n");
    return 0;
}

#else
/* Stub when CONFIG_USB_DEVICE_STACK is not set */
int usb_cdc_init(void) { return 0; }
#endif
```

### DTS Overlays — Split by Feature

PTY UART overlay goes in the board overlay file directly.
USB CDC-ACM overlay goes in a **separate** overlay, only applied when
the USB extra-conf is used. Otherwise the `cdc_acm_uart0` node triggers
build errors when USB is disabled.

**boards/native_sim.overlay** (always applied — PTY UART only):
```dts
/ {
    chosen {
        zephyr,console = &uart0;
    };
};

&uart0 {
    status = "okay";
    compatible = "zephyr,native-pty-uart";
    current-speed = <115200>;
};
```

**boards/native_sim_usb.overlay** (applied with USB extra-conf):
```dts
&zephyr_udc0 {
    cdc_acm_uart0: cdc_acm_uart0 {
        compatible = "zephyr,cdc-acm-uart";
    };
};
```

**boards/xiao_ble.overlay** (always applied — nRF UART only):
```dts
&uart0 {
    status = "okay";
    current-speed = <115200>;
};
```

**boards/xiao_ble_usb.overlay** (applied with USB extra-conf):
```dts
&zephyr_udc0 {
    cdc_acm_uart0: cdc_acm_uart0 {
        compatible = "zephyr,cdc-acm-uart";
    };
};
```

## Kconfig Strategy

### File overview

| File | When applied | What it does |
|---|---|---|
| `prj.conf` | Both targets, always | Shared serial + command config |
| `boards/native_sim.conf` | `-b native_sim` | PTY UART, no flash layout |
| `boards/xiao_ble.conf` | `-b xiao_ble` | UF2 output, flash partition, GPIO |
| `boards/native_sim_usb.conf` | `-DEXTRA_CONF_FILE=...` | USB device stack + virtual USB controller |
| `boards/xiao_ble_usb.conf` | `-DEXTRA_CONF_FILE=...` | USB device stack (nRF USBD auto-selected) |

### Contents

**prj.conf** (common — both targets):
```ini
CONFIG_SERIAL=y
CONFIG_UART_INTERRUPT_DRIVEN=y
CONFIG_UART_LINE_CTRL=y
CONFIG_RING_BUFFER=y
CONFIG_HWINFO=y
CONFIG_LOG=y
CONFIG_CONSOLE=n
CONFIG_UART_CONSOLE=n
# USB disabled by default — enable via extra conf for 1200-baud touch
# CONFIG_USB_DEVICE_STACK is not set
```

**boards/native_sim.conf:**
```ini
CONFIG_UART_NATIVE_PTY=y
CONFIG_UART_NATIVE_PTY_0_ON_STDINOUT=y
# native_sim has no flash layout — no UF2/partition config needed
```

**boards/xiao_ble.conf:**
```ini
CONFIG_BUILD_OUTPUT_UF2=y
CONFIG_USE_DT_CODE_PARTITION=y
CONFIG_BOOTLOADER_MCUBOOT=n
CONFIG_GPIO=y
# nrf-uarte selected by DTS automatically
```

**boards/native_sim_usb.conf** (extra conf for USB CDC-ACM testing):
```ini
# Enable USB device stack
CONFIG_USB_DEVICE_STACK=y
# Enable CDC-ACM UART class (creates /dev/ttyACMx on host after usbip attach)
CONFIG_USBD_CDC_ACM_CLASS=y
# Enable native POSIX USB device controller (exports over USB/IP)
CONFIG_USB_NATIVE_POSIX=y
# Optional: enable DTE rate callback for baud change detection
# (not strictly needed — USBD_MSG_CDC_ACM_LINE_CODING works without it)
# CONFIG_CDC_ACM_DTE_RATE_CALLBACK_SUPPORT=y
```

**boards/xiao_ble_usb.conf** (extra conf for USB CDC-ACM testing):
```ini
# Enable USB device stack
CONFIG_USB_DEVICE_STACK=y
# Enable CDC-ACM UART class
CONFIG_USBD_CDC_ACM_CLASS=y
# nRF USBD driver is auto-selected by DTS when zephyr,cdc-acm-uart node exists
```

## Build Commands

```bash
# ── Tier 1: native_sim + PTY UART (fast, no USB) ──
west build -b native_sim firmware/
west build -t run
# Prints: UART connected to pseudotty: /dev/pts/5
# In another terminal: screen /dev/pts/5
# Type: ping<Enter> → pong

# ── Tier 2: native_sim + USB CDC-ACM via USB/IP ──
west build -b native_sim firmware/ \
  -- -DEXTRA_CONF_FILE=boards/native_sim_usb.conf \
     -DEXTRA_DTC_OVERLAY_FILE=boards/native_sim_usb.overlay
#
# Run with networking for USB/IP:
sudo zephyr.exe  # (needs net_admin for USB/IP)
#
# On host, in another terminal:
sudo modprobe vhci_hcd usbip-core usbip-host
sudo usbip list -r 127.0.0.1           # should show exported device
sudo usbip attach -r 127.0.0.1 -b 1-1  # creates /dev/ttyACM1
#
# Now serial-mcp can open /dev/ttyACM1 at 1200 baud

# ── XIAO BLE (no USB — PicoProbe commands only) ──
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble firmware/ --pristine

# ── XIAO BLE (with USB CDC-ACM — for 1200-baud touch over native USB-C) ──
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble firmware/ --pristine -- \
    -DEXTRA_CONF_FILE=boards/xiao_ble_usb.conf \
    -DEXTRA_DTC_OVERLAY_FILE=boards/xiao_ble_usb.overlay
```

### USB/IP Host Setup (Tier 2 only)

```bash
# One-time setup — install usbip tool and load kernel modules
sudo apt install usbip hwdata  # or: linux-tools-generic
sudo modprobe vhci_hcd usbip-core usbip-host

# After zephyr.exe starts, in a second terminal:
usbip list -r 127.0.0.1          # should show exported device
# Output example:
#   - 127.0.0.1
#       1-1: NordicSemiconductor : unknown product (2fe3:0001)
#          : /sys/bus/usb/devices/usb1/1-1
#          :  0 - Communications / Abstract (modem) / None (02/02/00)
#          :  1 - CDC Data / Unused / unknown protocol (0a/00/00)

sudo usbip attach -r 127.0.0.1 -b 1-1

# Verify the device appeared:
ls -la /dev/ttyACM*
# → /dev/ttyACM1  (or next available number)

# Cleanup after test:
sudo usbip detach -p 0
sudo modprobe -r vhci_hcd usbip-core usbip-host
```

## Test Layers

```
Tier 1: native_sim + PTY UART               (fast CI, < 1s)
  test: tests/native_sim_validation.rs
  └─ Spawn zephyr.exe, open PTY, run ping/spam/trace/framing

Tier 2: native_sim + USB CDC-ACM via USB/IP  (medium CI, < 5s)
  test: tests/bootloader_touch_emulated.rs
  └─ usbip attach, open /dev/ttyACMx at 1200, DTR toggle, verify exit

Tier 3: XIAO BLE hardware + PicoProbe        (CI with hardware, ~4s)
  test: tests/xiao_ble_validation.rs         (existing tests)
  └─ Open /dev/ttyACM0, run ping/spam/trace/framing

Tier 4: XIAO BLE hardware + native USB CDC   (CI with hardware)
  test: tests/bootloader_touch_hardware.rs
  └─ Open native USB CDC port, 1200 baud, DTR toggle, verify UF2 drive
```

## Status

| Step | Description | Status |
|------|-------------|--------|
| 0 | Toolchain research | ✅ Done |
| 1 | native_sim board support | ✅ Done (see divergences) |
| 2 | Device-agnostic UART driver | ✅ Done |
| 3 | USB CDC-ACM driver | ✅ Done (dual-stack: legacy for native_sim, device-next for xiao_ble) |
| 4 | `tests/native_sim_validation.rs` | ✅ Done |
| 5 | `tests/bootloader_touch_emulated.rs` | ✅ Tested (8.6s, NOPASSWD sudoers) |
| 6 | pm_static.yml update | ✅ Done |
| 7 | AGENTS.md rewrite | ✅ Done |
| 8 | Build verification | ✅ Done (gccMultiStdenv + glibc-multi + _GNU_SOURCE) |
| 9 | CI integration | 🚧 Implemented (native-sim job added, pending CI verification) |
| 10 | Hardware UF2 bootloader | ⏸️ Paused |

## Completed Steps (0-3, 6-7)

### Step 0: Toolchain Note

- **native_sim**: Uses the Zephyr SDK host toolchain. Compiles to **32-bit x86**
  (`-m32`). The Zephyr SDK at `~/ncs/toolchains/911f4c5c26/opt/zephyr-sdk/`
  provides cmake and dtc, but the actual GCC is the host system's GCC.
  On NixOS this requires 32-bit multilib glibc (see Step 8 blocker).
- **xiao_ble**: Uses the Nordic ARM toolchain via nrfutil:
  ```bash
  nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
    west build -b xiao_ble firmware/ --pristine
  ```

All required drivers confirmed present in NCS v3.3.0:
- `zephyr/boards/native/native_sim/` — native_sim board
- `zephyr/drivers/serial/uart_native_pty.c` — PTY UART driver
- `zephyr/drivers/usb/device/usb_dc_native_posix.c` — virtual USB controller

### Step 1: Add native_sim board support ✅

**Divergence from plan:** The plan called for `firmware/boards/native_sim.overlay`
to define the PTY UART. This was **not needed** — the native_sim board DTS
(`native_sim.dts`) already declares `uart0` as `zephyr,native-pty-uart` with
`zephyr,console = &uart0`. Only the Kconfig fragment was required.

Created: `firmware/boards/native_sim.conf`
- `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=y` — dedicated PTY, path printed to stderr
- `CONFIG_CONSOLE=y` — satisfy board's `select POSIX_ARCH_CONSOLE`
- `CONFIG_UART_CONSOLE=n` — prevent Zephyr shell from stealing the PTY channel

**Kconfig conflict resolved:** The native_sim board unconditionally `select`s
`POSIX_ARCH_CONSOLE` which depends on `CONSOLE=y`. Setting `CONSOLE=n` in
`prj.conf` created an unfixable select warning. Solution: override `CONSOLE=y`
in `native_sim.conf` while keeping `UART_CONSOLE=n`. `POSIX_ARCH_CONSOLE` only
routes `printk` to host stdout — it does not touch the PTY.

### Step 2: Make uart_drv.c device-agnostic ✅

Changed `uart_drv.c` from:
```c
drv->dev = DEVICE_DT_GET(DT_NODELABEL(uart0));
```
to:
```c
drv->dev = DEVICE_DT_GET(DT_CHOSEN(zephyr_console));
```

`DT_CHOSEN(zephyr_console)` is set by every Zephyr board DTS:
- xiao_ble → `&uart0` (nrf-uarte)
- native_sim → `&uart0` (native-pty-uart)

No other code changes needed — the Zephyr UART API (`uart_irq_callback_set`,
`uart_poll_out`, etc.) is driver-agnostic.

### Step 3: Add USB CDC-ACM support ✅

**Major divergence from plan:** The device-next USB stack (`CONFIG_USB_DEVICE_STACK_NEXT`)
has **no UDC driver for native_sim**. Only legacy `USB_NATIVE_POSIX` exists.
Solution: dual-stack `usb_cdc.c`:
- `#ifdef CONFIG_USB_DEVICE_STACK_NEXT` — device-next API (xiao_ble, has `udc_nrf`)
- `#elif defined(CONFIG_USB_DEVICE_STACK)` — legacy API (native_sim, has `USB_NATIVE_POSIX`)

Legacy stack uses:
- `cdc_acm_dte_rate_callback_set()` for baud rate changes (polling-based)
- `k_work_delayable` polling every 50ms for DTR state via `uart_line_ctrl_get()`
- `CDC_ACM_DTE_RATE_CALLBACK_SUPPORT=y` Kconfig

Created `firmware/src/usb_cdc.c` and `firmware/src/usb_cdc.h`.

Key behaviors:
- `usb_cdc_init()` returns `-ENODEV` when no USB stack is configured
- Device-next: event-driven DTR/baud via `usbd_msg_register_cb()`
- Legacy: polling-driven DTR/baud via timer work item
- DTR falling edge at 1200 baud → `do_bootloader_entry()`
  - native_sim: `sim_gpregret = 0x57; exit(42)`
  - xiao_ble: direct register writes to GPREGRET (0x4000051C) and AIRCR (0xE000ED0C)

Created board files for USB opt-in:
- `firmware/boards/native_sim_usb.conf` — legacy USB stack + CDC-ACM + rate callback
- `firmware/boards/native_sim_usb.overlay` — `cdc_acm_0` node on `zephyr_udc0`
- `firmware/boards/xiao_ble_usb.conf` — device-next USB + CDC-ACM (nrf-usbd auto-selected)
- `firmware/boards/xiao_ble_usb.overlay` — override `zephyr,console` to `&uart0`

### Step 6: Update firmware/pm_static.yml ✅

Updated with comments noting native_sim ignores it. App address confirmed at
`0x27000` for xiao_ble.

### Step 7: Update firmware/AGENTS.md ✅

Rewritten for multi-target with: build commands for both targets, USB opt-in
flow, 1200-baud touch sequence, command reference, known pitfalls, recovery
checklist.

---

## Remaining Work

### Step 8: Fix native_sim build on NixOS ✅ RESOLVED

native_sim compiles as a 32-bit x86 executable (`-m32`). Two issues resolved:

1. **32-bit glibc headers missing**: The `nix develop` flake now includes
   `pkgs.gccMultiStdenv.cc` (multilib GCC) and the `CC`/`CXX` env vars point
   to it. The Nix wrapper automatically links against `glibc-multi` which
   provides `gnu/stubs-32.h` and the 32-bit dynamic linker
   (`lib/32/ld-linux.so.2`). Verified: `glibc-multi-2.42-61-dev` is used.

2. **`strtok_r` implicit declaration**: The native_sim build uses `-std=c17`
   which does not expose POSIX extensions like `strtok_r`. Added
   `target_compile_definitions(app PRIVATE _GNU_SOURCE)` to
   `firmware/CMakeLists.txt`.

Build command (inside `nix develop`):
```bash
west build -b native_sim firmware/
```

Build output is at `build/firmware/zephyr/zephyr.exe` (32-bit ELF).

Verify:
```bash
./build/firmware/zephyr/zephyr.exe
# Prints: uart connected to pseudotty: /dev/pts/N
# Connect: screen /dev/pts/N
# Type: ping<Enter> → pong
```

Helper scripts:
```bash
fw-build-native   # build
fw-run-native     # run (reads build/firmware/zephyr/zephyr.exe)
```

### Step 4: Write `tests/native_sim_validation.rs` (Tier 1) ✅ DONE

Self-contained integration test exercises the native_sim firmware through its
PTY. No hardware needed.

**Divergence from plan:**
- PTY path is printed to **stdout** (not stderr) in format `uart connected to pseudotty: /dev/pts/N`
- Uses `tokio::process::Command` for async spawn + `BufReader::lines()` to parse PTY path
- Spawns a background drain task for stdout to prevent pipe buffer from filling
- Each test spawns its own `zephyr.exe` + `TestServer`, supporting `--test-threads=N`
- Uses `kill_on_drop(true)` + `start_kill()` for cleanup
- Uses existing `TestServer` / `connect_client` from `tests/common/mod.rs` (same pattern as xiao_ble_validation)
- 11 test cases ported from xiao_ble_validation:
  1. `native_ping_roundtrip`
  2. `native_pending_read_then_write_ping_roundtrip`
  3. `native_split_writes_preserve_command_order`
  4. `native_framing_reports_single_split_command`
  5. `native_trace_reports_exact_split_byte_sequence`
  6. `native_read_match_on_spam_complete`
  7. `native_subscribe_match_stops_on_spam_complete`
  8. `native_subscribe_silence_timeout_after_spam`
  9. `native_read_buffer_budget_stops_under_flood`
  10. `native_subscribe_timeout_stops_under_flood`
  11. `native_close_while_subscribe_active`

**Running:**
```bash
cargo test --test native_sim_validation -- --ignored --test-threads=4
# All 11 tests pass in ~1.2s on native_sim
```

**Binary path:** Defaults to `build/firmware/zephyr/zephyr.exe` (repo-relative).
Override with `SERIAL_MCP_NATIVE_SIM_BIN` env var.

### Step 5: Write `tests/bootloader_touch_emulated.rs` (Tier 2) ✅ Tested

Tests the full 1200-baud touch → bootloader entry flow via USB/IP on native_sim.
Test passed on NixOS using `usbip-native-sim-attach` wrappers with NOPASSWD sudoers.

**Prerequisites (must be met before running):**
- USB firmware built: `fw-build-native-usb` (or manual `west build -b native_sim firmware/ -- -DEXTRA_CONF_FILE=boards/native_sim_usb.conf -DEXTRA_DTC_OVERLAY_FILE=boards/native_sim_usb.overlay`)
- Kernel modules: `vhci_hcd` loaded (`sudo -n usbip-native-sim-load-vhci`)
- Privileged USB/IP access — one of:
  - **Path A (NixOS, preferred):** NOPASSWD sudoers entry for `usbip-native-sim-attach` and `usbip-native-sim-detach`. The test auto-detects these wrappers and runs via `sudo -n`.
  - **Path B (any distro, one-time):** Udev rule to make vhci_hcd sysfs group-writable: `SUBSYSTEM=="platform", DRIVER=="vhci_hcd", GROUP="usbip", MODE="0660"`. Then the test falls back to raw `usbip` commands.

**Test: `bootloader_touch_via_usbip_exits_with_42`:**
1. Record existing `/dev/ttyACM*` devices
2. Spawn zephyr.exe (no special privileges needed — TCP socket)
3. Wait for USB/IP export on port 3241
4. `sudo -n usbip-native-sim-attach <busid>` (or raw `usbip attach` with udev rule)
5. Find the newly-appeared `/dev/ttyACMx` device
6. serial-mcp open at 1200 baud
7. `set_dtr_rts(dtr=true)`, then `set_dtr_rts(dtr=false)`
8. Verify zephyr.exe exits with code 42
9. `sudo -n usbip-native-sim-detach <port>` (or raw `usbip detach`), cleanup

**Status:** Test compiles and passes (8.6s on NixOS with NOPASSWD sudoers).

**Running:**
```bash
# NixOS with usbip-native-sim wrappers (NOPASSWD sudoers):
cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1

# Override wrapper paths:
USBIP_NATIVE_SIM_ATTACH_CMD=/path/to/usbip-native-sim-attach \
USBIP_NATIVE_SIM_DETACH_CMD=/path/to/usbip-native-sim-detach \
  cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1

# Override firmware binary:
SERIAL_MCP_NATIVE_SIM_USB_BIN=/path/to/zephyr.exe \
  cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1
```

### Step 9: CI integration 🚧 Implemented

Added `native-sim` job to `.github/workflows/ci.yml` for Tier 1 testing.

**Implementation:**
- New job `native-sim` runs on `ubuntu-latest` in parallel with other CI jobs
- Installs `nrfutil` via pip and uses `nrfutil sdk-manager install --ncs-version v3.3.0` for NCS SDK + toolchain
- Caches `~/ncs/` and `~/.nrfutil/` via `actions/cache@v4` — first run downloads ~3GB, subsequent runs hit cache
- Builds native_sim firmware with `west build -b native_sim firmware/`
- Runs `cargo test --test native_sim_validation -- --ignored`

**Tier 1 (no USB, fast):**
- ✅ Implemented as `native-sim` CI job
- Builds native_sim firmware (`west build -b native_sim firmware/`)
- Runs `cargo test --test native_sim_validation -- --ignored`
- Caches firmware build via NCS SDK cache

**Tier 2 (with USB/IP):**
- Requires `vhci_hcd` kernel module on runner
- Requires NOPASSWD sudoers for `usbip-native-sim-attach`/`usbip-native-sim-detach`, or udev rule for rootless vhci_hcd
- Build USB firmware variant (`fw-build-native-usb`)
- Run `cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1`
- Passes locally (8.6s on NixOS). CI needs privileged runner or container.

**Tier 3-4 (hardware):**
- Requires XIAO BLE + PicoProbe connected to runner
- Existing xiao_ble_validation + future bootloader_touch_hardware

### Step 10: Hardware UF2 bootloader ⏸️ PAUSED

See `firmware/UF2_BOOTLOADER_PLAN.md` for full plan. Blockers:
1. **SWD connection dead** — `pyocd` times out on all SWD connect attempts.
   PicoProbe enumerates on USB but SWD link to XIAO is broken. Needs physical
   debug (reset button, power cycle, RESET pin).
2. **XIAO native USB not connected** — `lsusb` only shows PicoProbe, no Seeed
   or Adafruit device. XIAO USB-C must be plugged directly into host.

Recovery steps before resuming:
1. Connect XIAO native USB-C to host
2. Recover SWD (reset button + power cycle)
3. Confirm `pyocd commander -t nrf52840 -c "show"` succeeds
4. User must explicitly approve `pyocd erase --chip` (destroys current firmware)

After hardware is ready:
- Flash bootloader hex at 0x0
- Build xiao_ble firmware with USB CDC-ACM
- Flash app at 0x27000 via pyocd
- Test 1200-baud touch over native USB → UF2 mass storage appears
- Write `tests/bootloader_touch_hardware.rs`

## Decision Points

All resolved:

- **USB CDC-ACM overlay**: Separate `native_sim_usb.overlay` and
  `xiao_ble_usb.overlay` files. Only applied with
  `-DEXTRA_DTC_OVERLAY_FILE=...`. Avoids build errors when USB is disabled.

- **GPREGRET command**: Deferred. Add a `gpregret` diagnostic command to the
  firmware that prints the current GPREGRET value. On native_sim this reads
  `sim_gpregret`. On xiao_ble this reads `NRF_POWER->GPREGRET`. Useful for
  verifying the touch handler wrote `0x57`. Not yet implemented.

- **Test port for native_sim**: Each test spawns its own `zephyr.exe` instance
  with its own PTY. No shared state. Allows `--test-threads=N` parallel
  execution. PTY path is parsed from `zephyr.exe` stdout.

- **USB/IP network**: native_sim uses offloaded sockets
  (`CONFIG_NET_NATIVE_OFFLOADED_SOCKETS`). No TAP setup needed. The USB/IP
  server binds to `INADDR_ANY`, client connects to `127.0.0.1`.

- **Build for native_sim**: Compiled with host GCC targeting 32-bit x86
  (`-m32`). Uses the Zephyr SDK for cmake/dtc, but the actual compiler is
  the system GCC. On NixOS this requires 32-bit multilib glibc. No
  `nrfutil sdk-manager` wrapper needed.

- **pm_static.yml**: Kept in `firmware/pm_static.yml`. Only affects
  `-b xiao_ble`. native_sim ignores it (POSIX arch, no flash layout).
