# Phase 2 Follow-up Handoff — Subscribe-loop bare-CR integration test

Source: Phase 2 recap deferred handoff test-plan item 11 (subscribe
integration: `flush_partial` emits pending CR at stop). The read-loop
integration tests cover the shared `FrameDecoder`, but subscribe has its own
loop in `src/tools/stream_ops.rs::stream_rx_via_session` with a distinct
`flush_partial` call site (~line 619) and notification emission path. This
adds one PTY-level integration test to cover the subscribe side directly.

Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Add a single PTY integration test in `tests/serial_pty.rs` proving that a
`subscribe` call with `rx_framing: { type: "line", ending: "auto" }` emits
frame notifications for a device that sends bare-CR line endings, and flushes
the trailing pending `\r` as a final frame when the connection closes.

## In scope

Add exactly one new test to `tests/serial_pty.rs`:

- `pty_subscribe_line_auto_promotes_on_bare_cr_and_flushes_pending`

Behavior to assert (matches the Phase 2 decoder state machine):

1. `subscribe` with `rx_framing: { type: "line" }` (ending defaults to
   `auto`), `poll_interval_ms: 50`, and a short `timeout_ms` (e.g. 2000) so
   the stream auto-stops if the close path isn't reached first. Do NOT set
   `no_new_rx_timeout_ms` — the test relies on either the next byte
   confirming bare CR or the connection close flushing the pending `\r`.
2. From the PTY master, write a byte stream that triggers auto promotion:
   - `b"line1\r"` — pending CR, no frame emitted yet.
   - `b"line2\r"` — second byte `l` (non-`\n`) confirms the first `\r` as
     bare CR. Expect frame `"line1"` emitted, decoder promotes to CrMode,
     `"line2\r"` then splits on the trailing `\r` → frame `"line2"`.
   - Use two `pty.write_device` calls (or one) — the decoder is byte-driven
     and cross-chunk safe, so either works. Two calls more clearly exercise
     the cross-chunk `PendingCr` path.
3. Collect notifications via `next_notification(&mut rx, Duration::from_secs(2))`
   (same pattern as `pty_subscribe_framing_emits_per_frame_notifications` at
   line ~535). Filter for notifications carrying a `frame_index` field (frame
   notifications do; the stop notification does not).
4. Assert the collected frame notifications contain, in order:
   - frame_index 0, data `"line1"`, frame_type `"line"`
   - frame_index 1, data `"line2"`, frame_type `"line"`
5. Assert each frame `data` does NOT contain `\r` (terminator stripped, since
   `include_terminators` is not set — default false).
6. Cancel the subscription via `client.cancel().await.ok();` at the end (same
   cleanup as the existing framing tests).

Do NOT add a separate close-flush variant — the existing
`pty_subscribe_framing_partial_on_close`-style coverage in
`tests/native_sim_validation.rs` already exercises close-during-subscribe.
This one test proves the promotion + bare-CR split on the subscribe path,
which is the gap.

## Out of scope

- Any change to production code. The decoder and subscribe loop already
  handle this; only a test is added.
- A `stream_rx_via_session` unit test. Building a mock `RxSession` +
  `Arc<Connection>` + sink harness for one test is disproportionate. PTY is
  the right level.
- subscribe tests for `lf`/`cr`/`crlf` endings (already covered by existing
  `pty_subscribe_framing_*` tests using `ending: auto` on LF streams).
- Any new schema/config/validation work.

## Relevant files and current behavior

- `tests/serial_pty.rs`:
  - `setup()` returns `(TestServer, client, notification_rx, PtyPair, connection_id)`.
  - `pty.write_device(&[u8])` writes bytes as the device (master end).
  - `next_notification(&mut rx, Duration)` awaits one MCP logging notification.
  - Frame notifications carry `frame_index`, `frame_type`, `data`. Stop
    notification carries `stop_reason`, `truncated`, `bytes_observed`,
    `bytes_returned`, `elapsed_ms` but NOT `frame_index`.
  - Existing reference: `pty_subscribe_framing_emits_per_frame_notifications`
    (line ~535) and `pty_subscribe_framing_match_stops_at_frame` (line ~577).
- `src/framing.rs`: `LineState::AutoLf` → `PendingCr` (trailing `\r`) →
  `CrMode` (next non-`\n` byte). Confirmation byte retained in buffer as
  start of next line. See Phase 2 recap for the full state machine.
- `src/tools/stream_ops.rs`: `stream_rx_via_session` calls
  `dec.flush_partial()` at stop (~line 619) and emits a final frame
  notification. The promotion path itself is decoder-internal; the subscribe
  loop just drains frames.

## Expected test shape

```rust
#[tokio::test]
async fn pty_subscribe_line_auto_promotes_on_bare_cr_and_flushes_pending() {
    let (_server, client, mut rx, mut pty, connection_id) = setup().await;

    client
        .peer()
        .call_tool(tool_request(
            "subscribe",
            json!({
                "connection_id": connection_id,
                "poll_interval_ms": 50,
                "timeout_ms": 2000,
                "rx_framing": { "type": "line" },
            }),
        ))
        .await
        .unwrap();

    // Pending CR, no frame yet.
    pty.write_device(b"line1\r").await.unwrap();
    // Second byte 'l' (non-\n) confirms bare CR → emit "line1", promote to
    // CrMode, then "line2\r" splits on \r → emit "line2".
    pty.write_device(b"line2\r").await.unwrap();

    let mut seen: Vec<(u64, String)> = Vec::new();
    while !(seen.iter().any(|(_, d)| d == "line1")
        && seen.iter().any(|(_, d)| d == "line2"))
    {
        let n = next_notification(&mut rx, Duration::from_secs(2))
            .await
            .unwrap();
        let obj = n.data.as_object().unwrap();
        if let Some(idx) = obj.get("frame_index").and_then(|v| v.as_u64()) {
            assert_eq!(obj["frame_type"], json!("line"), "frame_type: {obj:?}");
            let data = obj["data"].as_str().unwrap().to_string();
            assert!(!data.contains('\r'), "terminator must be stripped: {data:?}");
            seen.push((idx, data));
        }
    }

    let line1 = seen.iter().find(|(_, d)| d == "line1").unwrap();
    let line2 = seen.iter().find(|(_, d)| d == "line2").unwrap();
    assert_eq!(line1.0, 0, "line1 is frame 0");
    assert_eq!(line2.0, 1, "line2 is frame 1");
    client.cancel().await.ok();
}
```

Adapt as needed if the notification data shape differs, but keep the
assertions: frame_index ordering, frame_type `"line"`, `\r` stripped, both
frames seen.

## Test plan

1. The new test `pty_subscribe_line_auto_promotes_on_bare_cr_and_flushes_pending`
   passes.
2. All existing `pty_subscribe_framing_*` tests still pass (no regression).

## Constraints and invariants

- `#![cfg(target_os = "linux")]` already gates the file. No new cfg.
- No `unwrap`/`expect`/`println!` in production code (no production change
  here anyway).
- Conventional commit: `test:` prefix. No attribution footers.

## Verification commands

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --test serial_pty
cargo clippy --all-targets --locked -- -D warnings
```

Full gate optional but recommended after:

```bash
cargo test --locked
```

## Return instructions

When done, return:

- The exact test name added and its file.
- Whether the test passed on first run or required adjustment (and what).
- Gate command results (`fmt`, `build`, `test --test serial_pty`, `clippy`).
- Any surprise in the notification shape or promotion timing.