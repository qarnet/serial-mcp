# Handoff — RX Ring Phase 1.1: `src/rx_ring.rs` + exhaustive tests + proptest

> Branch: `rx-ring-redesign` (HEAD: `98e5349`, plan review updates). Do not
> switch branches.
> Source plan: [rx-ring-redesign-plan.md](rx-ring-redesign-plan.md) § Phase 1 step 1.
> Scope: one new module — the `RxRing` data structure — with exhaustive
> unit tests and a proptest. No integration with `RxSession` or the tools
> yet (that's steps 2-5). Lands green on its own; the module is exercised
> only by its own `#[cfg(test)]` block until step 2 wires it in.

## Goal

Create `src/rx_ring.rs` implementing the ring buffer described in the plan's
"Core model" and "RxRing" sections. It is a sliding window over a byte
stream with absolute u64 offsets, interior-mutable + `Notify`-based
wakeups, fixed capacity, wrap-on-overflow with observable `bytes_lost`.

## Why

The ring is the foundation for the entire Phase 1-3 rewrite. It must be
correct in isolation before `RxSession` is reworked onto it (step 2). The
plan calls for "exhaustive unit tests (wrap, gap accounting, clamp, notify
wakeups, zero-capacity rejection) + proptest (append/read sequences
preserve stream bytes and offset arithmetic)".

## In scope

### 1. Create `src/rx_ring.rs`

Implement the API from the plan (§ RxRing, lines 99-122):

```rust
pub struct RxRing {
    // ring storage, capacity fixed at construction
    // start_offset / end_offset as u64 absolute stream offsets
    // tokio::sync::Notify for "new data appended"
}

pub struct RingSlice {
    pub bytes: Vec<u8>,
    pub from_offset: u64,   // clamped-to-start_offset actual read position
    pub next_offset: u64,   // cursor value after this slice
    pub bytes_lost: u64,    // requested_cursor < start_offset gap, else 0
}

impl RxRing {
    pub fn new(capacity: usize) -> Self;
    pub fn append(&self, bytes: &[u8]);              // pump only; wraps, notifies
    pub fn read_from(&self, cursor: u64, max: usize) -> RingSlice;
    pub fn clear(&self);                             // start = end (flush)
    pub fn start_offset(&self) -> u64;
    pub fn end_offset(&self) -> u64;
    pub async fn wait_for_data(&self, after: u64);   // Notify-based
}
```

Design notes (from the plan + required for correctness):

- **Interior mutability + `Notify`.** The pump appends without async locks
  on the hot path. Use `Mutex<Vec<u8>>` for the ring storage (the ring is
  a `Vec<u8>` of length `capacity`, treated as a circular buffer via
  head/tail indices) plus `AtomicU64` for `start_offset`/`end_offset`
  (or a single `Mutex` holding both + the buffer — pick whichever is
  cleaner; the hot path is `append` which must be cheap). `Notify` for
  wakeups: `append` calls `notify_waiters()` after extending; readers
  blocked in `wait_for_data(after)` wake and re-check `end_offset()`.
- **Absolute u64 offsets.** `end_offset` = total bytes appended since
  construction (monotonic, never wraps even when the ring storage wraps).
  `start_offset` = `max(0, end_offset - retained)` where `retained` is the
  number of valid bytes currently in the ring (`min(end_offset -
  start_offset, capacity)`). Bytes below `start_offset` are gone (wrapped
  out or never stored).
- **`append` semantics.** Appends `bytes` to the live edge. If the ring
  has capacity `C` and `bytes.len() > C`, only the last `C` bytes are
  retained (older bytes in this append are dropped); `start_offset` and
  `end_offset` both advance by `bytes.len()`, so `start_offset` =
  `end_offset - C` after. If `bytes.len() <= C - retained`, the bytes
  fit and only `end_offset` advances. The wrap is the standard circular-
  buffer wrap: write into `buf[end_pos % C ..]`, splitting across the
  physical boundary as needed. `notify_waiters()` after.
