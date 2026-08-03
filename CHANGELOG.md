# Changelog

| Version | Date | Highlights |
|---|---|---|
| [0.9.1](#091) | 2026-08-01 | lossless RX byte preservation via shared encoding fallback (exact spaced hex, effective encoding reported, no drop accounting on success) across `read`/`subscribe`/`transact`/`capture_boot` raw/frame/partial/match-context paths; unified matcher-owned bounded window for raw read/subscribe with global indexes; framing/serial/RX-tool module splits (public surface unchanged); hermetic mandatory config-schema validation + release/drift guards; pinned `quinn-proto` 0.11.15 (RUSTSEC-2026-0185 / CVE-2026-25800); Rust 1.88.0 workflow/Nix alignment; weekly fuzz + mutation hardening; Windows serial E2E deferred |
| [0.9.0](#090) | 2026-08-01 | process-wide versioned `ProfileStore` + automatic high-confidence profile sessions (generated/reused, open overlay, observable bindings); write-through profile learning with revision-CAS/conflict/stale/close retry; `rollback_profile` + deletion guard; `list_ports` `profile_matches` discovery; decision-tree teaching + deterministic agent evaluator; atomic pump-gated cancellation-safe `capture_boot`; disabled-by-default `CaptureStore` with CLI quotas + no-clobber `export_log` (breaking: arbitrary paths removed); `flush(both)` RX backlog fix; tool count 25 → 27 |
| [0.8.1](#081) | 2026-07-19 | `configure` tool (profile + live-connection modes), `compute_checksum` tool (xor + lrc), `transact` tool (write-then-read); `max_buffered_bytes` and `poll_interval_ms` moved from per-call to connection defaults (via `ProfileDefaults` + `configure`); `save_profile` `rx_buffer_size` snapshot bug fixed; tool count 22 → 25 |
| [0.8.0](#080) | 2026-07-08 | RX ring buffer redesign: always-on pump + `RxRing` capture from open to close; `read` cat semantics (buffered bytes immediately + `peek` + offset fields); `seek` tool (non-destructive cursor move); `subscribe` cursor follower with `from` history replay + `bytes_lost` gap reporting; `flush(input)` ring clear; `get_status` ring fields; `open` `rx_buffer_size` (256 KiB default); `ConsumerRegistry`/`RxEvent` fanout deleted; unified read/subscribe semantics; budget ring at open |
| [0.7.4](#074) | 2026-07-07 | `server.json` becomes a packages-less registry template (publish workflow generates release URLs + hashes; drift guard forbids committed `packages`) |
| [0.7.3](#073) | 2026-07-07 | NMEA parser panic fix + P-talker proprietary split; decode-error semantics (`PushOutcome`, drop-and-count checksums, `read` partial results + `error` field + hex fallback); `docs/protocols.md` guide; framing.rs dedup (`emit_frame`/`check_checksum`/`take_frame`/`match_line_byte`/`xor_checksum`/`lrc` free functions); NMEA malformed-checksum body parse; `TxFramingMode::Nmea` auto `*XX` checksum; `--version` argv scan strictness; table-driven preset tests |
| [0.7.2](#072) | 2026-07-07 | `--version` flag + `version` subcommand, `BUILD_TARGET` in build.rs; removed dead `ProfileDefaults` fields; `slip` and `json_lines` protocol presets; COBS framing mode + `cobs` preset + `checksums` module; `ndjson` preset + `skip_empty` framing option; `nmea0183` preset + `Nmea` parser + `StartEnd` multi-marker + checksum validation; `modbus_ascii` preset + `ModbusAscii` parser + `Lrc` checksum; schema fix for `Frame.data` uint8 format |
| [0.7.0](#070) | 2026-06-26 | Frame pipeline: TX framing, SLIP, protocol presets, profile defaults, parser relocation |
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

## [0.8.0]

**Breaking (pre-1.0) — RX ring buffer redesign, seek folded into read, peek dropped:**
- The RX side is now an always-on ring buffer with absolute stream offsets.
  Every byte from `open` to `close` is captured to a per-connection ring
  (`rx_buffer_size`, default 256 KiB, configurable at open), whether or not
  any tool call is active. The pump runs from `open` to `close` and pauses
  on disconnect (resumes on reconnect, same ring, monotonic offsets).
- `read` behaves like `cat`: returns buffered-but-unread bytes immediately
  (`stop_reason: "drained"`), consuming by default. Pattern matching checks
  buffered history first, then waits for new bytes. Results carry
  `from_offset`/`next_offset`/`bytes_lost`/`buffered_remaining`/
  `start_offset`/`end_offset`.
- `read` gains a `from` parameter (`now`/`cursor`/`buffer_start`/
  `{"offset": N}`) for atomic seek+read. The `seek` tool is removed — `read`
  with `from` covers every seek use case except `Delta`, which is dropped
  (agents track absolute offsets via `next_offset`/`from_offset`).
  Re-passing the same `from` offset re-reads the same bytes non-destructively
  (replaces the deleted `peek` option). `max_buffered_bytes` default 2048 →
  32768 (32 KiB) so a default read captures a full boot log.
- `subscribe` is a cursor follower (`tail -f`) with a new `from` parameter
  (`"now"`/`"cursor"`/`"buffer_start"`/`{"offset": N}`) for history replay.
  `SubscribeFrom` renamed `ReadFrom` (shared with `read`), wire format
  unchanged. Slow consumers get `bytes_lost` gap notifications and continue —
  no silent drops. Subscriptions do NOT move the shared read cursor; `read`
  and `subscribe` coexist without stealing.
- `flush(input)` now clears the ring + clamps the cursor to the live edge
  (strictly more destructive than before). Use `read` with `from: "now"`
  as the non-destructive alternative.
- `get_status` gains `rx_buffer_size`/`rx_start_offset`/`rx_end_offset`/
  `rx_cursor`/`rx_buffered_unread`/`rx_bytes_wrapped_total`.
- `open`/`open_profile`/profiles gain `rx_buffer_size` (default 256 KiB,
  max 16 MiB; validated against the buffer budget pool).
- `ConsumerRegistry`/`RxEvent` fanout, `register_blocking`/
  `register_streaming`/`prune_consumers` deleted — both tools read from the
  ring now.
- Construction errors hard-fail both tools (subscribe stops degrading to
  raw mode — the old rationale is void now that the ring keeps buffering
  while the agent fixes its config).
- Hardware flow control loses its throttling side effect: the always-on
  pump drains continuously, so RTS never drops and the device streams
  freely. A setup that relied on flow control to pause a device until the
  host reads will behave differently.
- `bytes_returned` definition unified (one definition for read + subscribe:
  bytes emitted up to and including the match).
- Framing-error cursor contract: on `stop_reason: framing_error`, the
  cursor advances past all consumed bytes including the malformed
  sequence, so a plain retry always makes progress.
- `get_status`: `rx_bytes_lost_total` renamed `rx_bytes_wrapped_total`
  to disambiguate lifetime wrap-loss from per-read cursor-gap
  (`bytes_lost` on read/subscribe results).

**Internal:**
- `src/rx_ring.rs` (new): `RxRing` sliding-window buffer with absolute u64
  offsets, wrap+gap accounting, `Notify`-based wakeups, `bytes_wrapped_total`
  lifetime counter. Exhaustive unit tests + proptest.
- `src/rx_session.rs` reworked: ring ownership, always-on pump,
  pause/resume across disconnect, budget charge at open (RAII release at
  close), `RxSession::new` fallible.
- `src/tools/helpers.rs`: `read_bytes_from_ring` replaces
  `read_bytes_via_session` (deleted). 0.7.3 partial-result + `error` field
  + hex fallback contract preserved on framing errors.
- `StreamHandle` `unsafe` block replaced with `Option<JoinHandle>::take()`.
- `advance_cursor` helper extracts the 16× cursor-clamp duplication.
- Tool count drops from 23 → 22 (`seek` removed, folded into `read`).

## [Unreleased]

### Resource subscriptions (Phase 3)
- modern `2026-07-28` `subscriptions/listen` backed by one process-wide
  bounded resource event hub (capacity 256): `accepted_subscription_filter`
  keeps only valid, deduplicated concrete resource URIs in first-request
  order (list-change flags, templates, malformed ids, and unknown URIs
  stripped) and `listen` streams `notifications/resources/updated` hints for
  `serial://ports`, `serial://connections`, and recognized connection
  detail/raw/log URIs; lagged listeners conservatively recover one
  notification per accepted URI without ever blocking the publisher or the
  RX pump;
- modern discovery now advertises `resources.subscribe: true`; legacy
  `2025-11-25` initialize keeps resource subscriptions disabled and
  `subscriptions/listen` stays `-32601` for legacy clients;
- proactive port hotplug watcher: one per server process, canonicalized
  snapshots (sorted full `PortInfo` identity), first-success baseline,
  no false updates on reorder/unchanged/enumeration failure, recovery
  against the retained baseline, deterministic shutdown/join;
- resource hints published after successful public behavior: port
  open/close (connections list + detail), reconfigure/set-flow-control/
  reconnect/connection-mode configure/set_dtr_rts/send_break/write/transact
  (detail), RX ring append (detail + raw + log, after the ring append and
  outside the pump gate), clear_log (log), input flush (detail + raw);
  notifications never carry payloads and never move the shared read cursor;
- one `SystemPortProvider` and one `ResourceEventHub` per server process,
  shared by every HTTP handler factory and the watcher (stateless HTTP
  requires process-wide ownership — see `tests/resource_subscriptions.rs`
  for the modern-client proofs, including two stateless handler instances
  observing the same update).

### Migration: rmcp 3 server surface (MCP `2026-07-28` groundwork)
- rmcp 1.7 → 3.0 (`Meta` → `RequestMetaObject`, `RawResource`/`RawResourceTemplate`
  → `Resource`/`ResourceTemplate`, MRTR-aware `ReadResourceResponse`, `Role`
  prompt roles, constructor-built progress/notification params);
- MCP logging support removed (`notifications/message` no longer used) and the
  serial-mcp `subscribe`/`unsubscribe` tools deleted; legacy
  `resources/subscribe`/`resources/unsubscribe` handlers and the logging,
  resource-subscribe, and list-change capability flags removed;
- `poll_interval_ms` removed from open/profile/configure/connection defaults and
  the stream-only chunk/poll limit constants and schema helpers; existing
  profile files containing the old key still load (serde ignores it) and the
  next durable rewrite drops it — no schema-version bump;
- tool count drops from 27 → 25; `read` remains the complete RX path (buffered,
  match, framing, cursor replay, loss reporting, lossless hex fallback) and
  `capture_boot` is unchanged;
- MCP `2026-07-28` discovery and standard `subscriptions/listen` are NOT yet
  implemented (planned: dual-protocol Phase 2, resource events Phase 3).

### Dual MCP lifecycle (Phase 2)
- Preferred modern `2026-07-28` discovery/stateless lifecycle: exact
  supported-version slice `[2026-07-28, 2025-11-25]` (`server/discover`
  with ordered `supportedVersions`, `resultType: "complete"`, no session
  required), self-contained per-request `_meta` + SEP-2243 headers, and
  modern `-32602` remap for unknown resources (SEP-2164);
- compatible legacy `2025-11-25` initialize/session lifecycle unchanged:
  `Mcp-Session-Id` sessions, `resultType` stripped for legacy peers,
  `-32002` resource-not-found preserved;
- subscription advertisement stays disabled in Phase 2:
  `subscriptions/listen` is `-32601` for both protocols and legacy
  `resources/subscribe`/`resources/unsubscribe` remain `-32601`;
- new compatibility proofs: `tests/protocol_compatibility.rs` (typed
  discover/initialize matrix + raw-wire status/code/header assertions)
  and stdio modern/legacy lifecycle tests.

### Version-correct cache compliance + pinned conformance gates (Phase 4)
- modern `2026-07-28` peers now receive the SEP-2549 cache fields
  `ttlMs: 0` / `cacheScope: "private"` on every cacheable family
  (`tools/list`, `resources/list`, `resources/templates/list`,
  `resources/read` complete results for every URI kind, `prompts/list`)
  via a single pure `modern_cache_fields` gate on the negotiated protocol
  version; legacy `2025-11-25` peers continue to see neither field (rmcp
  strips `resultType` for legacy but not cache fields, so the server omits
  them itself — no leak to legacy clients);
- `tools/list` / `prompts/list` are now explicit handlers over the same
  routers (exact deterministic catalog, titles, schemas, prompt
  definitions) with cursor pagination; `#[prompt_handler]` was dropped
  because rmcp-macros 3.1.0 replaces any `list_prompts`/`get_prompt`
  outright (unlike `#[tool_handler]`) — the two methods are hand-written
  against the same `prompt_router`;
- pinned official `@modelcontextprotocol/conformance@0.2.0-alpha.10` gate
  in CI (`mcp-conformance` Ubuntu job, Node 22.19.0, 15-minute bound):
  planned scenario sets at exact protocol versions only — legacy
  `server-initialize`, `ping`, `completion-complete`, `tools-list`,
  `resources-list`, `prompts-list` (2025-11-25) and modern
  `server-stateless`, `completion-complete`, `tools-list`,
  `resources-list`, `prompts-list`, `caching`, `sep-2164-resource-not-found`
  (2026-07-28); the four documented fixture-dependent expected failures
  live in `conformance/expected-failures.yaml` (a baseline entry that
  starts passing fails the run as stale; no `--suite all`, no fixture
  endpoints added to the product); reports upload under stable
  `target/conformance-results/` paths on success and failure;
- pinned official `@modelcontextprotocol/inspector@2.0.0` CLI
  interoperability smoke (`scripts/inspector-smoke.mjs`, Node stdlib only)
  in the same job: asserts server identity `serial-mcp` with modern
  `2026-07-28` negotiation, exactly 25 unique tools with
  `compute_checksum`, `serial://ports` + `serial://connections` resources,
  `diagnose_port` + `interactive_terminal` prompts, and a
  `compute_checksum` call returning raw `111` / hex `6F`; non-interactive,
  per-command timeouts, hard gate (inspector, not conformance).

## [0.9.1]

### Security / maintenance
- locked `quinn-proto` 0.11.15 fixes RUSTSEC-2026-0185 / CVE-2026-25800;
- monthly grouped Cargo/GitHub Actions Dependabot;
- Rust 1.88.0 workflow/Nix alignment with explicit compiler reports.

### Validation / CI
- all three config examples are mandatory/hermetic; exact vendored models.dev
  resource registered under original URI; missing fixtures fail; Nix includes
  all schemas; scheduled upstream test remains networked;
- changelog/server/Cargo release consistency regression guard and named Ubuntu
  doc-drift gate;
- weekly/manual non-PR hardening: all three fuzz targets (pinned nightly +
  cargo-fuzz, five-minute bounded matrix) and focused checksums/parser mutation
  testing (pinned cargo-mutants, bounded), failure artifacts;
- Windows serial E2E deferred rather than installing privileged drivers.

### RX behavior fixes
- `read`, `subscribe`, `transact`, and `capture_boot` preserve unrepresentable
  RX bytes through exact lowercase spaced hex fallback, report effective
  encoding, never use lossy UTF-8, and do not count successful fallback as a
  dropped notification/frame; applies to raw/frame/partial/match-context paths;
- one matcher-owned bounded policy for raw read/subscribe with global indexes,
  literal overlap/context bounds, regex/glob allowance, safe truncated glob
  lines, and frame-local reset/context behavior.

### Internal / documentation
- split framing into config/codecs/decoder/parsers modules;
- split serial into config/connection/manager/port-info/test-support modules;
- split RX tool validation/read-loop/result-builder modules;
- flat Rust paths, 27-tool count, MCP tool input/result data-field sets, and
  lifecycle behavior stayed stable;
- matched-context subscribe stop notification additively reports effective
  `encoding` alongside `data` (omitted when no data);
- MCP tool output-schema size changes are documentation descriptions only;
- evaluator current catalog 27 tools / 288177 bytes; +1892 versus pre-refinement
  from approved output-schema descriptions only;
- roadmap/docs reconciled; UInt/schemars, long-loop decomposition, continuous
  capture remain deferred.

## [0.9.0]

**Breaking (pre-1.0) — `export_log` no longer accepts arbitrary output paths.**

### Added
- **Process-wide versioned `ProfileStore`** — profiles move from a
  handler-local vector to one `Arc<ProfileStore>` shared by every stdio and
  HTTP session. Persistent mutations serialize behind a process-local async
  mutex, run in `spawn_blocking` under an advisory `<file>.lock`, reload the
  file under that lock (separate server processes cannot lose each other's
  updates), and commit atomically (`NamedTempFile` + `sync_all` + rename);
  the in-memory cache is published from inside the transaction only after
  the durable write succeeds. The file format gains `schema_version` (legacy
  unversioned TOML migrates as v1; version 0 or > 2 rejects startup).
  `Profile` gains metadata (revision, created/updated/last-used timestamps,
  generated flag, use count) and a bounded 5-snapshot revision history. New
  `--profiles-path` CLI option; the OS user-config default no longer
  silently falls back to the current working directory.
- **Automatic high-confidence profile sessions** — a bare `open` of a
  uniquely identified USB device (transport + VID + PID + non-empty serial,
  interface when available) creates a durable generated profile
  (`auto-{label}`, revision 1, use count 1) whose defaults equal the
  effective open settings; close/reopen automatically selects the unique
  most-recently-used profile for the same device. Equal top ranks are
  reported as ambiguity with candidates, never vector-order selection;
  weak identity and duplicate live fingerprints open transient and never
  write a durable profile; `profile_mode="none"` disables selection/
  creation for troubleshooting. `open_profile` requires exactly one
  matching live port (multiple matches are a tool error) and marks the
  profile used.
- **Open overlay and observable bindings** — `open` fields are an overlay
  (explicit field > selected profile default > built-in 115200/8-N-1
  defaults; omitted baud still resolves to 115200). Every successful
  `open`/`open_profile` binds the connection to an observable profile
  session reported in `OpenResult`, `GetStatusResult`, and
  `ConnectionSummary` (`profile`: name, selection source, confidence,
  persistent, generated, revision, dirty, candidates, last persistence
  error).
- **Write-through profile learning** — durable live changes
  (`reconfigure`, `set_flow_control`, connection-mode `configure`, dirty
  open overrides, clean close retry) hold a per-connection learning lock
  across live mutation → `effective_defaults()` snapshot → revision-CAS
  store update → binding update. Hardware success + persistence failure
  stays a successful result with `state="failed"`, the binding turns
  dirty, and the next durable mutation or clean close retries; hardware
  failure keeps the tool error and never calls the store. Revision
  conflicts are reported with profile + expected/actual revision; a stale
  binding keeps reporting the conflict until reopened. Results carry
  additive `profile` + `profile_persistence`
  (`persisted`/`not_needed`/`transient`/`failed`).
- **`rollback_profile` tool** (25 → 26) — restores any retained prior
  revision (newest five snapshots, exposed via `list_profiles`
  `revisions`) as a new monotonic revision with CAS on
  `expected_revision`; a wrong or evicted revision is a tool error that
  leaves the file unchanged. Same-process bound connections are marked
  stale+dirty and counted in `active_connections_unchanged`.
  `delete_profile` refuses while any same-process open connection binds
  the profile (error lists connection IDs). `save_profile` promotes a
  generated-bound connection to a user-owned profile (`generated=false`).
- **`list_ports` discovery preview** — the result carries
  `profile_matches` parallel to `ports` (same order, always present):
  per-port `confidence` + `outcome`
  (`selected`/`ambiguous`/`ineligible`/`duplicate`/`none`) and ordered
  candidates. Read-only — no `mark_used`, no file writes; the
  `serial://ports` resource serves the same map.
- **Decision-tree teaching + deterministic agent evaluator** — server
  `instructions`, the 12 common tool descriptions, README flow, and both
  prompts teach the discover (`list_ports`) → open (bare `open`) →
  talk (`transact`/`read`/`write`) → verify learned `profile`/
  `profile_persistence` → escalate-on-demand decision tree. New xtask
  `agent-eval` subcommand measures the tool surface deterministically (no
  network, user config, or timestamps): catalog + fixed scenarios with
  byte metrics and fixed acceptance thresholds against a committed
  baseline. Decisions: automatic profiles, `transact`, and atomic
  `capture_boot` accepted; string shorthand, initial recipes, and a
  versioned facade rejected (modeled, not implemented).
- **`capture_boot` tool** (26 → 27) — one bounded, cancellation-safe
  operation replacing the racy arm-reset-read composition: purge unread OS
  input, mark the RX live edge atomically under the pump gate (an
  in-flight pre-reset pump read can never append after the mark), optionally
  pulse DTR/RTS under the per-connection control lock with guaranteed
  release (`ResetReleaseGuard`, drop-time retry through the public
  `set_dtr_rts`; closed ports count as released), then capture only
  post-mark bytes through the existing read pipeline with a private
  cursor. Bounded in-memory result, no file output; omitted/null
  `timeout_ms` resolves to 5000 ms; capture is transient (no profile
  learning).
- **Safe persistent capture foundation** — `export_log` now persists the
  event log as a bounded, atomic JSONL snapshot through a disabled-by-default
  `CaptureStore` (`src/capture_store.rs`), enabled only with an explicit
  absolute `--capture-dir`. Per-file (`--capture-max-file-bytes`, default
  16 MiB), total-byte (`--capture-max-total-bytes`, default 256 MiB), and
  file-count (`--capture-max-files`, default 256) quotas are enforced from a
  fresh scan of the root's direct children under a process-local mutex and an
  advisory cross-process lock (`.serial-mcp-captures.lock`). Startup validates
  the root (absolute, existing directory, not a symlink, working lock) and the
  quota relation; quota options without `--capture-dir` are startup errors.
  Tool count unchanged (27).

### Changed
- **Breaking:** `export_log` no longer writes to an arbitrary caller-supplied
  path. `path` is now a portable `.jsonl` filename (ASCII, 1–120 chars,
  alphanumeric/`.`/`_`/`-`, `.jsonl` suffix, no separators/traversal,
  Windows-reserved stems rejected) relative to the configured capture
  directory. Existing destinations are never overwritten (`persist_noclobber`,
  symlinks rejected). Result is additive: `bytes_written`, `files_used`, and
  `total_bytes_used` join `events_written`, `path` reports the canonical
  absolute final file, and a POST-commit Unix root-directory sync failure
  is reported in optional `durability_warning` (the file is committed and
  counted, never deleted); pre-commit failures still create no file.
- The profile file format is now schema-versioned.
  Legacy unversioned TOML migrates in memory to v1; a missing/unavailable
  config dir, corrupt file, or unsupported future version fails startup
  instead of silently starting empty.

### Fixed
- `flush(target="both")` now also discards the retained RX ring and clamps
  the shared read cursor to the live edge — identical RX semantics to
  `flush(target="input")`, routed through one shared helper.
- Profile store race and cancellation hardening: `configure(profile)` no
  longer has a TOCTOU window against concurrent deletes
  (`update_defaults_preserving_selector` returns the effective profile from
  the same locked transaction); the shared cache is an
  `Arc<RwLock<Vec<Profile>>>` published from inside the blocking
  transaction, so a cancelled awaiting tool can never leave disk updated
  with a stale in-memory view.
- Binding fixes: `open_profile` reports the matched port's own identity
  confidence; selected-profile `dirty` is computed before hardware open
  (invalid defaults fail the call instead of mapping to clean); a missing
  binding after a successful open errors instead of silently losing the
  profile.
- Learning review fixes: learned no-ops (CAS no-op) republish the fresh
  disk state to the cache without rewriting the file; `save_profile` holds
  the connection learning lock across snapshot + upsert so concurrent
  reconfigure/configure cannot yield a mixed snapshot.
- Capture review fixes: the release guard is disarmed only on a successful
  release, so a cancelled capture with a failed release attempt retries at
  drop through the public `set_dtr_rts` (queued on the control lock); the
  control lock is scoped to the pulse only, dropped before settle/read;
  the per-file quota is checked before the mutex + blocking work;
  `persist_noclobber` failures report the OS error cause-neutrally
  (the existing-destination precheck still identifies that case); a
  root-sync failure after a successful commit is a POST-commit
  `durability_warning`, never a false failure.
- Generated tool schemas now document the tagged `from` wire forms
  (`{"type":"now"}` / `{"type":"cursor"}` / `{"type":"buffer_start"}` /
  `{"type":"offset","offset":N}`) instead of bare string descriptions.

### Documentation/Internal
- Nix: the Nordic toolchain environment is scoped to the `nix-nrf-dev` dev
  shell — the shell stays clean (no `LD_LIBRARY_PATH`/`PYTHONHOME`/
  `PYTHONPATH`/`GIT_EXEC_PATH` pollution from the sdk-manager); the `west`
  wrapper loads `nrfutil sdk-manager toolchain env` per command.
- Nix source-filter fixes: the `docs/` tree is included in the flake source
  filter so doc-drift fixtures build under `nix flake check`; the
  `schemas`/`example-configs` prefixes match the leading-slash `relPath`
  so vendored config fixtures actually validate in the sandbox (the
  opencode schema stays excluded — eager network fetch, documented tech
  debt).
- Development docs consolidated: `docs/development/agent-interface-
  evaluation.md` (current 27-tool catalog, 286285 bytes) replaces the
  per-phase evaluation notes; the historical baseline remains for deterministic
  comparisons. Phase handoffs, the
  agent-interface simplification plan, and the nix-nrf-dev migration
  plans are removed; README + AGENTS.md updated for the profile-session,
  capture, and learning contracts.
- Tool count 25 → 27 (`rollback_profile`, `capture_boot`).

## [0.8.1]

### Added
- **`configure` tool** (22 → 25 total): two modes — profile (persist defaults to TOML) and connection (mutate live connection defaults for framing/parser/protocol, reconnect_policy, max_buffered_bytes, and poll_interval_ms). `log_capacity`/`log_enabled`/`rx_buffer_size`/serial-line params are profile-only (LogBuffer has no live setter).
- **`compute_checksum` tool**: pure utility — compute xor (NMEA-0183) and lrc (Modbus ASCII) checksums over caller-supplied bytes.
- **`transact` tool**: write-then-await-response in one call, default `from: "now"` to skip pre-write backlog.
- `ProfileDefaults` now carries `max_buffered_bytes` (32768), `poll_interval_ms` (200), `reconnect_policy`, `log_capacity` (1024), and `log_enabled` (true). These flow through `OpenArgs` → `ConnectionConfig` → `SerialConnection`.

### Changed
- **Breaking:** `ReadArgs.max_buffered_bytes` removed (use `configure` or connection default).
- **Breaking:** `SubscribeArgs.max_buffered_bytes` and `SubscribeArgs.poll_interval_ms` removed (use `configure` or connection defaults).
- `SerialConnection` framing defaults (`tx_framing`, `rx_framing`, `rx_parser`, `protocol`) are now `StdMutex<Option<T>>` for live mutation; accessors return by value.
- `precedence::resolve_field` now takes `conn_default: Option<T>` (by value) instead of `Option<&T>`.

### Fixed
- `save_profile` now snapshots `rx_buffer_size` from the live `RxSession` ring capacity instead of hardcoding `DEFAULT_RX_BUFFER_SIZE`.

## [0.7.4]

**Changed — `server.json` is now a packages-less registry template:**
- The committed `packages` array went stale on every release (0.5.1 asset
  URLs and `fileSha256` hashes survived in the repo until 0.7.3) because
  `publish-mcp-registry.yml` regenerates it from the actual release
  binaries at publish time — the committed copy was never what got
  published. `server.json` now carries only
  name/title/description/repository/version; the publish workflow remains
  the sole producer of package identifiers and hashes. No change to what
  the MCP registry serves.
- New `doc_drift` guard `server_json_omits_packages` fails the build if a
  `packages` array is ever committed again.
- Version bumps now touch a single `server.json` field instead of five.

## [0.7.3]

**Added — NMEA TX auto-checksum:**
- The `nmea0183` preset's TX path now appends the `*XX` XOR checksum
  automatically via a new `TxFramingMode::Nmea` variant. Payloads already
  ending in a valid `*HH` pass through un-doubled (existing checksum
  validated); a wrong `*HH` errors. Embedded `\r`/`\n` and non-printable
  bytes are rejected. Explicit `tx_framing: start_end` still wins via
  precedence for callers that want the old no-checksum behavior. The preset
  now round-trips correctly: `write(protocol: nmea0183)` → `read(protocol:
  nmea0183)` produces `checksum_valid: true`.

**Fixed — NMEA parser panic:**
- NMEA parser no longer panics on non-ASCII-but-valid-UTF-8 bodies; non-ASCII
  frames now return `Raw` (spec-conformant). The parser slices the address
  field by byte index, but byte index 2 need not be a char boundary in
  multi-byte UTF-8. A new `is_ascii()` guard mirrors `ModbusAsciiParser`'s
  non-ASCII → `Raw` behavior.

**Breaking (pre-1.0) — NMEA proprietary talker split:**
- NMEA proprietary sentences (`$P...`) now split as `talker_id: "P"` +
  `sentence_type:` the rest (e.g. `PGRMM` → `P` + `GRMM`), matching the
  NMEA proprietary convention. Previously the first two characters were used
  as the talker ID (`PG` + `RMM`).

**Fixed — NMEA malformed-checksum body parse:**
- A NMEA sentence with an invalid-hex or too-short checksum now returns the
  full parsed body (talker_id, sentence_type, fields) with
  `checksum_valid: Some(false)` when `validate: false`, consistent with the
  wrong-value case. Previously the malformed-checksum branches returned
  empty body fields. `validate: true` behavior is unchanged (Err → drop+count
  in the decoder).

**Fixed — `--version` argv scan strictness:**
- `serial-mcp --bind --version` no longer prints the version; `--version` is
  now correctly treated as the value of `--bind`, and the bind parse fails
  with an argument error. The version-flag scan is now value-position-aware
  (for `--transport`, `--allowlist`, `--bind`, `--max-program-buffered-bytes`,
  `--max-tool-buffered-bytes`) and stops at a `--` separator. The `--opt=value`
  form does not consume the next token, so `--bind=0.0.0.0:8000 --version`
  still prints the version.

**Breaking (pre-1.0) — decode-error semantics:**
- `FrameDecoder::push` no longer drops frames decoded before a stream-fatal
  error; the frames are returned alongside the error via `PushOutcome`.
  Checksum mismatches with `validate: true` (e.g. NMEA `nmea0183` preset,
  Modbus ASCII `modbus_ascii` preset) are now per-frame drop-and-count
  (counted in `frames_dropped`) instead of aborting the whole read/subscribe.
  `read` now returns partial results with `stop_reason: framing_error` on a
  fatal SLIP/COBS decode error, matching `subscribe`. The result carries a
  new `error: Option<String>` field with the `FrameDecodeError` text (parity
  with subscribe's final-notification `error` field). When the requested
  encoding can't represent the raw bytes (binary SLIP/COBS under `utf8`),
  the `data` field falls back to hex and `encoding` is set to `"hex"` so the
  partial bytes and the framing diagnostic both survive. Previously `read`
  returned a bare tool error and discarded all collected frames.

---

## [0.7.2]

**Added — CLI version readout:**
- `serial-mcp --version` / `-V` / `version` subcommand prints
  `serial-mcp <semver> (<git-hash>, <build-target>)` and exits 0.
- `build.rs` injects `BUILD_TARGET` alongside `GIT_HASH` so the output is
  self-describing. Falls back to `unknown` for source-tarball builds.

**Breaking — schema removal (pre-1.0):**
- Removed three dead, never-enforced `Option<String>` fields from
  `ProfileDefaults`: `reconnect_policy`, `decoder`, `safety_policy`. These
  were declared "Reserved … Not yet enforced" and read by nothing.
  `decoder` is superseded by the `protocol`/`rx_parser` fields; future
  intent for `reconnect_policy` and `safety_policy` is preserved in
  `FEATURES.md`. The live `Connection.reconnect_policy` (`ReconnectPolicy`
  struct) is unaffected.
- **Migration:** callers who set these fields in profile JSON can simply
  drop them — they were never enforced. Because `ProfileDefaults` does not
  carry `#[serde(deny_unknown_fields)]`, existing profile configs with the
  dead fields continue to load (fields are silently ignored).

**Added — protocol presets:**
- Added `slip` and `json_lines` protocol presets, making the `protocol:` knob
  uniform across every advertised protocol. The `slip` preset bundles SLIP
  (RFC 1055) byte-stuffed framing with a raw parser; `json_lines` bundles line
  framing (`ending: auto` RX, `lf` TX) with the JSON-lines parser. Previously,
  SLIP was only reachable as a bare framing mode and JSON-lines as a bare parser;
  now all three presets (`at_command`, `slip`, `json_lines`) are selectable via
  `{"type": "…"}` on `write`, `read`, and `subscribe`. Pure wiring — no new
  framing mode, parser, or option; the underlying primitives already shipped.

**Added — COBS framing + checksums module:**
- New `cobs` framing mode (`RxFramingMode::Cobs`, `TxFramingMode::Cobs`) with
  plain 0x00-delimited COBS per Cheshire/Baker. Modeled on the existing SLIP
  implementation. TX: `cobs_stuff` encoder with `encode()` arm. RX: stateful
  `cobs_decode` decoder via `FrameDecoder::push`, with `CobsState`
  (BeforeFirstDelim / InBlock) and `DecoderMode::Cobs`.
  The PPP-COBS draft variant (`0x7E`) is not supported in this release; its
  two-step zero-elimination + 7E-substitution will be tracked for a future PR.
  (*Breaking fix*: the initial implementation's `delimiter: u8` field on the
  `Cobs` variant was removed before a release because the `0x7E` path had a
  correctness bug — the decoder inserted the delimiter byte as the phantom
  zero instead of `0x00`, breaking roundtrip on 0x00-containing payloads.)
- New `cobs` protocol preset (`{"type": "cobs"}`) bundling 0x00 COBS framing
  with a raw parser. Selectable via the `protocol` knob on `write`, `read`,
  and `subscribe`.
- Malformed COBS code bytes surface through the existing `FramingError` stop
  reason via a new `FrameDecodeError::CobsInvalidCode(u8)` variant
  (read: `is_error: true`; subscribe: final notification with
  `stop_reason: "framing_error"` + `error` field).
- New `src/checksums.rs` `pub(crate)` module with a `Checksum` trait
  (`compute`, `validate`) and `XorChecksum` implementation (NMEA `*XX` XOR
  checksum). The trait is the extension point for LRC (Modbus ASCII, P2) and
  CRC-16 (Modbus RTU, P3) — those ship with their consumers. No in-tree
  consumer yet in this PR.

**Added — ndjson preset + skip_empty option:**
- New `ndjson` protocol preset (`{"type": "ndjson"}`) bundling line framing
  (`ending: auto` RX, `lf` TX) with the JSON-lines parser, and enabling
  `skip_empty: true` to skip empty/whitespace-only lines per the NDJSON spec.
  `ndjson` differs from `json_lines` only in `skip_empty: true` — both
  share the same line + JSON-lines primitives.
- New `skip_empty: bool` field (default `false`) on `RxFramingConfig`. When
  true, frames whose data is empty or contains only ASCII whitespace are
  silently dropped at the decoder level — they do not count toward `max_frames`
  and do not consume a frame index. The filter applies to all framing modes
  (line, delimiter, SLIP, COBS, length-prefixed, start/end) and runs inside
  `FrameDecoder::push`, so `consume_frames`/matcher only see kept frames.
   `flush_partial` intentionally bypasses `skip_empty` to preserve partial-frame
   signals at end-of-stream.

**Added — NMEA-0183 preset:**
- New `nmea0183` protocol preset (`{"type": "nmea0183"})` bundling `StartEnd`
  framing (start markers `$` / `!`, end `\r\n`, include_markers: false) with
  a `Nmea` parser and checksum validation. Selectable via the `protocol` knob
  on `write`, `read`, and `subscribe`.
- New `Nmea` parser type (`ParserType::Nmea`, `parser: "nmea"` in frame output).
  Parses NMEA-0183 sentences: strips optional leading `$`/`!` and trailing
  `\r\n` defensively, splits at `*` into content and checksum, computes an XOR
  checksum over the content and compares to the hex-decoded received checksum,
  then splits the address into `talker_id` (first 2 characters) and
  `sentence_type` (rest of the address before the first comma) and
  comma-separated `fields`. Returns `ParsedFrame::Nmea { talker_id,
  sentence_type, fields, checksum_valid }`. Non-NMEA frames (no `$`/`!`) return
  `Raw` — the parser is opt-in and does not error on non-matching frames.
- New `validate: bool` field on `ParserConfig` (default `false`; the `nmea0183`
  preset sets `true`). When `true`, a present checksum is enforced: a mismatch
  returns a `FrameDecodeError::ChecksumMismatch` that surfaces as a
  `framing_error` stop reason. When `false`, a present-but-invalid checksum is
  reported as `checksum_valid: false` without error; sentences without a
  checksum are always accepted (`checksum_valid: null`).
- New `FrameDecodeError::ChecksumMismatch { expected: Vec<u8>, received: Vec<u8> }`
  variant. Surfaces through the existing `FramingError` stop reason path
  (`read`: `is_error: true`; `subscribe`: final notification with
  `stop_reason: "framing_error"` + `error` field).
- `FrameParser::parse` is now fallible (`Result<ParsedFrame, FrameDecodeError>`)
  so checksum failures can propagate from parsers. The four existing parsers
  return `Ok(...)`. The three call sites in `slip_decode`, `cobs_decode`, and
  `FrameDecoder::push` propagate parser errors — a parser error drains consumed
  bytes, clears in-progress state, and returns the error, stopping the
  read/subscribe loop (mirroring how SLIP/COBS decode errors work).
- `StartEnd` framing extended to support multiple start markers: the `start`
  field is now `Vec<String>` on both `RxFramingMode::StartEnd` and
  `TxFramingMode::StartEnd`. RX matches ANY start marker in the list (earliest
  match wins); TX uses `start[0]`. The `nmea0183` preset uses
  `start: ["$", "!"]` — two start markers for standard and AIS sentences.
- `src/checksums.rs` `#![allow(dead_code)]` removed — `XorChecksum` is now
  consumed by `NmeaParser`.

**Breaking — schema changes (pre-1.0):**
- `StartEnd.start` changed from `String` to `Vec<String>` on both
  `RxFramingMode::StartEnd` and `TxFramingMode::StartEnd`. Callers must wrap
  their start marker in an array (e.g. `"<"` → `["<"]`).
- `FrameParser::parse` signature changed from `fn parse(&self, &[u8]) ->
  ParsedFrame` to `Result<ParsedFrame, FrameDecodeError>`. This trait is
  `pub(crate)`-ish and not part of the public API; no external callers are
  affected.

**Added — Modbus ASCII preset:**
- New `modbus_ascii` protocol preset (`{"type": "modbus_ascii"}`) bundling
  `StartEnd` framing (`:` start, `\r\n` end, include_markers: false) with a
  `ModbusAscii` parser and LRC validation. Selectable via the `protocol` knob
  on `write`, `read`, and `subscribe`.
- New `ModbusAscii` parser type (`ParserType::ModbusAscii`,
  `parser: "modbus_ascii"` in frame output). Hex-decodes the body between `:`
  and `\r\n`, validates the LRC (Longitudinal Redundancy Check — two's
  complement of the sum of address + function + data bytes), and exposes
  `address: u8`, `function_code: u8`, `data: Vec<u8>`, and `checksum_valid:
  Option<bool>` via `ParsedFrame::ModbusAscii`. Non-hex or malformed frames
  return `Raw`. A failed LRC with `validate: true` returns
  `FrameDecodeError::ChecksumMismatch` (surfacing as `framing_error`).
- New `Lrc` checksum implementation in `src/checksums.rs`: two's complement
  of the wrapping sum, consumed by `ModbusAsciiParser`. Mirrors the existing
  `XorChecksum` for NMEA.
- No breaking changes: all additions are new enum variants, a new struct,
  and a new checksum impl — existing field shapes are unchanged.

**Fixed — schema:**
- `Frame.data` (the decoded-frame byte array, shipped 0.6.0) was emitting the
  non-standard `"format": "uint8"` on its array items; fixed by applying
  `byte_array_schema` (the helper added in P2b for
  `ParsedFrame::ModbusAscii.data`) and adding `Frame` to the `check_schema!`
  regression-net list. Schema-only fix; no runtime behavior change.

**Changed — internal:**
- `Frame.frame_type` changed from `String` to `&'static str` (removes a
  per-frame allocation; JSON output unchanged). `Frame` no longer derives
  `Deserialize` (it was never deserialized in-tree; the derive was unused).
- `NmeaParser` now uses `from_utf8` instead of `from_utf8_lossy` for the
  sentence body; invalid UTF-8 returns `ParsedFrame::Raw` instead of being
  silently mangled with replacement chars (mirrors `ModbusAsciiParser`).

## [0.7.0]

Minor release adding symmetric TX/RX framing, a SLIP framing mode, protocol
presets, and profile/connection framing defaults. All new fields are optional
— existing callers see no behavior change unless they opt in.

**Added — TX framing (`write`):**
- New `tx_framing` option on `write`. Modes: `line` (with `ending`:
  `lf`/`cr`/`crlf`), `delimiter`, `length_prefixed`, `start_end`, `slip`.
- `WriteResult` gains `decoded_bytes` (payload length before framing)
  alongside `bytes_written` (framed bytes sent). When `tx_framing` is
  absent, `decoded_bytes == bytes_written`.

**Changed — RX framing rename + flatten:**
- `read`/`subscribe` field renamed `framing` → `rx_framing` and flattened:
  the `mode` wrapper is gone; `type` and variant fields live at the top
  level (`{"type":"line","ending":"auto"}`).
- `line` mode gains `ending`: `auto` (default, LF/CRLF-aware), `lf` (no CR
  strip), `cr` (bare CR), `crlf` (exact `\r\n`). `auto` now promotes to
  CR-split mode mid-stream when a bare `\r` is confirmed.
- New `slip` RX mode (RFC 1055): byte-stuffed frames between END markers.
- Parser config relocated from inside `rx_framing` to a sibling `rx_parser`
  field on `read`/`subscribe`.

**Added — protocol presets:**
- New `protocol` option on `write`/`read`/`subscribe`. Ships `at_command`
  preset: expands to TX line CR, RX line auto, RX AT-command parser.
- Explicit call fields win over preset components (preset fills gaps).

**Added — profile/connection defaults:**
- `ProfileDefaults` gains `tx_framing`, `rx_framing`, `rx_parser`,
  `protocol`. `open`/`open_profile` store them on the connection.
- `read`/`write`/`subscribe` apply connection defaults when call fields are
  omitted. Four-layer precedence per field: explicit call > call protocol >
  connection default > connection protocol preset.

**Added — error surfacing:**
- `FrameDecoder::push()` returns `Result<Vec<Frame>, FrameDecodeError>`.
  Only SLIP can error (malformed escape); other decoders always return `Ok`.
- New `RxStopReason::FramingError` stop reason. `read` surfaces decode
  errors as tool `is_error: true` results; `subscribe` emits a final
  notification with `stop_reason: "framing_error"` + `error` field.
- Both read and subscribe STOP on the first runtime decode error (no
  resume-on-error). Construction errors (bad config) keep the existing
  read-propagates / subscribe-degrades asymmetry.

**Tests:**
- native_sim e2e coverage expanded 37 → 51 tests, covering all framing
  modes over the real software-serial path (line/delimiter/length_prefixed/
  start_end/SLIP), TX modes via firmware `trace on`, explicit line endings,
  SLIP malformed + recovery, protocol presets + override precedence,
  connection defaults, and subscribe parsed-frame notifications.

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
