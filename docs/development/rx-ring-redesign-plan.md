# RX Ring Buffer & Cursor Redesign Plan

> Status: **design accepted 2026-07-07, all open items resolved; implementation
> not started. Prerequisite landed:** the `feature-additions` PR (including
> all phases of the since-completed-and-removed review-hardening plan) merged
> as PR #32 and released as **v0.7.3** on 2026-07-07. Implement this on a
> fresh branch off `main`. Source:
> external-agent usage review ([../agent-review.md](../agent-review.md)) plus
> design discussion with Thomas. Breaking changes are explicitly allowed —
> pre-1.0, no compatibility shims; this ships as **0.8.0**.
> The unified-semantics table and the "Cursor position on framing error"
> section were revised after the 0.7.3 decode-error semantics landed
> (partial results, `error` field, hex fallback, frames-before-error,
> checksum drop-and-count) — the rest of the plan text predates them.

## Why

An agent using serial-mcp for real firmware work failed the same way
repeatedly: `open` → flash/reset → `read`, expecting `cat`-like semantics.
Because `read` only returns bytes that arrive *after* the call, the boot
banner was gone by the time `read` started; `flush` before the read made it
worse by discarding the very bytes it wanted. The agent's own postmortem
(docs/agent-review.md) identifies the mental model everyone brings to a
serial port: the kernel buffers `/dev/ttyUSB0`, so reading after the event
works. serial-mcp's future-only semantics violate that model.

The root causes are two architecture decisions in `src/rx_session.rs`:

1. **Pump lifetime is tied to consumers.** The pump task only runs while a
   `read`/`subscribe` is registered (`ensure_pump_running` /
   `prune_consumers`). Between calls, nobody captures anything.
2. **Fanout is ephemeral channels, not a buffer.** Consumers see only
   post-registration bytes, and slow consumers are *silently removed* from
   the registry (`ConsumerRegistry::fanout` retains on `try_send`) — a
   subscription can die mid-stream without any error surfaced.

Both disappear with one structural change: extend the pump lifetime to the
connection lifetime, and put a ring buffer between the pump and all
consumers.

## Goals / non-goals

**Goals**

- Always-on capture: every RX byte from `open` to `close` lands in a
  per-connection ring buffer, whether or not any tool call is active.
- `read` behaves like `cat`: returns buffered-but-unread bytes immediately,
  consuming by default. Pattern matching checks history first, then waits.
- `subscribe` becomes a cursor follower (`tail -f`) with optional history
  replay and explicit gap reporting — no silent consumer drops, ever.
- Cursor-focused tool surface: shared read cursor, `seek` tool, offsets in
  every result. Data loss (ring wrap) is always *observable*, never silent.
- Unify the read/subscribe semantic asymmetries that existed only because
  bytes not consumed immediately used to be lost.

**Non-goals**

- No persistent per-connection framing decoder — framing stays per-call and
  is applied to the drained window (deferred; see FEATURES.md).
- No per-client cursors for multi-client HTTP — one shared read cursor
  (deferred; see FEATURES.md).
- No live ring resizing. Buffer size is fixed at `open`; reopen to resize.
- No change to TX side (`tx_session`, write framing) beyond doc updates.
- No change to the framing/parser/protocol 4-layer precedence model.

---

## Core model

The RX side of a connection is a **byte stream with absolute offsets**,
starting at 0 when the connection opens and increasing monotonically for
every byte received — across disconnect/reconnect cycles, until `close`.
The ring buffer is a sliding window over that stream:

```
                     ring capacity (fixed at open)
              |<------------------------------------>|
  stream:  ...[lost to wrap]...########################  → future bytes
                               ^                      ^
                          start_offset            end_offset
                               (oldest retained)  (live edge = total rx)
```

