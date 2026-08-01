# Code Cleanup Phase 2 Handoff

## Goal

Make `read_from_private_cursor` easier to understand by centralizing mutable
read accumulation and normal result finalization. Preserve every byte, cursor,
offset, framing, matching, timeout, and cancellation behavior.

Implement Phase 2 only. Commit completed work before returning.

## In scope

- `src/tools/read_loop.rs`
- Focused tests in the same module only when needed to lock a finalization
  invariant not already covered
- Existing public-boundary tests; modify them only if a true regression gap is
  found
- This handoff document

## Out of scope

- No changes to `ReadOutcome`, MCP `ReadResult`, schemas, tool descriptions,
  stop reasons, error text, timeout behavior, or framing/parser behavior.
- No changes to `read`, `transact`, `capture_boot`, `subscribe`,
  `RxStopController`, `RxRing`, `Matcher`, or `consume_frames` APIs.
- Do not merge private and shared cursor functions.
- Do not split the wait loop into asynchronous helper functions.
- Do not normalize saturating and wrapping cursor arithmetic unless an existing
  behavior test proves equivalence; preserve each current formula.
- Do not perform Phase 3 cleanup or unrelated comments/tests.

## Grounding and current invariants

`src/tools/read_loop.rs::read_from_private_cursor` currently spans roughly 570
lines and owns these accumulated values independently:

- `consumed_offset`
- `returned_bytes`
- `cursor_state`
- `collected_frames`
- `frames_seen`
- `frames_dropped`
- `frame_error_msg`

Its local `make_read_outcome` closure takes a long argument list and includes an
unused `_ctrl` parameter. Most return sites repeat the same payload/frame/error
movement and tuple construction.

Preserve these non-obvious rules:

1. `initial_cursor` is caller-owned. Private read never writes
   `session.read_cursor()`.
2. `read_bytes_from_ring` remains the sole wrapper that commits returned final
   cursor to shared session state.
3. `consumed_offset` tracks raw ring bytes. Returned bytes may differ after
   literal match-context shaping.
4. Generic outcome offsets are based on current ring start/end and original
   cursor, exactly as current `make_read_outcome` computes them.
5. Historical literal-context match is special: it currently uses
   `initial_slice.from_offset`, `initial_slice.bytes_lost`, and its snapshot
   values directly. Do not route this branch through a live-ring finalizer that
   could change offsets during concurrent append/wrap.
6. Initial framed chunks add to `consumed_offset`; history matcher bytes may
   already be present in `returned_bytes` before framing runs. Preserve current
   accumulation exactly.
7. Frames decoded before runtime framing error remain in result; malformed raw
   bytes remain in top-level data and cursor advances past consumed bytes.
8. Framed read records first matching frame but still includes frames decoded
   later in same chunk.
9. Cancellation returns a structured result, never an ad-hoc error.
10. `advance_private_cursor` uses saturating addition. Other existing branches
    use wrapping addition plus live-edge clamp. Preserve call-site choice.

## Exact implementation shape

### 1. Add private accumulator

Create a private struct near `advance_private_cursor`:

```rust
struct ReadAccumulator {
    consumed_offset: u64,
    returned_bytes: Vec<u8>,
    frames: Vec<crate::framing::Frame>,
    frames_seen: usize,
    frames_dropped: usize,
    frame_error: Option<String>,
}
```

Provide only concise methods that reduce repetition:

- `new(max_bytes)` initializes returned byte capacity and empty frame state.
- `into_outcome(...)` consumes accumulator and builds generic `ReadOutcome`
  plus supplied final cursor.

Do not add getters/setters for every field. Direct private field access inside
the same module is clearer for framing and matcher code.

### 2. Generic finalizer

`ReadAccumulator::into_outcome` (or an equivalent pure free function) must take
only stop-specific/current-call inputs not owned by accumulator:

- Ring reference
- Original private cursor
- `max_bytes`
- `elapsed_ms`
- `RxStopMetadata`
- `matched`
- `match_index`
- `match_frame_index`
- Optional returned-payload override
- Final private cursor

Accumulator supplies returned bytes (unless overridden), consumed bytes,
frames, frame-drop count, and frame error.

Copy the current generic closure's offset formulas exactly:

```text
start_off = ring.start_offset()
end_off = ring.end_offset()
clamped_from = cursor.max(start_off).min(end_off)
bytes_lost = start_off.saturating_sub(cursor)
used = consumed_offset.min(max_bytes as u64)
next_off = clamped_from + used
from_offset/next_offset = None only when returned payload is empty and
                          consumed_offset == 0
buffered_remaining = end_off.saturating_sub(next_off)
```

`returned_payload_override` changes only `ReadOutcome.bytes`; offsets continue
to use raw `consumed_offset`. This supports framed match data and live literal
context shaping without cloning/rebuilding whole outcomes.

Return `(ReadOutcome, final_private_cursor)` from this method so each exit is a
single finalization call.

### 3. Preserve historical-context snapshot branch

Keep a small dedicated pure function or direct constructor for the historical
literal-context match branch. It must continue using:

- `initial_slice.from_offset`
- `initial_slice.bytes_lost`
- `initial_slice.from_offset + consumed`
- Current ring start/end only for reported bounds

Do not force this branch through generic finalization.

### 4. Remove redundant mutable cursor state

`cursor_state` is assigned immediately before every return and is never used as
meaningful intermediate state. Remove it. Compute each existing final cursor at
its current branch and pass directly into finalization.

Do not rewrite formulas while moving them:

- Keep `advance_private_cursor(...)` where currently used.
- Keep `slice.from_offset.wrapping_add(take).min(ring.end_offset())` where
  currently used.
- Keep original-cursor wrapping formulas where currently used.

### 5. Route accumulated fields through struct

Replace independent mutable values with accumulator fields. `ReadFrameSink`
continues borrowing `&mut accumulator.frames`; `consume_frames` continues
receiving `&mut accumulator.frames_seen` and
`&mut accumulator.frames_dropped`. `frame_outcome_to_stop` continues receiving
`&mut accumulator.frame_error`.

Consume accumulator only on return paths. Do not clone frames to satisfy the
refactor.

### 6. Comments

- Replace `Phase 1.3` and `pre-Phase-5` chronology with present behavior.
- Keep concise comments for private/shared cursor ownership, historical
  snapshot offsets, match payload versus consumed bytes, and frames-before-
  error behavior.
- Remove comments that merely narrate assignments now explained by accumulator
  field names.

## Tests and observable acceptance

Existing module tests must remain green:

- `cancelled_token_read_returns_structured_cancelled_outcome`
- `private_cursor_read_leaves_shared_cursor_unchanged`

Existing PTY tests cover:

- Historical and live match-context shaping
- Buffered immediate read
- Ring wrap and `bytes_lost`
- Buffer-start/offset/now cursors
- Framed replay
- Transact cursor use

Existing HTTP controlled-backend tests cover runtime framing errors,
capture-boot cancellation, and capture ring wrap.

Add a unit test only if accumulator finalization needs direct coverage for this
critical distinction:

- Payload override length differs from `consumed_offset`, but `next_offset`
  still advances by consumed bytes.

That pure finalizer test is encouraged because it directly proves a public
offset invariant. Do not assert private struct shape or helper call count.

## Verification

Run:

```bash
cargo fmt --all -- --check
cargo test --lib tools::read_loop --locked
cargo test --test serial_pty --locked -- --test-threads=1
cargo test --test http_integration read_framing_error_keeps_valid_frame_utf8_and_raw_hex --locked -- --test-threads=1
cargo test --test http_integration capture_boot_cancellation_releases_lines_request_scoped --locked -- --test-threads=1
cargo test --test http_integration capture_boot_runtime_framing_error_returns_partial_result_and_releases_lines --locked -- --test-threads=1
cargo test --test http_integration capture_boot_ring_wrap_reports_bytes_lost_and_preserves_mark --locked -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

## Acceptance criteria

- Mutable read accumulation has one named owner.
- Generic result math exists once.
- Historical context branch retains snapshot-specific offsets.
- Return sites are shorter and no longer pass unused controller state.
- Shared cursor commit remains isolated to `read_bytes_from_ring`.
- No public or behavioral change.
- Requested tests pass with no warnings.
- Diff contains no unrelated cleanup.
- Working tree clean after commit.

## Commit and recap

Before returning:

1. Inspect status, diff, and recent log.
2. Stage only `src/tools/read_loop.rs` and this handoff document.
3. Commit with suggested message:

   `refactor: simplify private read state`

4. Do not push, merge, open PR, amend, force-push, or add attribution.
5. Return files, behavior preserved, tests/results, commit hash/message,
   deviations, blockers, and follow-up concerns.
