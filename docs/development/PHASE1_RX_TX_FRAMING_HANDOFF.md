# Phase 1 Handoff — RX framing rename + TX framing

Source plan: `docs/development/FRAME_PIPELINE_PLAN.md` (Phase 1).
Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Make serial byte handling easier for agents by exposing frame building blocks
clearly. Two coordinated breaking changes:

1. Rename and flatten the RX framing option on `read` and `subscribe`.
2. Add a symmetric TX framing option on `write`.

No high-level `protocol` preset, no parser relocation, no profile defaults yet.

## Decisions already made (do not re-litigate)

- **Raw representation = omit-only.** No `{"type":"raw"}` variant in either
  framing enum. Raw = the `rx_framing` / `tx_framing` field is absent (or
  `None`). This matches current RX behavior.
- **Payload length field name = `decoded_bytes`.** `WriteResult` gains
  `decoded_bytes` (decoded payload length before framing) alongside the
  existing `bytes_written` (bytes actually sent to the UART after framing).
- **RX `line` `ending: "auto"` = current behavior.** `auto` recognizes LF and
  CRLF and strips a preceding `\r` when splitting on `\n`. New `lf` mode splits
  on `\n` only and does NOT strip a preceding `\r` (breaking vs old default).
  `cr` splits on bare `\r`. `crlf` splits on exact `\r\n`.

## In scope

### A. Rename + flatten RX framing

Old request shape on `read` and `subscribe`:

```json
{
  "framing": {
    "mode": { "type": "line" },
    "parser": { "type": "at_command" }
  }
}
```

New request shape:

```json
{
  "rx_framing": {
    "type": "line",
    "ending": "auto",
    "parser": { "type": "at_command" }
  }
}
```

Required changes:

- Rename the request field `framing` to `rx_framing` on `ReadArgs` and
  `SubscribeArgs` (`src/tools/types.rs`).
- Flatten `FramingMode` fields directly into the framing config struct so the
  `mode` wrapper goes away. Suggested approach: rename `FramingConfig` to
  `RxFramingConfig`, make the mode enum field `#[serde(flatten)]` so the
  enum's `type` discriminator and variant fields live at the top level of
  `rx_framing`. Keep `parser`, `max_frames`, `include_terminators` as
  sibling fields. Verify schemars 1.0 emits a sensible schema for the
  flattened shape (one `type` discriminator, variant-specific properties, plus
  `parser` / `max_frames` / `include_terminators`). If flatten does not
  produce a clean schema, use an untagged-flatten alternative and document it.
- Add `ending` to the `Line` variant of the RX framing mode enum. Values:
  `auto` (default), `lf`, `cr`, `crlf`. Use `#[serde(default = ...)]` so
  existing `{"type":"line"}` payloads still deserialize to `auto`.
- Update `FrameDecoder` line mode to honor `ending`:
  - `auto`: current behavior — split on `\n`; if the byte before `\n` is `\r`,
    strip it from the frame (unless `include_terminators`).
  - `lf`: split on `\n` only; do not strip a preceding `\r`.
  - `cr`: split on `\r` only.
  - `crlf`: split on exact `\r\n` only.
- Update all internal call sites and types that reference `FramingConfig` /
  `FramingMode` by name. Grep targets:
  `src/framing.rs`, `src/tools/types.rs`, `src/tools/io_ops.rs`,
  `src/tools/stream_ops.rs`, `src/tools/helpers.rs`,
  `src/tools/rx_consume.rs`, `src/lib.rs`.
- Update `server.rs` tool descriptions for `read` and `subscribe` to say
  `rx_framing` instead of `framing`.
- Update `ReadResult` and `FrameResult` doc comments that mention `framing`
  to say `rx_framing` where they describe the request option.

### B. Add TX framing

New `write` request shape (additive — `tx_framing` is optional):

```json
{
  "connection_id": "abc",
  "data": "AT+CGMI",
  "encoding": "utf8",
  "tx_framing": { "type": "line", "ending": "cr" }
}
```

Required changes:

