# Handoff — RX Ring Phase 1.3+1.4+1.5: read/seek/flush rewrite + schema + tests

> Branch: `rx-ring-redesign` (HEAD: `d7941f1`, Phase 1.2 landed). Do not
> switch branches.
> Source plan: [rx-ring-redesign-plan.md](rx-ring-redesign-plan.md) § Phase 1 steps 3-5.
> Scope: the `read` rewrite onto the ring (consuming, peek, history-aware
> match, offset fields), the new `seek` tool, `flush` ring semantics,
> `get_status` ring fields, `open`/`open_profile`/profile `rx_buffer_size`,
> schema guards, tool-count drift (22→23), and un-ignoring the 6 tests
> Phase 1.2 deferred. This is the step that makes write-then-read work.

## Goal

Rewrite `read` to read from the `RxRing` via the shared cursor instead of
the fanout `event_rx`. This delivers the plan's headline behavior: `read`
behaves like `cat` (returns buffered-but-unread bytes immediately),
pattern matching checks history first, `peek` queries without consuming,
and every result carries absolute offset fields. Add `seek` (move the
shared cursor), rewrite `flush(input)` to clear the ring, add ring fields
to `get_status`, and add `rx_buffer_size` to `open`/`open_profile`/profiles.

## Why

Phase 1.2 made the pump always-on but left `read` on the future-only
fanout path, which broke 6 write-then-read tests (now `#[ignore]`d).
This step rewrites `read` onto the ring, un-ignoring them and delivering
the "boot banner readable after the fact" fix that motivated the whole
redesign.

## In scope

### 3.1 — `read` rewrite onto the ring

`src/tools/io_ops.rs:78-177` (`read` handler) + `src/tools/helpers.rs`
(`read_bytes_via_session` + `build_read_result`).

The current `read` registers a blocking consumer (`session.register_blocking()`)
and drives `read_bytes_via_session(event_rx, …)` which loops on
`event_rx.recv()`. Replace with a ring-driven loop:

- Remove `session.register_blocking()` + `event_rx` from `read`.
  `read` no longer uses the `ConsumerRegistry` fanout (Phase 2 deletes
  the registry entirely; for now `subscribe` still uses it).
- Remove `session.prune_consumers()` after read (`:153`) — read no longer
  registers a consumer.
- The per-call `max_buffered_bytes` budget reservation (`:109-112`) STAYS
  — it bounds the output accumulation buffer (the bytes copied out of the
  ring into the result). The ring has its own reservation from `open`.
- New `read_bytes_from_ring` function (or rewrite `read_bytes_via_session`
  in place — pick a clean name; the old `read_bytes_via_session` is now
  dead for `read` but `subscribe` in Phase 2 will have its own ring loop).
  Signature:
  ```rust
  pub async fn read_bytes_from_ring(
      session: Arc<RxSession>,
      max_bytes: usize,
      timeout_ms: Option<u64>,
      ct: &CancellationToken,
      progress_token: Option<ProgressToken>,
      peer: Option<&Peer<RoleServer>>,
      mut matcher: Option<Matcher>,
      no_new_rx_timeout_ms: Option<u64>,
      conn: Option<Arc<SerialConnection>>,
      framing: Option<RxFramingConfig>,
      parser: Option<ParserConfig>,
      peek: bool,
  ) -> Result<ReadOutcome, String>
  ```

Loop shape (high-level — the executor fills in the detail, preserving the
0.7.3 partial-result/framing-error/hex-fallback contract):

1. Read the session's shared cursor: `let cursor = session.read_cursor();`
2. `let slice = session.ring().read_from(cursor, max_bytes);`
   - `slice.bytes` = buffered-but-unread bytes (may be empty).
   - `slice.from_offset` / `slice.next_offset` / `slice.bytes_lost`.
3. **Immediate path (cat):** if `slice.bytes` is non-empty and no `match`
   given → return them now (up to `max_bytes`), `stop_reason: "drained"`.
   Advance the cursor to `slice.next_offset` UNLESS `peek: true` (peek
   never advances). Leftover beyond `max_bytes` stays in the ring.
