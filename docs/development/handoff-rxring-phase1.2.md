# Handoff — RX Ring Phase 1.2: RxSession rework (always-on pump + ring + budget, keep ConsumerRegistry)

> Branch: `rx-ring-redesign` (HEAD: `e0b72b7`, Phase 1.1 landed). Do not
> switch branches.
> Source plan: [rx-ring-redesign-plan.md](rx-ring-redesign-plan.md) § Phase 1 step 2 + Budget integration.
> Scope: rework `src/rx_session.rs` so the pump runs from `open` to `close`
> and appends to a per-connection `RxRing`; the existing
> `ConsumerRegistry`/`register_blocking`/`register_streaming`/
> `prune_consumers` API stays intact and working (Phase 2 deletes it).
> Charge the ring against `BufferBudget` at open, release at close.

## Goal

The plan's two root causes (pump lifetime tied to consumers; fanout is
ephemeral channels) both disappear here: the pump runs from `open` to
`close` and appends every RX byte to an `RxRing`. The ring is the
budgeted allocation. The existing `read`/`subscribe` tools keep working
unchanged via the `ConsumerRegistry` fanout — the pump appends to the ring
*and* fans out to consumers simultaneously. Phase 2 rewrites `subscribe`
onto the ring and deletes the registry; Phase 1.3 rewrites `read`.

## Why

Phase 1.1 built the `RxRing` primitive in isolation. Phase 1.2 wires it
into the live pump so bytes are captured from `open` to `close` regardless
of active tool calls — the "boot banner gone before read starts" failure
mode ends as soon as this lands. Keeping the `ConsumerRegistry` temporarily
means `read`/`subscribe` still function on their old fanout path, so the
tree stays green between Phase 1.2 and Phase 1.3/2.

## In scope

### 2.1 — `RxSession` owns the `RxRing` + a shared read cursor

`src/rx_session.rs:126-289` (`RxSession` struct + impl). Add:
- `ring: RxRing` (the Phase 1.1 type) — owns the per-connection ring.
- `read_cursor: StdMutex<u64>` — the shared read cursor (`read` consumes,
  `seek`/`flush` move it). `u64` absolute offset; starts at 0.
- `ring_capacity: usize` — stored so `get_status` (Phase 1.3) and
  budget release (close) can report it.
- `budget_reservation: StdMutex<Option<Box<dyn BufferReservation>>>` —
  RAII reservation for the ring; `None` means no budget charged (e.g. in
  tests using `UnlimitedBudget` where the reservation is still held but
  irrelevant). Set at construction (open), dropped at `shutdown_and_join`.

Keep all existing fields: `connection_id`, `connection`, `consumers`
(`Arc<StdMutex<ConsumerRegistry>>`), `pump_task`, `pump_token`.

### 2.2 — Construction charges budget

`RxSession::new` (`:138-147`) currently takes only `Arc<SerialConnection>`.
Change to:

```rust
pub fn new(
    connection: Arc<SerialConnection>,
    ring_capacity: usize,
    budget: &Arc<dyn BufferBudget>,
) -> Result<Self, String>
```

- Validate `ring_capacity > 0` (the `RxRing::new(0)` panic must not fire; the
  caller validates first but double-check here too — return `Err` not
  panic).
- `budget.try_reserve(ring_capacity)` → on `Err`, map via
  `crate::tools::helpers::map_budget_err("open.rx_buffer_size", e)` and
  return `Err(String)`. On `Ok`, store the reservation.
- Construct the `RxRing::new(ring_capacity)` (now safe — capacity > 0).
- Initialize `read_cursor: StdMutex::new(0)`.

`new` is now fallible. Update `RxSessionManager::get_or_create`
(`:388-401`) to take the budget + ring_capacity and propagate the `?` on
`RxSession::new`. The manager now needs the budget at construction time —
thread it through `RxSessionManager::new` (or pass per-call; pick the
cleaner shape — a stored `Arc<dyn BufferBudget>` on the manager mirrors
how `SerialHandler` already holds it).

