# Phase 3 Follow-up Handoff — SLIP resync fix + subscribe PTY test

Source: Phase 3 recap observations flagged during review. Last phase's
follow-up surfaced a real bug, so addressing these now rather than deferring.

Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Two small, independent fixes for Phase 3 SLIP:

1. **Resync-on-error fix.** Make the SLIP decoder honor the original
   Phase 3 handoff contract: on malformed escape, discard the in-progress
   frame buffer and transition to `BeforeFirstEnd` BEFORE returning `Err`.
   The current code returns `Err` immediately with no state change, leaving
   `InFrame { buf, escaped: true }` intact. Not a live bug today (read and
   subscribe both stop on the first `DecodeError`), but it is a latent
   footgun if any future caller continues after an error, and it contradicts
   the handoff spec and the recap.
2. **Subscribe PTY SLIP test.** Add a PTY-level test proving the subscribe
   loop surfaces a SLIP malformed escape as a final notification with
   `stop_reason: "framing_error"` and an `error` field. Mirrors the Phase 2
   subscribe-bare-CR follow-up pattern.

## In scope

### A. SLIP resync-on-error fix

`src/framing.rs`, in `slip_decode`, the malformed-escape arm (currently
lines ~513-515):

Current:
```rust
_ => {
    return Err(FrameDecodeError::SlipInvalidEscape(b));
}
```

Required behavior on the error path (before returning `Err`):

- Drain the in-progress frame buffer (`buf`): `buf.clear()`.
- Reset `*escaped = false`.
- Transition the decoder mode to `BeforeFirstEnd` so a subsequent `push`
  (if a caller ever continues) skips to the next END marker.
- Drain `buf_outer` up to and including the violating byte is NOT required —
  the caller stops on first error in phase 3, and `BeforeFirstEnd` will
  discard remaining bytes anyway on the next `push`. Keep it simple: just
  reset state, then `return Err`.

Concretely, replace the arm with something equivalent to:
```rust
_ => {
    buf.clear();
    *escaped = false;
    *state = SlipState::BeforeFirstEnd;
    return Err(FrameDecodeError::SlipInvalidEscape(b));
}
```
(Exact borrow shape may need adjusting since `state` is borrowed mutably
from `mode` at the top of `slip_decode`; the existing code already mutates
`*state` via `*state = SlipState::InFrame { ... }` in the `BeforeFirstEnd`
arm, so the same pattern works here.)