- `end_offset` = total bytes received since open (also a statistics value).
- `start_offset` = `max(0, end_offset - retained)`; bytes below it are gone.
- A **cursor** is an absolute offset owned by a reader. Reading from a
  cursor that has fallen below `start_offset` yields the retained bytes plus
  an explicit `bytes_lost = start_offset - cursor` count. The reader is
  never silently handed a gap.

Cursors in the system:

- **One shared read cursor** per connection, owned by `RxSession`, used by
  `read` (consuming) and moved by `seek` / `flush`.
- **One private cursor per subscription**, created at `subscribe` time.

### RxRing (`src/rx_ring.rs`, new)

```rust
pub struct RxRing {
    // ring storage, capacity fixed at construction
    // start_offset / end_offset as u64 absolute stream offsets
    // tokio::sync::Notify for "new data appended"
}

impl RxRing {
    fn new(capacity: usize) -> Self;
    fn append(&self, bytes: &[u8]);              // pump only; wraps, notifies
    fn read_from(&self, cursor: u64, max: usize) -> RingSlice;
    fn clear(&self);                             // start = end (flush)
    fn start_offset(&self) -> u64;
    fn end_offset(&self) -> u64;
    async fn wait_for_data(&self, after: u64);   // Notify-based
}

pub struct RingSlice {
    pub bytes: Vec<u8>,
    pub from_offset: u64,   // clamped-to-start_offset actual read position
    pub next_offset: u64,   // cursor value after this slice
    pub bytes_lost: u64,    // requested_cursor < start_offset gap, else 0
}
```

Interior mutability + `Notify` so the pump appends without async locks on
the hot path. `read_from` never blocks and never fails; emptiness and gaps
are data, not errors.

### RxSession rework (`src/rx_session.rs`)

- `RxSession` owns the `RxRing` and the shared read cursor.
- **Pump runs from `open` to `close`.** `ConnectionManager::open` (and
  `open_profile`) creates the session and starts the pump;
  `close` / connection teardown shuts it down and joins it. The pump's only
  job is `connection.read()` → `ring.append()` → event-log accounting.
- **Delete** `ConsumerRegistry`, `RxConsumer`, `RxEvent` fanout,
  `register_blocking`, `register_streaming`, `prune_consumers`, and the
  prune/join coordination dance in the tools. The entire "pump steals bytes
  from later tools" hazard class (rx_session.rs PLAN-1a comments) goes away:
  there is exactly one port reader, always, and everyone else reads the
  ring.
- **Disconnect/reconnect:** on a fatal disconnect the pump *pauses* (waits
  on connection state) instead of exiting; on reconnect it resumes appending
  to the *same* ring at the *same* offsets. The ring's lifetime equals the
  connection object's lifetime, bounded by the existing `ReconnectPolicy`:
  if reconnection is exhausted/disabled and the connection closes, the ring
  dies with it. Readers blocked waiting for data during a disconnect keep
  their existing behavior (timeouts pause while disconnected).

### Budget integration

The ring is the budgeted allocation: charge `rx_buffer_size` against the
`BufferBudget` pool **once at open**, released on close (RAII on the
session). Per-call `max_buffered_bytes` reservations for read/subscribe
accumulation buffers shrink to just the call's output accumulation (bounded
by `max_bytes`), or go away entirely where the ring itself bounds the data.
`open` fails cleanly if the pool cannot cover the requested ring — same
error taxonomy as today's reservation failures.

---

## Tool surface

### `open` / `open_profile` / profiles

- New optional `rx_buffer_size` (bytes; default **256 KiB**, ~23 s of
  continuous 115200-baud traffic). The ring is sized for the gap between an
  event and the agent getting around to reading — agent think-time plus tool
  round trips easily reach 10–30 s, which 64 KiB (~5.7 s) would not survive
  against a chatty device. Negligible vs the 1 GiB default budget pool. Also
  a profile field.
- Open-time only. Not reconfigurable; reopen to resize (deliberate — this
  is a rare operation and live resize with outstanding cursors is fiddly).
