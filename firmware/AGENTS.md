# serial-mcp Test Firmware

NCS/Zephyr firmware for XIAO BLE nRF52840 and the `native_sim` POSIX
emulator. Used by `tests/xiao_ble_validation.rs` (hardware) and
`tests/native_sim_validation.rs` (software).

## First Truths

- **Two build targets share one source tree:**
  - `native_sim` — runs as a Linux process, PTY-backed UART
  - `xiao_ble`   — nRF52840 hardware, real UART + native USB
- **Two transports, opt-in via separate conf fragments:**
  - **Always-on:** command channel on the physical/PTY `uart0`
  - **Opt-in (USB conf):** native USB CDC-ACM for 1200-baud touch
- The command channel uses `DT_CHOSEN(zephyr_console)`, which is
  `&uart0` on both targets. Device-agnostic.
- xiao_ble's CDC-ACM port is the entry point for the 1200-baud
  touch → UF2 bootloader sequence.
- The Adafruit UF2 bootloader (+ SoftDevice s140) lives at the top
  of flash on xiao_ble. **Application links at `0x27000`**.
- Flashing strategy for xiao_ble:
  - Bootloader: full-chip erase + pyocd flash of the hex
  - Application: pyocd flash of `zephyr.hex` at `0x27000`, or
    drag-drop `.uf2`

## File Tree

```
firmware/
├── CMakeLists.txt              # Zephyr build entry
├── prj.conf                    # SHARED Kconfig (both targets)
├── pm_static.yml               # xiao_ble flash layout (ignored by native_sim)
│
├── src/                        # SHARED source (both targets)
│   ├── main.c                  # super loop, command dispatch
│   ├── uart_drv.c              # Zephyr UART API (device-agnostic)
│   ├── uart_drv.h
│   ├── command.c               # ping, spam, trace, framing, etc.
│   ├── command.h
│   ├── usb_cdc.c               # USB CDC init + 1200-baud touch (guarded)
│   └── usb_cdc.h
│
└── boards/
    ├── native_sim.conf         # PTY UART Kconfig (always applied)
    ├── native_sim_usb.conf     # OPT-IN: USB device-next + CDC-ACM
    ├── native_sim_usb.overlay  # OPT-IN: CDC-ACM node
    ├── xiao_ble.conf           # UF2 output, flash partitions
    ├── xiao_ble_usb.conf       # OPT-IN: USB device-next + CDC-ACM
    └── xiao_ble_usb.overlay    # OPT-IN: force console back to uart0
```

## Build

### native_sim (no USB — Tier 1 test)

```bash
west build -b native_sim firmware/
# Run: ./build/zephyr/zephyr.exe
# Or:  west build -t run
# Connect to the PTY printed on stdout, e.g. /dev/pts/5
```

Inside `nix develop`, helper also available:

```bash
fw-build-native
fw-run-native
```

### native_sim (with USB — Tier 2 test, emulated 1200-baud touch)

```bash
west build -b native_sim firmware/ --pristine -- \
  -DEXTRA_CONF_FILE=boards/native_sim_usb.conf \
  -DEXTRA_DTC_OVERLAY_FILE=boards/native_sim_usb.overlay

# Run with USB/IP:
sudo ./build/zephyr/zephyr.exe
# In another terminal:
sudo modprobe vhci_hcd usbip-core usbip-host
sudo usbip attach -r 127.0.0.1 -b 1-1
# /dev/ttyACM1 now appears
```

Inside `nix develop`, USB helper also available:

```bash
fw-build-native-usb
```

### xiao_ble (no USB — Tier 3 test, PicoProbe-bridged)

```bash
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble firmware/ --pristine
```

Inside `nix develop`, shell auto-loads NCS toolchain env. No wrapper needed:

```bash
fw-build-xiao
# or: west build -b xiao_ble firmware/ --pristine
```

### xiao_ble (with USB — Tier 4 test, real 1200-baud touch)

