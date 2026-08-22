# AGENTS.md — serial-mcp

## Fast truth

- Root server: `src/main.rs` selects stdio vs HTTP transport, parses CLI limits (`--profiles-path`, `--capture-dir` + capture quotas included), and mounts HTTP at `/mcp`.
- MCP surface lives in `src/server.rs`; tool handlers are split under `src/tools/`, prompts under `src/prompts/`, resources under `src/resources/`.
- MCP version policy (`src/mcp_protocol.rs`): the product-owned `SUPPORTED_PROTOCOLS` table is the SINGLE source for advertised versions, lifecycle admission, capability views, and cache shaping. Exactly two rows — `2026-07-28` (preferred, `DiscoverStateless`, `ImmediatePrivate` cache, subscriptions on) and permanent `2025-11-25` (`InitializeSession`, `Omit` cache, subscriptions off). Lookup is exact-match only (`policy_for`); `cache_fields_for(Option<ProtocolVersion>)` grants fields ONLY to rows with `CachePolicy::ImmediatePrivate` — never by date/range, and unknown/future versions inherit no policy. `supported_protocol_versions()` / preferred policy derive from the table, never from `ProtocolVersion::KNOWN_VERSIONS`.
- Dual MCP lifecycle (`src/server.rs`): preferred modern `2026-07-28` discovery/stateless requests (`server/discover` + self-contained per-request `_meta`) and compatible legacy `2025-11-25` initialize/session requests. `get_info()` serves the MODERN view (rmcp intersects `subscriptions/listen` filters against `get_info().capabilities`), `initialize()` returns the legacy view with subscription disabled, `discover()` the modern view with `resources.subscribe: true`. Common surface: tools/resources/prompts/completions (no logging, list-change, or tasks). Phase 3 implements `accepted_subscription_filter`/`listen` backed by one process-wide `ResourceEventHub` (`src/resource_events.rs`, capacity 256) shared by every stdio/HTTP handler and the port hotplug watcher; legacy `subscriptions/listen` stays `-32601`.
- `SerialHandler` is built via `SerialHandler::builder()...build()` (`src/server.rs`); the old `with_manager*` telescoping constructors are gone and `with_profiles()` is gone. The builder defaults `profile_store` to an ephemeral store and `capture_store` to disabled. `SerialHandler::new()` tries the OS default profile store and falls back to an ephemeral store with a warning. Production `main.rs` injects the resolved `profile_store`, a `CaptureStore` built from `--capture-dir` + quota flags (disabled by default), and `SystemPortProvider` through the builder.
- Profiles live in a process-wide `Arc<ProfileStore>` (`src/profile_store.rs`), shared by every stdio/HTTP session handler. `main.rs` resolves the path (`--profiles-path` or the OS user-config default, failing startup on an unavailable config dir or invalid file) and injects one store. Persistent mutations take a process-local async mutex, then `spawn_blocking` + an advisory lock on `<file>.lock`, reload-under-lock, `NamedTempFile` + `sync_all` + rename; the cache (shared `Arc<RwLock>`) is published from inside the blocking transaction right after the durable write, before the lock is released — so a cancelled awaiting tool still converges (cache never changes on failed write). `update_defaults_preserving_selector` returns the effective profile atomically (no racy second lookup). File format is schema-versioned TOML (v1 legacy auto-migrates in memory; `schema_version == 0` or `> 2` rejects startup). `Profile` carries `metadata` (revision/timestamps/generated/use_count) and a bounded `revisions` history (max 5 prior snapshots) for the profile-session feature.
- Shared RX framing lives in `src/tools/rx_consume.rs` (`consume_frames` + `RxFrameSink` trait + `disconnect_state`); `read` routes framing through it, but its raw (no-framing) path stays per-tool by design (see "Invariants easy to break").
- Connection lifecycle is in `src/serial/`; shared RX/TX coordination is in `src/rx_session.rs` (always-on pump + ring buffer), `src/tx_session.rs`, and `src/stop_controller.rs`. The pump appends all received bytes to `src/rx_ring.rs`; `read` reads from the ring via cursors. `ConnectionManager`'s connection-opening boundary is an injectable `ConnectionOpener` (`with_opener`): production uses `SystemConnectionOpener` (`SerialConnection::open`); tests inject in-memory backends (`tests/common/controlled.rs::ControlledConnectionOpener`) so the public MCP surface runs cross-platform without an OS serial port (macOS tty opens fail ENOTTY).
- Low-level shared primitives: `src/util.rs` (`find_subsequence`, the byte-substring search imported directly by `framing` and the matcher) and `src/precedence.rs` (`resolve_field`, the four-layer framing/parser/protocol precedence helper shared by `io_ops`). Both `pub(crate)`.
- `build.rs` injects `GIT_HASH` / `GIT_HASH_AVAILABLE` / `BUILD_TARGET`.

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

# Required Rust PTY replacement suite (native_sim remains differential oracle)
cargo test --locked --test device_fixture -- --test-threads=1
cargo test --locked --test device_command_parity -- --test-threads=1
cargo test --locked --test device_framing_parity -- --test-threads=1
cargo test --locked --test device_protocol_parity -- --test-threads=1
cargo test --locked --test device_parity_repeat phase_e_public_boundary_repeat_gate \
  -- --ignored --test-threads=1

# The one complete MCP version compatibility gate (local and CI share this
# exact path): locked binary build, lockfile-pinned MCP validation tooling
# install (`npm ci --ignore-scripts` in compat/mcp-validation — lifecycle
# scripts disabled, local binaries only, never npx), focused Rust
# protocol/stdio/subscription tests, real historical rmcp 1.7.0 client over
# stdio + HTTP, official conformance scenario sets for both versions, and the
# pinned Inspector smoke. Exact pins/scenarios live in the script and the
# committed npm lockfile, not in this file.
bash scripts/test-mcp-compat.sh

# standalone historical fixture fmt/clippy (exact rmcp =1.7.0, own lockfile)
cargo fmt --manifest-path compat/rmcp-1-client/Cargo.toml --all -- --check
cargo clippy --locked --manifest-path compat/rmcp-1-client/Cargo.toml \
  --all-targets --target-dir target/mcp-compat-rmcp-1 -- -D warnings

