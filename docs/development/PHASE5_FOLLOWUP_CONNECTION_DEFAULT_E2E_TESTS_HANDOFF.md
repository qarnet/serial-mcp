# Phase 5 Follow-up Handoff — Connection-default e2e tests (layers 3-4)

Source: Phase 5 review. The new four-layer precedence (explicit > call
protocol > connection default > connection protocol preset) is in place
and clippy-clean, but only layers 1-2 are covered by the existing preset
e2e tests (which set `protocol` at call time). Layers 3-4 — the actual new
feature — are completely unverified at the tool level. Phase 4b follow-up
shipped a real defect by skipping `--ignored` verification; this phase
deferred the e2e tests outright. Close the gap now.

Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Add e2e tests proving that framing/parser defaults stored on a connection
(via `open` with the new optional framing fields) actually drive
`write`/`read`/`subscribe` when the call omits those fields, and that an
explicit call field beats the connection default. Use plain `open` with the
optional framing fields — no profile-file machinery needed (the
`open_profile` path is structurally identical for defaults seeding; plain
`open` is simpler to drive in a test).

## In scope

Add three native_sim e2e tests to `tests/native_sim_validation.rs`. All
`#[ignore = "requires native_sim firmware binary"]`, `#[cfg(unix)]`.

### A. `native_open_protocol_default_drives_write_and_read`

Proves layer 4 (connection protocol preset) drives both directions when
call omits all framing/parser/protocol fields.

1. Spawn firmware, start server, connect client.
2. `open` the PTY with `protocol: { "type": "at_command" }` AND the standard
   fields (`port`, `name`, `baud_rate`). Do NOT pass `tx_framing`/
   `rx_framing`/`rx_parser` on the `open` — only `protocol`. The connection
   stores `protocol = AtCommand`; the three primitive defaults stay `None`.
3. Spawn a `read` task with ONLY `connection_id`, `timeout_ms`, and
   `encoding`. Do NOT pass `rx_framing`/`rx_parser`/`protocol` on the read.
4. Sleep ~100ms; `write` with ONLY `connection_id` and `data: "ping"`. Do
   NOT pass `tx_framing`/`protocol` on the write. Use a direct `call_tool`
   (NOT `write_cmd`, which appends `\r\n`; NOT `write_preset`, which sets
   `protocol`). The connection's `protocol` default must expand to append
   `\r` on TX and line-auto + AT parser on RX.
5. Await the read result. Assert:
   - `is_error` not `true`.
   - `frames` non-empty.
   - Frame `parsed.parser == "at_command"`, `response_type == "data"`, with
     a `fields` entry containing `"pong"`.
6. Close + cancel.

This proves layer 4 end-to-end: connection-stored `protocol` preset fills
gaps in BOTH `write` (tx CR) and `read` (rx line auto + AT parser), with no
framing fields set on the individual calls. If this passes, the core Phase
5 feature works.

### B. `native_explicit_rx_framing_beats_connection_default`

Proves layer 1 beats layer 3/4 — explicit call `rx_framing` wins over a
connection default.

1. Spawn firmware, start server, connect client.
2. `open` with `rx_framing: { "type": "line", "ending": "lf" }` AND
   `protocol: { "type": "at_command" }`. The connection stores both: an
   explicit `rx_framing` default (lf) and a `protocol` default (which would
   expand to line auto). Layer 3 (connection rx_framing default) should win
   over layer 4 (connection protocol preset) for the rx_framing field —
   `protocol` only fills gaps left by `None` defaults.
3. Spawn a `read` task with ONLY `connection_id`/`timeout_ms`/`encoding`
   (no call-time framing fields).
4. `write_cmd "ping"` (appends `\r\n` — firmware responds `pong\r\n`).
5. Await read result. Assert:
   - A frame exists.
   - Frame `data` ends with `\r` — proving the connection's `rx_framing: lf`
     default (layer 3) was applied, NOT the `protocol` preset's `auto`
     (layer 4). `lf` mode retains the preceding `\r`; `auto` would strip it.
   - Frame `parsed.parser == "at_command"` — proving the connection's
     `protocol` default still filled the `rx_parser` gap (layer 4), since
     no `rx_parser` default was stored and the call set none.
