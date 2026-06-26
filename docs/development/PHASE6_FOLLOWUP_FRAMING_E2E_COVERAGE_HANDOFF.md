# Phase 6 Follow-up Handoff — native_sim e2e coverage for framing modes + parsers

Coverage audit of the full frame-pipeline feature (Phases 1-5) found that
several framing modes and the subscribe-with-parser path have ZERO e2e
coverage over the native_sim software-serial path. They are unit-tested and
read-integration-tested, but never exercised against the real
`tokio_serial::SerialStream` code path that production uses. The firmware's
`sendraw hex` command can emit any byte sequence (no terminator), making it
the ideal driver for non-line framing modes. This closes the high-value gaps.

Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Add five native_sim e2e tests proving that the framing modes beyond `line`,
the SLIP malformed-escape read path, and the subscribe-with-parser
notification path work over the real software-serial path. No production
changes expected; all five use existing firmware commands (`sendraw hex`,
`ping`). If a test surfaces a wiring bug, fix it in a separate `fix:`
commit.

## Why these five

| Gap | Risk if untested |
|-----|------------------|
| SLIP RX decode over real serial | Byte-stuffing is the most complex decoder; never run end-to-end |
| SLIP malformed → framing_error on READ path | PTY covers subscribe; read path's `is_error` surfacing unverified e2e |
| delimiter RX framing over real serial | Multi-byte boundary matching never run e2e |
| length_prefixed RX framing over real serial | Cross-chunk prefix hold + payload assembly never run e2e |
| subscribe + rx_parser per-frame parsed notifications | All subscribe framing tests use no parser; parsed-frame notification shape unverified |

`start_end` RX framing is unit + read-integration covered and structurally
similar to delimiter (subsequence search). Lower priority — not in this
batch. TX non-line framing modes are hard to observe through firmware
(commands need `\r`/`\n`); deferred.

## In scope

Add five tests to `tests/native_sim_validation.rs`. All
`#[ignore = "requires native_sim firmware binary"]`, `#[cfg(unix)]`.

### 1. `native_read_slip_decodes_frame`

Proves SLIP RX decode over the real software-serial path.

- `open_pty`, `sync_boot`, `flush_both`.
- Spawn a `read` task with `rx_framing: { "type": "slip" }`,
  `timeout_ms: 3000`, `max_buffered_bytes: 512`, `encoding: "utf8"`.
- Sleep ~100ms for the read to register.
- `write_cmd(&client, &id, "sendraw hex C0706F6E67C0")` — firmware emits
  raw bytes `\xC0pong\xC0` (SLIP frame containing "pong") with NO line
  terminator. `write_cmd` appends `\r\n` to the `sendraw hex ...` command
  itself, which the firmware parses; the emitted bytes are the raw SLIP
  frame.
- Await the read result. Assert:
  - `is_error` not `true`.
  - `frames` non-empty.
  - `frames[0].data == "pong"`.
  - `frames[0].frame_type == "slip"`.
- Close + cancel.

Hex breakdown: `C0` = END, `70 6F 6E 67` = "pong", `C0` = END. The decoder
skips to the first END, accumulates until the next END, emits "pong".

### 2. `native_read_slip_malformed_escape_surfaces_framing_error`

Proves the read path surfaces a SLIP malformed escape as a tool
`is_error: true` result (NOT a structured `stop_reason`, per the Phase 3
design where `read_bytes_via_session` returns `Err`).

- `open_pty`, `sync_boot`, `flush_both`.
- Spawn a `read` task with `rx_framing: { "type": "slip" }`,
  `timeout_ms: 3000`, `max_buffered_bytes: 512`, `encoding: "utf8"`.
- Sleep ~100ms.
- `write_cmd(&client, &id, "sendraw hex C0DB41C0")` — firmware emits
  `\xC0\xDB\x41\xC0` (END, ESC, invalid byte `0x41`, END).
- Await the read result. Assert:
  - `result.is_error == Some(true)` (the read tool returns an error result,
    per Phase 3 design).
  - The error is surfaced in the tool result. Inspect where the message
    lives: rmcp tool results with `is_error: true` carry the message in
    `result.content[0].text` (a JSON string) or in a structured `error`
    field — check the actual shape by printing `result` on first run and
    assert the message contains `"SLIP framing error"` and `"0x41"`.
    Adapt the assertion to the real shape; do NOT assume. The unit test
    `char_framing_slip_malformed_surfaces_error` in `src/tools/helpers.rs`
    asserts the underlying `Err` message contains "SLIP framing error"; the
    e2e test must confirm that message reaches the MCP tool result.
- Close + cancel.

This is the test most likely to surface a wiring gap — if the read tool
swallows the `Err` or maps it to a structured `stop_reason` instead of
`is_error`, the assertion will fail and the fix belongs in a `fix:` commit.

