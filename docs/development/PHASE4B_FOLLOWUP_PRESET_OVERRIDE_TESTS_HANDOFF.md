# Phase 4b Follow-up Handoff — Preset override-behavior + e2e tests

Source: Phase 4b review flagged that only expansion-function unit tests + a
schema assertion were added. The override-resolution wiring in
`io_ops::write`/`read` + `stream_ops::subscribe` has no test proving the
preset actually drives TX framing, that an explicit field overrides, or that
a full read↔preset round-trip parses AT frames end-to-end. This closes that
gap. Same shape as the Phase 2 and Phase 3 follow-ups that caught real bugs.

Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Add integration/e2e tests proving the `protocol: at_command` preset actually
expands and that explicit fields override it — at the tool level, not just the
expansion-function level. No production code change expected; if a test
surfaces a wiring bug, fix it in a separate `fix:` commit (do NOT bundle into
`test:`).

## In scope

### A. Override-behavior test for `write` (native_sim)

Add to `tests/native_sim_validation.rs`:

- `native_write_protocol_preset_appends_cr`

Shape (adapt from `write_cmd`/`write_raw` + the existing AT parser test at
~line 1970):

1. Open a connection (`open` the native_sim firmware port — copy the
   setup pattern from the existing AT parser test).
2. Spawn a `read` task with `protocol: { "type": "at_command" }` and NO
   explicit `rx_framing`/`rx_parser`. Use `timeout_ms: 3000`,
   `max_buffered_bytes: 512`, `encoding: "utf8"`.
3. Sleep ~100ms so the read registers, then send `ping` via `write` with
   `protocol: { "type": "at_command" }` and `data: "ping"` — critically,
   do NOT manually append `\r` (the preset must append it). Do NOT use the
   `write_cmd` helper (it appends `\r\n`); use a direct `call_tool("write",
   ...)` or a new helper that sends exactly the `data` field as given.
