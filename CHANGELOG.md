# Changelog

| Version | Date | Highlights |
|---|---|---|
| [Unreleased](#unreleased) | — | Software-only test migration: XIAO/E83/hardware-loopback tests replaced by native_sim + USB/IP; CI now runs without any hardware |
| [0.5.0](#050) | 2026-06-06 | RX redesign (Plans 1-7): session pump, unified stop controller, match options, buffer budgets, silence timeout, context shaping |
| [0.4.1](#041) | 2026-06-04 | CI/release hardening, schema-validated config examples, docs cleanup |
| [0.4.0](#040) | 2026-06-04 | Crate rename to `serial-mcp`, `read_line` + `get_version` tools, text encoding, RX guard, flexible args |
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

## [Unreleased]

Migration to software-only validation. No physical hardware, board
bring-up, USB-serial adapters, or `pyocd`/`PicoProbe` workflows
required to validate the server. See `docs/SOFTWARE_ONLY_TEST_MIGRATION_PLAN.md`
for the staged plan.

**Added:**
- `tests/native_sim_connection_lifecycle.rs` — 6 new software-only
  tests covering named-connection bookkeeping in `list_connections`,
  `set_flow_control` round-trip, close-while-read behavior, and
  PTY reopen. Run with `--test-threads=1`.
- `docs/SOFTWARE_ONLY_TEST_MIGRATION_PLAN.md` capturing the staged
  removal of hardware-only tests, board files, and helpers.
- CI job `native_sim firmware + test` (already present) now validated
  end-to-end on ubuntu-latest without NOPASSWD sudoers for production
  runs; the USB/IP path remains opt-in.

**Removed:**
- `tests/xiao_ble_validation.rs` — XIAO BLE hardware test suite
- `tests/hardware_loopback.rs` — USB-serial loopback test suite
- `tests/e83_live_validation.rs` — E83 live board test suite
- `firmware/boards/xiao_ble.conf`, `xiao_ble_usb.conf`, `xiao_ble_usb.overlay`
- `firmware/pm_static.yml`
- `firmware/bin/fw-build-xiao`, `fw-build-xiao-usb`, `fw-flash-xiao`
- `firmware/bootloader/Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.hex`
- `firmware/UF2_BOOTLOADER_PLAN.md`, `firmware/UNIFIED_FIRMWARE_PLAN.md`
- `pyocd` and `segger-jlink` from `flake.nix`

**Changed:**
- `firmware/src/usb_cdc.c` simplified to native_sim legacy USB stack
  only. Device-next / GPREGRET / NVIC reset code paths removed.
- `firmware/src/main.c` and `firmware/src/usb_cdc.h` comments updated
  to reflect native_sim-only target.
- `firmware/AGENTS.md` rewritten to drop xiao_ble / UF2 / PicoProbe
  workflows.
- `firmware/prj.conf` comments updated to reference only native_sim
  conf fragments.

---

## [0.5.0]

Full RX subsystem redesign (Plans 1-7). Breaking internal change; no tool API removed except `wait_for`.

**Added:**
- Per-connection `RxSession` pump — a single background task reads from each serial port and fans bytes out to registered consumers. `read` and `subscribe` both consume from this pump; they never read the port directly and no longer race each other.
- `match` option on `read` and `subscribe` — stops when a byte pattern is found. Current matching mode is literal byte-substring; `pattern_encoding` controls how the pattern string is decoded (for example UTF-8 or hex). `read` returns the matched data; `subscribe` emits a stop notification with `matched=true`, `match_index`, and optional shaped context.
- `context_amount_of_matched_bytes` in match config — shapes pre-match context window by returning up to N bytes before the matched bytes, plus the matched bytes themselves.
- `no_new_rx_timeout_ms` on `read` and `subscribe` — silence timeout: stops when no new bytes arrive for the specified duration. Distinct from the wall-clock `timeout_ms`.
- `--max-program-buffered-bytes` and `--max-tool-buffered-bytes` CLI flags — global buffer budget caps. Each `read`/`subscribe` call reserves from the program budget and is bounded by the tool limit. Prevents runaway memory use under high-volume streams.
- `RxStopController` — shared stop-condition evaluator used by both `read` and `subscribe`. Guarantees identical stop semantics for timeout, silence, match, max-buffer, connection-closed, and peer-disconnect across all RX tools.
- Hardware integration tests for XIAO BLE (nRF52840 CDC-ACM, `tests/xiao_ble_validation.rs`) — 7 ignored tests covering match stop, silence timeout, buffer budget, and close-under-stream using the RTT feedback firmware's `spam` command.

**Fixed:**
- `subscribe(match=...)` never stopped — two bugs: (1) `RxStopController::push_data` fired `MaxBufferedBytes` when `max_bytes=0` (subscribe's unlimited mode uses 0 as sentinel); guarded with `self.max_bytes > 0`. (2) `stream_rx_via_session` called `record_data` (counters only) then consumed `match_result` in an outer `if let` before passing it to `push_data`, so the controller never saw the match; replaced with a single `push_data(n, n, match_result)` call mirroring the `read` path.

**Removed:**
- `wait_for` tool — superseded by `read(match=...)`. Removed along with dead helpers `read_bytes`, `read_until_pattern`, and `stream_rx`.

---

## [0.4.1]

Release workflow and docs cleanup.

**Added:** vendored schema validation for config examples via Rust integration tests and a daily upstream schema drift workflow.

**Changed:** release automation now runs after successful `main` CI and builds Linux x86_64, Linux ARM64, macOS ARM64, and Windows artifacts.

**Changed:** agent configuration docs now point to schema-backed examples and official docs, with stale examples removed.

**Removed:** shell-based config linting, loopback hardware tests, stale compliance docs, and editor-specific repo files.

---

## [0.4.0]

Crate renamed to `serial-mcp`. New tools, new encoding, input hardening, and ownership guard.

**Added:**
- `get_version` tool for querying package version and build commit
- `read_line` for line-delimited REPL and firmware-log workflows
- `text` encoding — like `utf8` but strips ANSI/VT100 escape sequences
- flexible numeric deserialization — tool args accept both JSON numbers and stringified numbers
- single-RX-owner guard — concurrent `read`/`read_line`/`wait_for`/`subscribe` on one connection fail fast with owner-specific errors
- explicit docs for exclusive serial-port opens and RX ownership

**Fixed:**
- `read_line` now preserves trailing buffered bytes for follow-up line reads
- concurrent RX operations no longer race each other
- UTF-8/text reads use lossy decoding for invalid byte sequences

**Changed:**
- crate renamed from `serial-mcp-server` to `serial-mcp`

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