### 3. `native_read_delimiter_framing_decodes`

Proves delimiter RX framing over the real serial path.

- `open_pty`, `sync_boot`, `flush_both`.
- Spawn a `read` task with
  `rx_framing: { "type": "delimiter", "delimiter": "|", "delimiter_encoding": "utf8" }`,
  `timeout_ms: 3000`, `max_buffered_bytes: 512`, `encoding: "utf8"`.
- Sleep ~100ms.
- `write_cmd(&client, &id, "sendraw hex 7C706F6E677C")` — firmware emits
  `|pong|` (delimiter `|` around "pong").
- Await read result. Assert:
  - `frames[0].data == "pong"`.
  - `frames[0].frame_type == "delimiter"`.
- Close + cancel.

### 4. `native_read_length_prefixed_framing_decodes`

Proves length-prefixed RX framing (cross-chunk-safe prefix + payload
assembly) over the real serial path.

- `open_pty`, `sync_boot`, `flush_both`.
- Spawn a `read` task with
  `rx_framing: { "type": "length_prefixed", "prefix_size": 1, "endianness": "big" }`,
  `timeout_ms: 3000`, `max_buffered_bytes: 512`, `encoding: "utf8"`.
- Sleep ~100ms.
- `write_cmd(&client, &id, "sendraw hex 04706F6E67")` — firmware emits
  `\x04pong` (1-byte length prefix = 4, then "pong").
- Await read result. Assert:
  - `frames[0].data == "pong"`.
  - `frames[0].frame_type == "length_prefixed"`.
- Close + cancel.

### 5. `native_subscribe_line_framing_with_at_parser_emits_parsed_frames`

Proves subscribe emits per-frame notifications carrying `parsed` content
when an `rx_parser` is configured. All existing subscribe framing tests
use line framing WITHOUT a parser, so the parsed-frame notification shape
is unverified e2e.

- `open_pty`, `sync_boot`, `flush_both`.
- `subscribe` with `rx_framing: { "type": "line" }`,
  `rx_parser: { "type": "at_command" }`, `poll_interval_ms: 50`,
  `timeout_ms: 2000`, `encoding: "utf8"`, `max_buffered_bytes: 8192`.
- `write_cmd(&client, &id, "ping")` — firmware emits `pong\r\n`.
- Collect notifications (same loop as
  `native_subscribe_line_framing_emits_per_frame`, ~line 2077). Filter for
  notifications with `frame_index` (frame notifications). Assert at least
  one frame notification where:
  - `data["frame_type"] == "line"`.
  - `data["data"]` contains `"pong"` (the frame data).
  - `data["parsed"]["parser"] == "at_command"` (the parsed content).
  - `data["parsed"]["response_type"] == "data"` (AT data line).
- Then await the stop notification (`stop_reason` key) and break.
- Close + cancel.

This is the FIRST subscribe test to assert the `parsed` field on a frame
notification. If the parsed content is dropped or mis-shaped in the
subscribe notification path (vs the read result path), this test will
catch it.

## Out of scope

- `start_end` RX framing e2e (structurally similar to delimiter; lower
  priority).
- TX non-line framing modes e2e (firmware can't easily observe exact
  sent bytes; would need `trace on` byte-level inspection — marginal
  value).
- RX line endings `lf`/`cr`/`crlf` e2e (auto is covered; explicit endings
  are unit-tested and the `sendraw`-driven test for `lf` would mirror
  test 3 with different assertions — deferred).
- SLIP resync-after-error e2e (unit tests cover resync; the error path
  is covered by test 2).
- subscribe + SLIP / delimiter / length_prefixed e2e (subscribe's
  framing path is shared with read via `consume_frames`; read coverage
  is sufficient for the framing modes. The parser notification shape
  in test 5 is the distinct subscribe behavior worth covering).

## Relevant files and current behavior

- `tests/native_sim_validation.rs`:
  - `open_pty(client, pty_path)` (~line 116): opens with `port`/`name`/
    `baud_rate`. Use for all five tests (no framing fields needed on
    `open` — framing is set on read/subscribe).
  - `write_cmd(client, conn_id, cmd)` (~line 144): appends `\r\n` to the
    command. Use for `sendraw hex ...` — the `\r\n` terminates the
    `sendraw` command; the firmware emits the raw bytes without a
    terminator.
  - `sync_boot`, `flush_both`, `close_connection`: existing helpers.
  - `native_subscribe_line_framing_emits_per_frame` (~line 2077):
    reference for the notification-collection loop in test 5.
  - `native_read_at_parser_parses_pong` (~line 2012): reference for the
    read + AT-parser assertion shape (tests 1, 3, 4 reuse the
    frame/parsed assertion pattern but without a parser).
  - `char_framing_slip_malformed_surfaces_error` in
    `src/tools/helpers.rs` (~line 1744): reference for the error-message
    assertion in test 2 (asserts `Err` contains "SLIP framing error"; the
    e2e test asserts the same message reaches the MCP tool result).
