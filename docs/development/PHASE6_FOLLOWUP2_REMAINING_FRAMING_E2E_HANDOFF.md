# Phase 6 Follow-up #2 Handoff — Remaining framing e2e coverage gaps

Coverage audit follow-up. The first follow-up closed SLIP/delimiter/
length-prefixed/subscribe+parser. These remain (lower priority but
explicitly requested for full coverage before wrap-up):

1. `start_end` RX framing — structurally similar to delimiter but never
   run e2e.
2. TX non-line framing modes (delimiter, length_prefixed, start_end, SLIP)
   — verifiable via the firmware `trace on` command, which echoes
   `RX[n]=0xXX\r\n` per received byte (see `native_trace_reports_exact_split_byte_sequence`
   for the pattern).
3. RX `lf`/`cr`/`crlf` explicit line endings — auto is covered; the three
   explicit endings are unit-only. Observable via frame `data` shape.
4. SLIP resync-after-error over real serial — unit covers resync; an e2e
   test proves the decoder recovers and decodes a valid frame after a
   malformed escape, over the real serial path.

Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Add native_sim e2e tests for the four remaining framing-coverage gaps. No
production changes expected. All use existing firmware commands
(`sendraw hex`, `ping`, `trace on`). If a test surfaces a wiring bug, fix
in a separate `fix:` commit.

## In scope

Add four tests to `tests/native_sim_validation.rs`. All
`#[ignore = "requires native_sim firmware binary"]`, `#[cfg(unix)]`.

### 1. `native_read_start_end_framing_decodes`

Proves `start_end` RX framing over the real serial path.

