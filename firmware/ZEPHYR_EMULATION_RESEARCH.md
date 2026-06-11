# Zephyr Emulation Research — serial-mcp Testing

## Landscape

| Approach | Serial exposes | Works for serial-mcp? | 1200-baud touch? | Notes |
|---|---|---|---|---|
| **native_sim + PTY UART** | `/dev/pts/N` PTY | ✅ Yes — opens like a real serial port | ❌ No DTR/RTS on PTY | `CONFIG_UART_NATIVE_PTY_0_ON_STDINOUT=y` for pipe-based testing |
| **native_sim + USB CDC-ACM over USB/IP** | `/dev/ttyACMx` via `usbip attach` | ✅ Yes — full USB CDC device | ✅ **Yes!** DTR/RTS over USB CDC control transfers | Killer feature: test 1200-baud touch with zero hardware |
| **native_sim + TTY UART** | Real `/dev/ttyUSB0` | ✅ Yes — opens real serial port | Hardware-dependent | Opens actual host serial devices |
| **QEMU + chardev pty** | `/dev/pts/N` PTY | ✅ Yes | ❌ No DTR/RTS | No nRF52840 QEMU target |
| **QEMU + chardev socket** | `localhost:4321` TCP | ✅ Yes (via socat) | ❌ | Needs serial-mcp to support TCP or use socat bridge |
| **QEMU + chardev stdio** | stdin/stdout | ✅ Yes (pipe-based) | ❌ | Default when running interactive |
| **QEMU + chardev pipe** | Named fifo `.in`/`.out` | ⚠️ Possible | ❌ | Twister's default mode |
| **nrf52_bsim (BabbleSim)** | BSIM phy (internal) | ❌ Not host-accessible | ❌ | Simulates nRF52833; UART talks through BSIM phy layer to other simulated devices only |
| **uart-emul (loopback)** | In-process ring buffers | ❌ Not host-exposed | ❌ | `zephyr,uart-emul` driver; TX → RX internally via `loopback` property; API: `uart_emul_put_rx_data()`, `uart_emul_get_tx_data()` |
| **native_sim + USB CDC-ACM without USB/IP** | None (in-process only) | ❌ | ❌ | Needs USB/IP snippet to be host-visible |

## Key Technical Details

### native_sim PTY UART

- Compatible: `zephyr,native-pty-uart`
- Driver: `drivers/serial/uart_native_pty.c`
- Options:
  - `CONFIG_UART_NATIVE_PTY_0_ON_OWN_PTY` — creates `/dev/pts/N`
  - `CONFIG_UART_NATIVE_PTY_0_ON_STDINOUT` — maps to stdin/stdout
  - `--attach_uart` flag — auto-attaches terminal emulator
  - `--wait_uart` — blocks writes until client connects
  - `--<name>_stdinout` — per-instance stdin/stdout mapping
- Supports: poll mode, interrupt mode, async mode
- Does NOT support: runtime config, line control, DTR/RTS

### native_sim TTY UART

- Compatible: `zephyr,native-tty-uart`
- Opens real host serial ports (e.g. `/dev/ttyUSB0`)
- DT properties: `serial-port`, `current-speed`
- Cmdline: `--<name>_port`, `--<name>_baud`
- Supports: poll mode, interrupt mode, runtime config. No async, no line control.

### native_sim USB Device Controller

- Compatible: `zephyr,native-posix-udc`
- Exports over USB/IP protocol
- Host: `usbip list -r localhost`, `sudo usbip attach -r localhost -b 1-1`
- Creates real `/dev/ttyACMx` with full CDC-ACM capabilities
- DTR/RTS signals work over the USB/IP bridge
- This enables testing the 1200-baud touch flow entirely in software

### uart-emul

- Compatible: `zephyr,uart-emul`
- Pure software ring buffers. No host-visible port.
- `loopback` property copies TX → RX
- Host-side API: `uart_emul_put_rx_data()`, `uart_emul_get_tx_data()`, `uart_emul_flush_*()`
- Good for unit-testing Zephyr UART drivers, not for testing serial-mcp
- Used by Zephyr's own `tests/drivers/uart/uart_emul/`