- **`read_from(cursor, max)`** (never blocks, never fails):
  - Clamp `cursor` to `max(cursor, start_offset)`. If `cursor >
    end_offset`, return an empty slice (`bytes: vec![], from_offset:
    cursor (or end_offset — pick one and document), next_offset: same,
    bytes_lost: 0`). The plan says "emptiness and gaps are data, not
    errors".
  - If `cursor < start_offset`: `bytes_lost = start_offset - cursor`;
    the read starts at `start_offset`. Else `bytes_lost = 0`.
  - Read `min(max, end_offset - clamped_cursor)` bytes from the ring
    storage (reverse the modular mapping: physical index = `(head +
    (logical - start_offset)) % C`).
  - `from_offset` = the clamped read start (>= start_offset). `next_offset`
    = `from_offset + bytes.len()`. `bytes_lost` as above.
- **`clear`**: set `start_offset = end_offset` (retained = 0). The storage
  contents are irrelevant after this; a subsequent `read_from` returns
  empty until more `append`s land. Used by `flush(target: input)` in
  step 3.
- **`wait_for_data(after)`:** async; returns once `end_offset() > after`
  (new data appended past `after`). Implementation: loop `if end_offset()
  > after { return }` else `notify.notified().await`. Use the standard
  `Notify` pattern (register interest before re-checking to avoid lost
  wakeups — `Notify::notified()` returns a future that captures the
  permit; the canonical pattern is `loop { let fut = notify.notified();
  if end_offset() > after { return; } fut.await; }` or equivalent. Watch
  for the "registered after notify_waiters, missed wakeup" race).
- **Zero-capacity rejection:** `RxRing::new(0)` returns a ring that
  rejects all appends (or panics at construction — the plan says
  "zero-capacity rejection"; prefer returning a `Result` or panicking
  with a clear message). Decide: the plan's `new(capacity: usize) -> Self`
  signature suggests infallible; panic on `capacity == 0` with
  `assert!(capacity > 0, "RxRing capacity must be > 0")`. The caller
  (`open` in step 3) validates `rx_buffer_size > 0` before construction.
  Add a test that `RxRing::new(0)` panics (`#[should_panic]`).

### 2. Register the module

- Add `mod rx_ring;` (or `pub(crate) mod rx_ring;`) to `src/lib.rs`.
  Grep `mod rx_session` to find the existing module registration and
  place `rx_ring` near it.
- Export the types the rest of the crate will need in step 2-3:
  `pub(crate) use rx_ring::{RxRing, RingSlice};` from `lib.rs` if the
  crate pattern matches `rx_session`'s visibility. Grep how `rx_session`
  is exported and mirror it.

### 3. Exhaustive unit tests (in `src/rx_ring.rs` `#[cfg(test)]`)

The plan lists: "wrap, gap accounting, clamp, notify wakeups, zero-
capacity rejection". Concretely:

- `new_ring_has_zero_offsets` — fresh ring: `start_offset == 0`,
  `end_offset == 0`, `read_from(0, 10).bytes.is_empty()`.
- `append_advances_end_offset` — append `b"abc"` → `end_offset == 3`,
  `start_offset == 0`.
- `read_from_returns_appended_bytes` — append `b"abc"`, `read_from(0,
  10)` → `bytes == b"abc"`, `from_offset == 0`, `next_offset == 3`,
  `bytes_lost == 0`.
- `read_from_advances_cursor_via_next_offset` — append `b"abcdef"`,
  `read_from(0, 3)` → `bytes == b"abc"`, `next_offset == 3`; then
  `read_from(3, 3)` → `bytes == b"def"`, `next_offset == 6`.
- `read_from_clamps_cursor_below_start_offset` — append enough to wrap
  (see wrap test below), then `read_from(old_cursor, ...)` →
  `bytes_lost == start_offset - old_cursor`, `from_offset ==
  start_offset`.
- `read_from_clamps_cursor_above_end_offset` — `read_from(end_offset +
  10, ...)` → empty slice, `bytes_lost == 0`, no panic.
- `read_from_respects_max` — append `b"abcdef"`, `read_from(0, 3)` →
  exactly 3 bytes.