```bash
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble firmware/ --pristine -- \
    -DEXTRA_CONF_FILE=boards/xiao_ble_usb.conf \
    -DEXTRA_DTC_OVERLAY_FILE=boards/xiao_ble_usb.overlay
```

Inside `nix develop`, USB helper also available:

```bash
fw-build-xiao-usb
```

Expected post-build checks (xiao_ble):

- `~/ncs/v3.3.0/nrf/build/firmware/zephyr/linker.cmd` contains
  `FLASH (rx) : ORIGIN = 0x27000`
- `~/ncs/v3.3.0/nrf/build/firmware/zephyr/include/generated/pm_config.h`
  contains `PM_APP_ADDRESS 0x27000`

If build lands at `0x0`, inspect `pm_static.yml` first.

## Flash (xiao_ble only)

### Bootloader (one-time / recovery)

```bash
pyocd erase -t nrf52840 --chip
pyocd flash -t nrf52840 Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.hex
```

### Application

```bash
pyocd flash -t nrf52840 --base-address 0x27000 \
  ~/ncs/v3.3.0/nrf/build/firmware/zephyr/zephyr.hex
```

Inside `nix develop`, helper also available:

```bash
fw-flash-xiao
```

Or drag-drop the `.uf2` after entering UF2 mode via 1200-baud touch.

## Do Not Drift

- Do **not** use `DT_NODELABEL(uart0)` directly. Use
  `DT_CHOSEN(zephyr_console)`. Both targets route this to `uart0` but
  the device driver differs.
- Do **not** link the app at `0x0` on xiao_ble (overwrites bootloader).
- Do **not** use `west flash` — use `pyocd` for both bootloader and app.
- Do **not** re-enable `CONFIG_CONSOLE` or `CONFIG_UART_CONSOLE`.
  They steal bytes from the command channel.
- Do **not** remove `pm_static.yml` — without it the app lands at
  the wrong offset.
- Do **not** add a second `USBD_DEVICE_DEFINE()` instance. The single
  one in `usb_cdc.c` registers all CDC-ACM classes from devicetree.
- Do **not** put `#include "usb_cdc.h"` calls behind Kconfig in app
  code. The header's stub `usb_cdc_init()` is always available.

## Config Files That Matter

### `prj.conf` (shared)

```ini
CONFIG_SERIAL=y
CONFIG_UART_INTERRUPT_DRIVEN=y
CONFIG_UART_LINE_CTRL=y
CONFIG_RING_BUFFER=y
CONFIG_HWINFO=y
CONFIG_LOG=y
CONFIG_CONSOLE=n
CONFIG_UART_CONSOLE=n
# USB disabled by default — enable per-target via boards/<board>_usb.conf
# CONFIG_USB_DEVICE_STACK_NEXT is not set
# CONFIG_USBD_CDC_ACM_CLASS is not set
```

### `boards/native_sim.conf`

```ini
# PTY UART is auto-enabled by DT_HAS_ZEPHYR_NATIVE_PTY_UART_ENABLED.
# This fragment tunes the PTY mode for tests:
CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=y
```

### `boards/xiao_ble.conf`

```ini
CONFIG_BUILD_OUTPUT_UF2=y
CONFIG_USE_DT_CODE_PARTITION=y
CONFIG_BOOTLOADER_MCUBOOT=n
CONFIG_GPIO=y
```

### `boards/<board>_usb.conf` (opt-in)

```ini
CONFIG_USB_DEVICE_STACK_NEXT=y
CONFIG_USBD_CDC_ACM_CLASS=y
CONFIG_CDC_ACM_SERIAL_INITIALIZE_AT_BOOT=n
# Quiet logs:
CONFIG_USBD_LOG_LEVEL_ERR=y
CONFIG_UDC_DRIVER_LOG_LEVEL_ERR=y
CONFIG_USBD_CDC_ACM_LOG_LEVEL_OFF=y
```

### `pm_static.yml` (xiao_ble only)

```yml
app:
  address: 0x27000
  size: 0xC5000
```

## Architecture

