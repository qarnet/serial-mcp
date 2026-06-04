# Serial MCP Server

[![GitHub Release](https://img.shields.io/github/v/release/qarnet/serial-mcp)](https://github.com/qarnet/serial-mcp/releases)
[![crates.io](https://img.shields.io/crates/v/serial-mcp)](https://crates.io/crates/serial-mcp)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

An MCP server that lets AI assistants drive serial ports: open, read, write,
wait for prompts, stream RX bytes, toggle DTR/RTS, send BREAK.

> Be sure to ask your agent to give honest feedback on the tool after they finish using it. Always looking for ways to improve serial-mcp :)

**MCP 2025-11-25 compliant** · resource change notifications · port allowlist · stdio + HTTP transports

## What It Does

Exposes serial ports as MCP tools so agents like Claude can interact with
embedded devices, Arduino boards, STM32 microcontrollers, and any UART/USB-serial
hardware — all through natural language.

**13 tools** — list_ports, get_version, open, close, read, read_line, write, flush, set_dtr_rts, send_break, wait_for, subscribe, unsubscribe  
**3 resources** — `serial://ports`, `serial://connections`, `serial://connections/{id}`  
**2 prompt templates** — `diagnose_port`, `interactive_terminal`  

## Install

### Via cargo

```bash
cargo install serial-mcp
```

Pre-built binaries are available on the [releases page](https://github.com/qarnet/serial-mcp/releases).

### Via Nix

```bash
nix profile install github:qarnet/serial-mcp
```

Linux users: add yourself to the `dialout` group for port access: `sudo usermod -aG dialout $USER`

## Wire Up Your Agent

→ **[Agent configuration guide](docs/agent-config.md)** — Claude Code, Claude Desktop, Cursor, VS Code, Zed, opencode, Codex, Hermes, HTTP transport

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

## Development

```bash
cargo test                                          # ~140 tests
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

# Hardware tests (requires TX-RX loopback device)
SERIAL_MCP_TEST_PORT=/dev/ttyACM0 cargo test --test hardware_loopback -- --ignored
```

## Documentation

- [Agent Configuration](docs/agent-config.md)
- [CHANGELOG.md](CHANGELOG.md)
- [AGENTS.md](AGENTS.md) — contributor guidelines

## Acknowledgements

serial-mcp evolved from early experimentation with serial-port MCP tooling and has since grown into its own project. Thanks to everyone who contributed ideas, tested on real hardware, and reported bugs.

## License

MIT. See [LICENSE](LICENSE).