# registry manifest builder unittest suite (offline, deterministic)
python3 -m unittest discover -s scripts/tests -v
```

- CI runs exactly: fmt -> build -> test -> clippy, plus named Ubuntu gates for config-schema validation (`cargo test --locked --test config_schema_validation`), release/documentation consistency (`cargo test --locked --test doc_drift`), and the pinned `mcp-conformance` gate, which delegates ALL compatibility execution to `scripts/test-mcp-compat.sh` (lockfile-pinned conformance + Inspector tooling installed with `npm ci --ignore-scripts` and invoked as local binaries — no npx, no lifecycle scripts, no dynamic resolution — plus current typed/raw/stdio/subscription tests, actual rmcp 1.7.0 HTTP + stdio, both official conformance sets, and the Inspector 2.0.0 smoke — see the Phase 4 section).
- CI and schema workflows set `RUSTFLAGS="-D warnings"`. Treat warnings as errors locally too.
- `nix flake check` is part of CI. The source filter admits the complete `schemas/` tree (all four vendored schemas validate hermetically offline — missing fixtures fail) plus the `.github/workflows/` and `scripts/` trees (doc_drift and the registry-manifest builder tests read them at runtime — missing fixtures fail). The flake `checks` prove the filtered source ships every workflow fixture (`workflow-fixtures-present`) and that the builder unittest suite passes (`registry-manifest-builder-tests`). On Nix, prefer `nix develop` before changing firmware or release workflow bits.

## Invariants easy to break

- Tool failures should usually become MCP tool results with `is_error: Some(true)`, not protocol-level `McpError`. Keep malformed-request errors separate from operational errors.
- All tool outputs need `output_schema` and `title`; `verify_all_tool_schemas` enforces this.
- Do not emit non-standard schema `"format": "uint"` / `"uint8"` / `"uint16"` /
  `"uint32"` / `"uint64"`; schemars 1.x emits these for unsigned integer
  fields and validators log a warning per call and drop the constraint.
  Every `uN` / `Option<uN>` field on a struct that derives `JsonSchema` MUST
  be annotated with
  `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]`
  (or `option_uint_schema` for `Option<uN>`). If such a field is also
  omitted from serialized output via
  `#[serde(skip_serializing_if = "Option::is_none")]`, it MUST additionally
  carry `#[serde(default)]`: schemars 1.2.2 does not see through
  `schema_with` to the `Option` type, so without `default` the field lands
  in the schema's `required` array while serialization omits it (the
  PortInfo vid/pid/interface miss). Regression tests live in
  `serial::schema` (`src/serial/mod.rs`) — extend the `check_schema!` list when
  adding a new `JsonSchema`-deriving struct with unsigned integer fields.
  History: b12b09fd, bc37a0b0, and the PortInfo (vid/pid/interface) regression
  that slipped through because the old guard only checked uint/uint32/uint64.
- `open` must enforce allowlist checks before `ConnectionManager::open()`.
- `read` reads from the ring via the shared cursor: `read` advances the shared cursor (unless `from` jumps it elsewhere first). `read`'s `from` parameter resolves the start position and writes the shared cursor before reading (atomic seek+read).
- Raw `read` uses ONE matcher-owned bounded-window policy (`Matcher::push_bounded` in `src/match_config.rs`), used at read's initial-history and live paths. Retained window ≤ `max_buffered_bytes + overlap`, where literal overlap = `needle.len().saturating_sub(1)` and regex/glob overlap = 256 (`REGEX_GLOB_OVERLAP_ALLOWANCE`); retained length after every call never exceeds that limit, including after `NoMatch`. `Found(index)` is GLOBAL (total bytes fed since last `reset_window`): front truncation advances an internal base by exactly the bytes removed, so match indexes stay stream-relative. Literal pre-match context is matcher-owned: `shape_literal_match_context(global_index)` returns the payload shaped at match time — over the retained window for bounded raw paths (pre-match context capped at `min(requested_context, max_buffered_bytes)`) and over the matching frame's bytes for framed reads, which get full configured context bounded naturally by the frame (read's initial-history path keeps its exact hist-based shaping; read's live path uses the matcher's). Regex/glob store no shaped context. Glob truncation marks the first retained line partial when the byte before the new window start was not `\n`; an incomplete prefix is never treated as a complete line. `reset_window` (framed per-frame matching in `rx_consume`) clears window + base + saved context so frame indexes stay frame-local; `rx_consume` never uses bounded push. No wire fields/limits change.
- `bytes_returned` in `read` is cumulative emitted bytes.
- `from` parameter on `read` uses the `ReadFrom` enum: `{"type":"cursor"}` (default, shared read cursor), `{"type":"now"}` (live edge), `{"type":"buffer_start"}` (oldest retained byte), or `{"type":"offset","offset":N}` (absolute). Replayed history flows through the same framing/match pipeline as live data. `read`'s `from` parameter resolves the start position and writes the shared cursor BEFORE reading (atomic seek+read).
- `read` hard-fails framing construction errors (`FrameDecoder::new`).
- Production code convention here: no `unwrap`/`expect`, no `println!`, no committed `todo!()` / `unimplemented!()`.
- `.lock().expect("X mutex poisoned")` for std Mutex; `unwrap` in tests.

## Frame pipeline (TX + RX framing, parsers, presets, profile defaults)

- `src/framing/` owns both RX and TX framing (`config.rs` types + preset
  expansion, `decoder.rs` state machine, `codecs.rs` TX codecs, `parsers/`
  frame-content parsers; re-exported flat at `crate::framing::*`). RX:
  `RxFramingConfig` +
  `RxFramingMode` (line/delimiter/length_prefixed/start_end/slip/cobs),
  `FrameDecoder` (stateful, byte-driven), `FrameDecodeError`,
  `ParserConfig`/`ParserType`/`ParsedFrame`. TX: `TxFramingConfig` +
  `TxFramingMode` (mirrors RX minus `parser`/`max_frames`/
  `include_terminators`; SLIP + COBS; no `auto` line ending; adds `Nmea`
  for auto-checksum TX).
- `FrameDecoder::new(&RxFramingConfig, Option<&ParserConfig>)` — 2-arg; parser
  is a sibling, NOT nested in `RxFramingConfig`. `ReadArgs`/`TransactArgs`/
  `CaptureBootArgs` carry `rx_framing` + `rx_parser` + `protocol` as
  siblings. `WriteArgs` carries
  `tx_framing` + `protocol` (no parser).
- `FrameDecoder::push()` returns `PushOutcome { frames, frames_dropped,
  error }` — frames decoded before a stream-fatal error are preserved and
  dispatched to the sink BEFORE the error is surfaced (`consume_frames`
  dispatches first, returns `FrameOutcome::DecodeError` after). SLIP
  (malformed escape) **and COBS** (invalid code byte) produce stream-fatal
  errors; checksum mismatches with `validate: true` (NMEA `*XX`, Modbus LRC)
  are per-frame drop-and-count (increment `frames_dropped`, `warn!`, decoder
  continues — does NOT set `error`). Runtime decode errors (not construction
  errors) STOP `read` — there is NO resume-on-error in the
  loop (resync state in the decoder is defensive only; the loop stops on
  first stream-fatal error). This is a deliberate asymmetry exception vs
  the construction-error asymmetry below.
- `LineEnding::Auto` promotes to CR-split mode mid-stream when a bare `\r` is
  confirmed (next non-`\n` byte or stop flush). Per-call state — resets on the
  next read. Confirmation timer reuses `no_new_rx_timeout_ms`; the
  decoder is byte-driven (no timer callback).
- `ProtocolPreset` (7 variants: `at_command`, `slip`, `json_lines`, `cobs`,
  `ndjson`, `nmea0183`, `modbus_ascii`) is a `#[serde(tag = "type")]` enum —
  JSON shape `{"type": "nmea0183"}`, NOT a bare string. Expands via
  `preset_tx_framing`/`preset_rx_framing`/`preset_rx_parser`.
- Framing/parser/protocol field precedence is FOUR layers per field: explicit
  call field > call-time `protocol` preset > connection default (from
  profile/open) > connection `protocol` preset. Resolution lives in
  `src/precedence.rs` (`resolve_field`), called from `io_ops::write`/`read`.
  `ConnectionConfig` + `SerialConnection` store the
  defaults; accessors `*_default()`.
- **Lossless RX encoding:** `read` and `capture_boot`
  encode every RX payload via the shared `codec::encode_or_hex` primitive
  (`src/codec.rs`): try the requested encoding; on failure re-encode the SAME
  bytes as exact lowercase spaced hex and report the payload's effective
  `encoding` as `"hex"` (`EncodedPayload.fallback_reason` carries the original
  error text). A successful fallback warns (`tracing::warn!`) but is NEVER a
  drop: no drop log event, no drop counter, no `frames_dropped` increment.
  Applies to `read`, `transact`, and `capture_boot` raw bytes and each
  decoded frame (encoded independently from
  the REQUESTED encoding — a valid UTF-8 frame before malformed binary SLIP
  stays UTF-8 while the raw tail is hex). Never lossy UTF-8. Only a TRUE
  encode+hex failure counts as a drop: read-style paths become a tool-result
  construction error.
- `RxStopReason::FramingError` is a runtime decode-error stop reason (SLIP
  malformed escape, COBS invalid code). NOT a normal stop
  (`is_normal_stop` excludes it). `read` surfaces it as a normal tool result
  (`is_error: false`) with `stop_reason: "framing_error"`, an `error` field
  carrying the `FrameDecodeError` text, and a hex-fallback `data` field when
  the requested encoding can't represent the raw bytes (binary SLIP/COBS
  data under utf8 → falls back to hex with `encoding: "hex"`).
  The result carries partial data (frames decoded before the error + raw bytes).
