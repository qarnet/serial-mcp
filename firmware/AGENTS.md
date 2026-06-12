# serial-mcp Test Firmware

NCS/Zephyr firmware for the `native_sim` POSIX emulator. Used by
`tests/native_sim_validation.rs`, `tests/native_sim_connection_lifecycle.rs`,
and `tests/bootloader_touch_emulated.rs`.

## First Truths

- **Single build target:** `native_sim` — runs as a Linux process,
  PTY-backed UART. No real hardware required.
- **Two build variants selected via Zephyr snippets:**
  - **`-S plain`:** command channel only (no USB).
  - **`-S usb`:** command channel + USB CDC-ACM over USB/IP for
    1200-baud touch testing.
- **Without a snippet the build fails** — `main.c` contains a
  `#error` guard that fires when `CONFIG_SERIAL` is not set.
- The command channel uses `DT_CHOSEN(zephyr_console)`, which
  resolves to `&uart0` on `native_sim`. Device-agnostic.
- The CDC-ACM port (when enabled) is the entry point for the
  1200-baud touch → `exit(42)` flow validated by
  `tests/bootloader_touch_emulated.rs`.

## File Tree

```
firmware/
├── CMakeLists.txt              # Zephyr build entry
├── prj.conf                    # deliberately minimal (snippets provide real config)
│
├── config/
│   ├── plain.conf              # Kconfig for "plain" snippet (no USB)
│   └── usb.conf                # Kconfig for "usb" snippet (+ USB stack)
│
├── src/                        # firmware source
│   ├── main.c                  # super loop, command dispatch, snippet guard
│   ├── uart_drv.c              # Zephyr UART API
│   ├── uart_drv.h
│   ├── command.c               # ping, spam, trace, framing, etc.
│   ├── command.h
│   ├── usb_cdc.c               # USB CDC init + 1200-baud touch
│   └── usb_cdc.h
│
├── boards/
│   ├── native_sim.conf         # PTY UART tuning (always auto-loaded)
│   └── native_sim_usb.overlay  # CDC-ACM DT node (applied by usb snippet)
│
└── snippets/
    ├── plain/
    │   └── snippet.yml         # Snippet: plain Kconfig (no USB)
    └── usb/
        └── snippet.yml         # Snippet: USB Kconfig + CDC-ACM overlay
```

## Build

**Without a snippet the build fails** with a clear `#error` in `main.c`.
Always pass `-S plain` or `-S usb`.

### native_sim (no USB — Tier 1 test)

```bash
# -S plain is required. The "plain" snippet pulls in config/plain.conf.
west build -b native_sim firmware/ -d build/native_sim --pristine -S plain
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
# The "usb" snippet bundles both the USB Kconfig fragment and
# the CDC-ACM devicetree overlay. List available snippets with
# `west build -S help`.
west build -b native_sim firmware/ -d build/native_sim_usb --pristine -S usb

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
- Do **not** set `CONFIG_UART_CONSOLE=y` — it steals bytes from the
  command channel. `CONFIG_CONSOLE=y` is required on native_sim
  (for `POSIX_ARCH_CONSOLE`) and is set in `boards/native_sim.conf`.
- Do **not** add a second `USBD_DEVICE_DEFINE()` instance. The single
  one in `usb_cdc.c` registers all CDC-ACM classes from devicetree.
- Do **not** put `#include "usb_cdc.h"` calls behind Kconfig in app
  code. The header's stub `usb_cdc_init()` is always available.
- Do **not** reintroduce a `xiao_ble` target. The test firmware is
  `native_sim` only.

## Config Files That Matter

### `prj.conf` (stub)

```ini
# Build variants are selected via Zephyr snippets:
#
#   west build ... -S plain   # command-channel-only firmware
#   west build ... -S usb     # command channel + USB CDC-ACM
#
# Without a snippet, critical Kconfig symbols (CONFIG_SERIAL etc.)
# are missing and the build fails with a clear #error message.
```

This file deliberately contains no `CONFIG_` settings. All real
configuration lives in `config/plain.conf` and `config/usb.conf`,
each loaded by its snippet's `EXTRA_CONF_FILE`.

### `config/plain.conf` (applied by `-S plain` snippet)

```ini
CONFIG_SERIAL=y
CONFIG_UART_INTERRUPT_DRIVEN=y
CONFIG_UART_LINE_CTRL=y
CONFIG_RING_BUFFER=y
CONFIG_HWINFO=y
CONFIG_LOG=y
CONFIG_UART_CONSOLE=n
# CONSOLE is set by boards/native_sim.conf to y
# (needed for POSIX_ARCH_CONSOLE on native_sim).
```

### `config/usb.conf` (applied by `-S usb` snippet)

```ini
# Common settings (same as config/plain.conf) plus:
CONFIG_USB_DEVICE_STACK=y
CONFIG_USB_CDC_ACM=y
CONFIG_CDC_ACM_DTE_RATE_CALLBACK_SUPPORT=y
CONFIG_USB_DEVICE_INITIALIZE_AT_BOOT=n
CONFIG_USB_DRIVER_LOG_LEVEL_ERR=y
CONFIG_USB_DEVICE_LOG_LEVEL_ERR=y
```

### `boards/native_sim.conf`

```ini
CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=y
CONFIG_CONSOLE=y
CONFIG_UART_CONSOLE=n
```

Always auto-loaded for `native_sim` board. Sets `CONSOLE=y` to satisfy
`POSIX_ARCH_CONSOLE` select; `UART_CONSOLE=n` prevents Zephyr shell
from stealing command-channel bytes.

### `snippets/plain/snippet.yml`

```yaml
name: plain
append:
  EXTRA_CONF_FILE: ../../config/plain.conf
```

### `snippets/usb/snippet.yml`

```yaml
name: usb
append:
  EXTRA_CONF_FILE: ../../config/usb.conf
  EXTRA_DTC_OVERLAY_FILE: ../../boards/native_sim_usb.overlay
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
- **USB CDC-ACM** (when `-S usb` snippet is applied) —
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