- Validated against the budget pool and a sane per-connection ceiling
  (wire through `src/limits.rs` like the existing CLI limits).

### `read` — consuming, cursor-based, history-aware matching

Semantics: *return bytes from the shared cursor to the live edge; advance
the cursor past what was returned; optionally wait for more.*

- **Immediate path (the `cat` case):** unread bytes exist and no `match` is
  given → return them now (up to `max_bytes`), `stop_reason: "drained"`.
  Leftover beyond `max_bytes` stays in the ring, cursor advances only past
  what was returned — pagination for free.
- **Waiting path:** buffer empty, or `match` given and not yet satisfied →
  wait for new bytes exactly like today (`timeout_ms` overall,
  `no_new_rx_timeout_ms` silence stop). The matcher scans **buffered bytes
  first, then live bytes** — "wait for `Advertising`" returns instantly if
  the banner already arrived. This folds the wait-for use case into `read`.
- **`peek: true`:** identical output, cursor **never** advanced — not even
  on a match (peek is a pure query; a repeated peek finds the same match
  again). A peek may still *wait* via `timeout_ms` for a match to appear;
  it just doesn't consume when it does. The result's `next_offset` makes
  the follow-up consume a single `seek`/`read` away.
- **On match:** the call stops at the match and the cursor advances to the
  end of the matched region (raw) or matched frame (framed). Bytes after
  the match **stay in the ring** for the next read. (This resolves the old
  read-keeps-later-frames asymmetry — read only kept them because they'd
  otherwise be lost. Nothing is lost anymore.)
- **Result additions:** `from_offset`, `next_offset`, `bytes_lost`
  (ring-wrap gap since the cursor), `buffered_remaining` (unread bytes left
  in the ring), plus the existing framing/parser/encoding fields.
  `bytes_returned: 0` with `stop_reason: "timeout"` becomes rare and
  self-explanatory next to `buffered_remaining: 0`.
- Framing/parser resolution (4-layer precedence) unchanged; the decoder is
  constructed per call and fed the drained window. A torn first frame after
  wrap/partial-drain is possible — `bytes_lost > 0` tells the agent why.
  See "Deferred" for the persistent-decoder future goal.

### `seek` — new tool, moves the shared cursor

- Targets: `{ to: "live_edge" }` (skip everything — the non-destructive
  replacement for "flush before read"), `{ to: "buffer_start" }` (re-read
  everything retained), `{ to: { offset: N } }` (absolute, from a previous
  result), `{ to: { delta: -N } }` (relative; negative = re-read).
- Returns the clamped new cursor plus `start_offset` / `end_offset` /
  `buffered_remaining`, so it doubles as a cheap "what's in the buffer"
  query without moving data.
- Clamps out-of-range targets into `[start_offset, end_offset]` and reports
  the clamp rather than erroring.

### `subscribe` / `unsubscribe` — full rewrite as cursor follower

- Each subscription owns a private cursor and a task that loops:
  `ring.read_from(cursor)` → apply per-subscription framing/matching →
  emit notification → `wait_for_data`. No mpsc fanout, no registry.
- **New `from` parameter:** `"now"` (default; live edge — today's
  behavior), `"cursor"` (start at the shared read cursor — replay what
  `read` hasn't consumed), `"buffer_start"` (replay everything retained,
  then go live), or `{ offset: N }`. Replayed history flows through the
  same framing/match pipeline as live data. The flash-then-subscribe
  ordering mistake stops mattering: `subscribe(from: "buffer_start")` after
  the event still captures it.
- **Gap reporting replaces silent drop:** a subscriber that lags beyond the
  ring capacity gets a notification field `bytes_lost: N` on its next
  emission and *continues from `start_offset`*. The subscription never
  silently dies. (The ring also makes lag much rarer: the old 256-event
  channel is replaced by up to `rx_buffer_size` of slack.)
- Subscriptions do **not** move the shared read cursor; `read` and
  `subscribe` coexist without stealing from each other.