- `read` hard-fails framing construction errors (`FrameDecoder::new`) — a bad
  config returns a tool error, not a degraded raw-mode stream.

## Test map

- `cargo test --lib` covers core logic (incl. `serial::schema` uint-format regression tests and `profile_store` migration/revision/metadata unit tests).
- `tests/http_integration.rs` exercises real MCP HTTP transport in-process, including profile persistence: real-binary restart (`profiles_survive_real_process_restart`), shared HTTP sessions, concurrent same-process and cross-process writers (`concurrent_*_keep_both`), legacy migration, startup rejection of corrupt/future files, and Unix failed-write preservation. In-process servers: simple tests use `TestServer::start()`/`start_with`; specialized dependencies (port provider, profiles path, capture store, security) go through `TestServerBuilder` (`tests/common/mod.rs`).
- `tests/serial_pty.rs` is real PTY serial I/O on Unix.
- `tests/stdio_integration.rs` spawns binary over stdin/stdout.
- `tests/protocol_emulator*.rs` are protocol hardening tests.
- `tests/allowlist.rs` — port allowlist enforcement via the HTTP harness.
- `tests/resource_subscriptions.rs` — modern `subscriptions/listen` resource
  subscriptions over the in-process HTTP harness: capability split,
  accepted-filter stripping/dedup, RX-append-after-readable + cursor
  immutability, listener independence/cancellation, lag recovery,
  open/close/state/log hints, port hotplug watcher (mutable provider).
- `tests/blob_resources.rs` — blob resources and resource templates.
- `tests/tx_session.rs` — cross-module TxSession wiring.
- `tests/proptest.rs` — property-based and boundary-value tests.
- `tests/doc_drift.rs` — prose-vs-code drift guards: tool count across README/Cargo.toml/server.json, protocol-preset mentions, tagged `from` wire forms, capture CLI option sync, FEATURES.md shipped-items absence, `server.json` package/version rules, a CHANGELOG release contract (release-table row + body heading for the Cargo package version, `## [Unreleased]` before the current release) with synthetic negative proofs for each rule, and the Phase 4 gate guards: exactly the four documented expected-failure IDs in `conformance/expected-failures.yaml`, the lockfile-pinned MCP validation npm tree (`compat/mcp-validation/` — private `package.json` with exact direct versions `@modelcontextprotocol/conformance@0.2.0-alpha.10` / `@modelcontextprotocol/inspector@2.0.0`, committed `package-lock.json` with exact lockfile-root deps plus per-package versions and `sha512-` integrity for both locked packages), the runner's `npm ci --ignore-scripts` install and direct `node_modules/.bin` invocations with no `npx` anywhere in the validation flow (delegation to the shared runner only — no duplicated scenario loops, no `--suite all`, no `server-session-lifecycle`), the exact version-indexed scenario sets parsed from the runner's quoted shell assignments (`SCENARIOS_2025_11_25` / `SCENARIOS_2026_07_28` with exact `--spec-version` values and `-2025-11-25` / `-2026-07-28` report suffixes), the historical fixture pin (exact `=1.7.0`, `default-features = false`, required client/transport features, single lock entry, checksum `0810a9f7…f4058e`), the contract/docs wiring (policy doc, README, FEATURES, runner, expected-failure count), the Inspector smoke script wiring (local locked binary default, no npx fallback), and the README dual-protocol compliance claim.
- `tests/protocol_compatibility.rs` — version-indexed compatibility matrix indexed by exact `TestProtocol::{V2026_07_28,V2025_11_25}` (a table-driven coverage lock compares `TestProtocol::ALL` against the raw `server/discover` `supportedVersions` wire output) plus the Phase 4 cache wire proofs: typed modern `ttlMs: Some(0)` / `cacheScope: Private` on every cacheable family and typed legacy absence; raw modern `ttlMs: 0` / `cacheScope: "private"` presence and raw legacy absence; `resultType` modern-present/legacy-absent; cursor-page behavior of the manual `tools/list`/`prompts/list` handlers. Raw expectations are fixture-local, never derived from production `src/mcp_protocol.rs`.
- `tests/config_schema_validation.rs` validates all three vendored example configs (Claude Code, Codex, opencode) hermetically and offline — the vendored `models.dev` document is registered in memory under its original URI, a no-network retriever fails on anything else, and missing/malformed schema or instance fixtures fail the run (no skip path). Only the ignored case fetches latest upstream schemas.
- `tests/native_sim_validation.rs` — native_sim firmware over PTY. 43 tests, pure software, fast (no hardware). Env: `SERIAL_MCP_NATIVE_SIM_BIN` (default `build/native_sim/firmware/zephyr/zephyr.exe`). Thin wrapper; all tests + helpers live in `tests/native_sim_validation/unix.rs` (Unix-only via `#[cfg(unix)]` module gate), with an empty `windows.rs` stub for future Windows-specific tests.
- `tests/native_sim_connection_lifecycle.rs` — software-only lifecycle (6 tests): named connection, `set_flow_control`, close-while-read, reopen, touch-command bootloader entry. Run with `--test-threads=1`.
- Required Rust PTY replacement targets are `tests/device_fixture.rs` (7), `tests/device_command_parity.rs` (19), `tests/device_framing_parity.rs` (8), and Linux-only `tests/device_protocol_parity.rs` (15). Linux x86_64 CI runs all four explicitly after ordinary `cargo test`; macOS arm64 runs fixture/command/framing (protocol target naturally has zero cfg-gated cases); Windows stays compile + controlled-backend only. Linux x86_64 also runs ignored `device_parity_repeat::phase_e_public_boundary_repeat_gate` explicitly: 100 fixed-order public MCP lifecycles, seed `0x50484153455f4545`, with bounded real fixture/server/client teardown. `native_sim` 43+6 remains temporary required differential oracle until Phase F.
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

