# serial-mcp Test Firmware

NCS/Zephyr firmware for XIAO BLE nRF52840 used by `tests/xiao_ble_validation.rs`.

## First Truths

- **Transport is physical `uart0`, not USB CDC-ACM.**
- Host test port `/dev/ttyACM0` is **PicoProbe USB serial**, not XIAO native USB.
- XIAO TX/RX pins talk through PicoProbe UART bridge at `115200 8N1`.
- **No bootloader flow here.** Flash directly with SWD.
- This firmware must link at **`0x0`**. If it links at `0x27000`, agents drifted into XIAO UF2 partition config again.

## Do Not Drift

- Do **not** add USB CDC-ACM.
- Do **not** use `zephyr_cdc_acm_uart`.
- Do **not** wait for DTR.
- Do **not** re-enable `CONFIG_CONSOLE` or `CONFIG_UART_CONSOLE`.
- Do **not** use `west flash` here.
- Do **not** fetch bootloaders, UF2 images, SoftDevice blobs, or external recovery files.
- Do **not** remove `pm_static.yml` unless you replace it with another verified `0x0` flash layout.

Those paths caused dead ends before:

- USB path made firmware silent on Pico UART.
- Board default partition config pushed app to `0x27000`.
- Full-chip erase plus bootloader assumptions wasted time.

## Build

Only use this command style:

```bash
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble /home/thomas-workstation/repos/serial-mcp/firmware --pristine
```

Incremental rebuild:

```bash
nrfutil sdk-manager toolchain launch --ncs-version v3.3.0 --chdir ~/ncs/v3.3.0/nrf -- \
  west build -b xiao_ble /home/thomas-workstation/repos/serial-mcp/firmware
```

Expected post-build checks:

- `~/ncs/v3.3.0/nrf/build/firmware/zephyr/linker.cmd` contains `FLASH (rx) : ORIGIN = 0x0`
- `~/ncs/v3.3.0/nrf/build/firmware/zephyr/include/generated/pm_config.h` contains:
  - `PM_APP_ADDRESS 0x0`
  - `PM_ADDRESS 0x0`

If build still lands at `0x27000`, inspect `pm_static.yml` first.

## Flash

Only use `pyocd`:

```bash
pyocd flash -t nrf52840 ~/ncs/v3.3.0/nrf/build/firmware/zephyr/zephyr.hex
```

This project is intentionally configured for direct flash-at-`0x0` image.

## Config Files That Matter

### `prj.conf`

These settings are deliberate:

```ini
CONFIG_SERIAL=y
CONFIG_UART_INTERRUPT_DRIVEN=y
CONFIG_UART_LINE_CTRL=y
CONFIG_RING_BUFFER=y
CONFIG_HWINFO=y
CONFIG_LOG=y
CONFIG_CONSOLE=n
CONFIG_UART_CONSOLE=n
CONFIG_BUILD_OUTPUT_UF2=n
CONFIG_USE_DT_CODE_PARTITION=n
```

Why:

- `CONFIG_CONSOLE=n` and `CONFIG_UART_CONSOLE=n` stop Zephyr shell/console from stealing bytes and echoing prompts.
- `CONFIG_BUILD_OUTPUT_UF2=n` and `CONFIG_USE_DT_CODE_PARTITION=n` stop XIAO UF2 layout from pushing app away from `0x0`.

### `pm_static.yml`

This file is critical:

```yml
app:
  address: 0x0
  size: 0x100000
```

Reason:

- XIAO board ships `zephyr/boards/seeed/xiao_ble/pm_static.yml` with app at `0x27000`.
- App-local `firmware/pm_static.yml` overrides board one.
- Without this override, build may look fine but image lands at wrong flash offset.

## Architecture

```text
src/
  main.c          super loop, command dispatch
  uart_drv.c/h    physical uart0 IRQ RX + ringbuf TX
  command.c/h     all commands, spam timer, app state
```

Runtime path:

1. `main.c` initializes `uart0`
2. IRQ callback accumulates bytes into partial line buffer
3. Completed line becomes command for super loop
4. `command_process()` executes command
5. TX uses ring buffer so spam and traces do not block command loop

## Actual Device Path

- Driver binds `DEVICE_DT_GET(DT_NODELABEL(uart0))`
- Not `DEVICE_DT_GET_ONE(zephyr_cdc_acm_uart)`
- UART callback uses `uart_irq_*`
- Commands terminate on `\r` or `\n`

If serial smoke test on `/dev/ttyACM0` returns nothing, first suspect wrong transport path, not flashing.

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

Primary hardware test command:

```bash
SERIAL_MCP_TEST_PORT=/dev/ttyACM0 cargo test --test xiao_ble_validation -- --ignored --test-threads=1
```

`--test-threads=1` matters. Parallel tests fight over same serial port.

Known good result after latest fix:

- `11 passed; 0 failed`

## Important Implementation Notes

- `spam` uses `k_timer`
- PRNG is deterministic `xorshift32`
- TX ring buffer sized large enough to carry spam completion message after payload flood
- `spam stop` clears pending TX ring contents before printing stop line; this prevents stale flood bytes from leaking into next test
- `rxbuf` snapshots `cmd_buf` under `irq_lock()`
- `trace on` intentionally noisy; response interleaving is normal there

## Known Pitfalls

### Symptom: build succeeds but firmware silent on `/dev/ttyACM0`

Likely cause: code still using USB CDC path instead of `uart0`.

### Symptom: linker shows `ORIGIN = 0x27000`

Likely cause: board `pm_static.yml` won. App-local `firmware/pm_static.yml` missing or ignored.

### Symptom: serial output shows `>` prompt or echoed commands

Likely cause: Zephyr console/shell got re-enabled.

### Symptom: ping test reads random hex instead of `pong`

Likely cause: prior spam flood still draining. Clearing TX on `spam stop` fixed this.

## Minimal Recovery Checklist

When agents get lost, do this exact sequence:

1. Confirm `prj.conf` still has `CONFIG_BUILD_OUTPUT_UF2=n` and `CONFIG_USE_DT_CODE_PARTITION=n`
2. Confirm `firmware/pm_static.yml` still sets app address `0x0`
3. Build with `nrfutil sdk-manager toolchain launch --ncs-version v3.3.0`
4. Check linker origin is `0x0`
5. Flash with `pyocd flash -t nrf52840 .../zephyr.hex`
6. Test `ping` over `/dev/ttyACM0`
7. Run `cargo test --test xiao_ble_validation -- --ignored --test-threads=1`
