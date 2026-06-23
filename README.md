# Serial MCP Server

[![GitHub Release](https://img.shields.io/github/v/release/qarnet/serial-mcp)](https://github.com/qarnet/serial-mcp/releases)
[![crates.io](https://img.shields.io/crates/v/serial-mcp)](https://crates.io/crates/serial-mcp)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-orange.svg)](https://rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

**Serial monitors are something agents can't work with well natively. serial-mcp fixes this by giving agents powerful tools for reading, writing and subscribing to serial ports.**

Non-blocking reads with timeouts and pattern matching, background RX streaming,
frame decoding with AT/JSON/shell parsers, auto-reconnect, event logging,
and full line control (DTR/RTS, BREAK, flow control) — so Claude, Codex, or any
MCP client can flash, reset, and talk to your board without freezing the session.

**MCP 2025-11-25 compliant** · resource change notifications · port allowlist · stdio + HTTP transports

## What It Does

Exposes serial ports as MCP tools so agents like Claude can interact with
embedded devices, Arduino boards, STM32 microcontrollers, and any UART/USB-serial
hardware — all through natural language.

**22 tools** — list_ports, list_connections, open, close, read, write, flush, set_dtr_rts, set_flow_control, send_break, subscribe, unsubscribe, get_status, reconfigure, list_profiles, open_profile, save_profile, delete_profile, get_log, clear_log, export_log, reconnect  
**4 resources** — `serial://ports`, `serial://connections`, `serial://connections/{id}`, `serial://connections/{id}/raw`, `serial://connections/{id}/log` (3 resource templates + 1 static)  
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
ARCH=aarch64-macos
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

## How RX Works

All receive-side operations (`read`, `subscribe`) share a single **per-connection
RX pump** (`RxSession`). The pump is the only code that reads from the serial
port — it fans bytes out to registered consumers, so `read` and `subscribe`
never race each other.

**Future-only semantics:** `read` returns **only bytes received after the call
starts**, not previously buffered data. This means `read` sees a fresh stream
every time.

**Stop reasons:** Every `read` and `subscribe` stop payload includes a
`stop_reason` field with one of these values:

| Reason | Meaning |
|---|---|
| `data_complete` | Operation finished its data collection normally |
| `timeout` | Wall-clock timeout (`timeout_ms`) elapsed |
| `match_found` | A byte pattern was matched (raw or decoded frame data) |
| `max_buffered_bytes` | Buffer budget limit reached |
| `max_frames` | Frame count limit (`max_frames` in framing config) reached |
| `no_new_rx_timeout` | Silence timeout elapsed (no new bytes within window) |
| `connection_closed` | The underlying serial port was closed |
| `cancelled` | The MCP client cancelled the request |
| `read_error` | An I/O error occurred on the serial port |
| `channel_closed` | The internal RX pump channel closed |
| `peer_disconnected` | The MCP client disconnected during streaming |
| `budget_exhausted` | Program buffer budget was insufficient |

**Result metadata:** `read` and subscribe stop notifications carry:

| Field | Meaning |
|---|---|
| `bytes_observed` | Total bytes the operation saw from the RX stream |
| `bytes_returned` | Bytes actually returned in the result `data` |
| `truncated` | `true` when `bytes_returned < bytes_observed` (data was capped) |
| `matched` | `true` when a configured pattern was found |
| `match_index` | Byte offset of the match within the returned `data` |
| `match_frame_index` | When framing + match: which frame contained the match |
| `frames` | Decoded frames (present when `framing` option is used) |
| `frames_dropped` | Frames skipped due to encoding failures (rare) |

**Matching:** Set `match.pattern` + optional `match.pattern_encoding` on
`read` or `subscribe`. Three match modes:
- `literal_substring` — byte-substring match (default)
- `regex` — regular expression match on raw bytes
- `glob` — per-line glob pattern match (`*` and `?` wildcards)

When framing is active, match operates on **decoded frame data** (per-frame),
so agents can match against structured content (e.g., `"OK"` in an AT response,
`"sensor":"temp"` in a JSON object) without thinking about raw frame delimiters.
The `match_frame_index` field indicates which frame contained the match.

Add `match.context_amount_of_matched_bytes` to return up to N bytes
*before* the match plus the matched bytes (shaped context).

**Silence timeout:** Set `no_new_rx_timeout_ms` to stop when no new bytes
arrive within the specified window. Distinct from wall-clock `timeout_ms` —
the silence timer resets on every received chunk.

**Frame decoding:** Set `framing` on `read` or `subscribe` to split the byte
stream into structured frames. Four boundary detection modes:

| Mode | How boundaries are detected |
|---|---|
| `line` | Split on `\n` (optionally preceded by `\r`). Default. |
| `delimiter` | Split on a user-supplied byte sequence. |
| `length_prefixed` | Read a 1/2/4-byte length prefix, then that many bytes. |
| `start_end` | Find frames between start and end marker sequences. |

Each frame can be parsed by an optional parser:
- `at_command` — AT command responses, URCs, OK/ERROR status
- `json_lines` — each frame deserialized as a JSON object
- `shell_prompt` — detect `$`, `#`, `>`, `user@host:~$` prompt patterns
- `raw` — no parsing (default when parser omitted)

`read` returns frames in the `frames` array (with `data`, `frame_index`,
`frame_type`, and optional `parsed` fields). `subscribe` emits one
notification per frame (with `frame_index`, `frame_type`, `data`, `parsed`).
Set `max_frames` to stop after collecting N frames. Partial frames (incomplete
data without a boundary) are flushed on timeout/close and marked with
`"partial": true` in subscribe notifications.

**Auto-reconnect:** Set `reconnect_policy` on `open` to automatically
reconnect after a fatal disconnect (cable unplug, device reset). Configurable:
max attempts, initial delay, max delay, exponential backoff multiplier.
A background supervisor task manages reconnection. Use the `reconnect` tool
to trigger a manual reconnect. When reconnect is not enabled, read/subscribe
exit immediately on disconnect with `connection_closed` stop reason.

**Event log:** Each connection maintains a bounded ring buffer of events
(open, close, read, write, match found, truncation, notification drops,
disconnects, reconnects). Use `get_log` to query, `clear_log` to reset,
and `export_log` for a full dump. The `serial://connections/{id}/log`
resource provides access via MCP resource reads.

**Connection profiles:** Save and load named port configurations with
`save_profile` and `open_profile`. Profiles store baud rate, data bits,
parity, flow control, and optional reconnect policy. `list_profiles` shows
all saved profiles. `delete_profile` removes one.

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

> Be sure to ask your agent to give honest feedback on the tool after they finish using it. Always looking for ways to improve serial-mcp :)

## Documentation

- [Agent Configuration](docs/agent-config.md)
- [Testing Guide](docs/TESTING.md)
- [Simulation Matrix](docs/SIMULATION_MATRIX.md)
- [CHANGELOG.md](CHANGELOG.md)
- [AGENTS.md](AGENTS.md) — contributor guidelines

## MCP Registry

Available on the [MCP Registry](https://registry.modelcontextprotocol.io/) as:

mcp-name: io.github.qarnet/serial-mcp

## License

MIT. See [LICENSE](LICENSE).