- The release runs as a reusable workflow invoked from the trusted CI pipeline only: on a `push` to `refs/heads/main`, after every required CI job (`nix-flake`, `build-test`, `native-sim`) succeeds, the CI `release` job calls `.github/workflows/release.yml` with `mode: release` and the immutable `github.sha`; the `publish-mcp-registry` job then calls the registry workflow under the same trusted gate, passing `${{ needs.release.outputs.version }}` and the immutable `github.sha` — so a merge without a version bump still publishes the registry entry (release skips, registry still runs). Pull requests can run CI but can never reach either privileged call. Manual release testing uses the separate read-only `release-dry-run.yml` (`workflow_dispatch`, `contents: read`, `mode: dry-run` — builds the four platform binaries and uploads Actions artifacts, never touches releases or crates.io).
- The reusable release workflow derives `v<version>` from `Cargo.toml` and exposes it as the `workflow_call` `version` output. If that version already has a **published** GitHub release, the run skips entirely (a leftover draft from a failed run is retryable, not skipped). It creates a draft release, builds the four platform binaries, attaches them to the draft, then seals it (publishing is what creates the tag under immutable releases), and publishes the crate — an already-published crates.io version is a no-op skip.
- Bumping package version has release consequences.
- Release artifacts are built for: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`.
- `server.json` is a registry template: it carries name/description/version only. The `packages` array (release-asset URLs + `fileSha256`) is generated at publish time by `scripts/build_registry_manifest.py` from gh-downloaded release assets — never commit one (`tests/doc_drift.rs::server_json_omits_packages` enforces this). Version bump = `Cargo.toml` + the single top-level `server.json` version.
- **Registry publication** (`publish-mcp-registry.yml`): reusable `workflow_call` with required `ref` + strict `version` inputs. It checks out the current trusted `ref` (fetch-depth 0), validates `version` before any tag/path use, reads the historical template ONLY as data via `git show v<version>:server.json`, verifies the published GitHub release exists, keeps the exact registry-version idempotency gate, downloads the four assets fail-closed with `gh release download`, fetches release asset metadata (size + sha256 digest), builds the manifest with the offline builder (fails before output commit on version/template/metadata/size/digest problems; atomic write), validates the schema via the independent `nix shell .#jsonschema-cli` package (never `nix develop` — it would build serial-mcp), and publishes with mcp-publisher OIDC. `publish-mcp-registry-backfill.yml` (`workflow_dispatch` with a strict version input, `contents: read` + `id-token: write`, no secrets) calls the same publisher with `ref: ${{ github.sha }}` to publish already-released versions (e.g. 0.9.0/0.9.1) without checking out historical code.
- **Failure notification**: `ci.yml` `notify-release-failure` job runs `if: always() && trusted main push && (release failed || registry failed)` with `issues: write` only and no checkout; it creates or comments on one `[automation] Release pipeline failed` issue with the run URL/SHA and per-job results (values passed via env, never interpolated into shell source).
- **Builder tests**: `scripts/tests/test_build_registry_manifest.py` (standard-library unittest, offline/deterministic) covers the happy path and every failure class (invalid version, template mismatch/packages, tag mismatch, missing/duplicate/unexpected metadata, missing/empty/non-regular files, size mismatch, digest missing/bad/mismatched, HTTP-error analogue via zero-byte asset, fixed package order). Run by Ubuntu CI (`python3 -m unittest discover -s scripts/tests -v`) and by the `registry-manifest-builder-tests` flake check.

## Hardening (scheduled, not PR-gated)

`.github/workflows/hardening.yml` runs weekly (Sunday 06:00 UTC) and on
`workflow_dispatch` only — no push/PR trigger, `permissions: contents: read`,
`concurrency: { group: hardening, cancel-in-progress: true }`. Both jobs are
bounded; a hung or slow run must fail the job, not idle.

- **Fuzz smoke** — matrix over the three existing targets (`tool_call_json`,
  `codec_roundtrip`, `clamp_bounds`), `fail-fast: false`. Pinned toolchain
  `dtolnay/rust-toolchain@nightly-2026-07-15` (no floating nightly) + pinned
  `cargo install cargo-fuzz --locked --version 0.13.2`. The nightly pin is
  ALSO declared in the nested `fuzz/rust-toolchain.toml` (rustup resolves the
  nearest toolchain file, so every cargo/rustup invocation from `fuzz/` uses
  nightly — the declarative replacement for the old `RUSTUP_TOOLCHAIN` env
  hack, which was needed because the repo-root `rust-toolchain.toml` (1.97.1)
  overrides `rustup default` and made `cargo fuzz` reject
  `-Zsanitizer=address`). Keep the two pins in sync. Per target: libFuzzer
  `-max_total_time=300` wrapped in GNU `timeout 600` (must cover the cold
  nightly build, ~140s on a fresh runner — the old 360s cap killed the run
  with exit 124 before the 300s budget elapsed), job `timeout-minutes: 15`.
  Ubuntu + `libudev-dev pkg-config`. On failure, upload
  `fuzz/artifacts/<target>/` + `fuzz/corpus/<target>/` via
  `actions/upload-artifact@v7` (`if-no-files-found: warn`, `retention-days: 7`)
  — missing paths warn but never mask the original failure.
- **Mutation** — project Rust `dtolnay/rust-toolchain@1.97.1` (NOT nightly;
  cargo-fuzz/nightly are isolated fuzz tooling, not an MSRV bump) + pinned
  `cargo install cargo-mutants --locked --version 27.1.0`. Focused scope only:
  `--file src/checksums.rs` and `--file 'src/framing/parsers/**'` (quote the
  glob), with `--cargo-arg=--locked`, `--timeout 120`, `--jobs 4`, `-- --lib`.
  `--jobs 4` is REQUIRED: cargo-mutants' default is one job at a time, and
  the old `--jobs 2` made the 1500s cap unreachable (81 mutants × ~90s / 2
  jobs ≈ 61 min) so the run died with exit 124. The step also sets
  `CARGO_INCREMENTAL: "1"` to override the dtolnay action's global
  `CARGO_INCREMENTAL=0` — cargo-mutants reuses one scratch tree across all
  mutants specifically to benefit from incremental builds, and without it
  every mutant is a full ~50s rebuild (66 mutants never finished in 2400s).
  Baseline stays enabled. Whole command wrapped in GNU `timeout 2400`, job
  `timeout-minutes: 45`. Missed/time-out mutants fail the job — exit status
  is never suppressed. On failure upload `mutants.out/` (same warn/no-mask
  rule).
- These jobs are NOT a PR-required gate.
- Windows serial E2E is **deferred**: no privileged virtual-port driver
  installation on GitHub-hosted runners (com0com-style drivers are
  kernel-mode, typically test-signed, admin/reboot-sensitive). Decision and
  sources: `docs/development/windows-serial-e2e-investigation.md`. Revisit
  only with a pre-provisioned signed-driver runner or an approved design.

## Repo workflow

- Rust toolchain policy: CI, release, and schema-drift workflows install Rust 1.97.1 (`dtolnay/rust-toolchain@1.97.1`, each followed by a `rustc --version --verbose` report step); Nix derives the same version from `rust-toolchain.toml`. Bump both together.
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
- `test` — runs unit tests + required Rust PTY fixture/command/framing/protocol suites, then stdio, blob, native_sim validation, and native_sim lifecycle differential suites.
- `test-all` — same as `test` plus HTTP integration suite (spawned binary).
- `print-paths` — emits resolved test-asset paths for debugging.
- Both `test` and `test-all` pass `--test-threads=1` unless overridden.
- Required Rust PTY replacement suites run normally before native differential suites. The native_sim firmware suites are run with `--ignored` because their tests carry `#[ignore = "requires native_sim firmware binary"]`; ignored `device_parity_repeat` stays out of xtask so its 100 iterations run only in explicit Linux Phase E CI/verification commands.
- Non-firmware suites (stdio, blob, http) run without `--ignored`. The only non-firmware `#[ignore]` is `config_schema_validation::example_configs_match_latest_upstream_schemas` (network fetch; run via `cargo test --test config_schema_validation -- --ignored`).
- All test helpers (`tests/common/binaries.rs`, `tests/common/firmware.rs`, `tests/common/spawned.rs`) auto-build missing test assets on first use. `tests/common/firmware.rs` also owns the shared `NativeSimFirmware` process harness: build-on-demand, PTY-path discovery from stdout, background stdout drain, and kill-on-drop, with a Windows-compilable compile/runtime boundary.

