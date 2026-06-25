# Phase 2 Recap — RX line `auto` adaptive bare-CR handling

**Date:** 2026-06-26
**Source plan:** `docs/development/PHASE2_LINE_AUTO_BARE_CR_HANDOFF.md`
**Status:** Complete

---

## Summary

Taught RX `line` framing with `ending: "auto"` to detect bare `\r` line
endings (not part of a CRLF sequence) mid-stream, and promote the decoder to
CR-split mode for the remainder of the call. This makes `auto` work on devices
that emit bare-CR line endings without forcing agents to pre-pick `cr` or
`crlf`.

---

## State machine

Added `LineState` enum in `src/framing.rs`, replacing the flat `LineEnding`
field in `DecoderMode::Line`:

| State | Entry condition | Behavior |
|-------|----------------|----------|
| `Lf` | `ending: "lf"` | Split on `\n`, never strip `\r` |
| `Cr` | `ending: "cr"` | Split on bare `\r` |
| `Crlf` | `ending: "crlf"` | Split on exact `\r\n` |
| `AutoLf` | `ending: "auto"` (start of call) | Split on `\n`, strip preceding `\r` (CRLF-aware). Detect bare `\r` → transition to `PendingCr` or directly to `CrMode` |
| `PendingCr` | Saw `\r` at end of buffer, no `\n` followed | On next byte: `\n` → treat as CRLF, back to `AutoLf`. Non-`\n` → confirm bare CR, emit frame before `\r`, promote to `CrMode` |
| `CrMode` | Promoted from `PendingCr` or directly from `AutoLf` | Split on bare `\r` only, ignore `\n`. Stays in `CrMode` for the remainder of the call |

### Transition rules

- **Same-chunk bare CR:** If `\r` is found in `AutoLf` and a non-`\n` byte
  follows in the same chunk, promotion to `CrMode` is immediate — the frame
  before `\r` is emitted, and the remaining bytes are scanned in `CrMode`.
- **Cross-chunk bare CR:** If `\r` is the last byte in the buffer
  (`PendingCr`), the next chunk resolves it. If the next byte is `\n` →
  CRLF, emit line, back to `AutoLf`. If any other byte → bare CR confirmed,
  emit line before `\r`, promote to `CrMode`.
- **Flush at stop:** `flush_partial()` already drains the buffer as a frame.
  A pending `\r` in `PendingCr` state is flushed along with the rest of the
  buffer. The existing `finish!` / partial-flush paths in both
  `read_bytes_via_session` and `stream_rx_via_session` handle this without
  changes.
- **Confirmation via `no_new_rx_timeout_ms`:** No new timer. The existing
  silence timeout stops the read/subscribe loop; `flush_partial` then emits
  the pending `\r` as a frame. Untimed streams (subscribe, no timeout,
  no silence) retain the `\r` in the buffer until the next byte arrives —
  same as any partial line.
- **Stickiness:** Once in `CrMode`, `\n` is no longer a line terminator.
  No demotion back to `AutoLf`.

---

## Files changed

### `src/framing.rs`

- Replaced `DecoderMode::Line { ending: LineEnding }` with
  `DecoderMode::Line(LineState)`. Added `LineState` enum with six variants:
  `Lf`, `Cr`, `Crlf`, `AutoLf`, `PendingCr`, `CrMode`.
- Rewrote `match_line_auto` → `match_auto_lf` (stateful: detects bare `\r`,
  transitions to `PendingCr` or directly to `CrMode`).
- Added `match_pending_cr` (resolves pending `\r`: `\n` → CRLF → `AutoLf`;
  non-`\n` → bare CR → `CrMode`).
- Updated `FrameDecoder::new()` to map `LineEnding` → `LineState`.
- Updated `push()` line arm to dispatch on `LineState`.
- Updated `frame_type_str()` for new `DecoderMode::Line(..)` shape (still
  returns `"line"` for all line states).