- `firmware/AGENTS.md` command reference: `sendraw hex <hex>` emits raw
  bytes, no terminator. Confirmed by reading `firmware/src/command.c`
  `cmd_sendraw` (line 93): hex mode decodes pairs and sends raw bytes.
- `src/tools/io_ops.rs`: `read` returns `Err` from
  `read_bytes_via_session` on a framing decode error → rmcp maps to a
  tool result with `is_error: true`. Test 2 asserts this path.

## Expected test shape (test 1, sketch)

```rust
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_read_slip_decodes_frame() {
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
                "rx_framing": { "type": "slip" }
            }))).await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    // C0706F6E67C0 = END + "pong" + END
    write_cmd(&client, &id, "sendraw hex C0706F6E67C0").await;

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    assert_eq!(frames[0]["data"], json!("pong"));
    assert_eq!(frames[0]["frame_type"], json!("slip"));

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}
```

For test 2, the assertion shape depends on where rmcp surfaces the error
message in a tool result with `is_error: true`. On first run, print
`result` and locate the "SLIP framing error" string, then assert against
that path. Do NOT hardcode an assumed shape — confirm it.

For test 5, mirror the notification loop in
`native_subscribe_line_framing_emits_per_frame` (~line 2077) but add
`rx_parser` to the subscribe call and assert `data["parsed"]["parser"]
== "at_command"` on the frame notification.

## Test plan

1. `native_read_slip_decodes_frame` — `sendraw hex C0706F6E67C0`, read
   with `rx_framing: slip`, assert frame "pong", frame_type "slip".
2. `native_read_slip_malformed_escape_surfaces_framing_error` —
   `sendraw hex C0DB41C0`, read with `rx_framing: slip`, assert
   `is_error: true` with message containing "SLIP framing error" and
   "0x41".
3. `native_read_delimiter_framing_decodes` — `sendraw hex 7C706F6E67C0`
   (NOTE: drop trailing C0 — `|pong|` is `7C 70 6F 6E 67 7C`), read with
   `rx_framing: delimiter "|"`, assert frame "pong", frame_type
   "delimiter".
4. `native_read_length_prefixed_framing_decodes` —
   `sendraw hex 04706F6E67`, read with `rx_framing: length_prefixed
   prefix_size=1`, assert frame "pong", frame_type "length_prefixed".
5. `native_subscribe_line_framing_with_at_parser_emits_parsed_frames` —
   `write_cmd "ping"`, subscribe with `rx_framing: line` +
   `rx_parser: at_command`, assert frame notification carries
   `parsed.parser == "at_command"` and `parsed.response_type == "data"`.
6. All existing native_sim tests remain green.

If any test fails, root-cause before fixing. A failure in test 2 is the
most likely (error-surfacing path shape). Fix in a separate `fix:`
commit if production wiring is wrong; do not bundle into `test:`.

## Constraints and invariants

- `#[ignore]` + `#[cfg(unix)]` on all five tests.
- No `unwrap`/`expect`/`println!` in production code. Tests may use
  `.unwrap()`.
- No production code change expected. If a test fails and a fix is
  needed, separate `fix:` commit.
- Conventional commits: `test:` for the additions. No attribution
  footers.

## Verification commands

```bash
# Build firmware first if not present:
fw-build-native

cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --test native_sim_validation -- --ignored
cargo clippy --all-targets --locked -- -D warnings
```

Specifically confirm the five new tests pass:
```bash
cargo test --test native_sim_validation -- --ignored \
    native_read_slip_decodes_frame \
    native_read_slip_malformed_escape_surfaces_framing_error \
    native_read_delimiter_framing_decodes \
    native_read_length_prefixed_framing_decodes \
    native_subscribe_line_framing_with_at_parser_emits_parsed_frames
```

Full gate after:
```bash
cargo test --locked
```

CRITICAL: actually run `-- --ignored`. fmt/build/clippy do NOT execute
`#[ignore]`d tests. The Phase 4b follow-up shipped a defect by skipping
this step.

## Return instructions

When done, return:

- Tests added (names + file).
- For each test: pass/fail on first run. If a test failed, name the
  `fix:` commit (if any) and root-cause.
- For test 2: the exact path where the "SLIP framing error" message
  appears in the rmcp tool result with `is_error: true` (e.g.
  `result.content[0].text`, a structured field, etc.) — so the assertion
  shape is documented.
- For test 5: confirm the frame notification carries `parsed.parser` and
  `parsed.response_type` (the first subscribe test to assert parsed
  content on a notification).
- Gate results — INCLUDING `native_sim_validation -- --ignored` (do not
  skip). Note any failures with root cause.
- Any surprise in the sendraw-driven byte sequences or the notification
  shape.