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

## Implementation Steps

### Step 0: Toolchain Note

- **native_sim**: Uses the **Zephyr SDK** host toolchain (GCC targeting x86_64).
  The `nrfutil sdk-manager` wrapper is NOT needed. The Zephyr SDK is included
  in the nix dev shell at `~/ncs/v3.3.0/zephyr-sdk-*/`.
- **xiao_ble**: Uses the **Nordic ARM toolchain**. Must use:
  ```bash
  nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
    west build -b xiao_ble firmware/ --pristine
  ```

All three required drivers exist in NCS v3.3.0:
- `zephyr/boards/native/native_sim/` — native_sim board
- `zephyr/drivers/serial/uart_native_pty.c` — PTY UART driver
- `zephyr/drivers/usb/device/usb_dc_native_posix.c` — virtual USB controller

### Step 1: Add native_sim board support

Create two files:

- `firmware/boards/native_sim.overlay` — PTY UART DTS overlay (see above)
- `firmware/boards/native_sim.conf` — PTY UART Kconfig (see above)

Verify:

```bash
west build -b native_sim firmware/
west build -t run
# Should print: UART connected to pseudotty: /dev/pts/N
# Connect: screen /dev/pts/N
# Type: ping<Enter>
# Expected: pong
```

If the build fails because `DT_NODELABEL(uart0)` doesn't exist: need to also
change `main.c` to use `DT_CHOSEN(zephyr_console)` (see Step 2).

### Step 2: Make uart_drv.c device-agnostic

**Current code** — hard-coded to `uart0`:
```c
#define UART_DEV DEVICE_DT_GET(DT_NODELABEL(uart0))
```

**New code** — board-agnostic:
```c
#define UART_DEV DEVICE_DT_GET(DT_CHOSEN(zephyr_console))
```

`DT_CHOSEN(zephyr_console)` is set by every Zephyr board's DTS. For `xiao_ble`
it points to `&uart0`. For `native_sim` it points to `&uart0` (which we
override to `zephyr,native-pty-uart` in the overlay).

**No other code changes needed** — the Zephyr UART API is driver-agnostic.
`uart_irq_callback_set`, `uart_poll_out`, etc. work identically on both
drivers.

### Step 3: Add USB CDC-ACM support (optional, guarded by #ifdef)

Create `firmware/src/usb_cdc.c` and `firmware/src/usb_cdc.h` with the exact
code shown in the [USB CDC-ACM section](#usb-cdc-acm--conditional-compilation)
above. Key points:

- `usb_cdc_init()` calls `usbd_init_device(usb_msg_cb)` then `usbd_enable()`
- `usb_msg_cb` watches `USBD_MSG_CDC_ACM_CONTROL_LINE_STATE` (DTR) and
  `USBD_MSG_CDC_ACM_LINE_CODING` (baud rate)
- When DTR drops while baud is 1200: `do_bootloader_entry()`
  - native_sim: `sim_gpregret = 0x57; exit(42)`
  - xiao_ble: `NRF_POWER->GPREGRET = 0x57; NVIC_SystemReset()`
- Wrapped in `#ifdef CONFIG_USB_DEVICE_STACK`; stubs return 0 when disabled
- Call `usb_cdc_init()` from `main.c` inside `#ifdef CONFIG_USB_DEVICE_STACK`

Create board files for USB variant:

- `firmware/boards/native_sim_usb.overlay` — `cdc_acm_uart0` node
- `firmware/boards/native_sim_usb.conf` — USB device stack + native posix
- `firmware/boards/xiao_ble_usb.overlay` — `cdc_acm_uart0` node
- `firmware/boards/xiao_ble_usb.conf` — USB device stack

### Step 4: Write native_sim test suite

Create `tests/native_sim_validation.rs` (Tier 1 test):

- Build `zephyr.exe` before test (or use a pre-built binary)
- Spawn `zephyr.exe` as child process
- Read stdout until PTY path appears: `UART connected to pseudotty: /dev/pts/N`
- Extract PTY path, open with serial-mcp `open` tool
- Run the same ping/spam/trace/framing commands as `xiao_ble_validation.rs`
- Kill `zephyr.exe` on test teardown
- No `SERIAL_MCP_TEST_PORT` env var needed — self-contained

### Step 5: Write 1200-baud touch test (emulated)

Create `tests/bootloader_touch_emulated.rs` (Tier 2 test):

**Prerequisites on test host:**
```bash
sudo modprobe vhci_hcd usbip-core usbip-host
```

**Test flow:**
1. Build USB CDC-ACM firmware: `west build -b native_sim firmware/ -- -DEXTRA_CONF_FILE=...`
2. Spawn `zephyr.exe` as child process (may need `sudo` for USB/IP)
3. Run `usbip attach -r 127.0.0.1 -b 1-1` to create `/dev/ttyACMx`
4. serial-mcp: `open` at 1200 baud
5. serial-mcp: `set_dtr_rts dtr=true rts=false`
6. serial-mcp: `set_dtr_rts dtr=false rts=false` ← triggers bootloader entry
7. Verify: `zephyr.exe` process exits with code 42
8. Cleanup: `usbip detach -p 0`

**Alternative (simpler, no USB/IP):** Instead of full USB/IP, test the
1200-baud logic by writing a command to the virtual UART like
`test_bootloader_touch 1200 0` (baud + DTR state). The firmware calls
`do_bootloader_entry()` directly. Less realistic but simpler CI setup.

### Step 6: Update firmware/pm_static.yml for xiao_ble

```yaml
app:
  address: 0x27000
  size: 0xD9000
```

This file is only relevant for `-b xiao_ble` builds. native_sim ignores it
(POSIX architecture has no flash partitions).

### Step 7: Update firmware/AGENTS.md

Rewrite to reflect:
- Multi-target reality (native_sim + xiao_ble)
- USB CDC-ACM is allowed (guarded by `#ifdef`)
- Link origin: 0x27000 for xiao_ble, N/A for native_sim
- Build commands for both targets
- Test commands for both tiers

## Decision Points

All resolved:

- **USB CDC-ACM overlay**: Separate `native_sim_usb.overlay` and
  `xiao_ble_usb.overlay` files. Only applied with
  `-DEXTRA_DTC_OVERLAY_FILE=...`. Avoids build errors when USB is disabled.

- **GPREGRET command**: Add a `gpregret` diagnostic command to the firmware
  that prints the current GPREGRET value. On native_sim this reads
  `sim_gpregret`. On xiao_ble this reads `NRF_POWER->GPREGRET`. Useful for
  verifying the touch handler wrote `0x57`.

- **Test port for native_sim**: Each test spawns its own `zephyr.exe` instance
  with its own PTY. No shared state. Allows `--test-threads=N` parallel
  execution. PTY path is parsed from `zephyr.exe` stdout.

- **USB/IP network**: native_sim uses offloaded sockets
  (`CONFIG_NET_NATIVE_OFFLOADED_SOCKETS`). No TAP setup needed. The USB/IP
  server binds to `INADDR_ANY`, client connects to `127.0.0.1`.

- **DPI build for native_sim**: Build uses the system Zephyr SDK toolchain.
  No `nrfutil sdk-manager` wrapper needed — native_sim is a standard
  Zephyr board compiled with the host GCC.

- **pm_static.yml**: Kept in `firmware/pm_static.yml`. Only affects
  `-b xiao_ble`. native_sim ignores it (POSIX arch, no flash layout).