- Add `tx_framing: Option<TxFramingConfig>` to `WriteArgs`
  (`src/tools/types.rs`), `#[serde(default)]`.
- Define `TxFramingConfig` and `TxFramingMode` in `src/framing.rs`. Mirror the
  RX modes but directionally appropriate:
  - `line` with `ending`: `lf` => append `\n`, `cr` => append `\r`,
    `crlf` => append `\r\n`. No `auto` for TX — agents must be explicit.
    Reject `auto` at construction with a clear error.
  - `delimiter` with `delimiter` + `delimiter_encoding`: append decoded
    delimiter bytes after payload.
  - `length_prefixed` with `prefix_size` (1/2/4) + `endianness`: write prefix
    encoding the payload length, then payload. No `initial_offset` for TX.
  - `start_end` with `start` + `end` + `marker_encoding`: write start marker,
    payload, end marker.
- Add a `TxFramingMode::encode(payload: &[u8]) -> Result<Vec<u8>, String>`
  style method (or a free function) that turns decoded payload bytes into
  framed bytes. Validate inputs at construction (non-empty delimiter/markers,
  prefix_size in {1,2,4}, valid encoding) and surface sync errors from
  `io_ops::write` to the caller as a tool `is_error` result (not a protocol
  `McpError`).
- Wire `io_ops::write` to apply TX framing after `codec::decode` and before
  `TxSession::write`. Compute `decoded_bytes` = decoded payload length;
  `bytes_written` = framed bytes length actually sent.
- Update `WriteResult` to add `decoded_bytes: usize` (annotated with
  `uint_schema`). Keep `bytes_written`. When `tx_framing` is absent,
  `decoded_bytes == bytes_written`.
- Update `server.rs` `write` tool description to mention `tx_framing`.
- Add `TxFramingConfig` / `TxFramingMode` / `WriteArgs` (newly carries an
  unsigned field `decoded_bytes` on the result) to the `check_schema!` list in
  `src/serial.rs` `mod schema` if they carry unsigned fields. `WriteResult`
  is already in the list — extend its test to cover the new field.

### C. Round-trip helper

Add a shared round-trip path so TX-encoded bytes fed into the matching RX
framing mode return the original payload. This is test-facing, not a tool:
expose `TxFramingMode::encode` and `FrameDecoder` (RX) so a test can do
`rx_decoder.push(&tx_mode.encode(payload))` and assert the first frame equals
`payload`. No new public API beyond what is needed for the tests.

## Out of scope

- SLIP, COBS.
- Protocol presets (`at_command`, `json_lines`, `slip_json`).
- Moving `parser` out of `rx_framing` into a separate `rx_parser` field.
- AT-command TX builder, JSON serialization helper for TX.
- Profile defaults for framing.
- Adaptive bare-CR `auto` mode (deferred to Phase 2).
- Escaping / byte-stuffing in `start_end`.
- Any` ending` value other than `auto` / `lf` / `cr` / `crlf` for RX line.
- TX `line` `auto` mode.

## Relevant files and current behavior

- `src/framing.rs` — `FramingConfig`, `FramingMode`, `FrameDecoder`,
  `Frame`, `ParsedFrame`, `ParserConfig`, `ParserType`, `Endianness`.
  `FrameDecoder::new(&FramingConfig)` builds the stateful RX decoder. Line
  mode currently hard-codes LF-split + preceding-CR strip.
- `src/tools/types.rs` — `ReadArgs.framing`, `SubscribeArgs.framing`,
  `WriteArgs` (no framing yet), `WriteResult`, `ReadResult`, `FrameResult`.
- `src/tools/io_ops.rs` — `write()` decodes payload, looks up connection,
  sends via `TxSession::write(Arc<[u8]>)`. `read()` passes `args.framing`
  into `read_bytes_via_session`.
- `src/tools/stream_ops.rs` — `subscribe` passes `args.framing` into the
  RX stream loop. Subscribe degrades bad framing configs to raw mode with a
  `warn!` (read propagates the error).
