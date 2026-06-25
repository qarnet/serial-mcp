# Changelog

| Version | Date | Highlights |
|---|---|---|
| [Unreleased](#unreleased) | — | — |
| [0.6.2](#062) | 2026-06-25 | Schema fix: suppress non-standard `uint8`/`uint16` formats; expanded schema regression guards + AGENTS.md truth |
| [0.6.1](#061) | 2026-06-24 | RX refactor: shared framing sink, SerialHandler builder, config FromStr, dedup; docs cleanup |
| [0.6.0](#060) | 2026-06-20 | Frame decoding (4 modes + 3 parsers), regex/glob matching, auto-reconnect, event log, connection profiles, port identity, reconfigure, get_status, per-frame graceful degradation |
| [0.5.1](#051) | 2026-06-14 | Software-only test migration: native_sim PTY replaces all hardware tests |
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

## [0.6.2]

Patch release. Fixes the third recurrence of the schemars non-standard
`uint*` format regression and closes the test-coverage gaps that let it
slip through. No tool API or runtime behavior changes.

**Fixed — JSON Schema:**
- `PortInfo` (`vid`/`pid`/`interface`) and `FramingMode::LengthPrefixed::prefix_size`
  now carry `#[schemars(schema_with = ...)]` overrides. schemars 1.x was
  emitting non-standard `"format": "uint16"`/`"uint8"` keywords for these
  `u16`/`u8` fields, which validators (jsonschema, AJV, …) log as warnings
  and silently drop.
- History: `b12b09fd` and `bc37a0b0` fixed `u32`/`u64`/`usize` fields; this
  release covers `u8`/`u16`.

**Changed — regression guards (do not delete):**
- `serial::schema` module (src/serial.rs): 25 per-type tests via
  `check_schema!` macro scan every public `JsonSchema`-deriving struct for
  any `uint*` format keyword. Previously 14 types; now includes all 22 tool
  result types + `PortInfo`/`ConnectionStatus`/`Profile`/`ProfileSelector`.
- `verify_all_tool_schemas` and `tool_schemas_have_no_nonstandard_uint_formats`
  (src/tools/mod.rs): now cover all 22 `#[tool]` methods via a shared
  `all_tool_attrs()` list (previously 16). The uint-format scan now also
  covers `uint8`/`uint16` (previously only `uint`/`uint32`/`uint64`).

**Docs:**
- `src/schema_helpers.rs`: module-level doc explaining the rule, validator
  behavior, and full regression history with pointers to the regression tests.
- `AGENTS.md`: "Invariants easy to break" expanded to name `uint8`/`uint16`/
  `uint32`/`uint64`, state the required annotation, and point at the
  `serial::schema` tests. Test map now lists every test file (previously
  missing `allowlist`, `blob_resources`, `resource_subscriptions`,
  `tx_session`, `proptest`).
- `README.md`: corrected resource count from "4 (3 templates + 1 static)"
  to "5 (3 templates + 2 static)".

**Internal — TX flush tests:**
- Added `QueuedTxIo` mock backend coverage for fully-delivered,
  partially-queued, and flushed-before-delivery TX flush semantics.

---

## [0.6.1]

Internal refactor release. No tool API changes; all tool behavior and
error messages preserved byte-for-byte.

**Changed — RX framing:**
- New `src/tools/rx_consume.rs` module: `RxFrameSink` trait,
  `consume_frames`, `disconnect_state`. `read` and `subscribe` now route
  framed decoding through this shared driver instead of per-tool loops.
- `read` keeps later frames decoded from the same chunk after the first
  matching frame; `subscribe` stops on the matching frame and does not
  emit later frames from that chunk. This asymmetry is intentional.
- `subscribe` framing path loses ~100-line per-frame emit for-loop.
- Both tools share `validate_rx_request` preamble: encoding,
  connection, bounds, timeout, and matcher validation collapse into one
  path. Budget reservation and `poll_interval_ms` stay in callers
  (ordering sensitive).
- `read_bytes_via_session` cleaned up via `finish!` macro: 14 repeated
  `make_outcome` return tails collapsed; dead settle-phase decoder feed
  and post-loop flush removed (unreachable); `debug_assert!` invariant
  added at settle phase entry.

**Changed — SerialHandler construction:**
- `SerialHandler::builder()...build()` replaces 5 `with_manager*`
  telescoping constructors. Inject `connections`, `streams`, `security`,
  `budget` through the builder; `with_profiles()` stays as a post-build
  setter. `new()` is a thin wrapper over the builder.
- 3 call sites migrated (`main.rs` stdio/http, `tests/common`).

**Changed — Serial config parsing:**
- `FromStr` impls for `DataBits`, `StopBits`, `Parity`, `FlowControl` in
  `src/serial.rs`. 4 `parse_*` helpers and 3 `parse_string_*` duplicates
  deleted; all call sites (`open`, `reconfigure`, `set_flow_control`)
  route through `.parse()`. `reconfigure` now accepts mixed-case
  parity/flow_control (intended).

**Changed — Frame JSON serialization:**
- `ParsedFrameResult` twin enum deleted; `FrameResult.parsed` uses
  `framing::ParsedFrame` directly. `convert_parsed_frame` mapper
  deleted; `build_read_result` clones directly. Non-object JSON
  normalized to `Raw` in `JsonLinesParser`. Two hand-built `parsed_json`
  blocks in `stream_ops` replaced with `serde_json::to_value`.

**Changed — Error/lookup dedup:**
- `map_budget_err` helper extracted; used in `io_ops` and `stream_ops`.
- 7 `port_ops` connection lookups routed through `lookup_connection`.
- Dead mode block in `stream_ops.rs` removed.

**Added — Tests:**
- 11 `read_bytes_via_session` characterization tests (plain read,
  lifecycle, matcher, framing).
- 7 shared-validator unit tests.
- 4 builder characterization tests.
- 3 `consume_frames` unit tests + 1 characterization test.
- 2 `pty_subscribe_framing_*` characterization tests.
- Match-vs-`max_frames` priority tests (subscribe + read semantics).
- Matcher window reset per-frame test.
- Cross-chunk raw matcher test (pattern split across two `RxEvent::Data`
  chunks).
- Serialization shape regression tests.

**Removed:**
- `firmware/ZEPHYR_EMULATION_RESEARCH.md` — settled historical research
  doc. The native_sim + `touch` command decision already landed in
  `firmware/AGENTS.md` and the 0.5.1 changelog; the USB/IP CDC-ACM
  approach it explored was rejected. No live references elsewhere.
- `docs/SIMULATION_MATRIX.md`, `docs/TESTING.md`, top-level
  `FEATURES.md` — redundant with AGENTS.md / CHANGELOG; remaining
  feature backlog moved to `docs/development/FEATURES.md`.

**Fixed:**
- Wrong link (PR #23).

---

## [0.6.0]

Major feature release. 10 new tools (22 total), frame decoder, auto-reconnect,
event log, connection profiles, regex/glob matching, and port identity.

**Added — Tools (6 new):**
- `save_profile` / `delete_profile` — manage named port configurations
- `get_log` / `clear_log` / `export_log` — per-connection event log
- `reconnect` — manually trigger reconnection on an open connection

**Added — Tool enhancements:**
- `get_status` — connection introspection (state, counters, port info, reconnect attempts)
- `reconfigure` — hot serial port reconfiguration (baud, data bits, parity, flow control)
- `open` now accepts `reconnect_policy` for auto-reconnect configuration
- `list_ports` now returns VID/PID/serial number/transport for each port
- `read` and `subscribe` now accept `framing` option for frame decoding
- `read` and `subscribe` match option now supports `regex` and `glob` modes
- `read` result includes `frames`, `match_frame_index`, `frames_dropped`
- `subscribe` stop notification includes `match_frame_index`, `frames_emitted`
- `subscribe` emits per-frame notifications when framing is active
- `subscribe` flushes partial frames on close/timeout with `"partial": true`

**Added — Frame decoder (`src/framing.rs`):**
- 4 boundary detection modes: line, delimiter, length-prefixed, start/end marker
- 3 protocol parsers: AT command, JSON lines, shell prompt
- `max_frames` stop condition with `RxStopReason::MaxFrames`
- Partial frame flush on read end (incomplete data emitted as final frame)
- Per-frame graceful degradation (encoding failures skip frame, count drops)

**Added — Auto-reconnect:**
- `ConnectionState` enum (Open, Disconnected, Reconnecting)
- `ReconnectPolicy` struct (enabled, max_attempts, initial_delay, backoff)
- Background supervisor task with exponential backoff
- Read/subscribe loops pause during disconnect, resume on reconnect
- Exit immediately on disconnect when reconnect not configured

**Added — Event log (`src/log_buffer.rs`):**
- 19 event types (open, close, read, write, match, truncation, drops, etc.)
- Bounded ring buffer per connection
- `serial://connections/{id}/log` resource template

**Added — Connection profiles:**
- Save/load named port configurations
- Transport and hardware_id selector fields
- Forward-compatible fields (reconnect_policy, decoder, safety_policy)
- Atomic file writes via `tempfile::NamedTempFile::persist()`

**Added — Matching:**
- `regex` mode using `regex::bytes::Regex` on raw bytes
- `glob` mode with per-line whole-match via `glob::Pattern`
- When framing is active, match operates on decoded frame data (per-frame)

**Changed:**
- `subscribe` stop notification ordering: partial frame before stop notification
- Close handler waits for subscribe task to finish (`join_without_abort`)
- `build_read_result` uses `filter_map` for per-frame encoding (graceful degradation)
- `RxStopReason` enum: added `MaxFrames` variant
- `ReadResult` struct: added `match_frame_index`, `frames_dropped` fields
- `FrameResult` and `ParsedFrameResult` types for structured frame output

**Fixed:**
- Subscribe dropped partial frames on close/timeout (flush_partial now called)
- Subscribe silently ignored `match` option when `framing` was active
- `flush_partial` notification errors silently discarded (now logged + counted)
- Read/subscribe hung forever on disconnect without reconnect policy

**Dependencies:**
- Added `regex` crate for regex/glob matching
- Promoted `tempfile` from dev-dependency to production dependency

---

## [0.5.1]

Migration to software-only validation. No physical hardware, board
bring-up, USB-serial adapters, or `pyocd`/`PicoProbe` workflows
required to validate the server. All testing runs on the `native_sim`
POSIX emulator over PTY.

**Added:**
- `tests/native_sim_connection_lifecycle.rs` — 6 new software-only
  tests covering named-connection bookkeeping in `list_connections`,
  `set_flow_control` round-trip, close-while-read behavior, and
  PTY reopen. Run with `--test-threads=1`.
- `tests/native_sim_validation.rs` test 12: `native_bootloader_touch_exits_42` —
  sends `touch` command over PTY, verifies firmware exit(42).
- `firmware/src/command.c`: `touch` command triggers `exit(42)` for
  bootloader-entry validation.
- CI job `native_sim firmware + test` runs end-to-end on ubuntu-latest
  without NOPASSWD sudoers or kernel modules.

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
- `firmware/src/usb_cdc.c` and `firmware/src/usb_cdc.h` removed.
  Bootloader entry flow replaced with `touch` command on the PTY
  command channel — no USB CDC-ACM, USB/IP, or `vhci_hcd` required.
- `firmware/src/main.c` and `firmware/src/command.h` updated to
  implement the `touch` command and remove USB CDC references.
- `firmware/AGENTS.md` rewritten to drop USB variant, snippets,
  `fw-build-native-usb`, and USB/IP sections.
- `firmware/prj.conf` consolidated to full unified config (no
  snippets or `config/` directory needed).

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