6. Close + cancel.

This proves the four-layer precedence for `rx_framing` specifically:
connection default (lf) beats connection protocol preset (auto), and the
protocol preset still fills the `rx_parser` gap. The observable signal is
the retained `\r` in the frame data.

### C. `native_save_profile_snapshots_framing_defaults`

Proves save_profile round-trips framing defaults (handoff test plan item 8).

1. Spawn firmware, start server, connect client.
2. `open` with `protocol: { "type": "at_command" }`.
3. `save_profile` to a new name (e.g. "preset-snapshot").
4. `list_profiles`. Find the new profile in the result. Assert its
   `defaults.protocol == { "type": "at_command" }` (or the JSON-equivalent
   shape — match whatever `list_profiles` returns for the protocol field).
   Assert `tx_framing`/`rx_framing`/`rx_parser` defaults are `null`/absent
   (the connection stored only `protocol`, not the primitives).
5. `delete_profile` the new name (cleanup so the test is repeatable).
6. Close + cancel.

This proves save_profile snapshots the connection-stored `protocol` default
and that `list_profiles` exposes it. If `list_profiles` does not return the
framing default fields, root-cause — either the snapshot is wrong or the
list serialization drops them. Do not paper over.

## Out of scope

- PTY-level tests (native_sim covers it; the firmware echoes AT-like pong).
- subscribe connection-default tests. subscribe's resolution is structurally
  identical to read (same match-arm shape, verified in the diff). read's e2e
  coverage is sufficient; adding subscribe would duplicate without new
  signal. If you want one, mirror test A but collect frame notifications —
  optional.
- `open_profile`-from-file e2e tests. Plain `open` with the optional framing
  fields exercises the same `ConnectionConfig` seeding path. The
  `open_profile` code reads `profile.defaults.*` into the same
  `ConnectionConfig` fields — structurally identical. A file-based test adds
  filesystem plumbing without new wiring coverage.
- get_status framing-default exposure (deferred in Phase 5; remains
  deferred here).

## Relevant files and current behavior

- `tests/native_sim_validation.rs`:
  - `open_pty(client, pty_path)` (~line 116): opens with `port`/`name`/
    `baud_rate`. For these tests, call `open` directly with extra fields
    (`protocol`, `rx_framing`) — do NOT use `open_pty` since it doesn't
    accept the new framing fields. Inline the `call_tool("open", ...)`.
  - `write_cmd` (~line 144): appends `\r\n`. Use for test B (firmware just
    needs to respond). Do NOT use for test A (the preset must be the sole
    terminator source).
  - `write_preset` (~line 182): sets `protocol: at_command` on the write
    call. Do NOT use for test A — test A must have NO `protocol` on the
    write call (the connection default, not a call-time preset, drives TX).
    Inline a direct `call_tool("write", json!({"connection_id": id,
    "data": "ping"}))` for test A.
  - `NativeSimFirmware::spawn()`, `TestServer::start()`, `connect_client`,
    `sync_boot`, `flush_both`, `close_connection`: existing setup helpers.
  - `save_profile`/`list_profiles`/`delete_profile` tools: call via
    `call_tool("save_profile", ...)` etc. — see how other tools are invoked.
- `src/tools/types.rs`: `OpenArgs` now has `protocol`/`tx_framing`/
  `rx_framing`/`rx_parser` optional fields — these are what test A/B set on
  the `open` call.
- `src/tools/port_ops.rs`: `open_profile` seeds from `profile.defaults.*`;
  `save_profile` snapshots `conn.*_default()`. Test C exercises the save
  path.
- `src/tools/io_ops.rs`/`stream_ops.rs`: the four-layer resolution blocks
  under test.

## Expected test shape (test A, sketch)