- Stop semantics (`timeout_ms`, `match`, `max_bytes` budget, unsubscribe,
  close, framing error) keep the `RxStopController` vocabulary; final
  notification gains the offset fields.

### `flush`

- `target: "input"` = `ring.clear()` + clamp all cursors (shared + all
  subscriptions) to the live edge, plus the kernel `tcflush` (mostly empty
  anyway with an always-on pump). `output` / `both` unchanged.
- Description gets the explicit warning the review asked for: *"discards
  all unread buffered RX data; to skip past buffered data without
  destroying it, use `seek` to live_edge instead."* Most old flush-before-
  read habits should migrate to `seek`.

### `get_status`

Additive ring fields on `ConnectionStatus`: `rx_buffer_size`,
`rx_start_offset`, `rx_end_offset` (= total RX bytes), `rx_cursor`,
`rx_buffered_unread`, `rx_bytes_lost_total` (lifetime wrap loss), active
subscription count. Cheap, and gives agents a one-call answer to "did I
miss anything?"

### Resource exposure (Phase 3)

`serial://connections/{id}/rx` blob resource: zero-side-effect peek of the
retained window (optionally `?offset=`), reusing the existing blob-resource
and resource-subscription plumbing. Notifications on ring append can reuse
`notify_resource_changed` (rate-limited). Nice-to-have; ships last.

---

## Unified read/subscribe semantics

> **IMPLEMENTED in Phase 2** (commit: rx-ring-redesign branch). The
> AGENTS.md "Invariants easy to break" section has been updated to reflect
> the unified semantics. The old `rx-read-vs-subscribe-semantics` guidance
> ("the two loops legitimately differ") is SUPERSEDED by the table below.

The historical asymmetries (see AGENTS.md "Invariants easy to break" and
the `rx-read-vs-subscribe-semantics` design note) existed because
unconsumed bytes used to be unrecoverable. With the ring they are redefined
deliberately:

| Concern | Old `read` | Old `subscribe` | New (both) |
| --- | --- | --- | --- |
| Data before the call | lost | lost | in the ring; readable |
| Raw match scan extent | `chunk[..take]` up to `max_bytes` | full chunks, whole lifetime | each tool scans exactly the bytes it emits; sliding-window matcher across chunk boundaries |
| Frames after a match | kept (would be lost) | dropped | stop at match; remainder stays in ring |
| `bytes_returned` in match meta | accumulated len | cumulative emitted | one definition: bytes emitted by this call/subscription up to and including the match |
| Slow consumer | n/a (blocking) | silently dropped | falls behind → `bytes_lost` gap, continues |
| Framing construction error | tool error | degrade to raw + `warn!` | **tool error for both** — a bad config is an agent mistake and must be visible; degrading silently changed data semantics |
| Runtime decode error (SLIP/COBS) | partial result: `is_error: false`, `stop_reason: framing_error`, `error` text, frames-before-error kept, hex-fallback `data`/`encoding` when the requested encoding can't represent the raw bytes (0.7.3 contract) | same, as final notification | unchanged contract, plus offset fields — and now recoverable: see "cursor position on framing error" below |
| Checksum mismatch, `validate: true` (NMEA `*XX`, Modbus LRC) | per-frame drop-and-count: increment `frames_dropped`, `warn!`, decoder continues (0.7.3 contract) | same | unchanged — NOT stream-fatal; `frames_dropped` stays per-call decoder state, so a history replay re-decodes and recounts |

The construction-error unification (subscribe hard-errors instead of
degrading) is **decided**: the old rationale for degrading — failing the
call meant missing unrecoverable bytes — is void now that the ring keeps
buffering while the agent fixes its config and re-subscribes with
`from: "cursor"`. Silently delivering raw data when framed data was
requested misleads the client; the `warn!` it gets today is invisible over
MCP.

### Cursor position on framing error (baseline update from 0.7.3)

