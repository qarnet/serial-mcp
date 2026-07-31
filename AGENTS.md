# AGENTS.md — serial-mcp

## Fast truth

- Root server: `src/main.rs` selects stdio vs HTTP transport, parses CLI limits, and mounts HTTP at `/mcp`.
- MCP surface lives in `src/server.rs`; tool handlers are split under `src/tools/`, prompts under `src/prompts/`, resources under `src/resources/`.
- `SerialHandler` is built via `SerialHandler::builder()...build()` (`src/server.rs`). The old `with_manager*` telescoping constructors are gone; `new()` is a thin wrapper over the builder. Inject `connections`, `streams`, `security`, `budget` through the builder; `with_profiles()` stays as a post-build setter.
- Shared RX framing lives in `src/tools/rx_consume.rs` (`consume_frames` + `RxFrameSink` trait + `disconnect_state`); both `read` and `subscribe` route framing through it, but their raw (no-framing) paths stay per-tool by design (see "Invariants easy to break").
- Connection lifecycle is in `src/serial.rs`; shared RX/TX coordination is in `src/rx_session.rs` (always-on pump + ring buffer), `src/tx_session.rs`, and `src/stop_controller.rs`. The pump appends all received bytes to `src/rx_ring.rs`; both `read` and `subscribe` read from the ring via cursors.
- Low-level shared primitives: `src/util.rs` (`find_subsequence`, the byte-substring search used by `framing` + `tools::helpers` via a `find_subslice` re-export alias) and `src/precedence.rs` (`resolve_field`, the four-layer framing/parser/protocol precedence helper shared by `io_ops` + `stream_ops`). Both `pub(crate)`.
- `build.rs` injects `GIT_HASH` / `GIT_HASH_AVAILABLE`.

## Commands worth using

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# focused runs
cargo test --lib <test_name>
cargo test --test <file_stem> <test_name>
cargo test --test serial_pty
cargo test --test http_integration
cargo test --test stdio_integration
cargo test --test config_schema_validation

# networked schema drift check
cargo test --locked --test config_schema_validation -- --ignored

# native_sim tests (needs firmware built first — see Firmware section)
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

- CI runs exactly: fmt -> build -> test -> clippy, plus `cargo test --locked --test config_schema_validation` on Ubuntu.
- CI and schema workflows set `RUSTFLAGS="-D warnings"`. Treat warnings as errors locally too.
- `nix flake check` is part of CI. On Nix, prefer `nix develop` before changing firmware or release workflow bits.

## Invariants easy to break

- Tool failures should usually become MCP tool results with `is_error: Some(true)`, not protocol-level `McpError`. Keep malformed-request errors separate from operational errors.
- All tool outputs need `output_schema` and `title`; `verify_all_tool_schemas` enforces this.
- Do not emit non-standard schema `"format": "uint"` / `"uint8"` / `"uint16"` /
  `"uint32"` / `"uint64"`; schemars 1.x emits these for unsigned integer
  fields and validators log a warning per call and drop the constraint.
  Every `uN` / `Option<uN>` field on a struct that derives `JsonSchema` MUST
  be annotated with
  `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]`
  (or `option_uint_schema` for `Option<uN>`). Regression tests live in
  `serial::schema` (`src/serial.rs`) — extend the `check_schema!` list when
  adding a new `JsonSchema`-deriving struct with unsigned integer fields.
  History: b12b09fd, bc37a0b0, and the PortInfo (vid/pid/interface) regression
  that slipped through because the old guard only checked uint/uint32/uint64.