```rust
#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
#[cfg(unix)]
async fn native_open_protocol_default_drives_write_and_read() {
    let fw = NativeSimFirmware::spawn().await.expect("spawn zephyr.exe");
    let pty_path = fw.pty_path().to_string();

    let server = TestServer::start().await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    // Open with ONLY the protocol default — no explicit framing fields.
    let open_result = client.peer().call_tool(tool_request("open", json!({
        "port": pty_path,
        "name": NAME,
        "baud_rate": BAUD_RATE,
        "protocol": { "type": "at_command" }
    }))).await.expect("open");
    assert_ne!(open_result.is_error, Some(true), "open failed: {open_result:?}");
    let id = open_result.structured_content.expect("structured")
        ["connection_id"].as_str().expect("connection_id").to_string();
    sync_boot(&client, &id).await;
    flush_both(&client, &id).await;

    let read_handle = {
        let peer = client.peer().clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            // NO framing/parser/protocol fields — rely on connection default.
            peer.call_tool(tool_request("read", json!({
                "connection_id": id2,
                "timeout_ms": 3000,
                "max_buffered_bytes": 512,
                "encoding": "utf8"
            }))).await
        })
    };

    tokio::time::sleep(Duration::from_millis(100)).await;
    // NO tx_framing/protocol on the write — connection default drives CR.
    client.peer().call_tool(tool_request("write", json!({
        "connection_id": id,
        "data": "ping"
    }))).await.expect("write");

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

For test B, the `open` adds `"rx_framing": { "type": "line", "ending": "lf" }`
alongside `"protocol"`, and the read asserts `data.ends_with('\r')`. For
test C, use `call_tool("save_profile", ...)`, `call_tool("list_profiles",
...)`, parse the profile list, assert `defaults.protocol`, then
`call_tool("delete_profile", ...)`.

## Test plan

1. `native_open_protocol_default_drives_write_and_read` — connection
   `protocol` preset drives TX CR + RX auto/AT; `pong` parsed `at_command`.
2. `native_explicit_rx_framing_beats_connection_default` — connection
   `rx_framing: lf` default (layer 3) beats connection `protocol: auto`
   preset (layer 4); frame data retains `\r`; `rx_parser` still filled by
   the `protocol` preset (layer 4 gap-fill).
3. `native_save_profile_snapshots_framing_defaults` — save_profile captures
   `protocol`; list_profiles exposes it; delete_profile cleans up.
4. All existing native_sim tests remain green.

If any test fails, root-cause before fixing. A failure likely means the
four-layer resolution has a wiring bug (field move, clone, or the
connection-default branch never reached). Fix in a separate `fix:` commit;
do not bundle into `test:`. Note root cause in the return summary.

## Constraints and invariants

- `#[ignore]` + `#[cfg(unix)]` on all three tests (match existing native_sim
  convention).
- No `unwrap`/`expect`/`println!` in production code. Tests may use
  `.unwrap()`.
- No production code change expected. If a test fails and a fix is needed,
  separate `fix:` commit.
- Conventional commits: `test:` for the additions. No attribution footers.

## Verification commands

```bash
# Build firmware first if not present:
fw-build-native

cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --test native_sim_validation -- --ignored
cargo clippy --all-targets --locked -- -D warnings
```

Specifically confirm the three new tests pass:
```bash
cargo test --test native_sim_validation -- --ignored \
    native_open_protocol_default_drives_write_and_read \
    native_explicit_rx_framing_beats_connection_default \
    native_save_profile_snapshots_framing_defaults
```

Full gate after:
```bash
cargo test --locked
```

CRITICAL: actually run `-- --ignored`. The Phase 4b follow-up shipped a
defect by skipping this step. fmt/build/clippy do NOT execute
`#[ignore]`d tests.

## Return instructions

When done, return:

- Tests added (names + file).
- For each test: pass/fail on first run. If a test failed and you fixed
  wiring, name the `fix:` commit and root-cause (which layer/branch broke).
- Confirm test A's `pong` frame parses as `at_command` data with the
  connection `protocol` default driving both TX and RX (no call-time
  framing fields).
- Confirm test B's frame `data` ends in `\r` (proving `lf` default beat
  `auto` preset) and `parsed.parser == at_command` (proving the protocol
  preset still filled the parser gap).
- Confirm test C's `list_profiles` exposes `defaults.protocol` and
  `delete_profile` cleaned up.
- Gate results — INCLUDING `native_sim_validation -- --ignored` (do not
  skip). Note any failures with root cause.
- Any surprise in the four-layer resolution when driven solely from
  connection defaults (e.g. did the `connection.protocol_default()` accessor
  return the right value, or did `build_config` drop it on a path?).