- `open_pty`, `sync_boot`, `flush_both`.
- Spawn a `read` task with
  `rx_framing: { "type": "start_end", "start": "<<", "end": ">>", "marker_encoding": "utf8" }`,
  `timeout_ms: 3000`, `max_buffered_bytes: 512`, `encoding: "utf8".
- Sleep ~100ms.
- `write_cmd(&client, &id, "sendraw text <<pong>>")` — firmware emits
  `<<pong>>` raw (no terminator; `sendraw text` sends the string verbatim).
  Hex alternative if `sendraw text` mangles the angle brackets:
  `sendraw hex 3C3C706F6E673E3E` (`<<pong>>`).
- Await read result. Assert:
  - `frames` non-empty.
  - `frames[0].data == "pong"`.
  - `frames[0].frame_type == "start_end"`.
- Close + cancel.

Prefer the `sendraw hex` form for determinism (no shell/strtok
interaction with `<<`/`>>`). Hex: `3C 3C 70 6F 6E 67 3E 3E` = `<<pong>>`.

### 2. `native_write_tx_framing_modes_observed_via_trace`

Proves TX framing for delimiter, length_prefixed, start_end, and SLIP by
observing the exact bytes the firmware receives via `trace on`.

This is ONE test that exercises four TX modes in sequence (open one
connection, enable trace once, do four write+read cycles). Keeping it as
one test avoids four separate firmware spawns; the trace output
accumulates and each cycle is asserted on its own substring.

- `open_pty`, `sync_boot`, `flush_both`.
- `write_cmd(&client, &id, "trace on")` — firmware now echoes
  `RX[n]=0xXX\r\n` for every byte received.
- `flush_both` to clear the `trace on\r\n` ack.
- For each of the four TX modes, do:
  1. Spawn a `read` task with `match: { pattern: "pong" }` (so the read
     completes when the firmware echoes `pong` after the traced bytes —
     the trace lines AND the `pong` all arrive; matching on "pong"
     terminates cleanly). Use `timeout_ms: 3000`, `max_buffered_bytes:
     4096`, `encoding: "utf8"`.
  2. Sleep ~100ms.
  3. `write` with `tx_framing` set to the mode under test and `data:
     "ping"`:
     - delimiter: `tx_framing: { "type": "delimiter", "delimiter": "|", "delimiter_encoding": "utf8" }` → firmware receives `ping|`. Trace expects `RX[k]=0x70`..'RX[k+3]=0x67' (ping) then `RX[k+4]=0x7c` (`|`).
     - length_prefixed: `tx_framing: { "type": "length_prefixed", "prefix_size": 1, "endianness": "big" }` → firmware receives `\x04ping`. Trace expects `RX[k]=0x04` then `0x70`..`0x67`.
     - start_end: `tx_framing: { "type": "start_end", "start": "<<", "end": ">>", "marker_encoding": "utf8" }` → firmware receives `<<ping>>`. Trace expects `0x3c 0x3c 0x70..0x67 0x3e 0x3e`.
     - slip: `tx_framing: { "type": "slip" }` → firmware receives `\xC0ping\xC0`. Trace expects `0xc0 0x70..0x67 0xc0`.
  4. Await the read result. Assert `is_error` not true and the `data`
     field contains the expected trace hex substrings for that mode
     (e.g. `"RX[0]=0x04"` for length_prefixed — note: the RX counter
     persists across cycles since trace stays on, so assert on the
     distinctive bytes, not absolute indices; assert the byte VALUE
     `0x04` appears somewhere as `RX[*]=0x04`). For robustness, assert
     the sequence of values in order: find `0x70` (p), then later `0x69`
     (i), `0x6e` (n), `0x67` (g), plus the mode-specific framing byte
     (`0x7c` / `0x04` / `0x3c` / `0xc0`).
- After all four, `write_cmd(&client, &id, "trace off")` (cleanup so later
  tests aren't noisy — though each test spawns its own firmware, so this
  is optional).
- Close + cancel.

NOTE on the RX counter: `trace on` emits `RX[n]=0xXX` where `n` is a
per-firmware-process counter that resets to 0 only at `trace on` time
(see `cmd_binary` line 358: `state->uart->rx_seq = 0` — but `trace on`
in `cmd_trace` line 267 does NOT reset `rx_seq`; check `uart_drv.c` line
26 for the counter source). The counter is cumulative across all bytes
received since firmware start. Do NOT assert absolute indices across
cycles — assert byte VALUES and relative ordering only. If the counter
resets at `trace on`, indices start at 0 for the first cycle; for later
cycles they continue. Either way, asserting on the hex value substring
(e.g. `"RX[12]=0x04"` is fragile; prefer `"=0x04"` or scan the data for
the byte-value sequence).

Simplest robust assertion: collect all `RX[*]=0xXX` substrings from the
read `data`, parse the hex values into a byte sequence, and assert that
sequence contains the expected framed bytes in order. This avoids
index fragility entirely.

### 3. `native_read_explicit_line_endings_split_correctly`

Proves RX `lf`, `cr`, `crlf` explicit endings over the real serial path.
One test, three sub-cycles (each opens a fresh connection OR reuses one
with flush between cycles — fresh connection is cleaner since decoder
state is per-call anyway).

For each ending in `["lf", "cr", "crlf"]`:
- `open_pty`, `sync_boot`, `flush_both` (reuse the same connection across
  the three endings is fine — each `read` is a new decoder).
- Spawn a `read` task with
  `rx_framing: { "type": "line", "ending": "<ending>" }`,
  `timeout_ms: 3000`, `max_buffered_bytes: 512`, `encoding: "utf8"`.
- Sleep ~100ms.
- Emit a frame followed by the ending under test, then a second frame
  with a known terminator so the read sees at least one complete frame:
  - `lf`: `sendraw text "alpha\nbeta\n"` — `lf` splits on `\n`, no CR
    strip. Frame0 = "alpha", frame1 = "beta". Assert `frames[0].data
    == "alpha"` and `frames[1].data == "beta"`.
  - `cr`: `sendraw text "alpha\rbeta\r"` — but `sendraw text` takes the
    rest of the line after `text `; a literal `\r` in the command may
    terminate the command early (firmware commands terminate on `\r` OR
    `\n`). Use `sendraw hex` instead: `sendraw hex 616C7068610D626574610D`
    = `alpha\rbeta\r`. `cr` splits on `\r`. Frame0 = "alpha", frame1 =
    "beta".
  - `crlf`: `sendraw hex 616C7068610D0A626574610D0A` = `alpha\r\nbeta\r\n`.
    `crlf` splits on exact `\r\n`. Frame0 = "alpha", frame1 = "beta".
  For `lf`, to prove it does NOT strip a preceding CR (the Phase 1
  breaking behavior), emit `alpha\r\nbeta\n`:
  `sendraw hex 616C7068610D0A62657461 0A` = `alpha\r\nbeta\n`. `lf`
  splits on `\n` only. Frame0 = "alpha\r" (CR retained!), frame1 =
  "beta". Assert `frames[0].data == "alpha\r"` — this is the
  distinguishing assertion vs `auto` (which would strip the `\r`).
- Await read result. Assert the expected frame data per ending.
- Close + cancel (or flush and continue for the next ending).

Prefer `sendraw hex` for all three to avoid command-terminator
ambiguity. Hex for the three payloads:
- `lf` (proves CR retention): `616C7068610D0A62657461 0A` =
  `alpha\r\nbeta\n`.
- `cr`: `616C7068610D626574610D` = `alpha\rbeta\r`.
- `crlf`: `616C7068610D0A626574610D0A` = `alpha\r\nbeta\r\n`.

### 4. `native_read_slip_resyncs_after_error_and_decodes_next_frame`

Proves SLIP resync-after-error over the real serial path — the decoder
recovers after a malformed escape and decodes a subsequent valid frame.
This extends test 2 from the first follow-up (which only asserts the
error surfaces) by continuing past the error.

IMPORTANT: This test exercises the DECODER's resync state, but the
current read/subscribe loops STOP on the first `DecodeError` (Phase 3
design — `read_bytes_via_session` returns `Err`, subscribe emits a
final notification). So a single `read` call CANNOT see both the error
AND a recovered frame — the error terminates the call. To test resync
end-to-end, use TWO sequential `read` calls on the SAME connection
(the decoder is recreated per call, so resync state does NOT persist
across calls — see Phase 2 decision: promotion is per-call). This
means the "resync" tested here is really "after a read that errored, a
fresh read decodes a valid frame" — which is the per-call reset
behavior, NOT in-call resync.

Given the per-call decoder lifecycle, an in-call resync e2e test is NOT
possible with the current read loop (it stops on first error). The
unit tests `rx_slip_resyncs_after_malformed_escape` and
`rx_slip_resync_clears_stale_in_progress_buf` already cover the
decoder's internal resync state machine. So this e2e test should
instead verify the PER-CALL RESET: after a read that errored on a
malformed SLIP frame, a SECOND read on the same connection decodes a
valid SLIP frame cleanly (proving the error didn't leave the
connection in a bad state).

- `open_pty`, `sync_boot`, `flush_both`.
- Read #1: spawn a `read` with `rx_framing: slip`, `timeout_ms: 2000`.
  Sleep ~100ms. `sendraw hex C0DB41C0` (malformed). Await — assert
  `is_error: true` (malformed surfaces).
- Read #2: spawn a fresh `read` with `rx_framing: slip`, `timeout_ms:
  2000`. Sleep ~100ms. `sendraw hex C0706F6E67C0` (valid "pong"). Await
  — assert `is_error` not true, `frames[0].data == "pong"`,
  `frame_type == "slip"`.
- Close + cancel.

This proves the connection is usable after a SLIP decode error (per-call
decoder reset, no connection-level corruption). Rename the test to
`native_read_slip_recovers_after_error_on_next_call` to reflect what it
actually proves (per-call reset, not in-call resync). If you prefer to
keep the original name, document the distinction in a comment.

## Out of scope

- In-call SLIP resync (impossible with current read loop that stops on
  first error; unit tests cover the decoder state machine).
- subscribe + delimiter/length_prefixed/start_end e2e (subscribe's
  framing path is shared with read via `consume_frames`; read coverage
  is sufficient).
- RX line `auto` bare-CR promotion e2e over native_sim (covered by PTY;
  native_sim firmware emits CRLF, not bare CR, so it can't trigger
  promotion without `sendraw hex` crafting — and the PTY test already
  covers it).

## Relevant files and current behavior

- `tests/native_sim_validation.rs`:
  - `native_trace_reports_exact_split_byte_sequence` (~line 572):
    reference for the `trace on` + read + assert `RX[n]=0xXX` pattern.
    The test reads `data` as a UTF-8 string and asserts substrings.
  - `native_read_slip_decodes_frame` / `native_read_delimiter_framing_decodes`
    / `native_read_length_prefixed_framing_decodes` (added in follow-up #1):
    reference for the `sendraw hex` + read + assert frame pattern.
  - `write_cmd`, `write_raw`, `open_pty`, `sync_boot`, `flush_both`,
    `close_connection`: existing helpers.
- `firmware/src/command.c`:
  - `cmd_sendraw` (line 93): `sendraw hex <hex>` emits raw bytes, no
    terminator. `sendraw text <str>` emits the string verbatim, no
    terminator — BUT the command itself is terminated by `\r`/`\n`, so
    a literal `\r` in the `text` payload would terminate the command
    early. Use `sendraw hex` for any payload containing `\r` or `\n`.
  - `cmd_trace` (line 259): `trace on` sets `state->uart->trace_on = true`.
    Does NOT reset `rx_seq` (unlike `binary on` which does). The trace
    counter is cumulative.
- `firmware/src/uart_drv.c` (line 25): emits `RX[%u]=0x%02x\r\n` per
  received byte when `trace_on`.

## Expected test shape (test 1, sketch)

```rust
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_start_end_framing_decodes() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();
    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();
    let id = open_pty(&client, &pty_path).await;
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request("read", json!({
                "connection_id": id2,
                "timeout_ms": 3000,
                "max_buffered_bytes": 512,
                "encoding": "utf8",
                "rx_framing": {
                    "type": "start_end",
                    "start": "<<",
                    "end": ">>",
                    "marker_encoding": "utf8"
                }
            }))).await
        })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    // 3C3C706F6E673E3E = <<pong>>
    write_cmd(&client, &id, "sendraw hex 3C3C706F6E673E3E").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    assert_eq!(frames[0]["data"], json!("pong"));
    assert_eq!(frames[0]["frame_type"], json!("start_end"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}
```

For test 2 (trace-observed TX), build a helper that extracts the byte
sequence from the trace `data` string:

```rust
fn extract_trace_bytes(data: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for cap in data.lines() {
        // Lines look like "RX[12]=0x70" (possibly with trailing \r).
        if let Some(idx) = cap.find("=0x") {
            let hex = &cap[idx+3..].trim_end();
            if hex.len() >= 2 {
                if let Ok(b) = u8::from_str_radix(&hex[..2], 16) {
                    bytes.push(b);
                }
            }
        }
    }
    bytes
}
```

Then assert the extracted byte sequence contains the expected framed
bytes as a contiguous subsequence (use a window search, same as
`find_subsequence` in `src/framing.rs`). This avoids index fragility.

For test 3 (explicit line endings), three cycles with `sendraw hex`
payloads. For `lf`, the distinguishing assertion is
`frames[0]["data"] == "alpha\r"` (CR retained — proving `lf` does NOT
strip, unlike `auto`).

For test 4 (per-call reset), two sequential reads; assert read #1 errors
and read #2 succeeds with "pong".

## Test plan

1. `native_read_start_end_framing_decodes` — `<<pong>>` decodes to
   "pong", frame_type "start_end".
2. `native_write_tx_framing_modes_observed_via_trace` — four TX modes;
   trace-observed bytes match the expected framed sequences:
   `ping|`, `\x04ping`, `<<ping>>`, `\xC0ping\xC0`.
3. `native_read_explicit_line_endings_split_correctly` — three endings;
   `lf` retains CR in frame data, `cr` and `crlf` split as expected.
4. `native_read_slip_recovers_after_error_on_next_call` — read #1 errors
   on malformed SLIP; read #2 decodes valid "pong" frame cleanly.
5. All existing native_sim tests remain green.

If any test fails, root-cause before fixing. Separate `fix:` commit if
production wiring is wrong.

## Constraints and invariants

- `#[ignore]` + `#[cfg(unix)]` on all four tests.
- No `unwrap`/`expect`/`println!` in production code. Tests may use
  `.unwrap()`.
- No production code change expected. If a test fails and a fix is
  needed, separate `fix:` commit.
- Conventional commits: `test:` for the additions. No attribution
  footers.

## Verification commands

```bash
fw-build-native

cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --test native_sim_validation -- --ignored
cargo clippy --all-targets --locked -- -D warnings
```

Specifically confirm the four new tests pass:
```bash
cargo test --test native_sim_validation -- --ignored \
    native_read_start_end_framing_decodes \
    native_write_tx_framing_modes_observed_via_trace \
    native_read_explicit_line_endings_split_correctly \
    native_read_slip_recovers_after_error_on_next_call
```

Full gate after:
```bash
cargo test --locked
```

CRITICAL: actually run `-- --ignored`. fmt/build/clippy do NOT execute
`#[ignore]`d tests.

## Return instructions

When done, return:

- Tests added (names + file).
- For each test: pass/fail on first run. If a test failed, name the
  `fix:` commit (if any) and root-cause.
- For test 2: confirm the trace-observed byte sequences for all four TX
  modes match expectations (`ping|`, `\x04ping`, `<<ping>>`,
  `\xC0ping\xC0`). Note any surprise in the trace counter or byte
  ordering.
- For test 3: confirm `lf` retains the CR (`frames[0].data == "alpha\r"`)
  — the distinguishing assertion vs `auto`.
- For test 4: confirm read #2 succeeds after read #1 errored (per-call
  decoder reset, connection not corrupted).
- Gate results — INCLUDING `native_sim_validation -- --ignored` (do not
  skip). Note any failures with root cause.
- Any surprise in the `sendraw hex` payloads, the trace byte extraction,
  or the explicit-ending frame shapes.