# Handoff — RX Ring Phase 1.5 fix: update native_sim + protocol-emulator + lifecycle tests for ring semantics

> Branch: `rx-ring-redesign` (HEAD: `d6c9962`, Phase 1.3 landed). Do not
> switch branches.
> Source: follow-up to Phase 1.3. The read rewrite onto the ring changed
> read semantics from future-only (fanout) to cat-like (buffered bytes
> returned immediately). 17 native_sim_validation + 1 lifecycle + 2
> protocol-emulator tests assert the OLD future-only contract and fail.
> This handoff updates those tests to the new ring semantics. NO
> production code change — the production code is correct (read returns
> buffered bytes immediately, which is the plan's headline behavior).

## Goal

Update all tests that assert the old future-only read contract to the new
ring-based cat semantics. The failing tests fall into 3 patterns; each
has a specific fix. After this, the full gate (including native_sim 56/56
+ lifecycle 6/6) is green.

## Why

Phase 1.3's read rewrite is correct production behavior (the plan's whole
point). The tests were written for the old pump where read only saw
post-registration bytes. Under the ring, read sees all bytes from the
shared cursor forward — including boot-banner remnants, command echoes,
and data that arrived between flush and read. The tests must account for
this.

## The 3 failure patterns + fixes

### Pattern A: `flush` → `read` → `write` expects read to see ONLY the write's response

The test flushes (clearing the ring + kernel buffer), starts a read,
then writes a command. Under the old model, the read registered a
future-only consumer and saw only the write's response. Under the ring
model, the read drains whatever is in the ring at cursor position — which
may include boot-banner bytes that arrived BETWEEN flush and read, or
echo bytes from the sync_boot sequence.