The pre-ring architecture made framing-error recovery trivial by accident:
when a `read` stopped on a decode error, the bytes still in the pump channel
were discarded as the call ended, so the *next* read started from clean live
data. The ring removes that accidental cleanup — nothing is discarded — so
the contract must be explicit or a plain retry re-feeds the same corrupt
bytes to a fresh decoder and errors forever:

- On `stop_reason: framing_error`, the shared cursor advances past **all
  bytes the call consumed from the ring, including the malformed sequence
  that triggered the error** (the decoder consumed them; they are part of
  the partial result's raw `data`). A plain retry therefore always makes
  progress.
- The result's `from_offset`/`next_offset` bracket the consumed window, so
  an agent that wants to forensically re-read the corrupt region does it
  with `seek { offset: from_offset }` + `read` raw — this is the "read raw
  or seek past the corruption" recovery path, now with exact coordinates.
- Subscriptions behave identically with their private cursor: the final
  notification's offsets tell the agent where decoding died, and a
  re-subscribe with `from: "cursor"` (or an explicit offset) resumes after
  the corruption.

Once this lands, update the `rx-read-vs-subscribe-semantics` design note —
its "the two loops legitimately differ" guidance is superseded by this
table.

---

## Behavior changes to document prominently

- **Hardware flow control loses its throttling side effect.** Today an idle
  serial-mcp doesn't drain the kernel RX buffer, so with RTS/CTS enabled the
  kernel deasserts RTS and *the device pauses* — nothing is lost, the data
  waits on the device. The always-on pump drains continuously, so RTS never
  drops, the device streams freely, and sustained unread traffic eventually
  wraps the ring (oldest bytes lost, but *observably*, via `bytes_lost`).
  This matches every terminal program's behavior and is the right default,
  but a setup that relied on flow control to pause a device until the host
  reads will behave differently. README + tool descriptions get a note.
- **`flush(input)` now clears our ring, not just the kernel buffer** — it
  is strictly more destructive than before. Description rewritten; `seek`
  offered as the non-destructive alternative.
- **Tool descriptions are half the fix.** The review's top complaint was
  read-vs-subscribe ambiguity. New one-liners: `read` = "read buffered +
  incoming bytes from the connection's cursor (like `cat`); consuming by
  default", `subscribe` = "follow the RX stream in the background via
  notifications (like `tail -f`), optionally replaying buffered history."

## Deferred (tracked in FEATURES.md)

- **Persistent per-connection framing decoder** — carry decoder state across
  read calls so drained windows never tear frames at call boundaries.
  Requires binding framing config to the connection rather than the call;
  interacts with the 4-layer precedence model. Deferred.
- **Per-client RX cursors** — multiple HTTP clients currently share the one
  read cursor, so concurrent consuming reads interleave. Offsets in every
  result make this diagnosable; cursor groups (one named cursor per client)
  are the future fix if shared multi-agent access becomes real. Deferred.

## Phasing

Each phase lands green (fmt, clippy `-D warnings`, full test suite) and is
independently shippable.

### Phase 1 — ring core + always-on pump + `read`/`seek`/`flush`

1. `src/rx_ring.rs` with exhaustive unit tests (wrap, gap accounting,
   clamp, notify wakeups, zero-capacity rejection) + proptest (append/read
   sequences preserve stream bytes and offset arithmetic).
2. `RxSession` rework: ring ownership, pump from open to close,
   pause/resume across disconnect, budget charge at open. Keep the old
   consumer registry temporarily so `subscribe` still works unchanged
   (pump appends to ring *and* fans out until Phase 2).
3. `read` rewrite onto the cursor (consuming, peek, history-aware match,
   offset fields), `seek` tool, `flush` ring semantics, `get_status`
   fields, `open`/profile `rx_buffer_size`.
4. Schema guards: every new `u64`/`usize` offset field needs
   `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]` and an
   entry in the `serial::schema` `check_schema!` list. New tool + changed
   descriptions hit the README/`server.json` tool-count drift guards —
   update in the same commit.
5. Tests: PTY suite (`serial_pty`) for capture-before-read boot-log
   scenario (write → *then* read → bytes present), wrap + `bytes_lost`,
   peek/seek round-trips; native_sim lifecycle for reconnect ring
   persistence; stdio/http integration for the new tool surface.

### Phase 2 — `subscribe` rewrite + semantics unification

1. Cursor-follower subscription task; `from` replay parameter; gap
   notifications. Delete `ConsumerRegistry`/`RxEvent` fanout and all
   prune/join coordination. **Keep `src/tools/rx_consume.rs`**
   (`consume_frames` + `RxFrameSink`): it is per-window frame dispatch, not
   fanout, and it owns the 0.7.3 frames-before-error contract (frames
   decoded before a stream-fatal error are dispatched to the sink BEFORE
   `FrameOutcome::DecodeError` is returned) plus the `frames_dropped`
   accounting. Both loops keep routing framed windows through it.
2. Apply the unified-semantics table: match-stop behavior, `bytes_returned`
   definition, construction-error hard-failure for subscribe.
3. Tests: replay-from-history, lag → gap → continue (the old silent-drop
   test inverts), read+subscribe coexistence (subscribe doesn't consume),
   framing-error recovery via seek. The native_sim SLIP framing-error e2es
   (`native_read_slip_malformed_escape_returns_partial_result`,
   `native_read_slip_recovers_after_error_on_next_call`) already assert the
   0.7.3 partial-result contract (`stop_reason: framing_error`, `error`
   field, hex-fallback data) — extend them with the offset fields and
   rewrite the recovery test's premise: it currently passes because the old
   pump channel discards the corrupt bytes between calls; under the ring it
   must instead prove the "cursor advances past the malformed sequence"
   contract (retry progresses without a seek).
4. Update AGENTS.md invariants section + the superseded design note.

### Phase 3 — polish + release

1. `serial://connections/{id}/rx` blob resource + change notifications.
2. README / CHANGELOG / tool descriptions / FEATURES.md sweep (the
   `transact` tool rationale in FEATURES changes: the write-then-read race
   is largely solved by the ring; `transact`'s remaining value is halving
   round trips — reword, keep).
3. Version bump to **0.8.0** (auto-tags a release on merge to main — bump
   only when the tree is release-ready, CHANGELOG rolled). Since 0.7.4 the
   bump touches `Cargo.toml` + the single top-level `server.json` version
   field (server.json is a packages-less registry template; the publish
   workflow generates package URLs + hashes, and `doc_drift` guards both
   the version match and the packages absence).

## Resolved decisions (2026-07-07, with Thomas)

1. **Consuming-by-default `read`** with a `peek` option; cursor-focused
   tool design including a cursor-move tool.
2. **Full `subscribe` rewrite** as cursor follower (not a fanout retrofit).
3. **Wait-for semantics folded into `read`** — match checks history first.
4. **Open-time-only ring sizing** — no live resize; reopen if too small.
5. **Ring persists across disconnect/reconnect**, bounded by the existing
   `ReconnectPolicy`.
6. **Breaking changes allowed** — pre-1.0, no compat shims; ships as 0.8.0.
7. **Construction errors hard-fail both tools** — subscribe stops degrading
   to raw (see the unified-semantics table for rationale).
8. **Default ring size 256 KiB** — sized for agent think-time between event
   and read, not just for the payload; overridable at `open`.
9. **Tool name `seek`** — completes the POSIX file vocabulary
   (open/close/read/write/seek/flush); semantics match `lseek`.
10. **`peek` never moves the cursor, even on match** — pure query; may
    still wait via `timeout_ms`.
11. **Flow-control pause-on-full is a future feature**, not part of this
    plan — tracked in [FEATURES.md](FEATURES.md) ("Flow-control-aware ring
    backpressure").