### QEMU in Zephyr

- 16 QEMU targets (cortex_m0, cortex_m3, cortex_a53, riscv32, x86, etc.)
- **No QEMU target for Cortex-M4 or nRF52840** — closest is `qemu_cortex_m0` (nRF51822)
- UART forwarding modes:
  - `stdio` (default) — interactive terminal
  - `QEMU_PTY=1` — pseudo-terminal `/dev/pts/N`
  - `QEMU_PIPE=<path>` — named pipe (twister default)
  - `QEMU_SOCKET=1` — TCP `localhost:4321`
- Twister uses `QEMUHandler` which creates fifo pipes at `<build_dir>/qemu-fifo.in` / `.out`

### Zephyr Robot Framework / pytest-twister-harness

- `NativeSimLibrary` (Python): starts native_sim binary, reads stdout, writes stdin
- `QemuSimLibrary` (Python): same but for QEMU via named pipes
- `DeviceAdapter` class: unified interface for hardware, QEMU, and native_sim
  - `readline()`, `readlines_until(regex=...)`, `write(data)`
  - `SerialConnection` — opens real `/dev/tty*` via pySerial
  - `ProcessConnection` — child process stdin/stdout
  - `FifoConnection` — named pipes for QEMU

## Zephyr Multi-Target Capabilities

Zephyr supports building the same application for multiple boards:

| Mechanism | Path pattern | Purpose |
|---|---|---|
| Board overlay | `boards/<board>.overlay` | Per-board devicetree additions |
| Board conf | `boards/<board>.conf` | Per-board Kconfig settings |
| Board defconfig | `boards/<board>_defconfig` | Per-board Kconfig defaults |
| Common config | `prj.conf` | Shared across all targets |
| Scenarios | `sysbuild/<board>.conf` | Multi-image builds |
| In-code guards | `#ifdef CONFIG_BOARD_NATIVE_SIM` | Target-specific C code paths |
| DT checks | `DT_HAS_COMPAT_STATUS_OKAY(zephyr_native_pty_uart)` | Compile-time DT queries |

This means one `firmware/` directory can produce:
- `native_sim` → Linux executable with PTY UART + optional USB CDC-ACM
- `xiao_ble` → ARM hex with `nrf-uarte` on physical `uart0` + optional USB CDC-ACM

## Build Commands

```bash
# Build for native_sim
west build -b native_sim firmware/

# Build for XIAO BLE
west build -b xiao_ble firmware/ --pristine

# Build with USB CDC-ACM enabled (either target)
west build -b native_sim firmware/ -- -DEXTRA_CONF_FILE=overlay-usb-cdc.conf
```

## 1200-Baud Touch via USB/IP — Full Flow

```text
┌──────────────────────────────────────────────────────┐
│  Host Linux                                          │
│                                                      │
│  serial-mcp  ─── opens /dev/ttyACM0 at 1200 baud     │
│     │         ─── set_dtr_rts(dtr=true, rts=false)   │
│     │         ─── set_dtr_rts(dtr=false, rts=false)  │
│     │                                                │
│  usbip attach    │                                   │
│     │            │                                   │
│  ┌──▼────────────▼─────────────────────────────┐     │
│  │  Linux CDC-ACM driver → /dev/ttyACM0         │     │
│  │  USB/IP vhci                                  │     │
│  │  TCP localhost:3240                           │     │
│  └───────────────────────────────────────────────┘     │
│                   │                                    │
│  ┌────────────────▼──────────────────────────────┐     │
│  │  zephyr.exe (native_sim process)               │     │
│  │                                              │     │
│  │  App:                                        │     │
│  │    - CDC-ACM callback: DTR falling at 1200   │     │
│  │    - Writes GPREGRET=0x57, NVIC_SystemReset  │     │
│  │    - Exit process (simulates reset)          │     │
│  └──────────────────────────────────────────────┘     │
└──────────────────────────────────────────────────────┘
```
