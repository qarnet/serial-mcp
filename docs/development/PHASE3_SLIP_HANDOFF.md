# Phase 3 Handoff — SLIP framing (TX + RX)

Source plan: `docs/development/FRAME_PIPELINE_PLAN.md` (Phase 3).
Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Add SLIP (RFC 1055) as a symmetric framing mode on both `write` (TX) and
`read` / `subscribe` (RX). SLIP byte-stuffs payloads between END markers so
binary payloads can be transported over a byte stream without ambiguity. This
is the first framing mode with a runtime decode error path (malformed escape
sequence), so the decoder API gains a `Result` return and a new
`framing_error` stop reason.

## Decisions already made (do not re-litigate)

- **SLIP variant = RFC 1055 classic.** Constants:
  - `END` = `0xC0` — frame delimiter.
  - `ESC` = `0xDB` — escape introducer.
  - `ESC_END` = `0xDC` — escaped representation of `END` inside payload.
  - `ESC_ESC` = `0xDD` — escaped representation of `ESC` inside payload.
  - A frame is the decoded bytes between two END markers. Leading END is
    optional and skipped on RX. TX wraps every frame as
    `END [stuffed payload] END`.
- **RX stream sync = skip to first END.** Bytes arriving before the first END
  marker at stream start are discarded. Standard SLIP receiver behavior;
  lets a receiver join mid-stream.
- **Malformed escape = error, drop frame.** `ESC` followed by a byte other
  than `ESC_END` (`0xDC`) or `ESC_ESC` (`0xDD`) is a protocol violation.
  Surface as a `framing_error` (see below). The partial frame in progress is
  discarded; the decoder resyncs by skipping to the next END.
- **Truncated escape = hold pending.** `ESC` as the last byte of a chunk with
  no following byte is held in the decoder buffer; the next chunk resolves it.
  `flush_partial` at stop emits the pending `ESC` as a raw partial frame
  (best-effort, no escape decode).
- **Error surfacing API = `push()` returns `Result`.** Change
  `FrameDecoder::push(chunk: &[u8])` from `Vec<Frame>` to
  `Result<Vec<Frame>, FrameDecodeError>`. All existing decoders (line,
  delimiter, length-prefixed, start_end) always return `Ok`. Only SLIP can
  return `Err`. This forces every call site to handle the error case.
- **New `framing_error` stop reason.** Add `RxStopReason::FramingError`. read
  returns a tool result with `is_error: true` and a message naming SLIP and
  the violating byte. subscribe emits a final stop notification with
  `stop_reason: "framing_error"` plus an `error` field naming the violation.
  This is the first new stop_reason value in a while — update the
  `ReadResult.stop_reason` doc comment and the `is_normal_stop` predicate.

## In scope

### A. SLIP constants and shared codec

Add a small SLIP codec section in `src/framing.rs`:

- Private constants: `SLIP_END: u8 = 0xC0`, `SLIP_ESC: u8 = 0xDB`,
  `SLIP_ESC_END: u8 = 0xDC`, `SLIP_ESC_ESC: u8 = 0xDD`.
- A free function or `impl` block for the stuffing/unstuffing logic, shared
  between TX encode and RX decode so there is exactly one definition of each
  direction. Prefer:
  - `fn slip_stuff(payload: &[u8]) -> Vec<u8>` — used by TX.
  - The RX unstuffing lives inside the decoder state machine (it is
    streaming, not whole-payload).

### B. TX framing

- Add `TxFramingMode::Slip` variant (no fields — SLIP is parameterless).
  `{"type": "slip"}`.
- `TxFramingMode::encode(payload)` for `Slip`: emit `END`, stuffed payload,
  `END`. Stuffing: scan payload, replace `0xC0` with `ESC ESC_END`, replace
  `0xDB` with `ESC ESC_ESC`, all other bytes pass through. No length limit
  beyond the existing `MAX_WRITE_BYTES` clamp in `io_ops::write`.
