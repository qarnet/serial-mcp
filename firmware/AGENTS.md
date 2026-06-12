# serial-mcp Test Firmware

NCS/Zephyr firmware for the `native_sim` POSIX emulator. Used by
`tests/native_sim_validation.rs`, `tests/native_sim_connection_lifecycle.rs`,
and `tests/bootloader_touch_emulated.rs`.

## First Truths

- **Single build target:** `native_sim` — runs as a Linux process,
  PTY-backed UART. No real hardware required.
- **Two transports, opt-in via separate conf fragments:**
  - **Always-on:** command channel on the PTY-backed `uart0`
  - **Opt-in (USB conf):** native USB CDC-ACM over USB/IP for
    1200-baud touch testing
- The command channel uses `DT_CHOSEN(zephyr_console)`, which
  resolves to `&uart0` on `native_sim`. Device-agnostic.
- The CDC-ACM port (when enabled) is the entry point for the
  1200-baud touch → `exit(42)` flow validated by
  `tests/bootloader_touch_emulated.rs`.

## File Tree

```
firmware/
├── CMakeLists.txt              # Zephyr build entry
├── prj.conf                    # SHARED Kconfig
│
├── src/                        # firmware source
│   ├── main.c                  # super loop, command dispatch
│   ├── uart_drv.c              # Zephyr UART API
│   ├── uart_drv.h
│   ├── command.c               # ping, spam, trace, framing, etc.
│   ├── command.h
│   ├── usb_cdc.c               # USB CDC init + 1200-baud touch
│   └── usb_cdc.h
│
└── boards/
    ├── native_sim.conf         # PTY UART Kconfig (always applied)
    ├── native_sim_usb.conf     # OPT-IN: USB legacy stack + CDC-ACM
    └── native_sim_usb.overlay  # OPT-IN: CDC-ACM node
```

## Build

### native_sim (no USB — Tier 1 test)

```bash
# Plain variant uses a dedicated build tree.
west build -b native_sim firmware/ -d build/native_sim --pristine
# Run: ./build/native_sim/firmware/zephyr/zephyr.exe
# Or:  west build -d build/native_sim -t run
# Connect to the PTY printed on stdout, e.g. /dev/pts/5
```

Inside `nix develop`, helper also available:

```bash
fw-build-native
fw-run-native
```

`fw-build-native` also emits LSP metadata at:

```text
build/native_sim/firmware/compile_commands.json
```

### native_sim (with USB — Tier 2 test, emulated 1200-baud touch)

```bash
# USB variant uses its own dedicated build tree so it cannot
# contaminate the plain variant's Kconfig/devicetree state.
west build -b native_sim firmware/ -d build/native_sim_usb --pristine -- \
  -DEXTRA_CONF_FILE=boards/native_sim_usb.conf \
  -DEXTRA_DTC_OVERLAY_FILE=boards/native_sim_usb.overlay

# One-time host prep:
sudo -n usbip-native-sim-load-vhci

# Repo-local helper:
fw-run-native-usb-attached

# Manual fallback:
./build/native_sim_usb/firmware/zephyr/zephyr.exe
usbip --tcp-port 3241 list -r 127.0.0.1
usbip --tcp-port 3241 attach -r 127.0.0.1 -b 1-1
# /dev/ttyACM1 now appears
```

Inside `nix develop`, USB helper also available:

```bash
fw-build-native-usb
fw-run-native-usb-attached
```

`fw-build-native-usb` also emits LSP metadata at:

```text
build/native_sim_usb/firmware/compile_commands.json
```

`fw-run-native-usb-attached` starts `zephyr.exe`, waits for local USB/IP export on
`127.0.0.1:3241`, attaches it, prints `/dev/ttyACMx`, then detaches on exit.

## LSP / clangd

- Project LSP routing lives in `firmware/.clangd`.
- Default route: plain firmware files use `../build/native_sim/firmware/compile_commands.json`.
- USB route: `src/usb_*.c` and `src/usb_*.h` use `../build/native_sim_usb/firmware/compile_commands.json`.
- `.clangd` also strips clangd-hostile GCC flags: `-fno-reorder-functions` and `-fno-freestanding`.
- Build both variants at least once after clone or clean:

```bash
fw-build-native
fw-build-native-usb
```

- opencode launches `clangd` through `direnv exec .` so Nix toolchain paths resolve. If Zephyr headers go missing in LSP, first check that both compile DB files exist and are fresh.

## Do Not Drift

- Do **not** use `DT_NODELABEL(uart0)` directly. Use
  `DT_CHOSEN(zephyr_console)`. The driver is `zephyr,native-pty-uart`.
- Do **not** re-enable `CONFIG_CONSOLE` or `CONFIG_UART_CONSOLE`.
  They steal bytes from the command channel.
- Do **not** add a second `USBD_DEVICE_DEFINE()` instance. The single
  one in `usb_cdc.c` registers all CDC-ACM classes from devicetree.
- Do **not** put `#include "usb_cdc.h"` calls behind Kconfig in app
  code. The header's stub `usb_cdc_init()` is always available.