- `open` must enforce allowlist checks before `ConnectionManager::open()`.
- Open/close changes must notify resource subscribers via `notify_resource_list_changed()`.
- `read` and `subscribe` share stop-reason vocabulary via `RxStopController`. Both read from the ring via cursors: `read` advances the shared cursor (unless `from` jumps it elsewhere first); `subscribe` uses a per-call private cursor via the `from` parameter (`{"type":"now"}`/`{"type":"cursor"}`/`{"type":"buffer_start"}`/`{"type":"offset","offset":N}`). Subscriptions do NOT move the shared read cursor. `read`'s `from` parameter resolves the start position and writes the shared cursor before reading (atomic seek+read).
- Both tools catch cross-chunk matches in raw mode because matcher state is sliding-window based. In framed mode both match per-frame, so patterns spanning frames are intentionally not matched.
- `bytes_returned` in both tools is cumulative emitted bytes. `read` match meta now uses the same definition as `subscribe`.
- `from` parameter on `read` and `subscribe` shares the `ReadFrom` enum: `{"type":"now"}` (default for subscribe, live edge), `{"type":"cursor"}` (default for `read`, shared read cursor), `{"type":"buffer_start"}` (oldest retained byte), or `{"type":"offset","offset":N}` (absolute). Replayed history flows through the same framing/match pipeline as live data. `read`'s `from` parameter resolves the start position and writes the shared cursor BEFORE reading (atomic seek+read). `subscribe` defaults to `{"type":"now"}` and does NOT move the shared cursor.
- Slow subscriptions observe `bytes_lost` gap in notifications — never silently die.
- Both tools hard-fail framing construction errors (`FrameDecoder::new`). Read already did; subscribe now validates before spawning the background task.
- Production code convention here: no `unwrap`/`expect`, no `println!`, no committed `todo!()` / `unimplemented!()`.
- `.lock().expect("X mutex poisoned")` for std Mutex; `unwrap` in tests.

## Frame pipeline (TX + RX framing, parsers, presets, profile defaults)

- `src/framing.rs` owns both RX and TX framing. RX: `RxFramingConfig` +
  `RxFramingMode` (line/delimiter/length_prefixed/start_end/slip/cobs),
  `FrameDecoder` (stateful, byte-driven), `FrameDecodeError`,
  `ParserConfig`/`ParserType`/`ParsedFrame`. TX: `TxFramingConfig` +
  `TxFramingMode` (mirrors RX minus `parser`/`max_frames`/
  `include_terminators`; SLIP + COBS; no `auto` line ending; adds `Nmea`
  for auto-checksum TX).
- `FrameDecoder::new(&RxFramingConfig, Option<&ParserConfig>)` — 2-arg; parser
  is a sibling, NOT nested in `RxFramingConfig`. `ReadArgs`/`SubscribeArgs` carry
  `rx_framing` + `rx_parser` + `protocol` as siblings. `WriteArgs` carries
  `tx_framing` + `protocol` (no parser).
- `FrameDecoder::push()` returns `PushOutcome { frames, frames_dropped,
  error }` — frames decoded before a stream-fatal error are preserved and
  dispatched to the sink BEFORE the error is surfaced (`consume_frames`
  dispatches first, returns `FrameOutcome::DecodeError` after). SLIP
  (malformed escape) **and COBS** (invalid code byte) produce stream-fatal
  errors; checksum mismatches with `validate: true` (NMEA `*XX`, Modbus LRC)
  are per-frame drop-and-count (increment `frames_dropped`, `warn!`, decoder
  continues — does NOT set `error`). Runtime decode errors (not construction
  errors) STOP both read and subscribe — there is NO resume-on-error in the
  loop (resync state in the decoder is defensive only; the loops stop on
  first stream-fatal error). This is a deliberate asymmetry exception vs
  the construction-error asymmetry below.
- `LineEnding::Auto` promotes to CR-split mode mid-stream when a bare `\r` is
  confirmed (next non-`\n` byte or stop flush). Per-call state — resets on the
  next read/subscribe. Confirmation timer reuses `no_new_rx_timeout_ms`; the
  decoder is byte-driven (no timer callback).
- `ProtocolPreset` (7 variants: `at_command`, `slip`, `json_lines`, `cobs`,
  `ndjson`, `nmea0183`, `modbus_ascii`) is a `#[serde(tag = "type")]` enum —
  JSON shape `{"type": "nmea0183"}`, NOT a bare string. Expands via
  `preset_tx_framing`/`preset_rx_framing`/`preset_rx_parser`.
- Framing/parser/protocol field precedence is FOUR layers per field: explicit
  call field > call-time `protocol` preset > connection default (from
  profile/open) > connection `protocol` preset. Resolution lives in
  `src/precedence.rs` (`resolve_field`), called from `io_ops::write`/`read` +
  `stream_ops::subscribe`. `ConnectionConfig` + `SerialConnection` store the
  defaults; accessors `*_default()`.
