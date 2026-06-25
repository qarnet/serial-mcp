# Phase 2 Handoff — RX line `auto` adaptive bare-CR handling

Source plan: `docs/development/FRAME_PIPELINE_PLAN.md` (Phase 2).
Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Teach RX `line` framing with `ending: "auto"` to detect a trailing bare `\r`
that is not part of a CRLF sequence, and switch that decoder to CR-split mode
for the remainder of the call. This makes `auto` usable on devices that emit
bare-CR line endings instead of forcing agents to pre-pick `cr` or `crlf`.

## Decisions already made (do not re-litigate)

- **Promotion scope = per read/subscribe call.** The decoder starts each call
  in auto-LF mode (current default). Promotion to CR mode happens mid-stream
  and persists only for the remainder of THAT call. Next call starts fresh in
  auto-LF. No connection-level or profile-level state plumbing.
- **Confirmation timer = reuse `no_new_rx_timeout_ms`.** When a `\r` arrives
  and no `\n` follows before the next byte OR before the silence deadline
  elapses, confirm the `\r` as bare CR and promote. No new config field.
  When `no_new_rx_timeout_ms` is unset, fall back to immediate promotion on
  the next non-`\n` byte, and flush the pending `\r` as a frame at stop
  (no timer-based confirmation possible). Document this behavior clearly.
- **Sticky CR mode.** Once promoted, the decoder splits on bare `\r` for the
  rest of the call and stops treating `\n` as a line terminator. Next call
  resets to auto-LF.
- **Trailing bare-`\r` flush = flush as frame.** On stop, if the decoder
  holds a pending unconfirmed `\r` (or any partial frame), emit it via the
  existing `flush_partial` path. read and subscribe both already call
  `flush_partial` at stop; the pending-`\r` case rides that path.

## In scope

### A. Decoder state machine for `auto`

Update `RxFramingMode::Line { ending: LineEnding::Auto }` in
`src/framing.rs`:

- Extend `DecoderMode::Line` to carry a state enum for `auto`:
  - `AutoLf` — current behavior: split on `\n`, strip preceding `\r`.
  - `PendingCr` — saw a `\r` with no `\n` yet; waiting for confirmation.
  - `CrMode` — promoted; split on bare `\r` for the rest of the call.
- `lf`, `cr`, `crlf` endings are unaffected (single-state, no promotion).
- In `AutoLf`:
  - Split on `\n` as today, stripping a preceding `\r` (existing
    `match_line_auto` logic).
  - When a `\r` is encountered WITHOUT a following `\n` in the same chunk,
    do NOT immediately emit a frame. Transition to `PendingCr` keeping the
    bytes up to and including the `\r` in the buffer. Continue scanning the
    rest of the chunk for a `\n` — if found in the same chunk, treat as
    CRLF (existing auto behavior), emit the line, return to `AutoLf`.
- In `PendingCr`:
  - On next non-`\n` byte arriving: confirm bare CR, promote to `CrMode`.
    Emit the buffered bytes up to and including the `\r` as a frame (strip
    the `\r` unless `include_terminators`), then process the new byte in
    `CrMode`.
  - On next byte being `\n`: treat the held `\r\n` as CRLF, emit the line
    (existing auto behavior), return to `AutoLf`.
  - On stop before any further byte: `flush_partial` emits the pending
    bytes (including the `\r`) as a frame.
- In `CrMode`:
  - Split on bare `\r` (mirror `match_line_cr`).
  - Do NOT treat `\n` as a line terminator anymore.
  - Stay in `CrMode` until the call ends. No demotion.
- Confirmation via `no_new_rx_timeout_ms`:
  - When `PendingCr` is reached AND a silence deadline is available, the
    decoder does not need its own timer — the existing read/subscribe loop
    already stops on `no_new_rx_timeout`. When the loop stops, `flush_partial`
    emits the pending `\r` as a frame. So the "timer proves bare CR" path is
    really "loop stops with pending `\r`, flush emits it."
  - When `no_new_rx_timeout_ms` is UNSET and the stream is long-lived
    (subscribe, no timeout), a `\r` could stay pending indefinitely waiting
    for the next byte. This is acceptable and matches current partial-line
    behavior. The next byte resolves it (promote or CRLF). Document that
    `auto` mode promotion relies on either a following byte or a stop.
- Expose a way for the read/subscribe loop to drive confirmation without a
  new channel. Simplest: the decoder transitions purely on bytes pushed
  (next-byte confirms) and on `flush_partial` (stop flushes pending). No
  timer callback into the decoder. This keeps the decoder byte-driven, as
  today.
- Add a method to `FrameDecoder` (e.g. `pub fn pending_cr(&self) -> bool` or
  expose mode in a diagnostic accessor) only if a test needs it. Prefer
  behavior tests over introspection.

### B. read / subscribe loop integration

`src/tools/helpers.rs` `read_bytes_via_session` and
`src/tools/stream_ops.rs` `stream_rx_via_session` already call
`flush_partial` at stop. Verify both paths flush a pending `\r` correctly
under the new state machine. No new params needed.