- Add `Slip` to the `check_schema!` list in `src/serial.rs` only if
  `TxFramingMode` gains unsigned fields (it does not — `Slip` is unit). The
  existing `tx_framing_mode_has_no_uint_formats` test already covers the whole
  enum; no new entry needed.

### C. RX framing

- Add `RxFramingMode::Slip` variant (no fields). `{"type": "slip"}`.
- Add `DecoderMode::Slip { state: SlipState }` where `SlipState` tracks:
  - `BeforeFirstEnd` — discard bytes until END seen, then `InFrame`.
  - `InFrame { buf: Vec<u8>, escaped: bool }` — accumulate decoded payload.
    - On `END`: emit frame from `buf` (cleared), stay `InFrame` (next frame
      starts immediately). Reset `escaped = false`.
    - On `ESC`: set `escaped = true`, do NOT push ESC to buf.
    - On `ESC_END` when `escaped`: push `0xC0`, clear `escaped`.
    - On `ESC_ESC` when `escaped`: push `0xDB`, clear `escaped`.
    - On any other byte when `escaped`: **malformed** — return
      `Err(FrameDecodeError::SlipInvalidEscape(byte))`. Discard `buf`,
      transition to `BeforeFirstEnd` to resync (skip to next END). The
      caller's error handling stops the loop; resync state is for safety if
      the caller ever continues (it will not in phase 3).
    - On any byte when NOT `escaped`: push it to `buf`.
  - Truncated escape: `ESC` as last byte of chunk sets `escaped = true` and
    returns `Ok(Vec::new())` (no frame yet). Next chunk's first byte resolves
    it. `flush_partial` drains `buf` plus the literal `ESC` as a raw partial
    frame (best-effort).
- `FrameDecoder::new` maps `RxFramingMode::Slip` to
  `DecoderMode::Slip { state: SlipState::BeforeFirstEnd }`.
- `frame_type_str()` for `DecoderMode::Slip` returns `"slip"`.

### D. Decoder error type + `push()` signature change

- Add `FrameDecodeError` enum in `src/framing.rs`:
  ```rust
  #[derive(Debug, Clone)]
  pub enum FrameDecodeError {
      SlipInvalidEscape(u8),
  }
  impl std::fmt::Display for FrameDecodeError { ... }
  impl std::error::Error for FrameDecodeError {}
  ```
- Change `pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame>` to
  `pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, FrameDecodeError>`.
- All existing decoder arms return `Ok(frames)`. Only the SLIP arm can return
  `Err`.
- Update every call site:
  - `src/tools/rx_consume.rs` `consume_frames`: `decoder.push(chunk)` now
    returns `Result`. On `Err`, stop processing and return a new
    `FrameOutcome::DecodeError(FrameDecodeError)` variant. Add the variant to
    `FrameOutcome`.
  - `src/tools/helpers.rs` `read_bytes_via_session`: handle
    `FrameOutcome::DecodeError` by finishing with `RxStopReason::FramingError`
    metadata. The read tool then surfaces the error message via the existing
    `is_error: true` tool-result path (return `Err(message)` from
    `read_bytes_via_session`, which `io_ops::read` already maps to a tool
    error). Include the violating byte in the message.
  - `src/tools/stream_ops.rs` `stream_rx_via_session`: handle
    `FrameOutcome::DecodeError` by setting `stop_outcome` with
    `RxStopReason::FramingError` metadata, emit the final stop notification
    with an `error` field naming the violation, then exit the loop.
  - `src/framing.rs` `mod tests`: every existing `dec.push(...)` call must
    add `?` or `.unwrap()`. There are many — use `.unwrap()` for the existing
    decoders (they never error) and explicit `assert!(matches!(...Err...))`
    for the new SLIP malformed tests.
- `flush_partial` stays `Option<Frame>` (no error possible at flush; pending
  ESC is emitted raw). No signature change.

### E. New stop reason + metadata

