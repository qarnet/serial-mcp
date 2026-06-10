# Serial MCP Server

[![GitHub Release](https://img.shields.io/github/v/release/qarnet/serial-mcp)](https://github.com/qarnet/serial-mcp/releases)
[![crates.io](https://img.shields.io/crates/v/serial-mcp)](https://crates.io/crates/serial-mcp)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

**Serial monitors are something agents can't work with well natively. serial-mcp fixes this by giving agents powerful tools for reading, writing and subscribing to serial ports.**

Non-blocking reads with timeouts and pattern matching, background RX streaming,
and full line control (DTR/RTS, BREAK, flow control) — so Claude, Codex, or any
MCP client can flash, reset, and talk to your board without freezing the session.

> Be sure to ask your agent to give honest feedback on the tool after they finish using it. Always looking for ways to improve serial-mcp :)

**MCP 2025-11-25 compliant** · resource change notifications · port allowlist · stdio + HTTP transports

## What It Does

Exposes serial ports as MCP tools so agents like Claude can interact with
embedded devices, Arduino boards, STM32 microcontrollers, and any UART/USB-serial
hardware — all through natural language.

**12 tools** — list_ports, list_connections, open, close, read, write, flush, set_dtr_rts, set_flow_control, send_break, subscribe, unsubscribe  
**3 resources** — `serial://ports`, `serial://connections`, `serial://connections/{id}`  
**2 prompt templates** — `diagnose_port`, `interactive_terminal`  

## Install

### Linux

```bash
VERSION=$(curl -s https://api.github.com/repos/qarnet/serial-mcp/releases/latest | grep -oP '"tag_name": "\K[^"]+')
curl -L "https://github.com/qarnet/serial-mcp/releases/download/${VERSION}/serial-mcp-${VERSION#v}-x86_64-linux" \
  -o serial-mcp && chmod +x serial-mcp && sudo mv serial-mcp /usr/local/bin/
```

Add user to `dialout` group for port access: `sudo usermod -aG dialout $USER`

### macOS

```bash
VERSION=$(curl -s https://api.github.com/repos/qarnet/serial-mcp/releases/latest | grep -oP '"tag_name": "\K[^"]+')
ARCH=aarch64-macos   # Intel: x86_64-macos
curl -L "https://github.com/qarnet/serial-mcp/releases/download/${VERSION}/serial-mcp-${VERSION#v}-${ARCH}" \
  -o serial-mcp && chmod +x serial-mcp && sudo mv serial-mcp /usr/local/bin/
```

### Windows

Download `serial-mcp-{VERSION}-x86_64-windows.exe` from the [latest release](https://github.com/qarnet/serial-mcp/releases/latest) and place it on your `PATH`.

### Via cargo (all platforms)

```bash
cargo install serial-mcp
```

### Via Nix

```bash
nix profile install github:qarnet/serial-mcp
```

## Wire Up Your Agent

→ **[Agent configuration guide](docs/agent-config.md)** — Claude Code CLI, Claude Desktop, Cursor, VS Code, Zed, opencode, HTTP transport

<details>
<summary>Quick example (Claude Code, Linux/macOS)</summary>

```json
{
  "mcpServers": {
    "serial": {
      "type": "stdio",
      "command": "serial-mcp",
      "args": ["--allowlist=/dev/ttyACM*,/dev/ttyUSB*"]
    }
  }
}
```

</details>

## Options

```
serial-mcp [OPTIONS]

  --transport <stdio|http>   Transport to use (default: stdio)
  --allowlist <patterns>     Comma-separated glob patterns for allowed ports
  --bind <addr>              HTTP bind address (default: 127.0.0.1:8000)
  -h, --help                 Print help

  RUST_LOG                   Log level env var (error/warn/info/debug/trace)
```

## Transports

| Mode | How to activate | Use case |
|---|---|---|
| stdio | default | Desktop agents |
| HTTP | `--transport=http` | Remote / headless |

## Example Agent Flow

```
1. list_ports → ["/dev/ttyUSB0", "/dev/ttyACM0"]
2. open(port="/dev/ttyACM0", name="board-uart", baud_rate=115200) → { connection_id: "9f...", name: "board-uart" }
3. list_connections() → [{ connection_id: "9f...", name: "board-uart", port: "/dev/ttyACM0" }]
4. set_dtr_rts(id, dtr=false, rts=false)  # Arduino reset
   set_dtr_rts(id, dtr=true,  rts=true)
5. read(id, match={ pattern: "OK>" }, timeout_ms=3000)  # pattern match in RX data
6. write(id, data="status\r\n")
7. close(id)
```

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

# Hardware tests (requires TX-RX loopback device)
SERIAL_MCP_TEST_PORT=/dev/ttyACM0 cargo test --test hardware_loopback -- --ignored

# XIAO BLE firmware validation (requires dedicated serial-mcp test firmware)
SERIAL_MCP_XIAO_PORT=/dev/ttyACM0 cargo test --test xiao_ble_validation -- --ignored --test-threads=1
```

## Documentation

- [Agent Configuration](docs/agent-config.md)
- [Testing Guide](docs/TESTING.md)
- [CHANGELOG.md](CHANGELOG.md)
- [AGENTS.md](AGENTS.md) — contributor guidelines

## License

MIT. See [LICENSE](LICENSE).