- Updated `LineEnding::Auto` doc comment to describe promotion behavior.
- Added 9 unit tests for auto promotion, plus 2 read-loop integration tests
  in `src/tools/helpers.rs`.

### `src/tools/helpers.rs`

- Added `char_framing_auto_promotes_on_bare_cr`: drives `read_bytes_via_session`
  with `\r`-then-non-`\n` event stream; asserts promotion emits the pending
  line and subsequent bytes in CrMode.
- Added `char_framing_auto_flush_partial_on_timeout_emits_pending_cr`: drives
  a read that gets `"tail\r"` then times out; asserts `flush_partial` emits
  the pending `\r` as a frame.
- No changes to production code — existing `flush_partial` at stop already
  covers the pending-CR case.

---

## Confirmation integration

The decoder is purely byte-driven. No timer callback, no new channel. Two paths:

1. **Next byte confirms.** When a `\r` is pending, the very next byte pushed
   into `FrameDecoder` resolves it: `\n` → CRLF, anything else → bare CR.
   This works the same in both read and subscribe loops.

2. **Stop flushes pending.** Both `read_bytes_via_session` (via `finish!`
   macro + `finalize_frames`) and `stream_rx_via_session` (explicit
   `flush_partial` call before stop notification) already flush the decoder
   at every exit point. A pending `\r` in `PendingCr` state gets flushed as
   a partial frame. The existing `no_new_rx_timeout_ms` stop path → flush →
   pending `\r` becomes a frame without any new logic.

---

## Tests added

- **9 unit tests** in `src/framing.rs`:
  `auto_does_not_promote_on_crlf`, `auto_does_not_promote_on_lf`,
  `auto_promotes_on_next_non_lf_byte`, `auto_crlf_after_pending_cr_cancels_promotion`,
  `auto_flush_partial_emits_pending_cr`, `auto_flush_partial_emits_pending_cr_include_terminators`,
  `auto_promotes_and_stays_in_cr_mode`, `auto_promotion_include_terminators`,
  `auto_pending_cr_then_flush_keeps_frame_index_monotonic`.
- **2 read-loop integration tests** in `src/tools/helpers.rs`:
  `char_framing_auto_promotes_on_bare_cr`,
  `char_framing_auto_flush_partial_on_timeout_emits_pending_cr`.
- **All 87 pre-existing framing tests continue to pass** — no regression in
  LF, CR, CRLF, or original auto behavior on LF/CRLF streams.

---

## Gate results

| Command | Result |
|---------|--------|
| `cargo fmt --all -- --check` | PASS |
| `cargo build --all-targets --locked` | PASS (no warnings) |
| `cargo test --locked` | PASS (350 lib tests, all integration suites) |
| `cargo clippy --all-targets --locked -- -D warnings` | PASS |

---

## Scope decisions

- **Confirmation byte retained.** After `PendingCr` resolves bare CR, the
  confirmation byte (the non-`\n` byte that proved the `\r` was bare) stays
  in the buffer and becomes the start of the next line. This matches the
  handoff example where `"line1\rli"` → `"line1"` emitted, `"li"` buffered
  for the next line. The handoff test plan items 3/6/11 had expectation
  errors (expected confirmation byte to vanish); corrected in implementation.
- **No new schema types.** `LineState` is internal (no `JsonSchema` derive).
  No unsigned fields added.
- **`frame_type_str()` returns `"line"` for all line states** including
  `CrMode` — agents see a consistent `"line"` frame type regardless of
  internal promotion state.
- **Subscribe tests deferred.** The read-loop integration tests
  (`char_framing_auto_*`) cover the same `flush_partial` path that subscribe
  uses. Subscribe's `flush_partial` call at stop (stream_ops.rs ~line 619)
  is structurally identical — no subscribe-specific test added, as the
  behavior is exercised through the shared decoder.

---

## Out of scope

- Per-connection or per-profile sticky CR promotion.
- Dedicated `cr_confirmation_ms` config field.
- Mixed-terminator per-`\r` decision mode.
- TX `auto` mode (TX has no `auto`).
- New stop_reason values.
