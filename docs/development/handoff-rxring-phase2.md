# Handoff — RX Ring Phase 2: subscribe rewrite as cursor follower + semantics unification

> Branch: `rx-ring-redesign` (HEAD: `ccb897c`, Phase 1 + test fixes
> landed). Do not switch branches.
> Source plan: [rx-ring-redesign-plan.md](rx-ring-redesign-plan.md) § Phase 2.
> Scope: rewrite `subscribe` as a cursor follower reading from the `RxRing`
> (with `from` parameter for history replay + gap reporting), delete
> `ConsumerRegistry`/`RxEvent`/`register_*`/`prune_consumers`, keep
> `consume_frames`/`RxFrameSink`, apply the unified-semantics table
> (construction-error hard-failure for subscribe), un-ignore the
  protocol_emulator subscribe stage, update AGENTS.md + the design note.

## Goal

`subscribe` becomes a cursor follower (`tail -f`): each subscription owns
a private cursor and a task that loops `ring.read_from(cursor)` →
framing/matching → emit notification → `wait_for_data`. The new `from`
parameter replays buffered history. Slow consumers get `bytes_lost` gap
notifications instead of silently dying. Delete the fanout infrastructure
(`ConsumerRegistry`, `RxEvent`, `register_blocking`/`register_streaming`/
`prune_consumers`). Construction errors hard-fail subscribe (no more
degrade-to-raw).

## Why

Phase 1 made `read` ring-based; `subscribe` is still on the old fanout
path, which races with the always-on pump (the protocol_emulator
subscribe stage is still ignored). The plan's Phase 2 completes the
ring migration: both tools read from the ring, no fanout, no silent
consumer drops.

## In scope

### 2.1 — `subscribe` rewrite onto the ring

`src/tools/stream_ops.rs:65-203` (`subscribe`) + `:365-746`
(`stream_rx_via_session`).

- Remove `session.register_streaming()` + `event_rx` from `subscribe`.
  The subscription no longer uses the `ConsumerRegistry` fanout.
- Remove `session.prune_consumers()` from `unsubscribe` (`:239-241`).
- `stream_rx_via_session` becomes `stream_rx_from_ring` — same shape as
  `read_bytes_from_ring` (Phase 1.3) but:
  - Uses a **private cursor** (not the shared read cursor). Initialize
    from the `from` parameter (see 2.2).
  - Emits notifications per chunk/frame (via `SubscribeFrameSink`) instead
    of accumulating into a result.
  - Does NOT advance the shared read cursor (subscriptions don't
    consume; `read` and `subscribe` coexist without stealing).
  - Gap reporting: if `ring.read_from(private_cursor, …).bytes_lost > 0`,
    include `bytes_lost` in the next notification and continue from
    `start_offset` (the ring's clamped position). The subscription never
    silently dies.
  - Stop semantics (`timeout_ms`, `match`, `max_bytes` budget,
    unsubscribe, close, framing error) keep the `RxStopController`
    vocabulary; final notification gains offset fields
    (`from_offset`/`next_offset`/`bytes_lost`).
  - The `RxStopController` bytes_observed/bytes_returned track the
    subscription's private cursor, not the shared one.
  - Keep `consume_frames` + `SubscribeFrameSink` for per-window frame
    dispatch (the plan explicitly says keep `rx_consume.rs`).
  - The framing-error path (0.7.3 contract) stays: final notification
    with `stop_reason: framing_error` + `error` field + partial data.
    The private cursor advances past consumed bytes (same as read's
    cursor-position-on-framing-error contract).

### 2.2 — `from` parameter on `SubscribeArgs`

`src/tools/types.rs:154-194` (`SubscribeArgs`): add:
```rust
/// Where to start reading from. `"now"` (default; live edge — today's
/// behavior), `"cursor"` (start at the shared read cursor — replay what
/// `read` hasn't consumed), `"buffer_start"` (replay everything retained,
/// then go live), or `{"offset": N}` (absolute). Replayed history flows
/// through the same framing/match pipeline as live data.
#[serde(default)]
pub from: Option<SubscribeFrom>,
```
`SubscribeFrom` enum (serde-tagged or untagged — pick the cleanest):
```rust
pub enum SubscribeFrom {
    Now,          // "now"
    Cursor,       // "cursor"
    BufferStart,  // "buffer_start"
    Offset(u64),  // {"offset": N}
}
```
Default `Now` (when `from` is `None`). In `subscribe`, resolve `from` to
an initial private cursor:
- `Now` → `session.ring().end_offset()` (live edge).
- `Cursor` → `session.read_cursor()` (shared read cursor).
- `BufferStart` → `session.ring().start_offset()` (oldest retained).
- `Offset(n)` → `n` (clamped to `[start_offset, end_offset]` by
  `ring.read_from`).

### 2.3 — Delete fanout infrastructure

`src/rx_session.rs`:
- Delete `ConsumerRegistry` (`:88-118`), `RxConsumer` (`:64-77`),
  `RxEvent` (`:44-59`), `register_blocking` (`:191-204`),
  `register_streaming` (`:209-222`), `prune_consumers` (`:236-252`).
- Remove `consumers: Arc<StdMutex<ConsumerRegistry>>` from `RxSession`.
- `pump_loop` (`:293-358`): remove the `consumers.lock().fanout(…)` calls
  — the pump only appends to the ring now. Remove `reg.is_empty()` /
  `prune_closed()` checks (already done in Phase 1.2 but verify no
  remnants).
- `shutdown` (`:259-269`): remove the `consumers.lock().fanout(RxEvent::Closed)`
  call — subscriptions are independent tasks that detect close via
  `disconnect_state` / `connection.state()`.
- Remove `RxEvent` import from `tools/helpers.rs` (`:16`) and
  `tools/stream_ops.rs` (`:16`) — the enum is gone. Update
  `read_bytes_via_session` if it still references `RxEvent` (it should
  be dead — Phase 1.3 replaced it with `read_bytes_from_ring`; verify
  and delete the old function).
- Update `tests/tx_session.rs` — it uses `register_blocking` (`:239`,
  `:312`); replace with ring-based test setup or delete those test
  cases if they're testing the old fanout (they likely test
  `TxSession::flush_output` sequencing, which doesn't need the fanout —
  use the ring directly or mock around it).