### 2.3 — Pump runs from open to close; appends to ring + fans out

`pump_loop` (`:293-358`) currently fans out `RxEvent::Data(chunk)` to
`ConsumerRegistry`. Change so each `Ok(n)` read:
1. `connection.log().rx_data(n)` — unchanged.
2. `let chunk = buf[..n].to_vec();` — unchanged.
3. **NEW:** `ring.append(&chunk);` — the ring is `Arc`-cloned into the
   pump task (or the pump holds an `Arc<RxRing>` / the ring is interior-
   mutable so `Arc<RxRing>` works). `append` is cheap (Mutex held briefly,
   `notify_waiters` after).
4. **UNCHANGED:** `consumers.lock().fanout(RxEvent::Data(chunk))` — the
   registry still gets the chunk so `read`/`subscribe` work unchanged.

The pump no longer exits when `reg.is_empty()` (that was the
consumer-tied-lifetime bug). Remove the `if reg.is_empty() { break }`
checks at `:314-317` and `:325-328`. The pump now runs until
`token.is_cancelled()` (shutdown) or a fatal read error. On fatal
disconnect (`:338-345`): keep `mark_disconnected`, do NOT break — instead
*pause* (the plan says "waits on connection state; on reconnect resumes
appending to the same ring"). Implement the pause: loop with a short sleep
(e.g. `tokio::time::sleep(50ms)`) while the connection state is
`Disconnected`/`Reconnecting`, re-checking `token.is_cancelled()` each
iteration; on `Open` resume normal `connection.read()`. If the connection
goes `Closed` (reconnect exhausted/disabled), break. The existing
`start_reconnect` (`serial.rs:1374-1416`) calls
`session.ensure_pump_running()` on success — under the new model the pump
is already running (paused), so `ensure_pump_running` becomes a no-op or
just re-checks the handle is alive. Verify the reconnect path still works:
the pump pauses during disconnect and resumes appending to the *same*
ring at the *same* offsets when `start_reconnect` flips state to `Open`.

`ensure_pump_running` (`:154-185`): keep the idempotent "start if not
running" shape, but now it's called ONCE at open (by
`get_or_create`/open wiring — see 2.4) instead of per-consumer-register.
The `register_blocking`/`register_streaming` (`:191-222`) calls to
`ensure_pump_running` can stay (they're idempotent no-ops once the pump is
already running) OR be removed (cleaner — the pump is always running from
open). Pick the cleaner option; if removing, verify `register_*` still
works (they just push to the registry; the pump is already running).