- `src/rx_metadata.rs`: add `FramingError` to `RxStopReason`. Add
  `RxStopMetadata::framing_error(bytes_observed, bytes_returned)` constructor
  mirroring `read_error` shape. Update `ReadResult.stop_reason` doc comment
  in `src/tools/types.rs` to list `framing_error` in the allowed values.
- `src/stop_controller.rs` `is_normal_stop`: `FramingError` is NOT a normal
  stop (it is an error). Do not add it to the normal-stop set. Add a test
  asserting `!is_normal_stop(RxStopReason::FramingError)`.

### F. Tool descriptions

- `src/server.rs`: update `write` description to mention `slip` framing.
  Update `read` and `subscribe` descriptions to mention `slip` framing and
  that a malformed SLIP escape surfaces as `framing_error`.

### G. Round-trip

TX `slip` encode + RX `slip` decode must round-trip arbitrary binary payloads
(including payloads containing `0xC0` and `0xDB`). Covered by tests.

## Out of scope

- SLIP variants other than RFC 1055 classic (no strict END-framed, no
  minimal-no-escape).
- A `slip_checksum` or CRC extension. SLIP has no integrity check; do not add
  one.
- Compressed SLIP (CSLIP). Out of scope.
- Mid-stream resync after error as a first-class feature. The decoder
  transitions to `BeforeFirstEnd` for safety, but phase 3 stops the loop on
  the first error. Automatic resume-on-error is a future decision.
- Profile defaults for SLIP (Phase 5).
- Parser interaction with SLIP. `parser` remains nested under `rx_framing`
  and applies to decoded SLIP frame payloads like any other mode. No special
  handling.

## Relevant files and current behavior

- `src/framing.rs`:
  - `RxFramingMode` (line ~63): add `Slip` variant.
  - `TxFramingMode` (line ~150): add `Slip` variant.
  - `DecoderMode` (line ~399): add `Slip { state: SlipState }`.
  - `FrameDecoder::new` (line ~443): map `Slip` mode.
  - `FrameDecoder::push` (line ~518): signature change to `Result`.
  - `frame_type_str` (line ~681): add Slip arm.
  - `flush_partial` (line ~695): no signature change; ensure SLIP pending
    state flushes reasonably.
  - `mod tests`: many `dec.push(...)` calls need `.unwrap()` or `?`.
- `src/tools/rx_consume.rs`:
  - `FrameOutcome` (line ~25): add `DecodeError(FrameDecodeError)` variant.
  - `consume_frames` (line ~51): handle `Err` from `decoder.push`.
- `src/tools/helpers.rs`:
  - `read_bytes_via_session` (line ~294): handle `FrameOutcome::DecodeError`.
  - `mod tests`: existing `char_framing_*` tests call `dec.push` indirectly
    via `consume_frames` — no direct push calls to update. Verify.
- `src/tools/stream_ops.rs`:
  - `stream_rx_via_session` (line ~347): handle `FrameOutcome::DecodeError`,
    emit final notification with `error` field.
- `src/rx_metadata.rs`:
  - `RxStopReason` (line ~15): add `FramingError`.
  - `RxStopMetadata`: add `framing_error` constructor.
- `src/stop_controller.rs`:
  - `is_normal_stop` (line ~324): do NOT include `FramingError`.
- `src/tools/types.rs`:
  - `ReadResult.stop_reason` doc comment (line ~264): add `framing_error`.
- `src/server.rs`: `write`, `read`, `subscribe` tool descriptions.
- `src/serial.rs` `mod schema`: no new unsigned fields expected; existing
  `tx_framing_mode_has_no_uint_formats` covers the enum.
- `tests/proptest.rs`: `rx_framing_config_roundtrip_all_modes` — add a SLIP
  case. `write_args_with_tx_framing_roundtrip` — add a SLIP mode to the
  modes vector.
- `tests/serial_pty.rs`: optional SLIP PTY test (see test plan item 12).
- `tests/native_sim_validation.rs`: firmware does not emit SLIP; no native
  SLIP test unless you add a device-side SLIP command (out of scope — do not
  modify firmware).