### 2.4 — Construction-error hard-failure for subscribe

`stream_rx_from_ring`: the framing decoder init
(`stream_ops.rs:408-421`) currently degrades bad framing configs to raw
mode with `warn!` (because subscribe is a background task that already
returned `Ok(SubscribeResult)` and can't surface a sync error). The plan
says: "Construction errors hard-fail both tools — subscribe stops
degrading to raw."

But subscribe has already returned `Ok(SubscribeResult)` to the client
before the streaming task starts — it can't return an `Err` retroactively.
Instead: **validate the framing config BEFORE spawning the task**, in the
`subscribe` handler (`:65-203`). Call `FrameDecoder::new(cfg, parser)` in
the handler (not in the task); if it errors, return `Err(String)` from
`subscribe` (the client gets a tool error, not a degraded stream). This
matches `read`'s behavior (Phase 1.3: read propagates construction
errors). Move the decoder construction from the task into the handler,
pass the constructed `FrameDecoder` into the task. If construction fails,
return `Err` before spawning.

### 2.5 — Gap reporting

In `stream_rx_from_ring`, after each `ring.read_from(private_cursor, …)`:
- If `slice.bytes_lost > 0`: include `"bytes_lost": slice.bytes_lost` in
  the next data/frame notification (or emit a dedicated gap notification
  — pick the cleaner UX; the plan says "a notification field
  `bytes_lost: N` on its next emission"). Continue from `slice.from_offset`
  (the clamped start). The subscription never silently dies.
- The `bytes_lost` field is additive to the notification JSON payload.

### 2.6 — Un-ignore protocol_emulator subscribe stage

`tests/protocol_emulator.rs::protocol_emulator_workflow` — the subscribe
stage (Stage 2) was ignored because the fanout raced with the always-on
pump. Under the ring-based subscribe, use `from: "buffer_start"` (or
`from: "cursor"`) to replay buffered history — the write-then-subscribe
ordering works because the subscribe replays from the ring. Un-ignore
the test; update Stage 2 to use `from: "buffer_start"` in the subscribe
call (or reorder to subscribe-before-write, which also works). If the
test still fails after the rewrite, investigate — the subscribe should
see the buffered response.

### 2.7 — Update AGENTS.md + design note

- `AGENTS.md` "Frame pipeline" section: update the subscribe-specific
  bullets (the `ConsumerRegistry`/fanout text is gone; subscribe is a
  cursor follower now). Update the "Invariants easy to break" section:
  the read/subscribe asymmetries are unified (both read from the ring;
  the old "read keeps later frames / subscribe drops them" and
  "bytes_returned definition" asymmetries are gone). Add the cursor-
  position-on-framing-error contract. Add the `from` parameter +
  `bytes_lost` gap reporting.
- Find and update the `rx-read-vs-subscribe-semantics` design note (grep
  for it in `docs/` — the plan says "its 'the two loops legitimately
  differ' guidance is superseded by this table"). Replace with the
  unified-semantics table from the plan, or add a "SUPERSEDED by the RX
  ring redesign" header pointing to the plan.

## Out of scope

- Phase 3 (blob resource, README/CHANGELOG sweep, 0.8.0 bump).
- No `transact` tool.
- No persistent per-connection framing decoder.
- No per-client cursors.
- No version bump.

## Relevant files

- `src/tools/stream_ops.rs:65-203` `subscribe` + `:205-248` `unsubscribe`
  + `:250-300` `SubscribeFrameSink` + `:365-746` `stream_rx_via_session`
  — rewrite.
- `src/tools/types.rs:154-194` `SubscribeArgs` — add `from`/`SubscribeFrom`.
- `src/rx_session.rs` — delete `ConsumerRegistry`/`RxConsumer`/`RxEvent`/
  `register_*`/`prune_consumers`; update `pump_loop` (remove fanout);
  update `shutdown`.
- `src/tools/helpers.rs:16` — remove `RxEvent` import; delete dead
  `read_bytes_via_session` if still present.
- `src/tools/io_ops.rs` — remove `RxEvent`/`register_blocking` references
  if any remain.
- `tests/tx_session.rs:239,312` — update tests that use
  `register_blocking`.
- `tests/protocol_emulator.rs` — un-ignore + update Stage 2.
- `AGENTS.md` — update Frame pipeline + Invariants sections.
- `docs/` — find + update the rx-read-vs-subscribe-semantics design note.

## Expected API / UX shape

- `subscribe` takes `from` (`"now"` default / `"cursor"` /
  `"buffer_start"` / `{"offset": N}`).
- Subscriptions are cursor followers; no fanout; no silent drops.
- `bytes_lost` in notifications when the private cursor falls behind.
- Construction errors hard-fail subscribe (tool error, not degraded
  stream).
- `ConsumerRegistry`/`RxEvent`/`register_*`/`prune_consumers` deleted.
- protocol_emulator test un-ignored and passing.
- AGENTS.md + design note updated.

## Test plan

- `protocol_emulator_workflow` un-ignored and passing (Stage 2 with
  `from: "buffer_start"` or subscribe-before-write).
- `tests/tx_session.rs` updated (no `register_blocking`) + passing.
- native_sim validation 56/56 + lifecycle 6/6 (subscribe tests may need
  updates for `from` parameter — if any subscribe test does
  write-then-subscribe, add `from: "buffer_start"` or reorder).
- New tests: replay-from-history (subscribe with `from: "buffer_start"`
  after data arrives → gets the data), lag → gap → continue (small ring
  + slow consumer → `bytes_lost` notification + continues),
  read+subscribe coexistence (subscribe doesn't move shared cursor).
- `cargo test --lib serial::schema` — `SubscribeArgs` with `from` field
  (if `SubscribeFrom` has uint fields, guard them).
- `cargo test --test doc_drift` — tool count still 23 (no new tool).
- Full gate: fmt, build, test, clippy `-D warnings`.

## Constraints and invariants (from AGENTS.md)

- No `unwrap`/`expect`/`println!`/`todo!`/`unreachable!` in production.
- Conventional commits. No attribution footers.
- `RUSTFLAGS="-D warnings"` is CI's bar.
- Keep `consume_frames`/`RxFrameSink` (plan explicitly says so).
- Subscriptions do NOT move the shared read cursor.
- `bytes_lost` is always observable, never silent.
- Construction errors hard-fail both tools (subscribe no longer degrades).
- `await_holding_lock`: ring Mutex not held across `.await`.
- Do not push, merge, open a PR, amend, or force-push.

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
cargo test --test protocol_emulator
cargo test --test protocol_emulator_binary
cargo test --test serial_pty
```

All must pass. native_sim 56/56 + lifecycle 6/6. protocol_emulator
un-ignored + passing.

## Deliverable

Implement 2.1-2.7. Do not push, merge, open a PR, amend, or force-push.
After verification passes, commit with a message like:

```
feat(rx): subscribe as cursor follower with from/history replay + gap reporting; delete ConsumerRegistry fanout
```

Stage only intended files.

Return a concise recap:
- files changed + key line ranges,
- the new subscribe loop shape (one paragraph),
- `from` parameter + `SubscribeFrom` shape,
- what was deleted (ConsumerRegistry/RxEvent/register_*/prune),
- construction-error hard-failure shape,
- protocol_emulator un-ignored + passing,
- native_sim + lifecycle results,
- AGENTS.md + design note updates,
- commit hash and message,
- any blockers or deviations,
- suggested follow-ups (Phase 3: polish + release 0.8.0).