`prune_consumers` (`:236-252`): currently cancels the pump when no
consumers remain. Under the new model the pump must NOT stop when
consumers hit zero — it must keep capturing to the ring. Change:
`prune_closed()` still runs, but the `if reg.is_empty() { cancel }`
branch is removed. `prune_consumers` returns `bool` (was "did we cancel
the pump"); under the new model it returns `false` always (pump never
cancels for empty registry) OR change the return to `()` / keep `bool`
but always `false`. The `subscribe` loop (`stream_ops.rs:237`) calls
`prune_consumers` and, if true, calls `session.join_pump().await` — under
the new model `join_pump` must NOT be called on unsubscribe (the pump
stays alive for the connection). Make `prune_consumers` always return
`false` (or restructure so the subscribe loop's join_pump call is dead).
Verify `tests/tx_session.rs` and `stream_ops` tests that exercise
`prune_consumers` + `join_pump` still pass (they assert the old "pump
stops on empty" contract — those tests MUST be updated; see test plan).

### 2.4 — Open wiring: start the pump at open

Currently the pump starts lazily on first `register_blocking`/
`register_streaming` (`:202`, `:220`). Under the new model it starts at
`open`. The cleanest path: `RxSessionManager::get_or_create` (which is
called by `read`/`subscribe` AND should now also be called by `open`)
creates the session and calls `session.ensure_pump_running()` before
returning. But `open` (`port_ops.rs:40-85`) doesn't call `get_or_create`
today — it calls `connections.open(config)` then sets reconnect policy.
Add: after `connections.open(config)` succeeds, call
`rx_sessions.get_or_create(connection, ring_capacity, budget).await`,
get the session, and `session.ensure_pump_running()`. This requires
`open`/`open_profile` handlers to receive `rx_sessions` + `budget` (they
don't today). Thread them through `SerialHandler` (already holds both) into
the `open`/`open_profile` tool handlers (`server.rs:441-460` for
`open_profile`; find `open` similarly). The `rx_buffer_size` argument on
`OpenArgs`/`OpenProfileArgs` is a Phase 1.3 concern (the `open` tool
surface change); for Phase 1.2 use a default capacity (256 KiB constant
in `src/limits.rs`) so the pump starts with a sane ring without the tool
field existing yet. Phase 1.3 adds the `rx_buffer_size` field + wiring.

Add to `src/limits.rs`:
```rust
pub const DEFAULT_RX_BUFFER_SIZE: usize = 256 * 1024; // 256 KiB
pub const MAX_RX_BUFFER_SIZE: usize = 16 * 1024 * 1024; // 16 MiB per-connection ceiling
```
(Phase 1.3 wires `MAX_RX_BUFFER_SIZE` as a validation ceiling; Phase 1.2
just uses `DEFAULT_RX_BUFFER_SIZE`.)

### 2.5 — Close releases budget + joins pump

`server.rs:257` calls `self.rx_sessions.remove(&connection_id).await`
which calls `session.shutdown_and_join().await`. `shutdown_and_join`
(`:285-288`) calls `shutdown` (cancels token, fanouts `Closed`) then
`join_pump`. Add: drop the `budget_reservation` (take it out of its
`StdMutex` and let it drop, releasing the bytes back to the pool) — do
this in `shutdown` or `shutdown_and_join` before joining the pump, so the
pool bytes are available for the next open. The ring is dropped with the
session.

### 2.6 — Expose ring + cursor to Phase 1.3 (read-only accessors)

Phase 1.3 (`read`/`seek`/`flush` rewrite) needs to read the ring and move
the cursor. Add `pub(crate)` accessors on `RxSession`:
- `pub(crate) fn ring(&self) -> &RxRing` — borrow the ring (or `Arc<RxRing>`
  if the pump owns a clone — pick a shape that lets `read`/`seek` access
  without copying).
- `pub(crate) fn read_cursor(&self) -> u64` — current shared cursor.
- `pub(crate) fn set_read_cursor(&self, off: u64)` — `seek` uses this
  (Phase 1.3). Clamp inside `seek`'s logic, not here (raw setter).
- `pub(crate) fn ring_capacity(&self) -> usize`.

These are used by Phase 1.3 only; for Phase 1.2 they're unused (add
`#[allow(dead_code)]` is NOT needed — clippy only warns if truly unused
across the crate; since Phase 1.3 follows immediately, leave them. If
clippy complains in this commit, add `#[allow(dead_code)]` with a
"Phase 1.3 uses this" comment and remove it in 1.3).

## Out of scope

- Phase 1.3 (`read`/`seek`/`flush`/`get_status`/`open` `rx_buffer_size`
  field rewrite). The accessors in 2.6 are added but unused by the tools
  in this commit.
- Phase 1.4/1.5 (schema guards, integration tests).
- Phase 2 (subscribe rewrite, registry deletion).
- No `OpenArgs`/`OpenProfileArgs`/`Profile` field changes (Phase 1.3).
- No `ReadResult`/`GetStatusResult` schema changes (Phase 1.3).
- No CHANGELOG (internal; Phase 3 writes 0.8.0).
- No version bump.

## Relevant files and current behavior

- `src/rx_session.rs` — whole module rework. Key sites:
  - `:126-289` `RxSession` struct + impl (add ring/cursor/budget fields;
    change `new` signature; keep `register_*`/`prune_consumers`/`shutdown`
    working).
  - `:293-358` `pump_loop` (append to ring + fanout; pause on disconnect
    instead of break; remove empty-registry-exit).
  - `:236-252` `prune_consumers` (stop cancelling pump on empty).
  - `:367-427` `RxSessionManager` (thread budget + ring_capacity; ensure
    pump at get_or_create).
  - `:431-765` tests (update the ones asserting "pump stops on empty
    registry" / "join_pump after prune" — those contracts change).
- `src/serial.rs:1374-1416` `start_reconnect` — calls
  `session.ensure_pump_running()` after reconnect; verify still correct
  (pump pauses then resumes; the call is a no-op if already running).
- `src/server.rs:250-264` close handler — already calls
  `rx_sessions.remove`; budget release happens inside
  `shutdown_and_join` now.
- `src/server.rs:441-460` `open_profile` + the `open` handler — thread
  `rx_sessions` + `budget` through (find the `open` handler invocation;
  likely in `server.rs` near `:441`).
- `src/tools/port_ops.rs:40-85` `open` — add `rx_sessions` + `budget`
  params, call `get_or_create` + `ensure_pump_running` after
  `connections.open`.
- `src/tools/stream_ops.rs:237` `session.prune_consumers()` + `:238`
  `session.join_pump()` — the join_pump call becomes dead (prune never
  returns true); remove the join_pump call or gate it (cleaner: remove).
- `src/tools/io_ops.rs:108-116` `read` — reserves budget for
  `max_buffered_bytes` (the per-call accumulation). UNCHANGED in Phase
  1.2 (the ring has its own reservation; the per-call one stays for the
  output accumulation buffer). Leave as-is.
- `src/limits.rs` — add `DEFAULT_RX_BUFFER_SIZE` + `MAX_RX_BUFFER_SIZE`.
- `src/buffer_budget.rs` — `BufferBudget::try_reserve` (unchanged API;
  used by `RxSession::new`).
- `src/rx_ring.rs` — the Phase 1.1 primitive (unchanged; consumed here).

## Expected API / UX shape

- `RxSession::new` is fallible and takes `ring_capacity` + `budget`.
- `RxSessionManager::get_or_create` takes `ring_capacity` + `budget`.
- The pump runs open→close, appends to ring + fans out, pauses on
  disconnect, resumes on reconnect.
- `prune_consumers` no longer cancels the pump (returns false / no-op on
  empty registry).
- `register_blocking`/`register_streaming` still work (Phase 2 deletes
  them).
- `read`/`subscribe` behavior unchanged from the user's view (still
  future-only; Phase 1.3 makes `read` history-aware).
- Budget: the ring is charged at open, released at close; per-call
  `max_buffered_bytes` stays for output accumulation.

## Test plan

Update the `rx_session.rs` tests that assert the OLD pump-stops-on-empty
contract. Grep `rx_session.rs:431-765` for `prune_consumers`,
`join_pump`, "no consumers", `is_empty`. Likely:
- `pump_exits_when_no_consumers` (or similar) — DELETE or rewrite to
  assert the pump KEEPS RUNNING on empty registry (append to ring, no
  break). New test: `pump_continues_capturing_to_ring_when_no_consumers`
  — open session, don't register any consumer, write bytes to the
  loopback, assert `ring.end_offset() > 0` after a short wait.
- `prune_consumers_cancels_pump` (or similar) — DELETE or rewrite to
  assert `prune_consumers` returns false / pump keeps running.
- `join_pump_after_prune` tests — the `join_pump` call in
  `stream_ops.rs:238` is removed; any test asserting that path is dead.
  Update `tests/tx_session.rs` `flush_tool_handler_sequences_through_tx_session`
  if it exercises prune+join.

Add new tests:
- `ring_captures_bytes_without_consumers` — loopback connection, open
  session (no register), write `b"hi"`, assert `session.ring().end_offset()
  == 2` and `ring.read_from(0, 10).bytes == b"hi"`.
- `pump_pauses_on_disconnect_resumes_on_reconnect` — if feasible without
  hardware, simulate by marking the connection disconnected, verify the
  pump doesn't read (or reads time out), then mark open and verify it
  resumes appending. If too heavy for a unit test, defer to native_sim
  (Phase 1.5).
- `budget_charged_at_open_released_at_close` — use `AtomicBudget` with a
  known limit, open a session with `ring_capacity = X`, assert
  `budget.available() == limit - X`; close the session, assert
  `budget.available() == limit`.
- `budget_open_fails_if_insufficient` — request `ring_capacity` > pool,
  assert `RxSession::new` returns `Err` (not panic).
- `read_cursor_starts_at_zero` — `session.read_cursor() == 0` after open.

Existing tests that must still pass:
- All `register_blocking`/`register_streaming` fanout tests (the registry
  still works).
- `tx_session.rs` tests that use `rx_session` (verify they still compile
  with the new `new`/`get_or_create` signatures — they'll need the budget
  arg; use `UnlimitedBudget` or `AtomicBudget::new(large, large)`).
- `stream_ops.rs` subscribe tests (prune_consumers no longer joins; verify
  they don't hang on the removed join_pump).
- `io_ops.rs` read tests (unchanged; per-call budget reservation still
  works alongside the ring reservation).
- Full gate: fmt, build, test, clippy `-D warnings`, schema, doc_drift.
- native_sim: run after (Phase 1.2 changes pump behavior; the
  native_sim_connection_lifecycle reconnect tests are the real check).

## Constraints and invariants (from AGENTS.md)

- No `unwrap`/`expect`/`println!`/`todo!`/`unimplemented!` in production.
  (`.lock().expect("X mutex poisoned")` is the codebase convention for
  `std::sync::Mutex`; `unwrap` in tests fine.)
- Conventional commits. No attribution footers.
- `RUSTFLAGS="-D warnings"` is CI's bar. Watch `clippy::await_holding_lock`
  — the pump's `ring.append()` holds the ring `Mutex` briefly and does NOT
  cross `.await` (append is sync). The pause-on-disconnect sleep must NOT
  hold the ring or consumers lock.
- Open must enforce allowlist checks before `ConnectionManager::open()`
  (unchanged — `port_ops::open` already does this; the new
  `get_or_create` call is AFTER the allowlist check + open, so no
  change to the security invariant).
- Open/close changes must notify resource subscribers via
  `notify_resource_list_changed()` (unchanged — `server.rs` close handler
  already does this; verify the open path doesn't need a new notification
  for the session creation — it doesn't, the session is internal).
- `end_offset` monotonic across disconnect/reconnect (the ring persists;
  pause/resume appends to the same offsets).
- The ring is `Send + Sync` (verified in Phase 1.1); the pump task can
  hold an `Arc<RxRing>` or the session can hand out `&RxRing` borrows via
  the accessor — pick the shape that satisfies borrowck for `read`/`seek`
  in Phase 1.3.

## Verification commands

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --lib
cargo test --test tx_session
cargo test --test stdio_integration
cargo test --test http_integration
cargo test --test doc_drift
cargo clippy --all-targets --locked -- -D warnings
cargo test --lib serial::schema
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
cargo test --test native_sim_validation -- --ignored
```

All must pass. native_sim is the critical check for the pause/resume-on-
reconnect behavior.

## Deliverable

Implement 2.1-2.6. Do not push, merge, open a PR, amend, or force-push.
After verification passes, commit with a message like:

```
feat(rx): always-on pump + RxRing capture from open to close, budget ring at open
```

Stage only intended files (likely `src/rx_session.rs`, `src/serial.rs`,
`src/server.rs`, `src/tools/port_ops.rs`, `src/tools/stream_ops.rs`,
`src/limits.rs`, and any test files updated).

Return a concise recap:
- files changed + key line ranges,
- the new `RxSession::new` / `get_or_create` signatures,
- the pump pause/resume shape (one paragraph),
- what stayed compatible (register_*/ConsumerRegistry still working),
- tests updated (old pump-stops-on-empty rewritten) + new tests + results,
- native_sim validation + lifecycle results,
- commit hash and message,
- any blockers or deviations,
- suggested follow-ups (Phase 1.3: read/seek/flush rewrite onto the ring).