4. Await the read result. Assert:
   - `is_error` is not `true`.
   - `frames` is non-empty.
   - At least one frame's `parsed` is `{"parser": "at_command",
     "response_type": "data", ...}` with a `fields` entry containing
     `"pong"` (the firmware echoes `pong\r\n` per `firmware/AGENTS.md`).
5. Close the connection, cancel the client.

This proves the preset's `tx_framing: line CR` actually appended `\r`
(without it the firmware would not see a complete command and would not
respond), AND the preset's `rx_framing: line auto` + `rx_parser: at_command`
expanded and parsed the response.

### B. Explicit-override test for `write` (native_sim)

Add to `tests/native_sim_validation.rs`:

- `native_write_explicit_tx_framing_overrides_protocol`

Same as A, but the `write` call sets BOTH `protocol: { "type":
"at_command" }` AND `tx_framing: { "type": "line", "ending": "crlf" }`.
Firmware accepts `\r` OR `\n` termination (firmware/AGENTS.md line 162), so
the CRLF override still triggers a `pong` response. The read side stays
preset-driven (no explicit `rx_framing`/`rx_parser`).

Assert the same outcome as A: a `pong` frame parsed as `at_command` data.
This proves the explicit `tx_framing` won out (CRLF, not the preset's CR) and
the command still landed. The distinguishing byte (`\r` vs `\r\n`) is not
directly observable through the MCP read result, so the assertion is
behavioral: the firmware responded, proving the explicit framing was applied
correctly (and not broken by the preset).

### C. Explicit-override test for `read` (native_sim)

Add to `tests/native_sim_validation.rs`:

- `native_read_explicit_rx_framing_overrides_protocol`

1. Open a connection.
2. Spawn a `read` task with `protocol: { "type": "at_command" }` AND an
   explicit `rx_framing: { "type": "line", "ending": "lf" }` (overrides the
   preset's `auto`). Keep `rx_parser` unset (preset fills it).
3. Send `ping\r` via `write` (manual `\r`, or use `write_cmd` which appends
   `\r\n` — either triggers the firmware).
4. Firmware responds `pong\r\n`. With `rx_framing: line lf`, the frame data
   is `pong\r` (LF split, `\r` NOT stripped — Phase 1 `lf` semantics).
5. Assert:
   - A frame exists.
   - The frame is parsed (`parsed.parser == "at_command"`) — the preset's
     `rx_parser` filled in despite the explicit `rx_framing` override.
   - The frame `data` field ends in `\r` (the `lf`-mode retention of the
     preceding CR). This distinguishes the override from the preset's `auto`
     (which would strip the `\r`).
6. Close + cancel.

This proves explicit `rx_framing` overrode the preset's `auto` (visible in
the retained `\r`) while the preset's `rx_parser` still applied.

### D. Schema regression — no change

The `protocol_field_present_in_schemas` test already covers schema presence.
Do NOT add a second schema test; the integration tests cover behavior.

## Out of scope

- PTY-level preset tests. native_sim covers the real software-serial path and
  the AT firmware echo; PTY would duplicate without adding signal. (If you
  prefer a PTY test instead, that's acceptable — but native_sim is
  recommended since the firmware already echoes AT-like `pong`.)
- subscribe preset tests. The override resolution in `stream_ops::subscribe`
  is structurally identical to `read` (same match arms, same preset
  functions). subscribe's distinct behavior (per-frame notifications, stop at
  match) is already covered for framing; adding preset-specific subscribe
  coverage is marginal. If you want one, mirror test A but collect frame
  notifications instead of a read result — but this is optional.
- Any new preset. Only `at_command`.
- Any change to `ProtocolPreset`, the expansion functions, or the override
  match arms — unless a test fails, in which case fix in a separate `fix:`
  commit.

## Relevant files and current behavior

- `tests/native_sim_validation.rs`:
  - `write_cmd(client, conn_id, cmd)` (~line 144): appends `\r\n` to `cmd`
    and calls `write`. Do NOT use this for the preset test — it would hide
    whether the preset appended the CR.
  - `write_raw(client, conn_id, data)` (~line 167): sends `data` verbatim.
    Use this (or a direct `call_tool("write", ...)`) for the preset test so
    the preset is the sole source of the terminator.
  - Existing AT parser test (~line 1960, `native_read_at_parser_parses_pong`):
    reference shape for spawning a read, sleeping, sending `ping`, and
    asserting the `pong` frame's `parsed.parser == "at_command"` and
    `response_type == "data"` with a `fields` entry containing `pong`.
  - `read_str`/`read` helpers: see how a read result is fetched and
    `structured_content` parsed.
- `firmware/AGENTS.md` line 162: "Commands terminate on `\r` or `\n`" —
  confirms the preset's CR-only terminator triggers a response, and the
  explicit CRLF override in test B also works.
- `firmware/AGENTS.md` line 176: `ping` → `pong\r\n`. The response uses
  CRLF, so RX `line auto` strips the `\r` (frame data `pong`), and RX
  `line lf` retains it (frame data `pong\r`). This is the observable
  difference test C asserts on.
- `src/tools/io_ops.rs`: `write` resolves `tx_framing` from
  `args.protocol` (~line 33); `read` resolves `rx_framing` + `rx_parser`
  (~line 118). These are the wiring points under test.
- `src/tools/stream_ops.rs`: `subscribe` resolves the same way (~line 148).
  Not directly tested here (out of scope), but covered structurally.

## Expected test shape (test A, native_sim)

```rust
#[tokio::test]
async fn native_write_protocol_preset_appends_cr() {
    let fw = ensure_firmware();
    let (server, client) = startup().await;
    let id = open_conn(&client).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id2,
                    "timeout_ms": 3000,
                    "max_buffered_bytes": 512,
                    "encoding": "utf8",
                    "protocol": { "type": "at_command" }
                }),
            )).await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    // Send "ping" with the preset — NO manual terminator.
    write_raw(&client, &id, "ping").await;
    // Add protocol to the write call so the preset appends CR.
    // NOTE: write_raw does not accept extra fields; call tool directly:
    // (see below)

    let result = read_handle.await.unwrap().expect("read task");
    assert_ne!(result.is_error, Some(true), "{result:?}");
    let s = result.structured_content.expect("structured");
    let frames = s["frames"].as_array().expect("frames array");
    assert!(!frames.is_empty(), "expected at least one frame");
    let f0 = &frames[0];
    let parsed = f0["parsed"].as_object().expect("parsed object");
    assert_eq!(parsed["parser"], json!("at_command"), "parser: {parsed:?}");
    assert_eq!(parsed["response_type"], json!("data"));
    let fields = parsed["fields"].as_array().expect("fields array");
    assert!(
        fields.iter().any(|f| f.as_str().unwrap().contains("pong")),
        "fields should contain pong: {fields:?}"
    );

    close_connection(&client, &id).await;
    client.cancel().await.ok();
    drop(fw);
}
```

IMPORTANT: the preset must be applied on the WRITE call, not just the read.
`write_raw` as defined does not pass `protocol`. Either:
- Add a `write_with_protocol` helper that sends
  `json!({ "connection_id": id, "data": cmd, "protocol": { "type": "at_command" } })`,
  OR
- Inline the `call_tool("write", ...)` directly in the test with `protocol`
  set.

The read call ALSO uses `protocol` (so its `rx_framing`/`rx_parser` fill in).
This tests the full preset round-trip: TX CR-terminated by the preset,
firmware echoes `pong\r\n`, RX line-auto split + AT parse by the preset.

For test B, the write call adds `"tx_framing": { "type": "line", "ending":
"crlf" }` alongside `"protocol"`. For test C, the read call adds
`"rx_framing": { "type": "line", "ending": "lf" }` alongside `"protocol"`,
and the write uses `write_cmd` (which appends `\r\n`) so the firmware
definitely responds.

## Test plan

1. `native_write_protocol_preset_appends_cr` — preset drives TX CR + RX
   auto/AT; `pong` frame parsed as `at_command` data.
2. `native_write_explicit_tx_framing_overrides_protocol` — explicit
   `tx_framing: Crlf` wins; firmware still responds (behavioral proof the
   override landed).
3. `native_read_explicit_rx_framing_overrides_protocol` — explicit
   `rx_framing: line lf` wins (frame data ends `\r`), preset `rx_parser`
   still applies (frame parsed `at_command`).
4. All existing native_sim tests remain green.

If any of 1-3 fails, root-cause before "fixing" the test. The override logic
is by-inspection correct; a failure likely means a wiring detail (field move,
clone vs take) silently broke the preset. Fix in a separate `fix:` commit
with a clear message, and note the root cause in the return summary.

## Constraints and invariants

- `#![cfg(...)]` gating: native_sim tests carry `#[ignore = "requires
  native_sim firmware binary"]` and run via `--ignored`. Match the existing
  ignored-test convention.
- No `unwrap`/`expect`/`println!` in production code. Tests may use
  `.unwrap()`.
- No production code change unless a test fails. If a fix is needed, do not
  bundle it into a `test:` commit.
- Conventional commits: `test:` for the additions. Separate `fix:` if a
  wiring bug surfaces. No attribution footers.

## Verification commands

```bash
# Build firmware first if not present:
fw-build-native

cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --test native_sim_validation -- --ignored
cargo clippy --all-targets --locked -- -D warnings
```

Full gate after:

```bash
cargo test --locked
```

## Return instructions

When done, return:

- Tests added (names + file).
- For each test: pass/fail on first run. If a test failed and you fixed
  wiring, name the `fix:` commit and root-cause (which field/move/clone
  broke the override).
- Confirm the `pong` frame parses as `at_command` `data` in test A.
- Confirm test C's frame `data` ends in `\r` (proving the `lf` override beat
  the preset's `auto`).
- Gate results (`fmt`, `build`, `native_sim_validation --ignored`, `clippy`,
  `cargo test --locked` if run).
- Any surprise in the override resolution (e.g. did `args.protocol` move
  incorrectly, or did `#[serde(default)]` on `protocol` interact with an
  absent field in the write call?).