- `RxStopReason::FramingError` is a runtime decode-error stop reason (SLIP
  malformed escape, COBS invalid code). NOT a normal stop
  (`is_normal_stop` excludes it). `read` surfaces it as a normal tool result
  (`is_error: false`) with `stop_reason: "framing_error"`, an `error` field
  carrying the `FrameDecodeError` text, and a hex-fallback `data` field when
  the requested encoding can't represent the raw bytes (binary SLIP/COBS
  data under utf8 → falls back to hex with `encoding: "hex"`); subscribe as a
  final notification with `stop_reason: "framing_error"` + `error` field.
  Both carry partial data (frames decoded before the error + raw bytes).
- Both tools hard-fail framing construction errors (`FrameDecoder::new`).
  `subscribe` validates the decoder in the handler before spawning the
  background task — a bad config returns `Err(String)` (tool error), not
  a degraded raw-mode stream. This matches `read`'s behavior.

## Test map

- `cargo test --lib` covers core logic (incl. `serial::schema` uint-format regression tests).
- `tests/http_integration.rs` exercises real MCP HTTP transport in-process.
- `tests/serial_pty.rs` is real PTY serial I/O on Unix.
- `tests/stdio_integration.rs` spawns binary over stdin/stdout.
- `tests/protocol_emulator*.rs` are protocol hardening tests.
- `tests/allowlist.rs` — port allowlist enforcement via the HTTP harness.
- `tests/blob_resources.rs` — blob resources and resource templates.
- `tests/resource_subscriptions.rs` — MCP resource subscribe/unsubscribe protocol.
- `tests/tx_session.rs` — cross-module TxSession wiring.
- `tests/proptest.rs` — property-based and boundary-value tests.
- `tests/config_schema_validation.rs` validates generated schemas against vendored examples; ignored case fetches upstream schemas.
- `tests/native_sim_validation.rs` — native_sim firmware over PTY. 56 tests, < 13s, pure software. Env: `SERIAL_MCP_NATIVE_SIM_BIN` (default `build/native_sim/firmware/zephyr/zephyr.exe`). Thin wrapper; all tests + helpers live in `tests/native_sim_validation/unix.rs` (Unix-only via `#[cfg(unix)]` module gate), with an empty `windows.rs` stub for future Windows-specific tests.
- `tests/native_sim_connection_lifecycle.rs` — software-only lifecycle (6 tests): named connection, `set_flow_control`, close-while-read, reopen, touch-command bootloader entry. Run with `--test-threads=1`.
- There are no hardware-required tests in this repo. All test coverage is runnable on a normal Linux host.

## Firmware / NCS

- Read `firmware/AGENTS.md` before touching Zephyr code; root file only keeps top-level gotchas.
- Nordic toolchain env is owned by the `nix-nrf-dev` flake input (`mkNrfShell`): the dev shell itself stays clean (no `LD_LIBRARY_PATH`/`PYTHONHOME`/`PYTHONPATH`/`GIT_EXEC_PATH` pollution from the sdk-manager); the `west` wrapper loads `nrfutil sdk-manager toolchain env --ncs-version v3.3.0 --as-script sh` per command. The shell hook derives `ZEPHYR_BASE` and exposes firmware helpers on `PATH`.
- Use helpers instead of retyping wrappers:

```bash
fw-build-native
fw-run-native
```

- `native_sim` is a 32-bit host build (`-m32`). `nix-nrf-dev`'s `mkNrfShell` supplies multilib GCC; do not reintroduce "NixOS unsupported" guidance.
- The XIAO BLE nRF52840 target was removed. The test firmware now targets `native_sim` only.
- Do not switch firmware command channel away from `DT_CHOSEN(zephyr_console)`.
- native_sim tests need firmware built first: `fw-build-native`. Firmware lives in dedicated build tree `build/native_sim`.
- Firmware helpers also export `compile_commands.json` by default for LSP: writes `build/native_sim/firmware/compile_commands.json`.
- Firmware LSP routing lives in `firmware/.clangd`: all firmware C/H files use the single compile DB. Keep this aligned with the build dir.
- `opencode.json` runs `clangd` through `direnv exec .` with `--query-driver=/nix/store/*/bin/*` so Nix toolchain headers resolve. If opencode LSP regresses, check `opencode.json`, `firmware/.clangd`, then rebuild.