- `src/tools/helpers.rs` — `read_bytes_via_session(..., framing: Option<FramingConfig>)`
  owns the `FrameDecoder`. `ReadFrameSink` collects frames. `ReadOutcome`
  carries `frames: Vec<Frame>`. Match metadata uses `accumulated.len()` for
  `bytes_returned` (read) — preserve this.
- `src/tools/rx_consume.rs` — shared `consume_frames` + `RxFrameSink` trait.
  Tests at line ~140 construct `FramingConfig { mode: FramingMode::Line, .. }`.
- `src/lib.rs` — `pub mod framing;`.
- `src/serial.rs` `mod schema` — `check_schema!` regression list. Add new
  JsonSchema structs with unsigned fields here.
- `src/tools/mod.rs` — `all_tool_attrs()` lists 22 tools; `write` is already
  present. No new tool, but the `write` schema will change shape.
- `src/server.rs` — `#[tool]` descriptions for `read`, `subscribe`, `write`.
- `tests/proptest.rs` — `read_args_roundtrip` and `subscribe_args_roundtrip`
  construct `ReadArgs` / `SubscribeArgs` with `framing: None` by name.
  `framing_config_roundtrip_all_modes` constructs `FramingConfig { mode, parser,
  max_frames, include_terminators }` directly.
- `tests/serial_pty.rs` — `pty_subscribe_framing_*` send JSON with
  `"framing": { "mode": { "type": "line" } }`.
- `tests/native_sim_validation.rs` — many tests send `"framing": {...}` JSON
  to `read` / `subscribe`.
- `tests/config_schema_validation.rs` — validates tool schemas; not directly
  framing-aware but will catch schema regressions.
- `firmware/AGENTS.md` mentions `framing on/off` test commands — those are
  firmware-side, unrelated to the MCP request field rename. Leave firmware
  alone.

## Expected API / UX shape

After Phase 1:

- `read` and `subscribe` accept `rx_framing` (not `framing`). Old `framing`
  field is rejected (serde unknown field). Modes: `line` (with `ending`),
  `delimiter`, `length_prefixed`, `start_end`. `parser` nested inside
  `rx_framing`.
- `write` accepts optional `tx_framing`. Modes: `line` (with `ending` =
  `lf`/`cr`/`crlf`), `delimiter`, `length_prefixed`, `start_end`. No `parser`.
- `WriteResult` has `decoded_bytes` (payload length) and `bytes_written`
  (framed bytes sent). When `tx_framing` is absent, both equal the decoded
  payload length.
- All schemas expose `rx_framing` / `tx_framing`; none expose `framing`.

## Test plan

Add and update tests to cover:

1. **Default/raw TX preserves exact-byte behavior.**
   `tests/native_sim_validation.rs` or `tests/serial_pty.rs`: `write` without
   `tx_framing` writes exact decoded bytes; `decoded_bytes == bytes_written`.

2. **TX line LF/CR/CRLF writes exact bytes.**
   Unit test in `src/framing.rs`: encode `"AT+CGMI"` with each ending, assert
   appended bytes are `\n` / `\r` / `\r\n` respectively.

3. **TX delimiter appends exact decoded delimiter bytes.**
   Unit test: utf8 and hex/base64 delimiter encodings; empty delimiter
   rejected.

4. **TX length prefix writes correct prefix size and endianness.**
   Unit test: prefix_size 1/2/4, big and little endian. Reject prefix_size 3.

5. **TX start/end writes exact markers around payload.**
   Unit test: `include_markers` is implicit true for TX (markers always
   written); empty markers rejected.

6. **RX `line auto` recognizes LF and CRLF.** Already covered by
   `line_decoder_single_line` and `line_decoder_crlf` — keep passing with
   `ending: auto` default.

7. **RX `line lf` does not strip preceding CR.** New test: push
   `"hello\r\n"` with `ending: lf`, assert frame data == `b"hello\r"`.

8. **RX `line cr` splits on bare CR.** New test: push `"a\rb\r"`, assert
   frames `["a", "b"]`.

9. **RX `line crlf` waits for exact CRLF.** New test: push `"a\r"` then
   `"b\n"`, assert frame `b"a\r\n"` stripped to `b"a"` (or kept with
   `include_terminators`). Push `"a\rb\n"` with `crlf` — assert no split on
   the bare `\r`.