## Expected API / UX shape

TX:
```json
{ "tx_framing": { "type": "slip" } }
```
Writes `END + stuffed payload + END`. `decoded_bytes` = payload length;
`bytes_written` = stuffed+framed length.

RX:
```json
{ "rx_framing": { "type": "slip" } }
```
Skips to first END, then emits one frame per END-delimited segment with
escapes decoded. Malformed escape → read returns `is_error: true` tool result
with message like `SLIP framing error: invalid escape byte 0x41`; subscribe
emits final notification with `stop_reason: "framing_error"` and
`error: "SLIP framing error: invalid escape byte 0x41"`.

## Test plan

Add tests in `src/framing.rs` `mod tests`:

1. **slip_stuff replaces 0xC0 and 0xDB.** Unit test on the stuffing helper:
   payload `[0xC0, 0xDB, 0x41]` → stuffed `[ESC, ESC_END, ESC, ESC_ESC, 0x41]`.

2. **tx slip encodes END...END.** `TxFramingMode::Slip.encode(b"hi")` →
   `[END, b'h', b'i', END]`.

3. **tx slip stuffs payload with END byte.** `encode(&[0xC0])` →
   `[END, ESC, ESC_END, END]`.

4. **tx slip stuffs payload with ESC byte.** `encode(&[0xDB])` →
   `[END, ESC, ESC_ESC, END]`.

5. **rx slip skips to first END.** Push `b"junk\xC0hi\xC0"` → one frame
   `b"hi"`. The `junk` before the first END is discarded.

6. **rx slip decodes basic frame.** Push `b"\xC0hello\xC0"` → frame `b"hello"`.

7. **rx slip decodes ESC_END.** Push `b"\xC0\xDB\xDC\xC0"` → frame `b"\xC0"`.

8. **rx slip decodes ESC_ESC.** Push `b"\xC0\xDB\xDD\xC0"` → frame `b"\xDB"`.

9. **rx slip malformed escape returns Err.** Push `b"\xC0\xDB\x41\xC0"` →
   `push` returns `Err(FrameDecodeError::SlipInvalidEscape(0x41))`. No frame
   emitted.

10. **rx slip two frames in one chunk.** Push `b"\xC0aa\xC0bb\xC0"` → frames
    `[b"aa", b"bb"]`.

11. **rx slip cross-chunk frame.** Push `b"\xC0hel"` then `b"lo\xC0"` → one
    frame `b"hello"` across two chunks.

12. **rx slip truncated escape holds pending.** Push `b"\xC0\xDB"` → no
    frame, no error. Push `b"\xDC\xC0"` → frame `b"\xC0"`.

13. **rx slip flush_partial emits pending.** Push `b"\xC0hel"` then
    `flush_partial()` → one frame `b"hel"` (raw partial).

14. **round-trip slip arbitrary binary.** Payload containing `0xC0`, `0xDB`,
    `0x41`: TX encode → RX decode → assert original payload. Use
    `b"\xC0\xDB\x41\xDB\xDD\xC0"` as the payload.

15. **round-trip slip empty payload.** `encode(b"")` → `[END, END]`; RX decode
    → frame `b""`.

16. **framing_error stop reason added.** In `src/rx_metadata.rs` tests (or a
    new test), assert `RxStopReason::FramingError` serializes to
    `"framing_error"` and `is_normal_stop` returns false for it.

17. **read integration: slip decode success.** In `src/tools/helpers.rs`
    tests, drive `read_bytes_via_session` with a SLIP event stream; assert
    `ReadOutcome.frames` contains the decoded payload.

18. **read integration: slip malformed surfaces error.** Drive
    `read_bytes_via_session` with a malformed SLIP stream; assert the
    function returns `Err` with a message containing `SLIP framing error`.

19. **subscribe integration: slip malformed emits framing_error notification.**
    In `src/tools/stream_ops.rs` tests (or PTY), drive a subscribe with a
    malformed SLIP stream; assert a final notification with
    `stop_reason: "framing_error"` and an `error` field. PTY is acceptable
    here since `stream_rx_via_session` has no unit-test harness (see Phase 2
    follow-up pattern).