**Fix:** Replace `flush_both` (or add after it) with a `seek {to:
"live_edge"}` call before starting the read. `seek` to live_edge moves
the cursor past all buffered data without destroying it, so the read
starts at the live edge and sees only bytes that arrive AFTER the seek
(= the write's response). This is the non-destructive equivalent of
flush-before-read that the plan explicitly recommends ("Most old
flush-before-read habits should migrate to `seek`").

Alternatively, if the test's flush is meant to clear stale state (not
just to position the read), keep the flush AND add the seek. The flush
clears the ring; the seek positions the cursor at the (now-empty) live
edge. Either order works since flush clears + clamps cursor to live_edge
already (Phase 1.3's flush semantics). So if flush is already called, the
read SHOULD start at live_edge... unless bytes arrived between flush and
read. The fix is to ensure the read starts AFTER any stale data is past
the cursor — either seek right before the read, or start the read task
AFTER the write (see Pattern B).

### Pattern B: `read` spawned → sleep → `write` — read drains ring instantly, returns `drained` before write happens

The test spawns a read task, sleeps 100ms, then writes. Under the old
model, the read blocked waiting for data (future-only). Under the ring,
the read sees buffered bytes (if any) and returns immediately with
`stop_reason: "drained"` — BEFORE the write happens. The test then
asserts on the write's response, which never arrived.

**Fix:** Either:
- (a) Start the read task AFTER the write (write → then read → assert).
  This is the simplest and matches the cat semantics: write first, then
  read sees the buffered response. For tests that need to verify
  concurrent read-during-write, use `match` to force the read to wait
  for the specific response pattern.
- (b) Keep the read-before-write order but add a `match` pattern that
  forces the read to wait for the write's response (e.g. `match: {pattern:
  "pong"}`). The read will drain any buffered bytes, not find the match,
  then wait for new bytes (the write's response) and return on match.
- (c) For framing tests that need to capture multiple lines as frames:
  start the read AFTER writing all commands, with a short timeout. The
  ring has all the responses; the read drains them as frames.

Pick (a) where the test doesn't need concurrency, (b) where it does.

### Pattern C: frame count / data assertions see extra frames from ring remnants

The test writes commands, reads with framing, and asserts exact frame
count / first-frame data. Under the ring, the read may see extra frames
from boot-banner remnants or partial lines that the old fanout path
missed.

**Fix:** Ensure the ring is clean before the test's writes: `flush_both`
+ `seek {to: "live_edge"}` (or just `flush_both` which now clears the
ring + clamps cursor). If extra frames still appear, adjust the
assertion to search for the expected frame(s) in the frames array (like
the Phase 2 recovery test's `.find()` pattern) instead of asserting
`frames[0]` / exact count.

## Specific tests to fix

### native_sim_validation (17 failing)

Each test below needs one of the pattern fixes. The executor should read
each test, identify which pattern applies, and apply the fix. The tests
are in `tests/native_sim_validation/unix.rs`.

1. `native_read_line_framing_splits_lines` (~:1880) — Pattern B/C: read
   spawned before write, gets empty first frame. Fix: start read AFTER
   both writes, or use `match` to wait for "pong".
2. `native_read_explicit_line_endings_split_correctly` (~:3356) — Pattern
   C: 3 frames instead of 2. Fix: ensure ring clean before writes, or
   search for expected frames.
3. `native_read_json_parser_decodes_jsonout` (~:1940) — similar.
4. `native_read_at_parser_parses_pong` — similar.
5. `native_read_length_prefixed_framing_decodes` — similar.
6. `native_read_framing_plus_match_combined` — Pattern B: use match to
   force waiting.
7. `native_read_buffer_budget_stops_under_flood` — Pattern B: read drains
   ring instantly instead of blocking on flood. Fix: write first, then
   read; or adjust the flood timing.
8. `native_read_explicit_rx_framing_overrides_protocol` — similar.
9. `native_explicit_rx_framing_beats_connection_default` — similar.
10. `native_open_protocol_default_drives_write_and_read` — similar.
11. `native_write_explicit_tx_framing_overrides_protocol` — similar.
12. `native_write_protocol_preset_appends_cr` — similar.
13. `native_flush_during_arm_cmd_delay` — Pattern A: flush semantics
    changed (now clears ring). Adjust test to account for ring clear.
14. `native_flush_output_after_full_delivery_is_safe` — similar.
15. `native_flush_after_write` — similar.
16. `native_ack_command_provides_pre_execution_ack` — Pattern B.
17. `native_txbuf_status_reports_pending` — may be a timing issue; check.
18. `native_auto_reconnect_preserves_connection` — reconnect + ring
    persistence; the test may expect fresh state after reconnect but the
    ring persists. Adjust to account for retained bytes.

### native_sim_connection_lifecycle (1 failing)

19. `native_close_while_read_active_returns_close_error` (~:372) — the
    test expects `close` to return an error when a read is active
    (`is_error: Some(true)`). Under the ring, the read drains instantly
    (`stop_reason: "drained"`) and returns before close. Fix: the test's
    premise (read blocks, close interrupts) is no longer valid. Either:
    - (a) Make the read block: use a `match` pattern that won't be found
      (so the read waits), then close → the read gets
      `connection_closed` stop_reason, and close succeeds. Adjust
      assertions.
    - (b) Accept that read drains instantly and close always succeeds;
      rewrite the test to verify close-while-read-active returns success
      (not error) and the read got `drained`.

### protocol_emulator (2 re-ignored)

20. `protocol_emulator_workflow` — re-ignored by Phase 1.3 executor. The
    subscribe stage needs Phase 2 (subscribe from ring). BUT the read
    stages (Stage 3-6) should now work with ring semantics. Un-ignore
    and fix the read stages (Pattern B: write-before-read or match); keep
    the subscribe stage ignored ONLY if it still fails (it uses the
    fanout path which races with the always-on pump). If the subscribe
    stage is the only failure, split the test or keep it ignored with a
    Phase 2 note.
21. `protocol_emulator_binary_workflow` — similar.

## Out of scope

- No production code change. The read/seek/flush/ring code from Phase
  1.3 is correct.
- Phase 2 (subscribe rewrite).
- Phase 3 (release).
- No version bump.

## Verification commands

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
cargo test --test protocol_emulator
cargo test --test protocol_emulator_binary
cargo test --test serial_pty
cargo test --test doc_drift
cargo test --lib serial::schema
```

All must pass. native_sim validation 56/56 + lifecycle 6/6. The
protocol_emulator tests either pass or are ignored with a Phase 2 note
(if the subscribe stage is the only remaining failure).

## Deliverable

Update all failing tests. Do not push, merge, open a PR, amend, or
force-push. After verification passes, commit with a message like:

```
test: update native_sim + protocol-emulator + lifecycle tests for ring-based read semantics
```

Stage only test files (and any test-support changes).

Return a concise recap:
- tests changed + the pattern fix applied to each,
- native_sim validation + lifecycle results,
- protocol_emulator status (passing or ignored with Phase 2 note),
- commit hash and message,
- any blockers or deviations,
- suggested follow-ups (Phase 2: subscribe rewrite).