# Changelog

| Version | Date | Highlights |
|---|---|---|
| [0.3.2](#032) | 2026-06-04 | `read_line` buffering fix, single-RX-owner guard, exclusivity docs |
| [0.3.1](#031) | 2026-06-04 | `get_version`, `read_line`, lossy UTF-8/text encoding |
| [0.3.0](#030) | 2026-05-30 | Single binary, CLI args replace env vars, multi-platform builds + crates.io |
| [0.2.6](#026) | 2026-05-27 | Protocol emulator integration tests (ESP32 workflow, binary payloads) |
| [0.2.5](#025) | 2026-05-27 | Property-based tests (54 strategies), fuzz targets, allowlist tests |
| [0.2.4](#024) | 2026-05-27 | Schema fix: optional fields serialize as `null` not omitted |
| [0.2.3](#023) | 2026-05-26 | `subscribe(timeout_ms)` blocking mode |
| [0.2.2](#022) | 2026-05-26 | MCP compliance fixes, pagination, input validation, race-condition fix |
| [0.2.1](#021) | 2026-05-24 | MCP 2025-11-25, resource change notifications, port allowlist, stdio tests |
| [0.2.0](#020) | 2026-05-23 | Project reset: rmcp 1.7 rewrite, 6 new tools, resources, prompts, HTTP transport |
| [0.1.0](#010) | — | Initial release (5 tools, STM32 demo) |

---

## [0.3.2]

Physical-device hardening and operator-facing docs.

**Added:** explicit docs for exclusive serial-port opens and single active RX ownership, including common conflicts with tools like `picocom` and IDE serial monitors.

**Fixed:** `read_line` now preserves trailing buffered bytes for follow-up line reads instead of dropping them when multiple lines arrive in one burst.

**Fixed:** concurrent RX operations on one connection now fail fast with owner-specific errors such as `Connection busy: subscribe already owns RX` instead of racing each other.

---

## [0.3.1]

Tooling and encoding improvements.

**Added:** `get_version` tool for querying package version and build commit.

**Added:** `read_line` for line-delimited REPL and firmware-log workflows.

**Fixed:** UTF-8/text reads now use lossy decoding for non-UTF-8 byte streams instead of hard-failing on invalid sequences.

---

## [0.3.0]

**Breaking:**
- `serial-mcp-http` binary removed — use `serial-mcp --transport=http`
- `SERIAL_MCP_ALLOWLIST`, `SERIAL_MCP_HTTP_BIND`, `SERIAL_MCP_TRANSPORT` env vars removed — use `--allowlist=<patterns>`, `--bind=<addr>`, `--transport=<stdio|http>` CLI flags

**Added:**
- `--transport`, `--allowlist`, `--bind`, `--help` CLI flags via `pico-args`
- Pre-built binaries for macOS arm64/x86_64 and Windows x86_64
- Multi-platform CI (Linux + macOS + Windows on every PR)
- `cargo publish` step in release workflow
- Agent config examples for Claude Code CLI, Cursor, VS Code, Zed

---

## [0.2.6]

Protocol emulator integration tests — full ESP32 weather-station agent workflow and binary payload roundtrips via PTY. No hardware required. Test count: 157 (2 hardware-ignored).

**Added:** `tests/protocol_emulator.rs` (13-stage MCP workflow), `tests/protocol_emulator_binary.rs` (binary encoding edge cases), PTY test helpers.

---

## [0.2.5]

Property-based testing and fuzz targets.

**Added:** `tests/proptest.rs` (54 strategies covering all tools, result schemas, encoding roundtrips, lifecycle sequencing), 3 `cargo-fuzz` harnesses, `tests/allowlist.rs` (5 tests), PTY `wait_for` pattern test.

---

## [0.2.4]

Schema fix: optional fields now serialize as `null` instead of being omitted. Fixes rejection by strict MCP clients.

---

## [0.2.3]

`subscribe(timeout_ms)` blocking mode — when `timeout_ms` is provided, blocks and returns accumulated data as a single result instead of fire-and-forget.

---

## [0.2.2]

MCP specification compliance audit.

**Added:** cursor-based pagination for `list_resources`/`list_resource_templates`, resource `size` metadata, `src/limits.rs` (centralized bounds), minimum-bound validation on all bounded inputs, cross-session subscribe test.

**Fixed:** pagination `next_cursor` always-`None` bug, concurrent `open` race condition, peer-disconnect panic in `stream_rx`, non-standard `"format": "uint"` in tool schemas.

---

## [0.2.1]

MCP 2025-11-25 compliance, CDC-ACM hardware fixes, port allowlist.

**Added:** protocol version bump to 2025-11-25, `resources/list_changed` capability + push notifications on `open`/`close`, port allowlist (`--allowlist`), stdio integration tests, hardware loopback tests.

**Fixed:** CDC-ACM read truncation (poll interval 5ms → 50ms).

---

## [0.2.0]

Project reset with an aggressive rewrite. Removed ~80% dead scaffolding and migrated to rmcp 1.7.

**Added:** `flush`, `set_dtr_rts`, `send_break`, `wait_for`, `subscribe`, `unsubscribe` tools; `serial://` resources; `diagnose_port` + `interactive_terminal` prompts; task cancellation; HTTP transport; `codec` module; `SerialIo` trait abstraction.

**Removed:** `src/session/` (815 LOC), `src/utils.rs` (506 LOC), `src/config.rs` (312 LOC), `clap`, `toml`, `anyhow`, and other unused dependencies.

---

## [0.1.0]

Initial release. Five tools: `list_ports`, `open`, `close`, `write`, `read`. STM32 demo firmware included.