20. **proptest roundtrip additions.** Add SLIP case to
    `rx_framing_config_roundtrip_all_modes` and SLIP mode to
    `write_args_with_tx_framing_roundtrip` modes vector.

21. **regression: existing decoders unaffected by push() Result change.**
    All existing `framing.rs` unit tests pass after adding `.unwrap()` to
    their `dec.push(...)` calls. No behavior change for line/delimiter/
    length-prefixed/start_end.

## Constraints and invariants (from repo docs)

- **No `unwrap`/`expect`/`println!`/`todo!()`/`unimplemented!()`** in
  production code. Tests may use `.unwrap()` on `push` for non-SLIP decoders.
- **Tool failures become MCP tool results with `is_error: true`**, not
  protocol-level `McpError`. SLIP malformed on read → `Err` from
  `read_bytes_via_session` → `io_ops::read` already maps that to a tool
  error. Do NOT introduce a protocol-level error.
- **Every `uN`/`Option<uN>` field on a `JsonSchema`-deriving struct uses
  `uint_schema`/`option_uint_schema`.** `Slip` variants are unit (no fields).
  `FrameDecodeError` does NOT derive `JsonSchema` (it is internal). No new
  schema regression entries needed, but verify `tx_framing_mode_*` and the
  tool schema tests still pass.
- **read/subscribe raw-path asymmetry preserved.** read bounded and scans
  `chunk[..take]`; subscribe scans full chunks. Do not merge raw paths.
- **Framing semantics differ by design:** read keeps later frames from the
  same chunk after the matching frame; subscribe stops on the matching frame.
  Preserve. SLIP emits frames the same way as other modes — the
  `consume_frames` sink flow handles this.
- **Match metadata:** read uses `accumulated.len()` for `bytes_returned`;
  subscribe uses cumulative `total_returned`. Preserve.
- **subscribe degrades bad framing configs to raw with `warn!`;** read
  propagates the error. NOTE: this applies to CONSTRUCTION errors
  (`FrameDecoder::new`). SLIP has no construction errors (parameterless).
  RUNTIME decode errors (malformed escape) are new — both read and subscribe
  STOP on the first runtime decode error. This is a deliberate asymmetry
  exception: construction errors are agent config mistakes (recoverable by
  fixing the request); runtime decode errors are stream corruption (not
  recoverable by retrying the same bytes). Document this distinction in the
  `FrameDecodeError` doc comment.
- **`flush_partial` increments `frame_count` and emits a `Frame` with
  `parsed: None`.** Keep this contract. SLIP pending state flushes the same
  way.
- **`frame_type_str()` returns a stable per-mode string.** SLIP returns
  `"slip"`.
- **Conventional commits:** `feat:` for the SLIP addition (this is a new
  feature, not a fix). If you split the `push()` signature change into its
  own commit, use `refactor:`. No attribution footers.

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
cargo test --lib rx_metadata
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
- Final SLIP state machine shape (`SlipState` variants + transitions) and
  where it lives in `src/framing.rs`.
- The `push()` signature change blast radius — which call sites were updated
  and how (`consume_frames`, `read_bytes_via_session`,
  `stream_rx_via_session`, tests).
- The new `FrameDecodeError` type and the new `RxStopReason::FramingError`
  value — confirm the stop_reason doc comment and `is_normal_stop` were
  updated.
- How malformed escape surfaces on read (tool `is_error` message shape) and
  subscribe (final notification `error` field shape).
- New tests added and which existing tests were updated (`.unwrap()` on
  `push` calls).
- Gate command results (`fmt`, `build`, `test`, `clippy`). Note any
  failures with root cause.
- Any scope decision you had to make that is not covered above — especially
  around the construction-error vs runtime-error asymmetry documentation.
- Any surprise in the `push()` Result migration (e.g. a call site that was
  awkward to update).