- `append_wraps_and_drops_oldest` — capacity 4, append `b"abcd"` (full),
  append `b"efgh"` → `end_offset == 8`, `start_offset == 4`, `read_from(4,
  4).bytes == b"efgh"`, `read_from(0, 4).bytes_lost == 4`.
- `append_larger_than_capacity_keeps_only_tail` — capacity 4, append
  `b"abcdefgh"` (8 bytes) → `end_offset == 8`, `start_offset == 4`,
  `read_from(4, 4).bytes == b"efgh"`. (This is the "append > capacity"
  case; the first 4 bytes are dropped.)
- `append_empty_is_noop` — append `b""` → offsets unchanged, no notify
  storm (or a benign one — document either).
- `clear_resets_retained` — append `b"abc"`, `clear()`, then `read_from(0,
  10)` → empty; `start_offset == end_offset` (both == 3, since end_offset
  is monotonic — verify the plan's intent: clear makes retained=0 but
  end_offset stays monotonic so a reader at the old cursor sees no gap
  and no bytes. Document this in the test).
- `clear_then_append_resumes_at_end_offset` — after `clear`, append
  `b"xy"` → `end_offset == 5` (3 + 2), `read_from(3, 2).bytes == b"xy"`.
- `read_from_zero_max_returns_empty` — append `b"abc"`, `read_from(0, 0)`
  → empty, no panic.
- `wait_for_data_wakes_on_append` — async test: spawn a task that calls
  `wait_for_data(0)`, yield, append `b"x"`, the task wakes. Use
  `#[tokio::test]`. Verify the no-lost-wakeup pattern (the task must
  register interest BEFORE checking end_offset).
- `wait_for_data_returns_immediately_when_data_already_past_after` —
  append `b"abc"`, then `wait_for_data(2)` returns immediately (end_offset
  == 3 > 2).
- `new_zero_capacity_panics` — `#[should_panic]` on `RxRing::new(0)`.

### 4. Proptest (in `src/rx_ring.rs` `#[cfg(test)]`, or
`tests/proptest.rs` — pick the file that matches the repo convention;
`tests/proptest.rs` already has framing/codec proptests, so adding there
keeps all proptests in one place. The decision is the executor's; if
adding to `tests/proptest.rs`, the test needs `use serial_mcp::rx_ring::...`
which requires `RxRing` to be `pub` (not `pub(crate)`). If `pub(crate)`,
the proptest must live in `src/rx_ring.rs` `#[cfg(test)]`. Prefer
`src/rx_ring.rs` to keep `RxRing` `pub(crate)` — the ring is an internal
primitive, not part of the crate's public API.)

Proptest: model a sequence of `(action, expected)` operations and assert
the ring's state stays consistent. Concretely:

- Generate a stream of `Append(Vec<u8>)` and `Read { cursor: u64, max:
  usize }` operations.
- Maintain a reference model: a `Vec<u8>` of all bytes ever appended
  (the full stream) + the ring's `capacity`.
- After each operation, assert:
  - `ring.end_offset() == total_appended_bytes`.
  - `ring.start_offset() == total_appended_bytes.saturating_sub(capacity)`
    (when total > capacity; else 0).
  - For `Read { cursor, max }`: the ring's `bytes` equal the model's
    bytes from `max(cursor, start_offset)` to `min(end_offset,
    cursor+max)` (clamped); `bytes_lost == max(0, start_offset -
    cursor)`; `next_offset == from_offset + bytes.len()`.
- Use a small capacity (e.g. 0..16) so wraps happen often. Append
  lengths 0..32. Cursor 0..(total + a margin). Max 0..32.
- Name: `rx_ring_append_read_preserves_stream_and_offset_arithmetic`.

## Out of scope

- Phase 1 steps 2-5 (`RxSession` rework, `read`/`seek`/`flush` rewrite,
  schema guards, integration tests). This handoff is ONLY the ring data
  structure + its own tests.
- No change to `rx_session.rs`, `tools/`, `serial.rs`, `server.rs`,
  `buffer_budget.rs`, `limits.rs`.
- No schema changes (no new `JsonSchema` structs exposed to MCP yet).
- No CHANGELOG entry (internal; the Phase 3 release sweep will write the
  0.8.0 CHANGELOG).