10. **RX delimiter, length-prefixed, start_end still behave under flattened
    `rx_framing` shape.** Update existing `framing.rs` tests to construct
    the new `RxFramingConfig` shape (flattened mode fields). Behavior
    unchanged.

11. **Round-trip tests.** For `delimiter`, `length_prefixed`, `start_end`,
    and `line` with `lf`/`cr`/`crlf` endings: TX-encode a payload, feed into a
    matching RX `FrameDecoder`, assert the first frame equals the original
    payload. (`line auto` round-trip is not required — TX has no `auto`.)

12. **Tool schemas expose `rx_framing` and `tx_framing`, not `framing`.**
    Add a schema assertion in `src/tools/mod.rs` tests (or a new test) that
    generates the `write`, `read`, `subscribe` tool input schemas and asserts
    `rx_framing` / `tx_framing` are present and `framing` is absent.

13. **Schema regression tests still reject non-standard unsigned integer
    formats.** Add `TxFramingConfig`, `TxFramingMode` (if it has unsigned
    fields), and the updated `WriteResult` to the `check_schema!` list in
    `src/serial.rs` `mod schema`. `WriteResult` is already listed — verify
    the new `decoded_bytes` field is annotated.

14. **Request shape migration.** Update `tests/serial_pty.rs` and
    `tests/native_sim_validation.rs` JSON payloads from `"framing": {...}`
    to `"rx_framing": {...}` with flattened mode fields. All framing tests
    must pass against the new shape.

15. **proptest roundtrip.** Update `tests/proptest.rs`:
    `read_args_roundtrip` and `subscribe_args_roundtrip` to use the renamed
    `rx_framing` field. Replace `framing_config_roundtrip_all_modes` with
    `rx_framing_config_roundtrip_all_modes` constructing the new shape
    (including `ending` for `line`).

## Constraints and invariants (from repo docs)

- **No `unwrap`/`expect`/`println!`/`todo!()`/`unimplemented!()`** in
  production code.
- **Tool failures become MCP tool results with `is_error: true`**, not
  protocol-level `McpError`. Bad `tx_framing` config must surface as a tool
  error, not a transport error.
- **Every `uN` / `Option<uN>` field on a `JsonSchema`-deriving struct MUST
  use `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]`**
  (or `option_uint_schema`). Extend the `check_schema!` list in
  `src/serial.rs` `mod schema` for any new struct with unsigned fields.
- **`read` and `subscribe` share stop-reason vocabulary via
  `RxStopController`, but their RX loops are NOT interchangeable.** Raw-path
  semantics differ by design: `read` is bounded and scans
  `chunk[..take]` up to `max_bytes`; `subscribe` scans full chunks across the
  whole subscription lifetime. Do not merge the raw paths.
- **Framing semantics differ by design:** `read` keeps later frames decoded
  from the same chunk after the first matching frame; `subscribe` stops on
  the matching frame and does not emit later frames from that chunk. Preserve
  these differences.
- **Match metadata differs:** `read` uses `accumulated.len()` for
  `bytes_returned`; `subscribe` uses cumulative emitted bytes
  (`total_returned`). Preserve.
- **Subscribe degrades bad framing configs to raw mode with a `warn!`**;
  `read` propagates the error. Keep this asymmetry for `rx_framing`.
- **`open` must enforce allowlist checks before `ConnectionManager::open()`.**
  Not touched by this phase.
- **Open/close changes must notify resource subscribers.** Not touched.
- **Conventional commits:** `feat:`, `fix:`, `refactor:`, `test:`,
  `docs:`. No attribution footers.

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
- Final API shape for `rx_framing` and `tx_framing` (include the Rust struct
  names and the JSON request shape).
- New tests added and which existing tests were updated.
- Gate command results (`fmt`, `build`, `test`, `clippy`). Note any failures
  with root cause.
- Any schemars flatten surprise or workaround.
- Any scope decision you had to make that is not covered above.