## Connection defaults & profiles

- **`configure`** has two modes: profile (persist defaults to TOML) and connection (mutate live connection defaults). Live mutation covers the four framing defaults (StdMutex<Option<T>> wrappers on `SerialConnection`), `reconnect_policy` (existing StdMutex), and `max_buffered_bytes_default` (AtomicUsize). `log_capacity`/`log_enabled` are profile-only — LogBuffer has no live setter.
- **`ProfileDefaults`** carries `max_buffered_bytes`, `reconnect_policy`, `log_capacity`, `log_enabled` plus the serial-line fields; they flow profile → `OpenArgs` → `ConnectionConfig` → `SerialConnection`. Per-call `max_buffered_bytes` does NOT exist on `ReadArgs` — it comes from connection defaults (mutable via `configure`). The obsolete `poll_interval_ms` key from older profile files is ignored on load and dropped on the next durable rewrite (no schema-version bump).
- **`precedence::resolve_field`** takes `conn_default` by value (`Option<T>`); the framing-default accessors on `SerialConnection` return cloned values.
- **`transact`** = write-then-await-response in one call, read half defaults `from: {"type":"now"}` to skip pre-write backlog (`src/tools/io_ops.rs`). **`compute_checksum`** (xor/lrc) is a pure utility with no connection (`src/tools/utility_ops.rs`). Shared TX preparation (`decode_tx_payload` / `apply_tx_framing` / `TxFramingError`) lives in `io_ops.rs` and serves both `write` and `transact` — route future TX paths through it instead of duplicating decode/framing.
- **`PortProvider` trait + `SystemPortProvider`** (`src/serial/port_info.rs`): process-wide injectable port enumeration used by `list_ports`, bare `open` identity capture, `open_profile` matching, `serial://ports`, resource port counts, and completions. Tests inject `StaticPortProvider` (`tests/common/mod.rs`) whose `PortInfo.name` points at a real PTY slave.
- **Open overlay:** `OpenArgs` default-bearing fields are `Option<T>` with `#[serde(default)]` plus `profile_mode` (`auto`/`none`); omitted baud resolves to 115200. Resolution lives in `tools::helpers::{OpenOverlay, ResolvedOpenSettings}` — explicit field > selected profile default > built-in default; `from_profile` compares for `dirty`.
- **Session plan + binding:** `port_ops::open` decides a `SessionPlan` (disabled/transient/selected/explicit/generate) BEFORE hardware open; `open_connection` opens, starts RX, and only then marks used / creates the generated profile / attaches the binding. `ActiveProfileBinding` (StdMutex on `SerialConnection`) converts losslessly to wire `ProfileSessionResult` (`src/profiles.rs`) on `OpenResult`, `GetStatusResult`, `ConnectionSummary`. Never close a working port on profile-metadata failure — surface `last_persistence_error`. `open_connection` obtains the `Arc<SerialConnection>` once after hardware open and errors if it is missing; every successful public open returns `OpenResult.profile: Some(...)`; selected-profile `dirty` is computed BEFORE hardware open; invalid profile defaults fail the call instead of mapping to clean.
- **Identity rules:** `IdentityConfidence` High = USB + VID + PID + non-empty serial (+ interface); Medium = USB VID/PID only; Low = other identity; None = path-only. Automatic reuse only for High and only when the high fingerprint is unique among live ports. Canonical generated selector = transport/VID/PID/serial/interface only. Pure helpers in `src/profiles.rs`: `high_identity`, `identity_confidence`, `canonical_high_selector`, `selector_matches_high_identity`, `rank_candidates` (None sorts oldest), `normalize_generated_label` (lowercase, run→`-`, ≤32 chars, `serial-device` fallback), `allocate_generated_name` (base, -2, -3, ...).
- **Store (`src/profile_store.rs`):** `resolve_automatic` (fresh read under advisory lock, republishes cache, unique-max-last-used winner or `ambiguous` + candidate names), `create_generated` (atomic name allocation, revision 1 / use_count 1), `mark_used` (use_count+1, monotonic `max(now, max_existing+1)` timestamp, NO revision/history bump). Read path is `run_read`; mutations use `run_mutation` / `run_conditional_mutation` — a `None` apply result means no file write BUT the fresh disk state is still published to the cache; an apply error also refreshes the cache from the fresh disk state so CAS conflicts and external writers stay observable.
- **`open_profile`** errors when its selector matches more than one live port (was first-match), marks the profile used (explicit source), and reports `identity_confidence(matched PortInfo)` — weak selectors are an explicit caller choice, not a hardcoded `high`.
- **`list_profiles`** exposes `metadata` + `revisions` (capped at 5 prior snapshots) in `ProfileSummary`. **`save_profile`** snapshots `rx_buffer_size` from `SerialConnection::rx_buffer_size()` (stored at open; no handler-local `RxSessionManager`) and promotes generated-bound connections: a NEW name creates a user-owned profile (`generated=false`); overwriting an existing name keeps the target profile's generated flag.
- **`rollback_profile`** (`src/tools/port_ops.rs`): restores a retained prior revision as a new monotonic revision via `ProfileStore::rollback` (CAS on `expected_revision`; evicted/missing target = tool error, file unchanged). Live hardware is never touched: same-process bound connections are marked stale+dirty and counted in `active_connections_unchanged`.
- **Write-through learning** lives in `src/learning.rs` (`learning::learn`): no/non-persistent binding → `Transient` (no store call); CAS no-op → `NotNeeded` (no file write); CAS changed → `Persisted` (binding revision bumped, dirty/stale/error cleared); store failure/conflict → `Failed` (binding dirty; `stale=true` only for revision-conflict/missing errors — plain I/O failure stays retryable). `ProfileStore::update_learned_defaults` detects no-ops via `ProfileDefaults: PartialEq` (`changed=false` WITHOUT rewriting the file) and names profile + expected + actual revision on conflict.
- **Durable ops hold the connection `learning_lock`** across live mutation → `effective_defaults()` snapshot → CAS → binding update: `reconfigure`, `set_flow_control`, connection-mode `configure`, clean `close` (snapshot retries dirty state after hardware close; failure never reopens hardware or errors the close), and dirty open-override learning (in `open_connection`, before `OpenResult` returns).
- **Result fields:** `OpenResult.profile_persistence`; `ReconfigureResult`/`SetFlowControlResult`/`ConfigureResult`/`CloseResult` carry additive `profile` + `profile_persistence`. Hardware success + persistence failure = `is_error != true` with `state="failed"`; hardware failure keeps the existing tool error and performs no store call.
- **`delete_profile`** refuses while any same-process open connection binds the profile (error lists connection IDs).
- **Never persisted:** DTR/RTS, BREAK, read cursor, flush, payloads/encoding/match, per-call read/write/transact framing/parser/protocol overrides, health/reconnect counters/logs.

## Dual MCP lifecycle (Phase 2 + Phase 3 subscriptions)