- No version bump.
- The `Notify`-based `wait_for_data` is in scope (it's part of the ring
  API), but no callers use it yet — step 2 will.

## Relevant files and current behavior

- `src/rx_ring.rs` — NEW. The whole module.
- `src/lib.rs` — add `mod rx_ring;` near `mod rx_session;`. Grep
  `mod rx_session` to find the exact line and visibility pattern.
- `src/schema_helpers.rs:46-63` — `uint_schema` / `option_uint_schema`
  (not needed yet — `RingSlice` is `pub(crate)` and not `JsonSchema`-
  deriving in this step; step 3 will add `JsonSchema` result structs with
  u64 offset fields).
- `src/limits.rs` — reference for where to add `DEFAULT_RX_BUFFER_SIZE`
  (256 KiB) and a per-connection ceiling in step 3, NOT this step.
- `src/buffer_budget.rs:32-53` — `BufferBudget` trait (step 2 will charge
  the ring against it; not needed here, but the ring's `capacity` is the
  value that will be reserved).
- `tokio::sync::Notify` — for `wait_for_data`. The crate already depends
  on tokio (grep `tokio::sync` in `src/`).

## Expected API / UX shape

`pub(crate) struct RxRing` + `pub(crate) struct RingSlice` with the
methods above. No MCP tool surface change. No schema change. The module
compiles and its tests pass; nothing else uses it yet.

## Test plan

- All 16+ unit tests above pass.
- The proptest passes (run with `cargo test --lib rx_ring`).
- `cargo test --lib` — the new module's tests run; existing tests
  unaffected (the ring isn't wired in yet).
- `cargo test --test proptest` — unaffected (proptest is in
  `src/rx_ring.rs` if you keep it `pub(crate)`).
- `cargo test --test doc_drift` — unaffected (no tool count / preset
  change).
- `cargo test --lib serial::schema` — unaffected (no new `JsonSchema`
  types).
- Full gate: fmt, build, test, clippy `-D warnings`.

## Constraints and invariants (from AGENTS.md)

- No `unwrap`/`expect`/`println!`/committed `todo!`/`unimplemented!` in
  production code. (`unwrap`/`expect` in tests fine. The `assert!` in
  `new(0)` is a panic-on-misuse, acceptable per the existing
  `AtomicBudget::new` pattern at `buffer_budget.rs:80-84`.)
- Conventional commits. No attribution footers.
- `RUSTFLAGS="-D warnings"` is CI's bar. Watch for clippy lints on the
  `Mutex`/`Notify` usage (e.g. `clippy::await_holding_lock` — do NOT
  hold the ring `Mutex` across `.await`; `wait_for_data` must drop the
  lock before awaiting `notify.notified()`).
- The ring must be `Send + Sync` (it's shared between the pump task and
  reader tasks). `Mutex<Vec<u8>>` + `AtomicU64` + `Notify` are all
  `Send + Sync`.
- `end_offset` is monotonic (never decreases). `clear` keeps it monotonic
  (only `start_offset` catches up to `end_offset`; `end_offset` doesn't
  reset). Document this.

## Verification commands

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --lib rx_ring
cargo test --lib
cargo clippy --all-targets --locked -- -D warnings
cargo test --test doc_drift
cargo test --lib serial::schema
```

All must pass. (native_sim not needed — no integration change.)

## Deliverable

Implement the ring + tests + proptest + module registration. Do not
push, merge, open a PR, amend, or force-push. After verification passes,
commit with a message like:

```
feat(rx): add RxRing sliding-window buffer with absolute offsets, wrap+gap accounting, Notify wakeups
```

Stage only `src/rx_ring.rs` and `src/lib.rs`.

Return a concise recap:
- files changed + approximate line count,
- the `RxRing`/`RingSlice` final shape (fields + methods),
- the concurrency primitive choice (Mutex+AtomicU64+Notify or other) +
  the no-lost-wakeup pattern for `wait_for_data`,
- unit test count + proptest name + results,
- commit hash and message,
- any blockers or deviations,
- suggested follow-ups (Phase 1.2: `RxSession` rework onto the ring).