This makes the code match the Phase 3 handoff ("Discard `buf`, transition
to `BeforeFirstEnd` to resync") and the Phase 3 recap ("transitions to
`BeforeFirstEnd`").

### B. Subscribe PTY SLIP test

Add one test to `tests/serial_pty.rs`:

- `pty_subscribe_slip_malformed_escape_emits_framing_error`

Shape (adapt from `pty_subscribe_line_auto_promotes_on_bare_cr_and_flushes_pending`):

1. `subscribe` with `rx_framing: { type: "slip" }`, `poll_interval_ms: 50`,
   no `timeout_ms` (match existing framing subscribe tests; the close/error
   path stops the stream).
2. Write a malformed SLIP stream from the PTY master:
   `b"\xC0\xDB\x41\xC0"` — END, ESC, invalid byte `0x41`, END.
3. Collect notifications via `next_notification(&mut rx, Duration::from_secs(2))`.
4. Assert a final stop notification appears with:
   - `stop_reason` == `"framing_error"` (as a JSON string)
   - an `error` field whose string value contains `"SLIP framing error"`
     and the violating byte (`0x41`).
5. Cancel the subscription via `client.cancel().await.ok();`.

Do NOT assert frame notifications — the malformed escape should produce no
frame; only the final stop notification. If a frame notification for an
empty/partial frame appears before the stop, ignore it (filter for the
stop notification by absence of `frame_index`, same convention as the
existing PTY framing tests).

## Out of scope

- Automatic resume-on-error as a first-class feature. The decoder still
  returns `Err` on the first malformed escape; read and subscribe still
  STOP. The resync is purely defensive state hygiene for hypothetical
  future callers.
- Any change to the `FrameDecodeError` type or `framing_error` stop reason.
- TX side changes (TX encode cannot produce a malformed escape).
- Subscribe unit test via a mock `stream_rx_via_session` harness (out of
  scope — PTY is the right level, same as Phase 2 follow-up).
- Correcting the recap prose itself is optional; if you do touch
  `PHASE3_RECAP.md` to match the now-fixed code, keep it to the one
  sentence about resync. Not required.

## Relevant files and current behavior

- `src/framing.rs`:
  - `slip_decode` free function (lines ~468-541). The malformed arm is at
    lines ~513-515.
  - `SlipState::InFrame { buf, escaped }` — `buf` is the in-progress frame
    payload; `escaped` is true after a bare `ESC`.
  - `SlipState::BeforeFirstEnd` — discards bytes until the next END.
  - The `BeforeFirstEnd` arm (lines ~482-492) shows the existing pattern for
    transitioning `*state` and continuing the loop.
- `tests/serial_pty.rs`:
  - `setup()` returns `(TestServer, client, notification_rx, PtyPair, connection_id)`.
  - `pty.write_device(&[u8])` writes as the device.
  - `next_notification(&mut rx, Duration)` awaits one MCP logging notification.
  - Frame notifications carry `frame_index`; stop notifications do not.
  - Existing reference tests: `pty_subscribe_framing_emits_per_frame_notifications`
    (line ~535), `pty_subscribe_framing_match_stops_at_frame` (line ~577),
    `pty_subscribe_line_auto_promotes_on_bare_cr_and_flushes_pending` (end of file).
- `src/tools/stream_ops.rs`:
  - `stream_rx_via_session` handles `FrameOutcome::DecodeError` (line ~518):
    sets `stop_outcome` via `ctrl.framing_error(e)`, saves `frame_error_msg`.
  - Stop notification payload includes `error` field (line ~687):
    `stop_payload["error"] = serde_json::json!(e);`.

## Expected test shape

```rust
#[tokio::test]
async fn pty_subscribe_slip_malformed_escape_emits_framing_error() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "poll_interval_ms": 50,
                "rx_framing": { "type": "slip" },
            }),
        ))
        .await
        .unwrap();

    // END, ESC, invalid byte 0x41, END → malformed escape.
    pty.write_device(b"\xC0\xDB\x41\xC0").await.unwrap();

    // Collect notifications until the stop notification appears.
    let mut stop: Option<serde_json::Value> = None;
    for _ in 0..16 {
        let n = next_notification(&mut rx, Duration::from_secs(2))
            .await
            .unwrap();
        let obj = n.data.as_object().unwrap();
        if obj.get("stop_reason").is_some() {
            stop = Some(n.data.clone());
            break;
        }
    }
    let stop = stop.expect("received framing_error stop notification");
    assert_eq!(stop["stop_reason"], json!("framing_error"));
    let err = stop["error"].as_str().expect("error field present");
    assert!(err.contains("SLIP framing error"), "error msg: {err}");
    assert!(err.contains("0x41"), "error msg names violating byte: {err}");
    client.cancel().await.ok();
}
```

Adapt as needed if the notification shape differs, but keep the assertions:
`stop_reason == "framing_error"`, `error` field contains `"SLIP framing
error"` and the violating byte hex.

## Test plan

1. **resync leaves decoder in BeforeFirstEnd.** New unit test in
   `src/framing.rs` `mod tests`: push a malformed SLIP stream, assert
   `push` returns `Err`, then push a valid SLIP frame
   (`b"\xC0ok\xC0"`) and assert it decodes to `b"ok"`. This proves the
   decoder resyncs after an error rather than being stuck in `InFrame`
   with `escaped=true`. (Without the fix, the second push would try to
   resolve the stale `escaped=true` state against `0xC0`, producing wrong
   behavior.)
2. **resync clears in-progress buf.** New unit test: push
   `b"\xC0hello\xDB\x41"` (partial frame then malformed escape), assert
   `Err`, then push `b"\xC0world\xC0"` and assert frame `b"world"` — the
   stale `hello` bytes must NOT appear in the resynced frame.
3. **pty_subscribe_slip_malformed_escape_emits_framing_error** passes.
4. **existing SLIP tests still pass.** `rx_slip_malformed_escape_returns_err`
   and all other SLIP unit/integration tests remain green.

## Constraints and invariants

- **No `unwrap`/`expect`/`println!`/`todo!()`/`unimplemented!()`** in
  production code. Tests may use `.unwrap()`.
- **Tool failures become MCP tool results with `is_error: true`**, not
  protocol-level `McpError`. No change to error surfacing here — the fix
  is internal state hygiene only.
- **No new public API.** `SlipState` is internal; the resync is not exposed.
- **`flush_partial` contract unchanged.** Still emits a partial frame for
  SLIP `InFrame` state.
- **Conventional commits.** Suggested: one `fix:` commit for the resync
  state change (production code change), one `test:` commit for the PTY
  test. Or a single `fix:` if bundled — but the PTY test is additive, so
  `test:` is more accurate for it. Do not mix a production fix into a
  `test:` commit (the Phase 2 follow-up did that; avoid the pattern).
  No attribution footers.

## Verification commands

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --lib framing
cargo test --test serial_pty
cargo clippy --all-targets --locked -- -D warnings
```

Full gate after:

```bash
cargo test --locked
```

## Return instructions

When done, return:

- Files changed and why.
- The exact resync state mutation added (which fields reset, where).
- Confirm the decoder now decodes a valid frame after a prior malformed
  escape (test 1/2 results).
- The PTY test name and whether it passed first run.
- Gate command results (`fmt`, `build`, `lib framing`, `serial_pty`,
  `clippy`, and `cargo test --locked` if run).
- Any surprise in the resync borrow shape or notification timing.