- `src/server.rs` overrides `ServerHandler::supported_protocol_versions()`
  with the product slice `[V_2026_07_28, V_2025_11_25]` (NOT
  `ProtocolVersion::KNOWN_VERSIONS`), `discover()` with
  `DiscoverResult::from_server_info` (ordered versions + the modern
  discovery info view), and `initialize()` with a version gate: only
  `V_2025_11_25` may initialize (anything else returns
  `method_not_found::<InitializeResultMethod>` BEFORE peer bookkeeping),
  then `context.peer.set_peer_info(request.clone())` + the legacy info
  view — peer bookkeeping must not be dropped. `get_info()` returns the
  MODERN `2026-07-28` view (rmcp intersects `subscriptions/listen` filters
  against `get_info().capabilities`, so it must advertise the modern
  `resources.subscribe` capability). Capability views are small pure
  helpers (`common_capabilities`, `modern_capabilities`,
  `modern_discovery_info`, `legacy_initialize_info`); modern has
  `resources.subscribe: true`, legacy stays the common set.
- Modern stateless HTTP requests additionally require SEP-2243
  `Mcp-Method`/`Mcp-Name` headers and an `MCP-Protocol-Version` header
  matching the request `_meta` (rmcp 3.0.1 transport enforcement). A
  modern `initialize` is rejected by the handler and rmcp maps the
  `-32601` through the stateless routing to HTTP 404 (direct JSON, no
  `Mcp-Session-Id` header). Modern `ping`/`logging/setLevel`/
  `resources/subscribe`/`resources/unsubscribe`
  are `-32601` mapped to HTTP 404; legacy keeps `ping` working and
  `resources/subscribe`/`resources/unsubscribe`/`subscriptions/listen`
  at `-32601` inside SSE 200 responses (modern `subscriptions/listen` is
  implemented — Phase 3). Modern unknown resources are
  remapped to `-32602` (SEP-2164); legacy keeps `-32002`. Cache policy
  (`ttlMs`/`cacheScope`) is Phase 4 scope — see the Phase 4 section below.
- **Resource subscriptions (Phase 3):** one process-wide
  `ResourceEventHub` (`src/resource_events.rs`, `ResourceEvent::Updated`,
  capacity 256, synchronous non-blocking `publish_updated`) is created in
  production `main.rs` and injected through `SerialHandlerOptions`/builder
  into every stdio/HTTP handler plus the `PortWatcher` (1s poll, canonical
  sorted `PortInfo` snapshots, first-success baseline, no false updates on
  reorder/unchanged/error, recovery against retained baseline,
  `shutdown_and_join`). `accepted_subscription_filter` keeps valid,
  deduplicated concrete URIs (`serial://ports`, `serial://connections`,
  detail/raw/log via `is_subscribable_uri` round-trip) in first-request
  order and strips list-change flags/templates/unknown URIs; `listen`
  notifies matching `Updated(uri)` events, ignores unrelated ones, and on
  broadcast lag notifies every accepted URI once — never blocking the
  publisher or pump. **rmcp 3.0.1 ack artifact (documented, not patched):**
  the wire acknowledgement may echo a repeated requested VALID URI because
  rmcp computes the final accepted filter via
  `requested.intersection(&candidate).intersection(&advertised)`, both
  left-biased over the requested list. Handler/listener semantics always
  deduplicate (first-occurrence order): `accepted_subscription_filter`
  returns the deduplicated candidate, and `listen` re-deduplicates
  `context.accepted()` so normal matching and lag recovery never emit
  duplicate hints. Tests assert the acknowledged URI SET + first-occurrence
  order equal the deduplicated accepted set and explicitly permit the raw
  Vec echo; `repeated_requested_uri_does_not_cause_duplicate_lag_recovery_notifications`
  proves no duplicate recovery hints. The RX pump publishes detail/raw/log hints AFTER each
  successful `ring.append` and OUTSIDE the `pump_gate`
  (`pump_publishes_resource_hints_only_after_ring_append` unit proof).
  Tool paths publish after successful behavior only (open/close →
  connections+detail; reconfigure/set_flow_control/reconnect/connection
  configure/set_dtr_rts/send_break/write/transact → detail; clear_log →
  log; flush input/both → detail+raw). Notifications are hints — no
  payloads, no cursor movement. Modern HTTP is STATELESS: a fresh
  `SerialHandler` serves each request, so every process-wide dependency
  (`ConnectionManager`, `ProfileStore`, `CaptureStore`, `PortProvider`,
  hub, `RxSessionManager` — ring + pump + shared cursor) must be injected
  and shared — never handler-local. One `Arc<RxSessionManager>` per server
  process (built from the shared budget+hub) is injected through
  `SerialHandlerOptions::rx_sessions`/builder and cloned into every handler
  factory; `build()` constructs one only when none was injected.
  `stateless_requests_share_session_ring_and_cursor` proves two distinct
  stateless requests observe the same session/ring/cursor. Proofs:
  `tests/resource_subscriptions.rs` (typed modern clients over real
  in-process HTTP), `tests/protocol_compatibility.rs` raw legacy listen
  `-32601`.
- Proofs: `tests/protocol_compatibility.rs` (typed discover/initialize
  matrix + raw-wire status/code/header assertions against the spawned
  binary), stdio lifecycle tests in `tests/stdio_integration.rs`, and
  lifecycle helpers (`TestProtocol` with exact `V2026_07_28`/`V2025_11_25`
  variants, `VersionedClientHandler`, `connect_protocol_client` +
  spawned mirrors) in `tests/common/{mod,spawned}.rs`.

## Cache compliance + pinned conformance gates (Phase 4)

The full scenario/pin matrix (exact per-version conformance scenario sets,
package pins, report layout) lives in
`docs/development/mcp-version-compatibility-policy.md`; this section keeps
only the implementation invariants an agent must not break.

