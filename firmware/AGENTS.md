# serial-mcp Test Firmware

NCS/Zephyr firmware for the `native_sim` POSIX emulator. Used by
`tests/native_sim_validation.rs` and `tests/native_sim_connection_lifecycle.rs`.

## First Truths

- **Single build target:** `native_sim` — runs as a Linux process,
  PTY-backed UART. No real hardware required.
- **Single unified firmware** — one `prj.conf`, one build tree
  (`build/native_sim`), no snippets, no variants. The `touch` command
  triggers `exit(42)` directly over the PTY command channel.
- The command channel uses `DT_CHOSEN(zephyr_console)`, which
  resolves to `&uart0` on `native_sim`. Device-agnostic.
- The `touch` command is the entry point for the bootloader entry →
  `exit(42)` flow validated by `tests/native_sim_connection_lifecycle.rs`.

## File Tree

```
firmware/
├── CMakeLists.txt              # Zephyr build entry
├── prj.conf                    # Kconfig for the native_sim test firmware
│
├── src/                        # firmware source
│   ├── main.c                  # super loop, command dispatch
│   ├── uart_drv.c              # Zephyr UART API
│   ├── uart_drv.h
│   ├── command.c               # ping, spam, trace, framing, touch, etc.
│   └── command.h
│
└── boards/
    └── native_sim.conf         # PTY UART tuning (always auto-loaded)
```

## Build

### native_sim

```bash
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

## LSP / clangd

- Project LSP routing lives in `firmware/.clangd`.
- All firmware files use `../build/native_sim/firmware/compile_commands.json`.
- `.clangd` also strips clangd-hostile GCC flags: `-fno-reorder-functions` and `-fno-freestanding`.
- Build at least once after clone or clean:

```bash
fw-build-native
```

- opencode launches `clangd` through `direnv exec .` so Nix toolchain paths resolve. If Zephyr headers go missing in LSP, check that the compile DB file exists and is fresh.

## Do Not Drift

- Do **not** use `DT_NODELABEL(uart0)` directly. Use
  `DT_CHOSEN(zephyr_console)`. The driver is `zephyr,native-pty-uart`.
- Do **not** set `CONFIG_UART_CONSOLE=y` — it steals bytes from the
  command channel. `CONFIG_CONSOLE=y` is required on native_sim
  (for `POSIX_ARCH_CONSOLE`) and is set in `boards/native_sim.conf`.
- Do **not** reintroduce a `xiao_ble` target. The test firmware is
  `native_sim` only.

## Config Files That Matter

### `prj.conf`

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

This file contains all Kconfig settings for the unified firmware.

### `boards/native_sim.conf`

```ini
CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=y
CONFIG_CONSOLE=y
CONFIG_UART_CONSOLE=n
```

Always auto-loaded for `native_sim` board. Sets `CONSOLE=y` to satisfy
`POSIX_ARCH_CONSOLE` select; `UART_CONSOLE=n` prevents Zephyr shell
from stealing command-channel bytes.

## Architecture

```text
src/
  main.c          super loop, command dispatch
  uart_drv.c/h    DT_CHOSEN(zephyr_console), IRQ RX + ringbuf TX
  command.c/h     all commands (including touch), spam timer, app state
```

Runtime paths:

- **PTY uart0** — test commands, spam, trace, framing. The `touch`
  command triggers `exit(42)` over this same channel so the test
  process can verify the magic exit code.

## Actual Device Paths

- Command UART: `DEVICE_DT_GET(DT_CHOSEN(zephyr_console))`, IRQ
  callback via `uart_irq_*`
- Commands terminate on `\r` or `\n`

## `touch` Command → `exit(42)` Flow

1. Host sends `touch` command over the PTY command channel
2. Firmware receives `touch`, writes `0x57` to `sim_gpregret` and calls `exit(42)`
3. Test process observes exit code 42 → bootloader entry test passes

## Command Reference

### Core commands

| Command | Response | Notes |
|---------|----------|-------|
| `ping` | `pong\r\n` | health check |
| `info` | `board=native_sim build=0.1.0 <date> <time>\r\n` | static version + compile time |
| `touch` | (no response; `exit(42)` immediately) | writes GPREGRET=0x57 and exits |
| `spam <count> hex [last_data=".."] [delay=<ms>]` | `spam start count=N delay=N\r\n` then hex payload | 256-byte packet chunks |
| `spam stop` | `Spam stopped: N bytes sent\r\n` | also clears queued TX so later tests start clean |

### Completion strings tests depend on

- `Spam complete: N bytes sent\r\n`
- `Spam stopped: N bytes sent\r\n`

Tests match on exact phrase `Spam complete`.

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
| `txbuf status` | `txbuf len=N busy=B\r\n` | TX ring buffer occupancy and busy flag |
| `ack on` | `ack on\r\n` | enable pre-execution ack (`ack <seq>\r\n` before command runs) |
| `ack off` | `ack off\r\n` | disable pre-execution ack |
| `hold on` | `hold on\r\n` | halt firmware TX drain (ring buffer fills) |
| `hold off` | `hold off\r\n` | resume firmware TX drain |

## Test Expectations

### native_sim PTY UART (software, fast CI)

```bash
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

22 + 6 software-only tests, no hardware required. `--test-threads=1` is
required for the lifecycle suite because the firmware process is killed
on `Drop` and parallel close can race with the OS layer.
The bootloader touch flow is exercised via the `touch` command in the
lifecycle suite — no separate test binary needed.

## Important Implementation Notes

- `spam` uses `k_timer`
- PRNG is deterministic `xorshift32`
- TX ring buffer sized large enough to carry spam completion message
  after payload flood
- `spam stop` clears pending TX ring contents before printing stop
  line; this prevents stale flood bytes from leaking into next test
- `rxbuf` snapshots `cmd_buf` under `irq_lock()`
- `trace on` intentionally noisy; response interleaving is normal there
- `touch` command writes `sim_gpregret = 0x57` and calls `exit(42)`

## Known Pitfalls

### Symptom: build succeeds but firmware silent on stdout

Likely cause: `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=n` (the message
goes to stdio instead). Set it in `boards/native_sim.conf`.

### Symptom: native_sim PTY does not appear on stdout

Likely cause: `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=n` (the message
goes to stdio instead). Set it in `boards/native_sim.conf`.

## Minimal Recovery Checklist

When agents get lost, do this exact sequence:

1. Confirm `prj.conf` has `CONFIG_CONSOLE=n` and `CONFIG_UART_CONSOLE=n`
2. Confirm `boards/native_sim.conf` has `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY=y`
3. Build: `west build -b native_sim firmware/`
4. Test `ping` over the printed PTY
5. Run `cargo test --test native_sim_validation -- --ignored`
6. Run `cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1`