Check the existing `finish!` / partial-flush helpers in helpers.rs
(~line 369) and stream_ops.rs (~line 619) cover the pending-CR case the same
way they cover a pending partial LF-line.

### C. Documentation

- Update `LineEnding::Auto` doc comment in `src/framing.rs` to describe the
  promotion behavior: starts as LF/CRLF, promotes to CR mode when a bare `\r`
  is confirmed (next non-`\n` byte or stop with `no_new_rx_timeout_ms`).
- Update `read` and `subscribe` tool descriptions in `src/server.rs` only if
  they enumerate line-ending behavior. Currently they just say "line" — likely
  no change needed. Verify and leave alone if no enumeration.
- No CHANGELOG update required for this phase (recap doc will cover it).

## Out of scope

- Per-connection or per-profile sticky CR promotion.
- A dedicated `cr_confirmation_ms` config field.
- Mixed-terminator per-`\r` decision mode (sticky-only, not per-`\r`).
- Any change to `lf`, `cr`, `crlf` endings.
- TX side changes (TX has no `auto`).
- New stop_reason values. The pending-`\r` flush rides existing stop reasons
  (`timeout`, `no_new_rx_timeout`, `connection_closed`, etc.).
- Parser relocation (still Phase 4).
- Profile defaults (still Phase 5).

## Relevant files and current behavior

- `src/framing.rs`:
  - `RxFramingMode::Line { ending: LineEnding }`, `DecoderMode::Line { ending }`.
  - `FrameDecoder::new` constructs `DecoderMode::Line { ending: *ending }`.
  - `push()` dispatches to `match_line_auto/lf/cr/crlf`.
  - `match_line_auto` (line ~628): eager `\n` split + preceding-`\r` strip.
  - `flush_partial` (line ~496 in current file): drains `self.buf` into one
    final `Frame` with `frame_type` from `frame_type_str()` and `parsed: None`.
  - `frame_count` increments per emitted frame (including flush).
- `src/tools/helpers.rs`:
  - `read_bytes_via_session(..., framing: Option<RxFramingConfig>)`.
  - Builds `Option<FrameDecoder>`; partial-flush helper at ~line 369.
  - `finish!` macro emits final `ReadOutcome`.
  - Loop at ~line 455+: handles `RxEvent::Data`, `Closed`, `Error`.
- `src/tools/stream_ops.rs`:
  - `stream_rx_via_session(...)`. Partial flush at line ~619
    (`if let Some(partial) = dec.flush_partial()`).
  - Stop reasons emitted via final notification ~line 664.
- `src/stop_controller.rs`:
  - `RxStopController` owns `timeout_ms` deadline + `no_new_rx_timeout_ms`
    silence deadline. `push_data` records chunk and checks stop conditions.
  - `notify_data_received` / `reset_silence` reset the silence deadline on
    each non-empty chunk. No changes needed here.
- `src/tools/rx_consume.rs`: shared `consume_frames` — check whether the
  `PendingCr` transition interacts with cross-chunk frame matching. The
  matcher operates on decoded frames; a pending `\r` not yet a frame just
  means no new frame emitted this chunk. Verify no change needed.
- `tests/framing.rs` unit tests in `src/framing.rs` `mod tests`: existing
  `line_decoder_*` tests use `LineEnding::Auto` and expect current eager
  behavior. New tests must NOT break these — `auto` only promotes on a bare
  `\r` with no following `\n`, which the existing CRLF/LF tests do not
  trigger.
- `tests/native_sim_validation.rs`: framing tests use `rx_framing: line`.
  Verify firmware emits LF/CRLF (it does). No promotion expected. No test
  changes needed unless adding a new bare-CR native test.

## Expected API / UX shape

No public API change. `rx_framing: { type: "line", ending: "auto" }` behaves
the same for LF and CRLF streams. The only observable change: a device that
sends bare `\r` line endings now produces frames (after a one-byte
confirmation or at stop) instead of buffering everything until flush.

Example stream with promotion (3-byte confirmation window, subscribe with
`no_new_rx_timeout_ms`):
```text
in:  "line1\rli"
     → "\r" held pending; "li" arrives (non-\n) → confirm bare CR
     → emit frame "line1", promote to CrMode, buffer "li"
in:  "ne2\r"
     → CrMode splits on \r → emit frame "line2", buffer ""
stop (silence) → flush_partial emits nothing (buffer empty)
```

Example stream, LF only (no promotion):
```text
in:  "a\nb\n" → emit "a", "b" (unchanged from today)
```

## Test plan

Add tests in `src/framing.rs` `mod tests`:

1. **auto does not promote on CRLF.** Push `"a\r\nb\r\n"` with `ending: auto`;
   assert frames `["a", "b"]`, decoder stays in AutoLf state (introspect via
   behavior: subsequent bare `\r` still triggers promotion).

2. **auto does not promote on LF.** Push `"a\nb\n"`; assert frames `["a",
   "b"]`, no promotion.

