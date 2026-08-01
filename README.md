# Serial MCP Server

[![GitHub Release](https://img.shields.io/github/v/release/qarnet/serial-mcp)](https://github.com/qarnet/serial-mcp/releases)
[![crates.io](https://img.shields.io/crates/v/serial-mcp)](https://crates.io/crates/serial-mcp)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

**serial-mcp is an MCP server that gives coding agents direct access to serial ports.** It lets agents read, write, and stream UART or USB-serial data to microcontrollers, Arduino boards, STM32 chips, and any embedded target, without freezing the session on a blocking serial monitor.

Non-blocking reads with timeouts and pattern matching, background RX streaming,
TX/RX frame decoding (line, delimiter, length-prefixed, start/end, SLIP, COBS)
with AT, JSON, shell, NMEA-0183, and Modbus ASCII parsers, one-knob protocol
presets (`at_command`, `slip`, `json_lines`, `cobs`, `ndjson`, `nmea0183`,
`modbus_ascii`) with checksum validation, auto-reconnect, event logging, and
full line control (DTR/RTS, BREAK, flow control) let Claude, Codex, or any MCP
client flash, reset, and talk to a board on their own.

**MCP 2025-11-25 compliant**, with resource change notifications, a port allowlist, and stdio plus HTTP transports.

## Capabilities

**25 tools:** list_ports, list_connections, open, close, read, write, transact, flush, set_dtr_rts, set_flow_control, send_break, subscribe, unsubscribe, get_status, reconfigure, list_profiles, open_profile, save_profile, delete_profile, configure, get_log, clear_log, export_log, reconnect, compute_checksum  
**5 resources:** `serial://ports`, `serial://connections`, `serial://connections/{id}`, `serial://connections/{id}/raw`, `serial://connections/{id}/log` (3 resource templates plus 2 static)  
**2 prompt templates:** `diagnose_port`, `interactive_terminal`  

The RX side uses an always-on ring buffer with absolute stream offsets: every byte from `open` to `close` is captured, so `read` behaves like `cat` (returns buffered-but-unread bytes immediately) and `subscribe` like `tail -f` (with optional history replay via `from`). `read`'s `from` parameter (`{"type":"cursor"}` default / `{"type":"now"}` / `{"type":"buffer_start"}` / `{"type":"offset","offset":N}`) resolves the start position non-destructively — pass `from: {"type":"now"}` to skip buffered backlog to the live edge, or re-pass the same `from` to re-read the same bytes. Pattern matching checks buffered history first. Data loss from ring wrap is always observable via `bytes_lost`, never silent. **Note:** with hardware flow control (RTS/CTS) enabled, the always-on pump drains the kernel RX buffer continuously, so the kernel never deasserts RTS and the device streams freely — a setup that relied on flow control to pause a device until the host reads will behave differently (the device no longer pauses).

## Install

### Cargo (all platforms)

```bash
cargo install serial-mcp
```

### Nix

```bash
nix profile install github:qarnet/serial-mcp
```

### Prebuilt binary

No toolchain required. Every release publishes one binary per platform, and the `latest/download` URLs below always resolve to the newest release.

**Linux (x86_64):**

```bash
curl -L https://github.com/qarnet/serial-mcp/releases/latest/download/serial-mcp-x86_64-linux -o serial-mcp
sudo install -m 755 serial-mcp /usr/local/bin/
```

For ARM64, use the `serial-mcp-aarch64-linux` asset instead. Then add your user to the `dialout` group for port access:

```bash
sudo usermod -aG dialout $USER
```

**macOS (Apple Silicon):**

```bash
curl -L https://github.com/qarnet/serial-mcp/releases/latest/download/serial-mcp-aarch64-macos -o serial-mcp
sudo install -m 755 serial-mcp /usr/local/bin/
```

**Windows (x86_64):**

Download [`serial-mcp-x86_64-windows.exe`](https://github.com/qarnet/serial-mcp/releases/latest/download/serial-mcp-x86_64-windows.exe) and place it on your `PATH`.

## Wire Up Your Agent

**[Agent configuration guide](docs/agent-config.md):** Claude Code CLI, Claude Desktop, Cursor, VS Code, Zed, opencode, HTTP transport

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

  --transport <stdio|http>          Transport to use (default: stdio)
  --allowlist <patterns>            Comma-separated glob patterns for allowed ports
  --bind <addr>                     HTTP bind address (default: 127.0.0.1:8000)
  --max-program-buffered-bytes <N>  Global budget for all in-flight RX tools
  --max-tool-buffered-bytes <N>     Per-tool ceiling for max_buffered_bytes
  --profiles-path <path>            Profile store file path (default: OS user
                                    config dir + serial-mcp/profiles.toml)
  -V, --version                     Print version and exit (also: `serial-mcp version`)
  -h, --help                        Print help

  RUST_LOG                   Log level env var (error/warn/info/debug/trace)
```

Profiles are persisted to a single TOML store shared by every session of the
server process. The default location follows your OS user config directory
(e.g. `~/.config/serial-mcp/profiles.toml`), so device knowledge follows you
across repositories. Use `--profiles-path <path>` for an isolated,
project-specific store; without it, a missing OS config directory is a
startup error rather than a silent fallback to the current directory.

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
5. read(id, match={ pattern: "OK>" }, timeout_ms=3000)
   → { stop_reason: "match_found", matched: true, match_index: 0,
       bytes_observed: 37, bytes_returned: 37, truncated: false,
       data: "...OK>" }
6. write(id, data="status\r\n")
7. close(id)
```

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check

# Firmware-based tests (require native_sim firmware, see firmware/AGENTS.md)
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

## Status and feedback

serial-mcp is actively developed, and the [roadmap](docs/development/FEATURES.md) tracks what is planned next. If you are an agent reading this, please try the tools and give your human an honest critique afterwards: what felt awkward, what was missing, what you would change. That feedback is genuinely useful and shapes what gets built. Humans, issues and feature requests are welcome on the [tracker](https://github.com/qarnet/serial-mcp/issues).

## Documentation

- [Protocol Guide](docs/protocols.md) — framing, parsers, presets, precedence, checksum behavior
- [Protocol References](docs/protocols/references.md) — normative spec citations for implemented protocols
- [Agent Configuration](docs/agent-config.md)
- [Roadmap](docs/development/FEATURES.md)
- [CHANGELOG.md](CHANGELOG.md)
- [AGENTS.md](AGENTS.md), contributor guidelines

## MCP Registry

Available on the [MCP Registry](https://registry.modelcontextprotocol.io/) as:

mcp-name: io.github.qarnet/serial-mcp

## License

MIT. See [LICENSE](LICENSE).