```text
src/
  main.c          super loop, command dispatch, calls usb_cdc_init()
  uart_drv.c/h    DT_CHOSEN(zephyr_console), IRQ RX + ringbuf TX
  command.c/h     all commands, spam timer, app state
  usb_cdc.c/h     USB CDC-ACM init + 1200-baud touch handler
                  (guarded by CONFIG_USB_DEVICE_STACK_NEXT)
```

Runtime paths:

- **Physical/PTY uart0** — test commands, spam, trace, framing
- **Native USB CDC-ACM** (xiao_ble_usb / native_sim_usb) — 1200-baud
  touch → UF2 entry. xiao_ble: writes `NRF_POWER->GPREGRET = 0x57`
  then `NVIC_SystemReset()`. native_sim: writes `sim_gpregret = 0x57`
  then `exit(42)`.

## Actual Device Paths

- Command UART: `DEVICE_DT_GET(DT_CHOSEN(zephyr_console))`, IRQ
  callback via `uart_irq_*`
- USB CDC: `usbd_msg_register_cb()` watches
  `USBD_MSG_CDC_ACM_CONTROL_LINE_STATE` and
  `USBD_MSG_CDC_ACM_LINE_CODING`. Reads DTR/baud via
  `uart_line_ctrl_get()`.
- Commands terminate on `\r` or `\n`

## 1200-Baud Touch → UF2 Flow

1. Host opens native USB CDC port at **1200 baud**
2. Host asserts DTR (high)
3. Host de-asserts DTR (low) — the "touch"
4. Firmware's `usb_msg_cb` detects DTR-falling at 1200 baud
5. Firmware writes `0x57` to GPREGRET (or `sim_gpregret`) and resets
   (or exits with code 42 on native_sim)
6. Bootloader sees GPREGRET, enters UF2 mode
7. USB mass storage drive `XIAO-SENSE` / `XIAO BLE` appears
8. Drag-drop `.uf2` to flash, or `pyocd flash` to recover

## Command Reference

### Core commands

| Command | Response | Notes |
|---------|----------|-------|
| `ping` | `pong\r\n` | health check |
| `info` | `board=XIAO_BLE_nRF52840 build=0.1.0 <date> <time>\r\n` | static version + compile time |
| `spam <count> hex [last_data=".."] [delay=<ms>]` | `spam start count=N delay=N\r\n` then hex payload | 256-byte packet chunks |
| `spam stop` | `Spam stopped: N bytes sent\r\n` | also clears queued TX so later tests start clean |

### Completion strings tests depend on

- `Spam complete: N bytes sent\r\n`
- `Spam stopped: N bytes sent\r\n`

Hardware tests match on exact phrase `Spam complete`.

### Diagnostic commands

| Command | Response | Purpose |
|---------|----------|---------|
| `rxbuf status` | `rxbuf len=N data="<partial>"\r\n` | inspect partial line buffer |
| `rxbuf clear` | `rxbuf clear was_len=N\r\n` | drop partial line |
| `arm_cmd <delay_ms>` | `arm_cmd delay=N\r\n` | delay next command execution |
| `trace on` | `trace on\r\n` | emit `RX[n]=0xXX` per received byte |
| `trace off` | `trace off\r\n` | disable tracing |
| `framing on` | `framing on\r\n` | emit `LINE len=N data="..."` when line commits |
| `framing off` | `framing off\r\n` | disable framing messages |
| `slow on [<us>]` | `slow on delay=N\r\n` | sleep before command dispatch |
| `slow off` | `slow off\r\n` | disable slow mode |
| `write cmd <id> <rest>` | `ack N exec><rest>\r\n` then execute nested command | helps detect ordering/drop issues |
| `binary on` | `binary on\r\n` | mainly trace-focused mode |
| `binary off` | `binary off\r\n` | |

## Test Expectations

### Tier 1: native_sim PTY UART (software, fast CI)

```bash
cargo test --test native_sim_validation -- --ignored --test-threads=N
```

Each test spawns its own `zephyr.exe` with a fresh PTY. No shared state.
`--test-threads=N` is safe. The PTY path is parsed from stderr.