- Do **not** reintroduce a `xiao_ble` target. The test firmware is
  `native_sim` only.

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
# USB disabled by default — enable via boards/native_sim_usb.conf
# CONFIG_USB_DEVICE_STACK is not set
# CONFIG_USB_CDC_ACM is not set
```

### `boards/native_sim.conf`

```ini
# PTY UART is auto-enabled by DT_HAS_ZEPHYR_NATIVE_PTY_UART_ENABLED.
# This fragment tunes the PTY mode for tests:
CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=y
```

### `boards/native_sim_usb.conf` (opt-in)

```ini
CONFIG_USB_DEVICE_STACK=y
CONFIG_USB_CDC_ACM=y
CONFIG_CDC_ACM_SERIAL_INITIALIZE_AT_BOOT=n
# Quiet logs:
CONFIG_USB_DRIVER_LOG_LEVEL_ERR=y
CONFIG_USB_DEVICE_LOG_LEVEL_ERR=y
CONFIG_USB_CDC_ACM_LOG_LEVEL_DEFAULT=y
CONFIG_USB_CDC_ACM_LOG_LEVEL=3
```

## Architecture

```text
src/
  main.c          super loop, command dispatch, calls usb_cdc_init()
  uart_drv.c/h    DT_CHOSEN(zephyr_console), IRQ RX + ringbuf TX
  command.c/h     all commands, spam timer, app state
  usb_cdc.c/h     USB CDC-ACM init + 1200-baud touch handler
```

Runtime paths:

- **PTY uart0** — test commands, spam, trace, framing
- **USB CDC-ACM** (when `boards/native_sim_usb.conf` is applied) —
  1200-baud touch → `exit(42)` so the test process can verify the
  magic exit code.

## Actual Device Paths

- Command UART: `DEVICE_DT_GET(DT_CHOSEN(zephyr_console))`, IRQ
  callback via `uart_irq_*`
- USB CDC: `usbd_msg_register_cb()` watches
  `USBD_MSG_CDC_ACM_CONTROL_LINE_STATE` and
  `USBD_MSG_CDC_ACM_LINE_CODING`. Reads DTR/baud via
  `uart_line_ctrl_get()`.
- Commands terminate on `\r` or `\n`

## 1200-Baud Touch → `exit(42)` Flow

1. Host opens USB CDC port at **1200 baud** via USB/IP
2. Host asserts DTR (high)
3. Host de-asserts DTR (low) — the "touch"
4. Firmware's `dtr_poll_fn` detects DTR-falling at 1200 baud
5. Firmware writes `0x57` to `sim_gpregret` and calls `exit(42)`
6. Test process observes exit code 42 → `bootloader_touch_emulated`
   test passes

## Command Reference

### Core commands

| Command | Response | Notes |
|---------|----------|-------|
| `ping` | `pong\r\n` | health check |
| `info` | `board=native_sim build=0.1.0 <date> <time>\r\n` | static version + compile time |
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
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

11 + 6 software-only tests, no hardware required. `--test-threads=1` is
required for the lifecycle suite because the firmware process is killed
on `Drop` and parallel close can race with the OS layer.

### Tier 2: native_sim USB CDC-ACM via USB/IP (software, needs kernel modules + sudo)

```bash
cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1
```

Tests the 1200-baud touch → `exit(42)` flow. Passes in ~8.5s.

**Privilege setup** — one of:
- **Path A (NixOS):** NOPASSWD sudoers for `usbip-native-sim-attach` / `usbip-native-sim-detach`. Test auto-detects.
- **Path B (any distro):** Udev rule for rootless vhci_hcd: `SUBSYSTEM=="platform", DRIVER=="vhci_hcd", GROUP="usbip", MODE="0660"`.

**Env overrides:**
- `SERIAL_MCP_NATIVE_SIM_USB_BIN` — path to USB-enabled zephyr.exe
- `USBIP_NATIVE_SIM_ATTACH_CMD` / `USBIP_NATIVE_SIM_DETACH_CMD` — wrapper paths

## Important Implementation Notes

- `spam` uses `k_timer`
- PRNG is deterministic `xorshift32`
- TX ring buffer sized large enough to carry spam completion message
  after payload flood
- `spam stop` clears pending TX ring contents before printing stop
  line; this prevents stale flood bytes from leaking into next test
- `rxbuf` snapshots `cmd_buf` under `irq_lock()`
- `trace on` intentionally noisy; response interleaving is normal there
- USB CDC `dtr_poll_fn` reads DTR/baud atomically under `irq_lock`
- 1200-baud touch writes `sim_gpregret = 0x57` and calls `exit(42)`

## Known Pitfalls

### Symptom: build succeeds but firmware silent on stdout

Likely cause: `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=n` (the message
goes to stdio instead). Set it in `boards/native_sim.conf`.

### Symptom: native_sim PTY does not appear on stdout

Likely cause: `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=n` (the message
goes to stdio instead). Set it in `boards/native_sim.conf`.

### Symptom: native_sim USB/IP attach fails

Likely cause: built wrong target, `vhci_hcd` kernel module not loaded,
or attach command missing `--tcp-port 3241`. Rebuild with
`fw-build-native-usb`, run `sudo -n usbip-native-sim-load-vhci`, then use
`fw-run-native-usb-attached`.

## Minimal Recovery Checklist

When agents get lost, do this exact sequence:

1. Confirm `prj.conf` has `CONFIG_CONSOLE=n` and `CONFIG_UART_CONSOLE=n`
2. Confirm `boards/native_sim.conf` has `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=y`
3. Build: `west build -b native_sim firmware/`
4. Test `ping` over the printed PTY
5. Test 1200-baud touch over USB CDC (if `fw-build-native-usb` was used)
6. Run `cargo test --test native_sim_validation -- --ignored`
7. Run `cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1`
8. Run `cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1`