## Release workflow

- Release job derives tag from `Cargo.toml` version (`v<version>`), tags `main` automatically after CI success, uploads binaries, then publishes crate. Bumping package version has release consequences.
- Release artifacts are built for: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
- `server.json` is a registry template: it carries name/description/version only. The `packages` array (release-asset URLs + `fileSha256`) is generated at publish time by `publish-mcp-registry.yml` from the actual release binaries — never commit one (`tests/doc_drift.rs::server_json_omits_packages` enforces this). Version bump = `Cargo.toml` + the single top-level `server.json` version.

## Repo workflow

- Conventional commits used here: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`.
- Never add attribution footers or co-author lines.

## Orchestrator (xtask)

Single entry-point for building test assets + running all tests in order:

```bash
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test
cargo run --manifest-path xtask/Cargo.toml -- test-all
cargo run --manifest-path xtask/Cargo.toml -- print-paths
```

- `build-test-assets` — builds `serial-mcp` binary + native_sim firmware.
- `test` — runs unit tests + stdio, blob, native_sim validation, and native_sim lifecycle suites.
- `test-all` — same as `test` plus HTTP integration suite (spawned binary).
- `print-paths` — emits resolved test-asset paths for debugging.
- Both `test` and `test-all` pass `--test-threads=1` unless overridden.
- The native_sim firmware suites are run with `--ignored` because their tests carry `#[ignore = "requires native_sim firmware binary"]`.
- Non-firmware suites (stdio, blob, http) run without `--ignored`. The only non-firmware `#[ignore]` is `config_schema_validation::example_configs_match_latest_upstream_schemas` (network fetch; run via `cargo test --test config_schema_validation -- --ignored`).
- All test helpers (`tests/common/binaries.rs`, `tests/common/firmware.rs`, `tests/common/spawned.rs`) auto-build missing test assets on first use.

## v0.8.1 additions

- **Tool count: 25** (added `configure`, `transact`, `compute_checksum`). Update all references when adding/removing tools.
- **`configure` tool** — two modes: profile (persist defaults to TOML) and connection (mutate live connection defaults). Live mutation covers the four framing defaults (StdMutex<Option<T>> wrappers on `SerialConnection`), `reconnect_policy` (existing StdMutex), `max_buffered_bytes_default` (AtomicUsize), and `poll_interval_ms_default` (AtomicU64). `log_capacity`/`log_enabled` are profile-only — LogBuffer has no live setter.
- **`ProfileDefaults` got five new fields:** `max_buffered_bytes`, `poll_interval_ms`, `reconnect_policy`, `log_capacity`, `log_enabled`. These flow from profiles through `OpenArgs` → `ConnectionConfig` → `SerialConnection`.
- **Per-call `max_buffered_bytes` removed** from `ReadArgs` and `SubscribeArgs`. Per-call `poll_interval_ms` removed from `SubscribeArgs`. Both now come from connection defaults (mutable via `configure`).
- **`precedence::resolve_field`** changed signature: `conn_default` is now `Option<T>` (by value) instead of `Option<&T>` (borrowed). The framing-default accessors on `SerialConnection` return by value (cloned from StdMutex).
- **`compute_checksum` tool** — pure utility, no connection required. Algorithms: xor (NMEA) and lrc (Modbus ASCII). Lives in `src/tools/utility_ops.rs`.
- **`transact` tool** — write-then-await-response in one call. Default `from: "now"` to skip pre-write backlog. Composes existing write + read plumbing in `src/tools/io_ops.rs`.
- **`save_profile` `rx_buffer_size` bug fixed** — now snapshots from the live `RxSession` ring capacity instead of hardcoding `DEFAULT_RX_BUFFER_SIZE`. Signature changed: takes `rx_sessions: &Arc<RxSessionManager>`.
