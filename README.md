# serial-mcp for UART and USB serial access

[![GitHub Release](https://img.shields.io/github/v/release/qarnet/serial-mcp)](https://github.com/qarnet/serial-mcp/releases)
[![crates.io](https://img.shields.io/crates/v/serial-mcp)](https://crates.io/crates/serial-mcp)
[![Rust](https://img.shields.io/badge/rust-1.97.1-orange.svg)](https://rust-lang.org)
[![License](https://img.shields.io/badge/license-MIT-green.svg)](LICENSE)

`serial-mcp` is an MCP server for direct access to serial ports. It reads,
writes, and streams UART or USB-serial data to microcontrollers, Arduino boards,
STM32 chips, and other embedded targets. Reads use timeouts and pattern matching
instead of blocking on a serial monitor.

The server provides always-on RX capture. It decodes TX and RX frames using
line, delimiter, length-prefixed, start/end, SLIP, and COBS formats. It provides
AT, JSON, shell, NMEA-0183, and Modbus ASCII parsers.

Protocol presets provide checksum validation. The server also supports
auto-reconnect, event logging, DTR/RTS, BREAK, and flow control. MCP clients can
use these features with serial bootloaders, resets, and embedded boards.

## Quick start

1. Install the server. See [Install](#install) for Cargo, Nix, and prebuilt binary options.
2. Connect an agent. Follow the [agent configuration guide](docs/agent-config.md), or use the example below.
3. Discover devices. Call `list_ports()` and inspect `profile_matches`. The result shows what a bare `open` would reuse.
4. Open a port. Call `open(port=...)` with only the port.
   Baud defaults to 115200/8-N-1. The server reuses the most recently used
   high-confidence profile for a known device. It creates a durable generated
   profile for a new device.
5. Talk to the device. Use `transact()` for command and response exchanges. Use `read()` for buffered or unsolicited data. Use `write()` for send-only operations.

## Capabilities

| Area | What it provides |
|---|---|
| RX model | An always-on ring buffer captures bytes from open to close. `read` returns buffered bytes immediately. It can also wait, match, and replay history. |
| Framing and parsing | Both directions support line, delimiter, length-prefixed, start/end, SLIP, and COBS framing. Parsers include AT, JSON, shell, NMEA-0183, and Modbus ASCII. |
| Protocol presets | Seven presets are available. They are `at_command`, `slip`, `json_lines`, `cobs`, `ndjson`, `nmea0183`, and `modbus_ascii`. Checksum validation is included. |
| Device profiles | The server creates automatic profile sessions. High-confidence devices get durable generated profiles. Learned settings persist across sessions. |
| Boot capture | `capture_boot` handles Arduino auto-reset, power-cycle banners, and boot prompts in one atomic call. |
| Reliability | Ring wrap is reported through `bytes_lost`. Encoding fallback is lossless. The server also supports auto-reconnect and reports partial failures. |
| Operations | Event logging supports persistent JSONL capture through `export_log`. The server also provides port allowlisting and stdio and HTTP transports. |

## Tool catalog (25 tools)

| Group | Tools |
|---|---|
| Discovery | `list_ports`, `list_connections` |
| Connection lifecycle | `open`, `close`, `reconnect`, `get_status`, `reconfigure` |
| I/O | `read`, `write`, `transact`, `capture_boot`, `flush` |
| Line control | `set_dtr_rts`, `set_flow_control`, `send_break` |
| Profiles & config | `list_profiles`, `open_profile`, `save_profile`, `delete_profile`, `configure`, `rollback_profile` |
| Logs & capture | `get_log`, `clear_log`, `export_log` |
| Utility | `compute_checksum` |

## Resources and prompts

| Kind | Items |
|---|---|
| Resources (5) | `serial://ports`, `serial://connections` (static); `serial://connections/{id}`, `serial://connections/{id}/raw`, `serial://connections/{id}/log` (templates) |
| Prompts (2) | `diagnose_port`, `interactive_terminal` |

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

No toolchain is required. Each release publishes one binary per platform. The
`latest/download` URLs resolve to the newest release.

| Platform | Command |
|---|---|
| Linux x86_64 | `curl -L https://github.com/qarnet/serial-mcp/releases/latest/download/serial-mcp-x86_64-linux -o serial-mcp && sudo install -m 755 serial-mcp /usr/local/bin/` |
| Linux ARM64 | Same command with the `serial-mcp-aarch64-linux` asset |
| macOS (Apple Silicon) | Same command with the `serial-mcp-aarch64-macos` asset |
| Windows (x86_64) | Download [`serial-mcp-x86_64-windows.exe`](https://github.com/qarnet/serial-mcp/releases/latest/download/serial-mcp-x86_64-windows.exe) and place it on your `PATH` |

On Linux, add your user to the `dialout` group for port access:

```bash
sudo usermod -aG dialout $USER
```

## Connect an agent

For client-specific setup, see the [agent configuration guide](docs/agent-config.md).
It covers Claude Code CLI, Claude Desktop, Cursor, VS Code, Zed, opencode,
Codex, Hermes, and HTTP transport.

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

## Core workflow

Use this sequence for common work: discover, open, talk, verify the learned
profile, then use advanced tools when needed.

1. Call `list_ports()`. Its `profile_matches` entries correspond to `ports`.
   - `selected` means a bare `open` reuses `selected_profile`.
   - `ambiguous` means equal-ranked profiles require `open_profile`.
   - `duplicate`, `ineligible`, and `none` mean a bare open starts fresh or transient.
2. Call bare `open(port=...)`. The result includes the `profile` binding. The binding reports its name, source, confidence, persistence, generated flag, revision, and dirty state.
3. Use `transact(data=..., match=..., timeout_ms=...)` to write and await a response in one call. Use `read()` for buffered or unsolicited data.
4. After `reconfigure`, `set_flow_control`, or connection-mode `configure`,
   inspect `profile_persistence`. It reports `persisted`, `not_needed`,
   `transient`, or `failed`. Also inspect the updated `profile` binding.
5. Call `close()`. A clean close retries a dirty binding as a safety measure.

For boot and reset capture, call `capture_boot`. It handles Arduino auto-reset,
power-cycle banners, and boot prompts.

The call purges unread OS input. It marks the RX live edge. It can pulse DTR/RTS,
with guaranteed release. It captures only post-mark bytes on a private cursor.
The result is bounded in memory and does not write a file. See [RX and
reading](docs/rx-and-reading.md) for the `from` cursor model. See [Device
profiles](docs/device-profiles.md) for profile behavior.

## Protocols

The `protocol` field supplies framing and parser defaults for both directions.
NMEA and Modbus ASCII presets validate checksums:

| Preset | Wire name | Framing / parser |
|---|---|---|
| AT commands | `at_command` | Line (CR) + AT parser |
| SLIP | `slip` | RFC 1055 byte stuffing |
| JSON lines | `json_lines` | Line + JSON-lines parser |
| COBS | `cobs` | Consistent Overhead Byte Stuffing |
| NDJSON | `ndjson` | Line + JSON-lines parser, skips blank lines |
| NMEA-0183 | `nmea0183` | Start/end `$`/`!` + NMEA parser, `*XX` checksum |
| Modbus ASCII | `modbus_ascii` | Start/end `:` + Modbus ASCII parser, LRC |

Field precedence is explicit call field, call-time preset, connection default,
then connection preset. The [Protocol guide](docs/protocols.md) documents this
order, checksum behavior, and the framing and parser reference.

## Key concepts and guides

| Guide | What it covers |
|---|---|
| [RX and reading](docs/rx-and-reading.md) | Ring buffer and shared cursor. Tagged `from` forms. Timeouts, silence, and matching. Ring wrap and `bytes_lost`. Encoding fallback, flow control, `capture_boot`, and subscriptions. |
| [Device profiles](docs/device-profiles.md) | `profile_matches` outcomes and identity rules. Generated and reused selection. Learning, revision CAS, rollback, and deletion guards. |
| [Persistent capture](docs/persistent-capture.md) | The `export_log` contract. Quotas, portable filenames, atomicity, and failure semantics. |
| [Agent configuration](docs/agent-config.md) | Client setup. HTTP transport. Troubleshooting. |
| [Protocol guide](docs/protocols.md) | Framing and parsers. Presets and precedence. Checksum behavior. |
| [Documentation index](docs/README.md) | User and development guides |

## Transports and options

| Mode | How to activate | Use case |
|---|---|---|
| stdio | default | Desktop agents |
| HTTP | `--transport=http` | Remote and headless use |

<details>
<summary>CLI options</summary>

```
serial-mcp [OPTIONS]

  --transport <stdio|http>          Transport to use (default: stdio)
  --allowlist <patterns>            Comma-separated glob patterns for allowed ports
  --bind <addr>                     HTTP bind address (default: 127.0.0.1:8000)
  --max-program-buffered-bytes <N>  Global budget for all in-flight RX tools
  --max-tool-buffered-bytes <N>     Per-tool ceiling for max_buffered_bytes
  --profiles-path <path>            Profile store file path (default: OS user config dir + serial-mcp/profiles.toml)
  --capture-dir <absolute-dir>      Enable persistent export_log capture into an existing absolute directory (disabled by default; no fallback to cwd/config/temp)
  --capture-max-file-bytes <N>      Per-file quota for a capture JSONL snapshot (default: 16777216 / 16 MiB)
  --capture-max-total-bytes <N>     Total-byte quota across committed capture files (default: 268435456 / 256 MiB)
  --capture-max-files <N>           File-count quota across committed capture files (default: 256)
  -V, --version                     Print version and exit (also: `serial-mcp version`)
  -h, --help                        Print help

  RUST_LOG                   Log level env var (error/warn/info/debug/trace)
```

</details>

The profile store is one TOML file shared by every session. Use `--profiles-path`
for an isolated store. See [Device profiles](docs/device-profiles.md).

### Persistent capture

`export_log` writes portable `.jsonl` filenames into the `--capture-dir` root.
It never accepts arbitrary paths and never overwrites files. See [Persistent
capture](docs/persistent-capture.md).

## MCP compatibility

serial-mcp supports MCP `2025-11-25`. This version uses the legacy session
lifecycle.

It also supports MCP `2026-07-28`. This version uses modern discovery and
stateless requests with SEP-2549 cache fields. Both stdio and HTTP transports
support the port allowlist.

CI runs official conformance checks and Inspector interoperability checks. The
validation tools come from the committed npm lockfile. CI installs them with
`npm ci --ignore-scripts`. It runs them as local binaries, never through npx.

An actual historical `rmcp 1.7.0` client tests backward compatibility over HTTP
and stdio. Run the complete local and CI version gate with:

```bash
bash scripts/test-mcp-compat.sh
```

## Development

Before pushing or opening a pull request, run `cargo fmt --all`. CI runs
`cargo fmt --all -- --check` once in standalone Ubuntu format job first.
Formatting failures block dependent expensive Nix, build/test/Clippy, and MCP
conformance jobs.

```bash
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo fmt --all -- --check

# Linux-only required Rust PTY fixture suites
cargo test --locked --test device_fixture -- --test-threads=1
cargo test --locked --test device_command_parity -- --test-threads=1
cargo test --locked --test device_framing_parity -- --test-threads=1
cargo test --locked --test device_protocol_parity -- --test-threads=1
cargo test --locked --test device_parity_repeat public_boundary_repeat_gate -- --ignored --test-threads=1
```

Production-path real-PTY fixture tests run on Linux. macOS and Windows run
normal Rust build/test/clippy plus controlled-backend coverage.

## Documentation and status

The [product backlog](docs/BACKLOG.md) lists planned and in-progress work. The
[documentation index](docs/README.md) links user guides, and the
[development notes](docs/development/README.md) cover project maintenance.
Report issues and feature requests on the [tracker](https://github.com/qarnet/serial-mcp/issues).

- [CHANGELOG.md](CHANGELOG.md)
- [AGENTS.md](AGENTS.md), contributor guidelines

## MCP registry

The package is available on the [MCP Registry](https://registry.modelcontextprotocol.io/)
as:

mcp-name: io.github.qarnet/serial-mcp

## License

MIT. See [LICENSE](LICENSE).