Not yet implemented — see `firmware/UNIFIED_FIRMWARE_PLAN.md` Step 4.

### Tier 2: native_sim USB CDC-ACM via USB/IP (software, needs kernel modules)

```bash
cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1
```

Tests the 1200-baud touch → exit(42) flow. Requires `sudo modprobe vhci_hcd`.

Not yet implemented — see `firmware/UNIFIED_FIRMWARE_PLAN.md` Step 5.

### Tier 3: XIAO BLE hardware + PicoProbe

```bash
SERIAL_MCP_TEST_PORT=/dev/ttyACM0 cargo test --test xiao_ble_validation -- --ignored --test-threads=1
```

`--test-threads=1` matters — parallel tests fight over the same serial port.

### Tier 4: XIAO BLE hardware + native USB CDC (paused)

See `firmware/UF2_BOOTLOADER_PLAN.md`. Requires SWD recovery and native USB-C
connection to host.

## Important Implementation Notes

- `spam` uses `k_timer`
- PRNG is deterministic `xorshift32`
- TX ring buffer sized large enough to carry spam completion message
  after payload flood
- `spam stop` clears pending TX ring contents before printing stop
  line; this prevents stale flood bytes from leaking into next test
- `rxbuf` snapshots `cmd_buf` under `irq_lock()`
- `trace on` intentionally noisy; response interleaving is normal there
- USB CDC `dtr_changed` callback uses `irq_lock` to read the last-seen
  baud rate atomically
- 1200-baud touch writes `0x57` to GPREGRET then resets
- native_sim bootloader entry uses `exit(42)` so tests can verify
  the magic code via process status

## Known Pitfalls

### Symptom: build succeeds but firmware silent on `/dev/ttyACM0` (xiao_ble)

Likely cause: code still using USB CDC path for commands instead of
`uart0`. USB CDC is for 1200-baud touch only.

### Symptom: linker shows `ORIGIN = 0x0` (xiao_ble)

Likely cause: `pm_static.yml` missing or `CONFIG_USE_DT_CODE_PARTITION=n`.
The app would overwrite the bootloader.

### Symptom: serial output shows `>` prompt or echoed commands

Likely cause: Zephyr console/shell got re-enabled. Check
`CONFIG_CONSOLE=n` in `prj.conf`.

### Symptom: 1200-baud touch does nothing (xiao_ble)

Likely cause: USB CDC not enumerated (check `dmesg` for the native
USB device), or the `dtr_changed` callback wasn't wired in.

### Symptom: native_sim PTY does not appear on stdout

Likely cause: `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=n` (the message
goes to stdio instead). Set it in `boards/native_sim.conf`.

### Symptom: native_sim USB/IP attach fails

Likely cause: `vhci_hcd` kernel module not loaded, or `zephyr.exe`
not running with `CAP_NET_ADMIN`. Run `sudo modprobe vhci_hcd
usbip-core usbip-host` and use `sudo ./zephyr.exe`.

## Minimal Recovery Checklist

When agents get lost, do this exact sequence:

1. Confirm `prj.conf` has `CONFIG_CONSOLE=n` and `CONFIG_UART_CONSOLE=n`
2. Confirm `boards/xiao_ble.conf` has `CONFIG_BUILD_OUTPUT_UF2=y` and
   `CONFIG_USE_DT_CODE_PARTITION=y`
3. Confirm `firmware/pm_static.yml` sets app address `0x27000`
4. Bootloader still intact? Double-tap reset → mass storage appears
5. Build: `west build -b native_sim firmware/` for native_sim,
   or use `nrfutil sdk-manager toolchain launch` for xiao_ble
6. Check linker origin is `0x27000` (xiao_ble)
7. Flash app with `pyocd flash -t nrf52840 --base-address 0x27000 ...`
8. Test `ping` over `/dev/ttyACM0` (xiao_ble) or the printed PTY
   (native_sim)
9. Test 1200-baud touch over native USB CDC
10. Run `cargo test --test xiao_ble_validation -- --ignored --test-threads=1`
