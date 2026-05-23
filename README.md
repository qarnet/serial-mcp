# Serial MCP Server

[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://rust-lang.org)
[![RMCP](https://img.shields.io/badge/RMCP-1.7-blue.svg)](https://github.com/modelcontextprotocol/rust-sdk)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

Model Context Protocol (MCP) server that lets AI assistants drive serial
ports: open, read, write, wait for prompts, stream RX bytes, toggle
DTR/RTS, send BREAK. Cross-platform (Windows / Linux / macOS). Two
transports: stdio and streamable HTTP.

> **This is a fork** of the original [adancurusul/serial-mcp-server](https://github.com/adancurusul/serial-mcp-server).
> The fork rebuilds the project against rmcp 1.7, removes ~80% of the
> original code as dead scaffolding, and adds resources, prompts,
> streaming, task cancellation, an HTTP transport, and six new tools.
> See [CHANGELOG.md](CHANGELOG.md) for the full delta.

## Surface at a glance

- **11 tools** for serial port operations
- **3 resources** (`serial://ports`, `serial://connections`, `serial://connections/{id}`)
- **2 prompt templates** for common agent workflows
- **Live RX streaming** via MCP `notifications/message` events
- **Task cancellation** on the long-running tools
- **Two transports**: stdio for desktop clients, streamable HTTP for remote use

## Install

```bash
git clone https://github.com/qarnet/serial-mcp-server.git
cd serial-mcp-server
cargo build --release
```

On Linux you need libudev headers:

```bash
sudo apt install libudev-dev      # Debian / Ubuntu / WSL
sudo dnf install systemd-devel    # Fedora
```

The build produces two binaries under `target/release/`:

| Binary | Transport | Default endpoint |
|---|---|---|
| `serial-mcp-server` | stdio | n/a |
| `serial-mcp-server-http` | streamable HTTP | `http://127.0.0.1:8000/mcp` |

`RUST_LOG=debug` enables verbose logs (written to stderr). The HTTP
binary reads `SERIAL_MCP_HTTP_BIND` to override the bind address.

## Configure an MCP client

### Claude Desktop (stdio)

```json
{
  "mcpServers": {
    "serial": {
      "command": "/path/to/serial-mcp-server/target/release/serial-mcp-server",
      "args": [],
      "env": { "RUST_LOG": "info" }
    }
  }
}
```

On Windows substitute the path with `C:\\path\\to\\...\\serial-mcp-server.exe`.

### Any MCP client (HTTP)

Run the HTTP binary on the host with the USB-serial dongle:

```bash
SERIAL_MCP_HTTP_BIND=0.0.0.0:8000 ./target/release/serial-mcp-server-http
```

Then point any streamable-HTTP-capable MCP client at
`http://<host>:8000/mcp`.

## Tools

| Tool | Purpose |
|---|---|
| `list_ports` | Enumerate serial ports the OS exposes (name, description, hardware id). |
| `open` | Open a port with full framing config (`baud_rate`, `data_bits`, `stop_bits`, `parity`, `flow_control`). Returns a `connection_id`. |
| `close` | Release a connection by id. |
| `write` | Send bytes. Payload is encoded utf8 / hex / base64. |
| `read` | Read up to `max_bytes` with optional `timeout_ms`. **Task-capable.** |
| `flush` | Discard buffered bytes (`input`, `output`, or `both`). |
| `set_dtr_rts` | Drive DTR/RTS lines explicitly. Used for Arduino auto-reset, ESP32 bootloader entry. |
| `send_break` | Assert BREAK on TX for `duration_ms`, then release. **Task-capable.** |
| `wait_for` | Read until a byte pattern matches or timeout. The agent's request/response loop tool. **Task-capable.** |
| `subscribe` | Spawn a background task that pushes every chunk read on this connection to the client as a `notifications/message` event. |
| `unsubscribe` | Stop a running subscription. |

All tool responses are **structured JSON** (rmcp `Json<T>`) — clients
get typed fields, not strings to regex over. Operational failures
(invalid args, unknown id, IO error) come back as
`CallToolResult { isError: true, ... }` so the LLM can recover;
protocol-level errors stay as `McpError`.

### Task-capable tools

`read`, `wait_for`, and `send_break` accept `task` metadata on the
`CallToolRequestParams`. When invoked as a task, the server returns
a `task_id` immediately and runs the tool in the background. The
client can then:

- `tasks/cancel` with the `task_id` to abort
- `tasks/list` to enumerate running tasks
- `tasks/getPayload` to fetch the result on completion

Short-lived tools (`open`/`close`/`write`/`flush`/`set_dtr_rts`/
`list_ports`/`subscribe`/`unsubscribe`) are not task-capable; they
return synchronously.

## Resources

| URI | Description |
|---|---|
| `serial://ports` | Live `ListPortsResult` JSON, re-enumerates on every read. |
| `serial://connections` | List of currently-open connections (id + port). |
| `serial://connections/{id}` | Templated resource — substitute `{id}` with a `connection_id` returned by `open`. |

Resources are pull-only at the moment (no `resources/subscribe`).

## Prompts

| Name | Args | Purpose |
|---|---|---|
| `diagnose_port` | `port`, `baud_rate?` | Step-by-step plan to identify an unknown serial device. |
| `interactive_terminal` | `connection_id`, `line_ending?`, `device_prompt?` | Conventions for running a REPL-style command/response loop. |

## Streaming RX

```text
client                          server                       device
  | subscribe(connection_id) -->  |                              |
  |                                |  read() loop  -- bytes -->  |
  |                                |                              |
  |  <-- notifications/message ---|  (logger="serial:<id>")      |
  |  <-- notifications/message ---|                              |
  |                                |                              |
  | unsubscribe(connection_id) ->|                              |
  |  (server aborts the task)    |                              |
```

Each chunk is delivered as `notifications/message`:

```json
{
  "level": "info",
  "logger": "serial:9f...",
  "data": {
    "connection_id": "9f...",
    "bytes_read": 32,
    "encoding": "utf8",
    "data": "hello from board\r\n"
  }
}
```

The reader yields the connection's IO mutex between chunks, so a
subscription does not block concurrent `write` / `wait_for` /
control-line calls on the same connection.

## Example agent flow

A typical "drive an embedded board" interaction:

```
1. list_ports               → ["/dev/ttyUSB0", "/dev/ttyACM0"]
2. open(port="/dev/ttyACM0", baud_rate=115200)
                            → { connection_id: "9f..." }
3. set_dtr_rts(id, dtr=false, rts=false)
   set_dtr_rts(id, dtr=true,  rts=true)    # Arduino soft-reset
4. wait_for(id, pattern="OK>", timeout_ms=3000)
                            → { matched: true, data: "...OK>", match_index: 27 }
5. write(id, data="status\r\n")
   wait_for(id, pattern="OK>", timeout_ms=1000)
6. close(id)
```

For long-running passive monitoring, swap step 5 for
`subscribe(id)` and let the server push everything the board prints
as MCP notifications.

## Supported hardware

Nothing about this server is device-specific — anything that exposes
a UART or USB-Serial endpoint to the host OS works. The configurations
below are the ones that have been exercised in practice:

**Boards**

- STM32 — any series with UART (USB-CDC variants or external USB-UART)
- Arduino — Uno, Nano, Leonardo (built-in USB), ESP32, ESP8266
- Industrial bridges — RS-485 transceivers, Modbus gateways
- IoT sensors / actuators with a serial console

**USB-Serial bridge chips**

- CH340 / CH341 / CH343 (WCH)
- CP2102 / CP2104 (Silicon Labs)
- FTDI FT232 / FT231 family
- Native USB-CDC (STM32 with USB peripheral, Arduino Leonardo)

**Platform support**

| Platform | Port name format | Examples |
|---|---|---|
| Windows | `COMx` | `COM1`, `COM3`, `COM19` |
| Linux | `/dev/tty*` | `/dev/ttyUSB0`, `/dev/ttyACM0` |
| macOS | `/dev/tty.*` or `/dev/cu.*` | `/dev/tty.usbserial-1234` |

On Linux, the user running the server typically needs to be in the
`dialout` (Debian/Ubuntu) or `uucp` (Arch/Fedora) group to access
`/dev/tty*` without root.

## Build, test, lint

```bash
cargo build --all-targets
cargo test
cargo clippy --all-targets -- -D warnings
```

The test suite is layered:

| Layer | File | Count | What it covers | Runs on |
|---|---|---|---|---|
| 1 — unit | `src/*.rs` | 36 | Codec, baud-rate validation, manager invariants, `wait_for` accumulator over an in-memory `DuplexStream`, URI parsing, prompt-router wiring. | every push, all platforms |
| 2 — HTTP integration (in-memory) | `tests/http_integration.rs` | 14 | Real `rmcp` HTTP client → `axum`-hosted `SerialHandler` → injected loopback connection. Covers the full MCP surface (tools / resources / prompts / notifications) over the wire. | every push, all platforms |
| 3 — HTTP integration (real PTY) | `tests/serial_pty.rs` | 6 | Same harness as Layer 2 but the server opens a real Linux PTY slave (`/dev/pts/N`) via `tokio_serial::SerialStream`. Exercises the production serial code path end-to-end. | every push, Linux only (`#[cfg(target_os = "linux")]`) |
| 4 — hardware-in-the-loop | `tests/hardware_loopback.rs` | 2 (ignored) | Real USB-Serial adapter with TX→RX jumper. Confirms behaviour on physical hardware. | manual: `SERIAL_MCP_TEST_PORT=/dev/ttyUSB0 cargo test -- --ignored` |

Layer 4 needs a USB-Serial dongle plugged in with its TX and RX pins
jumpered together (a single wire across the two pins). Without that,
`hw_loopback_write_then_read_roundtrip` will read no data and fail.

## STM32 demo

The original upstream ships a self-contained STM32 demo firmware
in `examples/STM32_demo/` that you can flash, run, and drive from
this server. The new tools (`wait_for`, `subscribe`, …) work
unchanged with it. See
[examples/STM32_demo/README.md](examples/STM32_demo/README.md).

## Architecture

```
┌──────────────────┐     stdio / streamable-HTTP    ┌──────────────────┐
│   MCP client     │ ─────────────────────────────▶ │  SerialHandler   │
│   (Claude, …)    │ ◀───── notifications/message ─ │  (rmcp 1.7)      │
└──────────────────┘                                 └────────┬─────────┘
                                                              │
                                                     ConnectionManager
                                                              │
                                              ┌───────────────┼───────────────┐
                                              ▼               ▼               ▼
                                       SerialConnection  SerialConnection  ...
                                       (Box<dyn SerialIo>)
                                              │
                                              ▼
                                         tokio_serial::SerialStream
                                         (real OS port)
                                         — or —
                                         test_support::LoopbackIo
                                         (in-memory DuplexStream)
```

## Acknowledgments

- Upstream [adancurusul/serial-mcp-server](https://github.com/adancurusul/serial-mcp-server)
- [serialport-rs](https://crates.io/crates/serialport) and [tokio-serial](https://crates.io/crates/tokio-serial) — port enumeration and async I/O
- [rmcp](https://github.com/modelcontextprotocol/rust-sdk) — official Rust MCP SDK
- [tokio](https://tokio.rs/) — async runtime

## License

MIT. See [LICENSE](LICENSE).