4. **Match path:** if `match` given, feed `slice.bytes` to the matcher
   (sliding window). If found in history → return immediately, cursor
   advances to end of matched region (raw) or matched frame (framed),
   UNLESS peek. If not found in history, enter the wait loop.
5. **Wait loop:** `ring.wait_for_data(cursor).await` (or
   `tokio::select!` with timeout/silence/cancel) → on wake, re-read
   `read_from(cursor, …)`, feed new bytes to matcher, check stop
   conditions (`timeout_ms`, `no_new_rx_timeout_ms`, `max_bytes`,
   `match found`, `cancel`, `connection_closed`). Accumulate into the
   output buffer. On framing decode error → 0.7.3 partial-result
   contract (`stop_reason: framing_error`, `error` field, hex fallback,
   frames-before-error kept — reuse `consume_frames` + `ReadFrameSink` +
   the `finish!` macro's framing-error arm). On match → cursor advances
   past matched region (unless peek); bytes after match stay in ring.
6. **Cursor advancement on framing_error** (plan § "Cursor position on
   framing error"): the cursor advances past ALL bytes consumed from the
   ring including the malformed sequence, so a plain retry progresses.
   `from_offset`/`next_offset` bracket the consumed window for forensic
   re-read via `seek`.
7. Build `ReadOutcome` with the new offset fields (see 3.2).

The `disconnect_state` pause check (`rx_consume.rs`) still applies —
timeouts pause while disconnected. Use `session.ring().wait_for_data`
which wakes on append (the pump resumes appending after reconnect).

`build_read_result` (`helpers.rs:714-776`) gains the offset fields (3.2)
and keeps the 0.7.3 hex-fallback + `error` field + `frames_dropped`
contract. The `is_framing_error` check becomes `outcome.error.is_some()`
as today.

### 3.2 — `ReadArgs` + `ReadResult` offset fields + `peek`

`src/tools/types.rs`:
- `ReadArgs` (`:85-123`): add `pub peek: bool` (default false,
  `#[serde(default)]`). Doc: "When true, return bytes without advancing
  the cursor. A peek may still wait via timeout_ms for a match; it just
  doesn't consume. A repeated peek finds the same match again."
- `ReadResult` (`:296-359`): add after `frames_dropped`/`error`:
  ```rust
  /// Absolute stream offset where this read's data starts (clamped to
  /// ring start_offset if the cursor had fallen behind). `null` only
  /// when the read produced no data and no cursor was consumed.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
  pub from_offset: Option<u64>,
  /// Absolute stream offset of the cursor after this read (where the next
  /// read starts). Equal to `from_offset + bytes_returned` for a consuming
  /// read; equal to `from_offset` for a peek.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
  pub next_offset: Option<u64>,
  /// Bytes lost to ring wrap since the cursor's original position. Non-zero
  /// means the cursor had fallen behind `start_offset` and the read
  /// started at `start_offset` instead. Always 0 for a healthy read.
  #[serde(default)]
  #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
  pub bytes_lost: u64,
  /// Unread bytes remaining in the ring after this read (between
  /// `next_offset` and `end_offset`). 0 when the read drained to the live
  /// edge.
  #[serde(default)]
  #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
  pub buffered_remaining: u64,
  ```
  All `u64` fields get `uint_schema`/`option_uint_schema` (per AGENTS.md
  uint-format guard). Add `ReadResult` to the `check_schema!` list if not
  already (it IS at `serial.rs:2149` — verify the new fields don't
  trigger; they're guarded).

### 3.3 — `seek` new tool

`src/server.rs`: add a new `#[tool(...)] async fn seek(...)` handler (23
tools total after this — update doc-drift counts, see 1.4). Place it
near `read`/`flush` (POSIX file vocabulary: open/close/read/write/seek/
flush).

`src/tools/types.rs`: add `SeekArgs` + `SeekResult`:
```rust
pub struct SeekArgs {
    pub connection_id: String,
    pub to: SeekTarget,
}
pub enum SeekTarget {
    LiveEdge,           // { "to": "live_edge" }
    BufferStart,        // { "to": "buffer_start" }
    Offset(u64),        // { "to": { "offset": N } }
    Delta(i64),         // { "to": { "delta": -N } } — negative = re-read
}
pub struct SeekResult {
    pub connection_id: String,
    pub name: Option<String>,
    /// The clamped new cursor position.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub cursor: u64,
    /// Requested target before clamping (for diagnostics).
    pub requested: String,  // or keep the enum; pick the cleaner shape
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub start_offset: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub end_offset: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub buffered_remaining: u64,
}
```
`SeekTarget` needs `#[serde(tag = "type", rename_all = "snake_case")]` or
the flat shape the plan shows (`{ to: "live_edge" }` / `{ to: { offset: N
} }`). Use the serde-tagged-enum shape that matches `TxFramingMode`'s
pattern (`#[serde(tag = "type")]` with `LiveEdge`/`BufferStart` as
unit-ish and `Offset`/`Delta` as struct variants) OR a simpler
`#[serde(untagged)]` enum. Pick the shape that round-trips the plan's
JSON examples cleanly and passes clippy.

`src/tools/io_ops.rs` (or a new `src/tools/seek.rs` — pick where `read`
lives): `pub async fn seek(connections, rx_sessions, args) -> Result<...>`:
- lookup connection + session (`rx_sessions.get(connection_id)` — must
  already exist; `seek` on an unopened connection is a not-found error).
- read current `start_offset`/`end_offset` from `session.ring()`.
- compute target cursor from `SeekTarget`:
  - `LiveEdge` → `end_offset`.
  - `BufferStart` → `start_offset`.
  - `Offset(n)` → `n`.
  - `Delta(d)` → `current_cursor as i64 + d` (saturate at 0 / clamp).
- clamp into `[start_offset, end_offset]`.
- `session.set_read_cursor(clamped)`.
- `buffered_remaining = end_offset - clamped`.
- return `SeekResult`.

Tool description (server.rs): "Move the shared RX read cursor. Non-
destructive: `live_edge` skips past buffered data without discarding it
(unlike `flush`); `buffer_start` re-reads everything retained; `offset`
seeks to an absolute stream offset from a previous result; `delta` seeks
relatively (negative = re-read). Out-of-range targets clamp into
[start_offset, end_offset] and report the clamp. Returns the new cursor
plus ring bounds, so it doubles as a 'what's in the buffer' query."

### 3.4 — `flush(input)` ring semantics

`src/tools/io_ops.rs:179-237` (`flush`). For `FlushTarget::Input`:
- keep `connection.flush_buffers(FlushTarget::Input)` (kernel `tcflush`).
- ADD: `session.ring().clear()` + `session.set_read_cursor(end_offset)`
  (clamp shared cursor to live edge). Also clamp all subscription
  cursors — but subscriptions are Phase 2; for Phase 1.2's temporary
  `ConsumerRegistry`, there are no subscription cursors yet. So just
  clear the ring + shared cursor. Phase 2 adds subscription-cursor
  clamping.
- Need `rx_sessions` in `flush` — thread it through (flush currently only
  takes `connections` + `tx_sessions`). Add `rx_sessions` param + update
  `server.rs:312-316` flush handler + the `io_ops::flush` signature.
- Update the `flush` tool description (`server.rs:308`): add the warning
  "discards all unread buffered RX data; to skip past buffered data
  without destroying it, use `seek` to live_edge instead."

### 3.5 — `get_status` ring fields

`src/tools/types.rs:411-447` (`GetStatusResult`): add:
```rust
#[schemars(schema_with = "crate::schema_helpers::uint_schema")]
pub rx_buffer_size: usize,
#[schemars(schema_with = "crate::schema_helpers::uint_schema")]
pub rx_start_offset: u64,
#[schemars(schema_with = "crate::schema_helpers::uint_schema")]
pub rx_end_offset: u64,
#[schemars(schema_with = "crate::schema_helpers::uint_schema")]
pub rx_cursor: u64,
#[schemars(schema_with = "crate::schema_helpers::uint_schema")]
pub rx_buffered_unread: u64,
#[schemars(schema_with = "crate::schema_helpers::uint_schema")]
pub rx_bytes_lost_total: u64,
```
`src/tools/port_ops.rs:113-128` (`get_status`): populate from
`session.ring()` + `session.read_cursor()`. Need `rx_sessions` in
`get_status` — thread it through (currently only takes `connections`).
`rx_bytes_lost_total` — the ring needs a lifetime counter for wrap loss
(Phase 1.1's `RxRing` may not track this; if not, add a `bytes_lost_total`
`AtomicU64` to `RxRing` incremented in `append` when bytes are dropped to
wrap, and expose via `pub fn bytes_lost_total(&self) -> u64`). If the
session doesn't exist (connection open but no session — shouldn't happen
after Phase 1.2 since open creates the session, but defensive), report
zeros.

### 3.6 — `open`/`open_profile`/profiles `rx_buffer_size`

- `OpenArgs` (`types.rs:13-49`): add
  ```rust
  #[serde(default = "default_rx_buffer_size")]
  #[schemars(schema_with = "crate::schema_helpers::read_max_buffered_bytes_schema")]
  pub rx_buffer_size: usize,
  ```
  + `fn default_rx_buffer_size() -> usize { DEFAULT_RX_BUFFER_SIZE }`.
  Doc: "Per-connection RX ring buffer size in bytes. The ring retains
  this much RX history between reads/subscribes. Default 256 KiB (~23s
  of 115200-baud traffic). Open-time only; reopen to resize. Validated
  against the buffer budget pool and a 16 MiB ceiling."
- `OpenProfileArgs` (`types.rs:226-240`): same field.
- `ProfileDefaults` (`profiles.rs:53`): same field (so profiles carry it).
- `parse_open_args` (`helpers.rs:787-804`): carry `rx_buffer_size` into
  `ConnectionConfig` (add the field to `ConnectionConfig` at
  `serial.rs:196`, or pass it separately to `port_ops::open` — pick the
  cleaner shape; `ConnectionConfig` is the natural home since it's the
  open-time config bundle).
- `port_ops::open` (`:40-85`): validate `rx_buffer_size` (1..=
  `MAX_RX_BUFFER_SIZE`), pass to `rx_sessions.get_or_create(connection,
  rx_buffer_size)`. Replace the `DEFAULT_RX_BUFFER_SIZE` placeholder from
  Phase 1.2 with the arg value.
- `port_ops::open_profile` (`:225-237`): same — use the profile's
  `rx_buffer_size` or the arg's.
- `ConnectionConfig` (`serial.rs:196`): add `pub rx_buffer_size: usize`.
  Update `SerialConnection::from_io_with_config` to store it (or the
  session reads it at `get_or_create` — but `get_or_create` takes
  `ring_capacity` as a param, so `port_ops::open` passes
  `config.rx_buffer_size`). Verify `from_io` / `from_io_with_config`
  + tests that build `ConnectionConfig` get the field (use
  `DEFAULT_RX_BUFFER_SIZE` default in test helpers).

### 1.4 — schema guards + tool-count drift (same commit)

- `src/serial.rs:2149` `check_schema!(read_result_has_no_uint_formats,
  ReadResult)` — already present; the new u64 fields are all guarded
  with `uint_schema`/`option_uint_schema`. Add `check_schema!` for
  `SeekResult`, `SeekArgs` (if it has uint fields), `GetStatusResult`
  (already at `:2159`? verify — if not, add). Run `cargo test --lib
  serial::schema` to confirm no `uintN` formats leak.
- Tool count 22 → 23: update README.md `**22 tools:**` → `**23 tools:**`
  + add `seek` to the capabilities list. Update `Cargo.toml`
  `description` "22 tools" → "23 tools". Update `server.json`
  `description` "22 tools" → "23 tools". The `doc_drift.rs` guards
  (`readme_tool_count_matches_code`, `cargo_toml_description...`,
  `server_json_description...`, `readme_tool_list_matches_count`) will
  fail until these are updated — update them in this commit.
- `read`/`flush`/`get_status` tool descriptions updated per 3.1/3.4/3.5.
- `read` description: rewrite the "Returns only future bytes — data
  received after the call starts, not previously buffered data" sentence
  (now FALSE) to "Returns buffered-but-unread bytes from the
  connection's cursor (like `cat`); consuming by default. With `peek:
  true`, returns without advancing the cursor. Pattern matching checks
  buffered history first, then waits for new bytes. With `rx_framing`/
  `rx_parser`/`protocol`, splits and interprets frames (…) [keep the
  framing/parser/preset/checksum/framing-error clauses]. Results carry
  `from_offset`/`next_offset`/`bytes_lost`/`buffered_remaining`."

### 1.5 — tests (un-ignore + new)

- **Un-ignore** the 6 tests Phase 1.2 deferred:
  `tests/protocol_emulator.rs::protocol_emulator_workflow`,
  `tests/protocol_emulator_binary.rs::protocol_emulator_binary_workflow`,
  `tests/serial_pty.rs::pty_device_write_then_client_read`,
  `pty_read_match_with_context_returns_shaped_payload`,
  `pty_read_match_with_zero_context_returns_only_matched_bytes`,
  `pty_read_match_without_context_returns_full_accumulated`.
  Remove the `#[ignore = …]` attributes. They should now pass because
  `read` reads from the ring (buffered bytes available immediately).
  If any still fails, investigate — the write-then-read race is the
  core contract this step delivers.
- **New PTY tests** (per plan step 5):
  - `pty_read_returns_buffered_bytes_immediately` (the cat case):
    write → read with no delay → bytes present, `stop_reason: "drained"`.
  - `pty_read_wrap_reports_bytes_lost`: small `rx_buffer_size` (e.g.
    8), write >8 bytes, read → `bytes_lost > 0`, `from_offset` =
    `start_offset`.
  - `pty_peek_does_not_advance_cursor`: write, peek → `next_offset` ==
    `from_offset`; read → returns same bytes.
  - `pty_seek_round_trip`: write, seek live_edge, read → empty; seek
    buffer_start, read → bytes return.
- **native_sim**: the reconnect-ring-persistence test (verify the ring
  retains bytes across disconnect/reconnect). Add or extend
  `native_sim_connection_lifecycle` if a suitable test exists; else add
  to `native_sim_validation`.
- **stdio/http integration**: the new tool surface (`seek` exists, read
  returns offset fields). `stdio_list_tools_returns_all_twenty_two_tools`
  (`stdio_integration.rs:70`) and `http_integration.rs:62`
  `list_tools_returns_all_twenty_two_tools` — rename + update count to
  23. Add a minimal `seek` round-trip test to http_integration if
  cheap.

## Out of scope

- Phase 2 (subscribe rewrite, ConsumerRegistry deletion, semantics
  unification). `subscribe` stays on the fanout path this commit.
- Phase 3 (blob resource, README/CHANGELOG sweep, 0.8.0 bump).
- No `transact` tool (FEATURES.md).
- No persistent per-connection framing decoder (deferred).
- No per-client cursors (deferred).
- No version bump.

## Relevant files

- `src/tools/io_ops.rs:78-177` `read` + `:179-237` `flush` — rewrite.
- `src/tools/helpers.rs:239-704` `read_bytes_via_session` → ring loop;
  `:714-776` `build_read_result` — offset fields + keep 0.7.3 contract.
- `src/tools/types.rs:13-49` `OpenArgs`; `:85-123` `ReadArgs`; `:126-130`
  `FlushArgs`; `:226-240` `OpenProfileArgs`; `:296-359` `ReadResult`;
  `:411-447` `GetStatusResult` — add fields + `SeekArgs`/`SeekResult`/
  `SeekTarget`.
- `src/server.rs:282-305` `read` handler; `:307-316` `flush` handler;
  `:113-128`-area `get_status` handler; add `seek` handler; thread
  `rx_sessions` into `flush`/`get_status`.
- `src/tools/port_ops.rs:40-85` `open`; `:113-128` `get_status`; `:225`
  `open_profile` — `rx_buffer_size` + `rx_sessions` threading.
- `src/serial.rs:196` `ConnectionConfig` — add `rx_buffer_size`;
  `from_io`/`from_io_with_config` + test support.
- `src/profiles.rs:53` `ProfileDefaults` — add `rx_buffer_size`.
- `src/rx_ring.rs` — add `bytes_lost_total` `AtomicU64` + accessor if not
  present.
- `src/limits.rs` — `DEFAULT_RX_BUFFER_SIZE`/`MAX_RX_BUFFER_SIZE` already
  present (Phase 1.2).
- `src/serial.rs:2149`-area `check_schema!` list — add `SeekResult` etc.
- `README.md`, `Cargo.toml`, `server.json` — 22→23 tools.
- `tests/protocol_emulator.rs`, `tests/protocol_emulator_binary.rs`,
  `tests/serial_pty.rs` — un-ignore 6 tests.
- `tests/stdio_integration.rs:70`, `tests/http_integration.rs:62` —
  rename + 23 count.
- `tests/http_integration.rs` — add minimal seek test.

## Expected API / UX shape

- `read` returns buffered bytes immediately (cat), `stop_reason: "drained"`
  when no more buffered; waits only when empty/match-unsatisfied. `peek:
  true` never advances cursor. Results carry `from_offset`/`next_offset`/
  `bytes_lost`/`buffered_remaining`.
- `seek` new tool (23 total). Moves shared cursor; clamps; returns bounds.
- `flush(input)` clears ring + clamps cursor to live edge.
- `get_status` carries ring fields.
- `open`/`open_profile`/profiles carry `rx_buffer_size` (default 256 KiB).
- 6 deferred tests un-ignored and passing.
- Schema: all new u64 fields guarded; doc_drift 23 tools consistent.

## Test plan

- 6 un-ignored tests pass.
- New PTY tests (cat case, wrap+bytes_lost, peek, seek round-trip) pass.
- native_sim validation 56/56 + lifecycle 6/6 (reconnect ring persistence).
- stdio `list_tools` 23; http `list_tools` 23 + seek test.
- `cargo test --lib serial::schema` — no uintN formats.
- `cargo test --test doc_drift` — 8 tests (incl. 23-count + version
  guard) pass.
- Full gate: fmt, build, test, clippy `-D warnings`.

## Constraints and invariants (from AGENTS.md)

- No `unwrap`/`expect`/`println!`/`todo!`/`unreachable!` in production.
  (`.lock().expect("X mutex poisoned")` for std Mutex; unwrap in tests.)
- Conventional commits. No attribution footers.
- `RUSTFLAGS="-D warnings"` is CI's bar. `await_holding_lock`: the ring
  `Mutex` is held inside `read_from`/`append` (sync, no `.await` cross);
  `wait_for_data` drops the lock before `.await` (Phase 1.1 pattern).
- Every `uN`/`Option<uN>` field on `JsonSchema` structs →
  `uint_schema`/`option_uint_schema` + `check_schema!` list entry.
- Tool outputs need `output_schema` + `title`; `verify_all_tool_schemas`
  enforces. `seek` needs both.
- `read` framing-error result is `Ok` with `stop_reason: framing_error`
  (NOT `is_error: true`) + `error` field + hex fallback (0.7.3 contract
  preserved). Cursor advances past consumed bytes incl. malformed seq.
- `flush(input)` is strictly more destructive now — description must warn.
- `peek` never moves cursor, even on match.
- `seek` clamps, never errors on out-of-range.

## Verification commands

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --lib serial::schema
cargo test --test doc_drift
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

All must pass (native_sim 56/56 + 6/6; the 6 un-ignored tests green).

## Deliverable

Implement 3.1-3.6 + 1.4 + 1.5. Do not push, merge, open a PR, amend, or
force-push. After verification passes, commit with a message like:

```
feat(rx): read from ring (cat semantics + peek + offset fields), seek tool, flush ring clear, get_status ring fields, open rx_buffer_size
```

(One commit — it's a large but cohesive step. If the executor judges it
should split, acceptable: 3.1+3.2 (read rewrite) first, then
3.3-3.6+1.4+1.5. Use judgment; keep each commit green.)

Return a concise recap:
- files changed + key line ranges,
- the new `read` loop shape (one paragraph),
- `seek` tool shape + the 23-tool drift updates,
- `flush`/`get_status`/`open` changes,
- 6 un-ignored tests + new tests + results,
- native_sim results,
- commit hash(es) and message(s),
- any blockers or deviations,
- suggested follow-ups (Phase 2: subscribe rewrite).