- **Version-correct SEP-2549 cache fields.** ONLY the explicit `2026-07-28`
  policy row receives `ttlMs: 0` / `cacheScope: "private"` on every cacheable
  family: `tools/list`, `resources/list`, `resources/templates/list`,
  `resources/read` complete results for every URI kind, and `prompts/list`.
  Legacy `2025-11-25` peers see NEITHER field — rmcp strips `resultType` for
  legacy but does NOT strip cache fields, so the server omits them itself.
  The gate is one pure helper `cache_fields_for(Option<ProtocolVersion>)` in
  `src/mcp_protocol.rs` (exact policy-row match — never a `2026-07-28+`
  date/range rule); a second helper `read_result_with_cache_fields` in
  `src/server.rs` applies the fields to read results. Tool calls,
  `prompts/get`, completion, and discovery (rmcp's own required zero/private
  fields) have no added cache fields. `tools/list`/`prompts/list` are
  explicit handlers over the SAME routers (`Self::tool_router()`/
  `Self::prompt_router()`) with cursor pagination (PAGE_SIZE 100,
  `paginate`). **Do not re-add `#[prompt_handler]`**: rmcp-macros 3.1.0
  unconditionally REPLACES any `list_prompts`/`get_prompt` in the annotated
  block (unlike `#[tool_handler]`'s has-method check), which would silently
  drop the cache fields — the two methods stay hand-written against the
  router. Wire proofs: `tests/protocol_compatibility.rs` (typed modern
  fields + typed legacy absence + raw modern `ttlMs:0`/
  `cacheScope:"private"` + raw legacy absence + cursor-page tests for the
  manual list handlers).
- **Single shared runner + expected-failure hard gate.** ALL compatibility
  execution — local and CI — delegates to `scripts/test-mcp-compat.sh`
  (`set -euo pipefail`, GNU `timeout` per fixture/conformance invocation),
  which first installs the lockfile-pinned validation tooling with
  `npm ci --ignore-scripts` and then invokes the local `node_modules/.bin`
  binaries directly (no npx);
  the CI `mcp-conformance` job owns only environment setup, the time bound,
  and report upload — no duplicated scenario loops, no `--suite all`.
  `conformance/expected-failures.yaml` baselines exactly the four documented
  fixture-dependent checks in per-check `<scenario>:<check-id>` form — a
  baseline entry that starts passing FAILS the run as stale; any other
  failure is an unexpected regression. Never add fixture endpoints to
  serial-mcp; runner exit status is never suppressed.
- **Historical rmcp 1.7.0 client fixture** (`compat/rmcp-1-client/`, exact
  `rmcp = "=1.7.0"` with `default-features = false` and only
  `client`/`transport-child-process`/`transport-streamable-http-client-reqwest`
  features, own committed lockfile, `publish = false`): a standalone package
  compiled against the pre-migration SDK proving the CURRENT server
  interoperates with a real historical client over BOTH HTTP and stdio
  (negotiated `2025-11-25`, server identity, exact 25-tool surface,
  resources/templates/prompts, `compute_checksum` → `111`/`6F`). The lock's
  single rmcp entry resolves `1.7.0` with checksum
  `0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e`
  (drift-guarded). It never depends on serial-mcp internals.
- **Lockfile-pinned MCP validation tooling** (`compat/mcp-validation/`):
  conformance and Inspector are installed ONLY from the committed
  `package-lock.json` (private `package.json`, exact direct versions
  `@modelcontextprotocol/conformance@0.2.0-alpha.10` and
  `@modelcontextprotocol/inspector@2.0.0` — no `^`/`~`/tags/ranges, every
  transitive version + `sha512-` integrity hash locked) via
  `npm ci --ignore-scripts` — lifecycle scripts are NEVER permitted to run
  (preinstall supply-chain hardening), and validation never resolves packages
  dynamically with `npx`. The runner invokes the project's local
  `node_modules/.bin/conformance` / `.bin/mcp-inspector` binaries directly.
  `node_modules/` stays gitignored; only lockfile semantics are committed.
- **Pinned Inspector 2.0.0 interoperability smoke** — interoperability, NOT
  conformance (named separately in the same CI job):
  `node scripts/inspector-smoke.mjs <server-url>` — Node-stdlib-only, exact
  installed binary (`INSPECTOR_CMD`/`--inspector-cmd`) with the lockfile-pinned
  `compat/mcp-validation/node_modules/.bin/mcp-inspector` as the default (no
  npx fallback, no dynamic resolution), per-command hard
  timeout, parses `--format json`, noninteractive
  (`MCP_AUTO_OPEN_ENABLED=false`, bounded `--connect-timeout`, non-TTY — no
  `--stored-auth-only`). It writes a temp session config with
  `protocolEra: "modern"` (the Inspector's ad-hoc `--server-url` default is
  legacy) and asserts the modern surface — identity, negotiated `2026-07-28`,
  exactly 25 unique tools, both resources, both prompts, `compute_checksum`
  → raw `111` / hex `6F`. Any assertion failure or nonzero CLI exit fails
  the script (hard gate).
- **Future protocol admission invariant.** A new MCP version is added only
  through one exact `ProtocolPolicy` row plus a complete test row (typed +
  raw-wire + stdio + conformance at the exact `--spec-version` + drift
  expectations). Adding a version never mutates or evicts another row, never
  enables cache fields by date, and never inherits support from
  `ProtocolVersion::KNOWN_VERSIONS`. `2025-11-25` is PERMANENT product
  compatibility — its row, fixture, raw-wire tests, conformance set, and
  drift guards must not be removed or weakened by a future protocol or rmcp
  update. Pre-`2025-11-25` revisions stay unsupported (demand-driven feature
  idea in `FEATURES.md` only).

## Discovery & evaluation

- **`list_ports` previews profile selection**: `ListPortsResult.profile_matches` parallels `ports` (same length/order, always serialized). Per-port: `confidence` + `outcome` (`selected`/`ambiguous`/`ineligible`/`duplicate`/`none`), `selected_profile`, and ordered `candidates` (name/generated/revision/last_used_at_ms). Preview is read-only — no `mark_used`, no file mutation. Pure computation in `port_ops::compute_profile_matches(ports, profiles)`: one `ProfileStore::list_fresh()` per `list_ports` call (corrupt store = tool error); high identity reuses the identity rules exactly (unique max `last_used_at_ms`, `None` sorts oldest, equal top rank = `Ambiguous`; name is display-only and never breaks a tie); duplicate live fingerprints = `Duplicate` for every such port; weak identity lists explicitly matching non-empty selectors as `Ineligible`. The `serial://ports` resource serves the same map.
- **Decision-tree teaching**: server `instructions` + the 12 common tool descriptions + README flow + both prompts teach `list_ports` → bare `open` → `transact`/`read`/`write` → inspect `profile`/`profile_persistence` → `open_profile` only for explicit choice/weak identity, `rollback_profile` for recovery → escalate to framing/cursor/reconnect/line-control/log tools only when needed. `from` wire examples stay tagged (`{"type":"now"}` etc.) — no string shorthand.
- **Tool count: 25** — `server::tool_catalog()` returns the exact 25 `rmcp::model::Tool` attrs served by MCP (the `subscribe`/`unsubscribe` tools were removed with MCP logging in the rmcp 3 migration); schema tests and the xtask evaluator consume it (exact-count test `tool_catalog_has_exactly_twenty_five_tools` guards drift). Update all references when adding/removing tools.
- **Evaluator**: `cargo run --manifest-path xtask/Cargo.toml -- agent-eval [--output-dir PATH] [--baseline PATH] [--write-baseline PATH]` — deterministic catalog + scenario metrics under `target/agent-interface-eval/` (`report.json`/`report.md`), no network/user config/timestamps. Committed baseline `docs/development/agent-interface-baseline.json` (26 tools / 258964 bytes) is HISTORICAL — it measures the pre-`capture_boot` catalog; the consolidated current report lives in `docs/development/agent-interface-evaluation.md` (25 tools / 268802 bytes). Thresholds and yes/no decisions (automatic profiles + `transact` + atomic `capture_boot` accepted; shorthand/recipes/versioned facade rejected) are computed by the evaluator from fixed rules. Modeled (non-implemented) candidates are marked `modeled` with their expansion into current calls.

## Atomic boot capture

- **`capture_boot`** (`src/tools/control_ops.rs`): one bounded operation — purge unread OS input, mark the RX live edge atomically, optionally pulse DTR/RTS, then capture ONLY post-mark bytes through the existing read pipeline with a PRIVATE cursor. `reset=null` = arm-only capture (lines never touched). Result stays in memory (connection `max_buffered_bytes` bounds it); no file output. `read.from_offset` equals `mark_offset` unless the ring wrapped (then `bytes_lost` reports it). Omitted/null `timeout_ms` resolves to a bounded 5000ms default; total op bounded by hold + settle + read timeout. Capture is transient: no profile learning.
- **Pump gate (`src/rx_session.rs`):** the pump holds `pump_gate` across one complete read + ring append and releases before disconnect pause/sleep; `capture_boot` acquires it via `pump_gate_guard()` for the purge→mark→assert sequence, so a byte physically read before the reset can never append after the mark. Deterministic unit test (`pump_holds_gate_across_inflight_read_and_ring_append`): while the pump's read is in flight the gate cannot be acquired and the ring stays empty; a gate acquisition succeeds only once the read's bytes are appended.
- **Line-control lock (`src/serial/connection.rs`):** per-connection `control_lock`; public `set_dtr_rts` takes it, crate-private `set_dtr_rts_unlocked` + `control_lock()` accessor cover capture's whole assert/hold/release sequence — a concurrent `set_dtr_rts` cannot interleave inside a pulse. The lock is scoped to the pulse only, dropped before settle/read so other line-control callers are not blocked for the whole capture.
- **Release guarantee:** `ResetReleaseGuard` (modeled on `BreakResetGuard`) is armed with the configured release state BEFORE assertion; every explicit path releases and disarms ONLY on success (`release_reset_lines`); its drop — after the pulse-scoped control guard in the unwind order — spawns a best-effort release through the PUBLIC `set_dtr_rts` (queued on the control lock). No unconditional disarm: a swallowed release failure on the cancellation path keeps the guard armed so drop retries. A closed/disconnected port counts as released. Assertion/release I/O failure = tool error with cleanup attempted.
- **Private cursor extraction (`src/tools/read_loop.rs`):** `read_from_private_cursor(session, initial_cursor, ...) -> (ReadOutcome, final_cursor)` is the extracted read core; the shared wrapper reads the shared cursor, delegates, and applies the returned cursor. `read`/`transact` behavior unchanged; capture starts at its mark and discards the final cursor.
- **Cancellation:** request-scoped `notifications/cancelled` during hold/settle releases lines first, then routes the already-cancelled token through the private read path → structured `stop_reason="cancelled"` result with offsets (unit-tested in read_loop.rs; rmcp's client discards post-cancel responses, so HTTP tests assert the observable release + control-lock release).
- **Controlled backend tests** (`tests/common/controlled.rs` + `tests/http_integration.rs`): real HTTP MCP + injected `ControlledIo` recording line transitions and injecting RX bytes synchronously at assertion/release. Covers: stale exclusion + cursor/history preservation, immediate-bytes match stop, cancellation release, assertion/release failure cleanup, invalid-framing-before-lines, runtime SLIP error, NDJSON + hex/base64, silence + wall timeouts, disconnect (`connection_closed`), ring wrap `bytes_lost`, concurrent `set_dtr_rts` serialization, arm-only. PTY arm-only test in `tests/serial_pty.rs`; native_sim arm-only test in `tests/native_sim_validation/unix.rs` (honest about no DTR observation on native_sim's PTY UART).

## Safe persistent capture

- **`CaptureStore` (`src/capture_store.rs`)** — process-wide persistent capture store, disabled by default (`CaptureStore::disabled()`; library/builder default). Production `main.rs` builds one `Arc<CaptureStore>` from `--capture-dir` + quota flags and injects it through `SerialHandlerOptions`/builder into every stdio/HTTP handler (`.capture_store(...)`). `CaptureStore::open` validates: absolute path, existing directory, not itself a symlink (via `symlink_metadata`), limits all `> 0` with `max_file_bytes <= max_total_bytes`, lock path regular non-symlink, and proves the advisory lock works (lock + unlock at startup). Root is canonicalized once. `write_new` runs blocking work in `spawn_blocking` under a process-local async mutex: exclusive advisory root lock (`.serial-mcp-captures.lock` via existing `fs2`), fresh scan of direct children (managed `.jsonl` regular files; lock + `.serial-mcp-capture-*` temp prefix ignored; symlink entries with managed names rejected, never followed; unknown/orphan entries never deleted), no-clobber destination check, count/total quotas with checked arithmetic, `tempfile` same-root temp + `sync_all` + `persist_noclobber`, root-dir sync on Unix (Windows crash-durability limitation documented). Zero-byte exports consume one file slot. No final file exists before successful commit; a surviving temp file is never treated as committed or deleted.
- **Portable filename contract** — `validate_capture_filename`: ASCII, 1–120 chars incl. `.jsonl` (case-sensitive suffix), starts alphanumeric, only alphanumeric/`.`/`_`/`-`, no `/`/`\`, not `.`/`..`, no `.serial-mcp-` reserved prefix, Windows-reserved stems (`CON`/`PRN`/`AUX`/`NUL`/`COM1`–`COM9`/`LPT1`–`LPT9`) rejected case-insensitively incl. with extension. Flat namespace by design: no subdirectories, no caller-created dirs, no absolute paths.
- **CLI** — `--capture-dir <absolute-existing-directory>` (no fallback to cwd/config/temp), `--capture-max-file-bytes` (16 MiB), `--capture-max-total-bytes` (256 MiB), `--capture-max-files` (256). All four are in `VALUE_TAKING_OPTIONS` (drift-guarded by `tests/doc_drift.rs::capture_cli_options_synced_between_value_list_and_help`). Quota options without `--capture-dir` = startup error; invalid root/quota relation = startup error.
- **`export_log` rework** — order: store enabled check (exact message "Persistent capture is disabled; start serial-mcp with --capture-dir <absolute-directory>") → filename validation → connection lookup → bounded `jsonl_snapshot(max_file_bytes)` in `spawn_blocking` → `write_new`. `ExportLogResult` gains `bytes_written`/`files_used`/`total_bytes_used` (all uint-schema annotated; `path` = canonical absolute final file) and optional `durability_warning` (POST-commit root-sync failure on Unix: export succeeded, file committed and counted, never deleted). Pre-commit failure creates no file; connection stays usable. Arbitrary-path writes and overwrite are intentionally removed (breaking migration documented in README/CHANGELOG).
- **Bounded JSONL snapshot (`src/log_buffer.rs`)** — `jsonl_snapshot(&self, max_bytes)`: single lock for point-in-time consistency, one line per event with trailing newline, checked arithmetic before extending (error = exact snapshot exceeds quota, no partial output), no deque clone / no second full String.
- **Behavior tests** — HTTP MCP in `tests/http_integration.rs`: disabled error ordering (before path/connection), JSONL content matching `get_log` with exact counts, zero-byte slot consumption, all bad-name classes fail without files, existing target byte-identical, symlink target rejected + outside untouched (Unix), concurrent same-name = exactly one success, per-file/total/count quotas (incl. persistence across fresh store instances and independent servers sharing a root via the advisory lock), failure leaves connection usable, point-in-time snapshot. Spawned binary: valid `--capture-dir` starts HTTP + stdio servers (`tests/http_integration.rs`, `tests/stdio_integration.rs`). CLI in `tests/stdio_integration.rs`: help documents all options, `--capture-dir --version` not mistaken for version flag, quota-without-root / relative / missing / file / symlink roots / zero / bad quota relation all reject startup. Unit (`src/capture_store.rs`, `src/log_buffer.rs`): validator table, quota boundaries, scanner classification, no-clobber, cross-store concurrency, post-commit root-sync failure = `durability_warning` (file kept), exact-limit/one-byte-over snapshot.
- **Trust boundary** — configured root + ancestors are operator-controlled; no hostile-root capability defense (documented non-goal). Advisory lock protects cooperating serial-mcp processes only.
- **Future design** — `docs/development/safe-continuous-capture-design.md` specifies the (NOT implemented) continuous-capture lifecycle; recommendation stays "do not implement until concrete task evidence".