3. **auto promotes on next non-`\n` byte.** Push `"line1\r"` (no frame yet,
   pending), then `"x"` → assert frame `"line1"` emitted on the `x`, decoder
   now in CrMode. Push `"more\r"` → assert frame `"more"`.

4. **auto CRLF after pending CR cancels promotion.** Push `"a\r"` (pending),
   then `"\nb"` → assert frame `"a"` (CRLF recognized), decoder back to
   AutoLf; push `"c\n"` → frame `"c"`.

5. **auto flush_partial emits pending CR.** Push `"tail\r"` then
   `flush_partial()` → assert one frame with data `b"tail"` (or `b"tail\r"`
   if `include_terminators`). Do not crash.

6. **auto promotes and stays in CrMode.** After promotion (test 3 setup),
   push `"x\ny\r"` → assert frames `["x\ny"]` (CrMode ignores `\n`, splits on
   `\r`). Confirms stickiness.

7. **auto promotion with include_terminators=true.** Same as test 3 but
   `include_terminators: true`; assert frame data includes the `\r`.

8. **auto pending CR then flush keeps frame_index monotonic.** Push two LF
   lines then a pending CR then flush; assert frame indices are 0,1,2.

9. **read integration: auto promotes over real loop.** In
   `src/tools/helpers.rs` tests, drive `read_bytes_via_session` with an event
   stream that delivers `\r` then a non-`\n` byte; assert the resulting
   `ReadOutcome.frames` contains the promoted line. (Existing
   `char_framing_*` tests show the harness.)

10. **read integration: flush_partial on timeout emits pending CR.** Drive a
    read that gets `"tail\r"` then times out; assert `frames` last entry is
    `b"tail"`.

11. **subscribe integration: flush_partial emits pending CR at stop.** In
    `src/tools/stream_ops.rs` tests (or a new test), drive the subscribe loop
    with `\r` then connection closed; assert a final frame notification with
    `b""..b"\r"` content depending on `include_terminators`.

12. **regression: existing line_decoder_* tests still pass.** No modifications
    to existing tests. They must remain green.

13. **regression: lf/cr/crlf endings unaffected.** Add one parametrized-ish
    test confirming `lf`, `cr`, `crlf` endings never enter PendingCr state
    (behavior unchanged). Easiest: existing tests already cover this; just
    ensure no state leak.

14. **schema regression.** No new JsonSchema types with unsigned fields
    expected. If a new field is added to `LineEnding` variants (unlikely —
    state is internal), add to `check_schema!` list. Otherwise no schema
    test changes.

## Constraints and invariants (from repo docs)

- **No `unwrap`/`expect`/`println!`/`todo!()`/`unimplemented!()`** in
  production code.
- **Tool failures become MCP tool results with `is_error: true`**, not
  protocol-level `McpError`.
- **Every `uN`/`Option<uN>` field on a `JsonSchema`-deriving struct uses
  `uint_schema`/`option_uint_schema`.** No new public unsigned fields
  expected this phase.
- **read/subscribe raw-path asymmetry preserved.** read bounded and scans
  `chunk[..take]`; subscribe scans full chunks. Do not merge raw paths.
- **Framing semantics differ by design:** read keeps later frames from the
  same chunk after the matching frame; subscribe stops on the matching
  frame. Preserve.
- **Match metadata:** read uses `accumulated.len()` for `bytes_returned`;
  subscribe uses cumulative `total_returned`. Preserve.
- **subscribe degrades bad framing configs to raw with `warn!`;** read
  propagates errors. No new config validation needed this phase (auto is the
  default ending), but if you add any, keep this asymmetry.
- **`flush_partial` increments `frame_count` and emits a `Frame` with
  `parsed: None`.** Keep this contract.
- **`frame_type_str()` for all Line states returns `"line"`.** Do not
  differentiate auto/CrMode in `frame_type`; agents see a consistent
  `"line"` frame type.
- **Conventional commits:** `feat:`, `fix:`, `refactor:`, `test:`, `docs:`.
  No attribution footers.

## Verification commands

Run after implementation, in this order:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# focused
cargo test --lib framing
cargo test --lib serial::schema
cargo test --lib tools::helpers
cargo test --test proptest
cargo test --test serial_pty
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
cargo test --test config_schema_validation
cargo test --test http_integration
cargo test --test stdio_integration
```

If native_sim firmware is not built, run `fw-build-native` first (see
`AGENTS.md` Firmware section). The orchestrator xtask can also be used:

```bash
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
```

## Return instructions

When done, return a concise summary covering:

- Files changed and why.
- Final state machine shape (AutoLf / PendingCr / CrMode transitions) and
  where it lives in `src/framing.rs`.
- How the confirmation integrates with `no_new_rx_timeout_ms` (byte-driven
  + flush at stop, no decoder timer).
- New tests added and which existing tests were updated (if any).
- Gate command results (`fmt`, `build`, `test`, `clippy`). Note any
  failures with root cause.
- Any scope decision you had to make that is not covered above.
- Any case where `auto` promotion semantics surprised you.