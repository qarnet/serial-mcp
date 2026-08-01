# Phase 5 Handoff — Atomic Boot Capture

## Goal

Add one bounded `capture_boot` tool that establishes a fresh RX mark, optionally
pulses DTR/RTS, and captures only post-mark boot output through the existing RX
framing/matching/timeout pipeline. Eliminate the arm/reset race demonstrated by
Phase 4 evaluation.

Phase 4 measured the current composition at five calls/886 request bytes with a
stale-data race. The modeled atomic call used one call/216 bytes. This phase
implements the justified operation, not a generic script engine.

## Public behavior

1. Tool waits for any in-flight pump read to be appended, discards unread OS
   input, marks the ring live edge, then asserts configured reset lines.
2. Bytes already retained before the mark never appear in capture result.
3. Bytes emitted immediately by reset assertion/release are not missed.
4. Capture uses existing match, framing/parser/protocol, encoding, timeout,
   silence, truncation, disconnect, and framing-error semantics.
5. Capture uses a private cursor and never changes shared `read` cursor.
6. Reset lines are released on normal completion, cancellation, assertion
   failure, runtime framing error, encoding error, timeout, and disconnect.
7. Tool keeps result in memory. No arbitrary path or continuous capture.

## In scope

- RX pump barrier/epoch around mark + reset assertion.
- Per-connection line-control serialization.
- Private-cursor extraction from existing read loop.
- Optional bounded DTR/RTS reset pulse.
- New tool/result/schema/docs/evaluator update.
- Controlled-backend public MCP tests plus existing PTY/native_sim regression
  where hardware capabilities permit.

## Non-scope

- No generic step/script language.
- No board-specific Arduino/ESP recipe catalog.
- No persistent/raw capture files.
- No profile learning from reset line states or captured bytes.
- No firmware/Zephyr native PTY driver fork for modem-line emulation.
- No automatic reboot command or power control.

## Why a live-edge snapshot alone is insufficient

Current pump can:

1. read serial bytes under connection I/O mutex
2. release I/O mutex
3. append bytes to RX ring

Reset could acquire I/O mutex between steps 2 and 3. A byte physically read
before reset would then append after a naive `ring.end_offset()` mark and leak
into boot capture.

Add an async pump gate to `RxSession`. Pump must hold gate across one complete
read + ring append. `capture_boot` acquires same gate, which waits for any
in-flight read/append to finish and prevents next read until mark/reset setup is
complete.

Atomic setup under pump gate:

```text
wait for in-flight read+append
> clear unread OS input
> mark = ring.end_offset()
> assert reset lines (if configured)
> release pump gate
```

Holding pump gate through full pulse would risk OS-buffer overflow and missed
early boot bytes. Release it immediately after assertion so pump captures bytes
during hold and line release.

## API

Add tool count 27.

```rust
pub struct CaptureBootReset {
    pub assert_dtr: bool,
    pub assert_rts: bool,
    pub release_dtr: bool,
    pub release_rts: bool,
    pub hold_ms: u64, // default 100, runtime bounded, minimum 1
}

pub struct CaptureBootArgs {
    pub connection_id: String,
    pub reset: Option<CaptureBootReset>,
    pub settle_ms: Option<u64>,
    pub timeout_ms: Option<u64>, // omitted defaults to Some(5000)
    pub no_new_rx_timeout_ms: Option<u64>,
    pub encoding: String, // default utf8
    pub r#match: Option<MatchRequest>,
    pub rx_framing: Option<RxFramingConfig>,
    pub rx_parser: Option<ParserConfig>,
    pub protocol: Option<ProtocolPreset>,
}

pub struct CaptureBootResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub reset: Option<CaptureBootReset>,
    pub mark_offset: u64,
    pub pre_mark_bytes: u64,
    pub os_input_flushed: bool,
    pub read: ReadResult,
}
```

Annotate every unsigned field with schema helpers. `reset=null` means arm-only
capture for externally reset/power-cycled devices; DTR/RTS is untouched. There
is no `from` field: capture always begins at its atomic mark. Connection default
`max_buffered_bytes` bounds in-memory result. `settle_ms` delays consumption,
not capture—the always-on pump still appends bytes from the mark during settle.

Timeout applies to read phase after pulse/settle; total operation is bounded by
hold + settle + read timeout. Explicit `timeout_ms=null` keeps current read
drain behavior.

Tool is destructive because configured reset lines may reboot hardware.

## Pump gate

In `RxSession` add `Arc<tokio::sync::Mutex<()>>`:

- pump acquires gate before `connection.read`
- holds through ring append or read outcome classification
- releases before disconnect pause/sleep
- capture obtains owned/borrowed guard through a narrow method

Do not expose ring internals publicly. Add focused deterministic tests proving
capture cannot acquire gate until an in-flight read's bytes are appended.

## OS input purge

While pump gate is held, call `connection.flush_buffers(Input)` before mark.
This removes bytes waiting in OS buffers that have not entered ring; otherwise
they could be appended after mark despite predating capture. Do not clear RX
ring or move shared cursor. Return `os_input_flushed=true` only after successful
purge. Purge failure is a tool error and must occur before line assertion.

## Line-control serialization and cleanup

Add per-connection async control lock. Existing `set_dtr_rts` and
`capture_boot` use it so another line-control request cannot interleave inside
pulse.

Refactor `SerialConnection` into:

- public `set_dtr_rts` acquiring control lock
- crate-private unlocked line setter for caller already holding lock
- crate-private control-lock accessor/guard

Capture acquires control lock for whole assert/hold/release sequence.

Arm cleanup before assertion because production `set_dtr_rts` sets DTR then
RTS; RTS failure can leave DTR changed. Use cancellation-safe guard modeled on
`BreakResetGuard`: on drop, spawn a best-effort release using configured release
state. On every explicit path, attempt release and disarm guard only after
success.

Request cancellation during hold/settle:

- release lines first
- route already-cancelled token through private read path to return structured
  `stop_reason="cancelled"` with offsets, rather than an ad-hoc error

Assertion/release I/O failure remains tool error with cleanup attempt logged.

## Private read cursor

Extract current `read_bytes_from_ring` core into a reusable private-cursor form:

```rust
read_from_private_cursor(session, initial_cursor, ...)
    -> Result<(ReadOutcome, final_private_cursor), String>
```

Existing `read_bytes_from_ring` becomes wrapper using shared cursor and applying
returned final cursor. Preserve all existing read/transact behavior exactly.
`capture_boot` starts at mark and discards returned private final cursor.

Construction validation must happen before reset:

- connection and encoding
- timeout/silence bounds
- matcher
- buffer budget
- framing/parser resolution
- `FrameDecoder::new` validation when framing configured
- reset hold/settle bounds

Runtime decode failures after reset remain structured read results with
`stop_reason="framing_error"`, partial frames, error text, and existing hex
fallback.

## Tool implementation sequence

1. Validate all args and resolve framing/parser/protocol precedence.
2. Reserve output budget and get existing RX session.
3. Acquire control lock when reset exists.
4. Acquire pump gate.
5. Purge OS input.
6. Compute `mark_offset` and `pre_mark_bytes`.
7. Arm release guard and assert reset lines, if configured.
8. Release pump gate immediately.
9. Hold reset state; always release lines.
10. Optional settle delay while pump captures.
11. Consume from mark using private read cursor.
12. Build nested `ReadResult`; record read/truncation/match logs like `read`.

Capture is transient: no profile learning or revision update.

## Stop and result semantics

Reuse current RX stop vocabulary. Expected structured outcomes include:

- `match_found`
- `timeout`
- `no_new_rx_timeout`
- `max_buffered_bytes`
- `connection_closed`
- `cancelled`
- `framing_error`
- `max_frames`
- `drained`

`read.from_offset` must equal mark unless ring wrapped before consumer caught up;
then existing `bytes_lost`/start-offset behavior reports loss. `mark_offset`
always records original atomic boundary.

## Behavior-first tests

### Controlled `SerialIo` through public HTTP MCP

Build test backend that records line transitions and can synchronously inject RX
bytes when assertion/release occurs. Do not mock tool/store logic.

Required:

1. Retained stale bytes excluded; immediate release-hook boot bytes captured;
   old shared cursor and ring history remain readable afterward.
2. Immediate bytes emitted inside line-change call are captured and match stops
   at pattern.
3. Pump barrier test proves in-flight pre-reset read appends before mark.
4. Cancellation during hold releases lines and returns structured cancelled
   outcome (request-scoped MCP cancellation, not whole-client teardown).
5. Assertion partial failure and release failure attempt configured cleanup.
6. Invalid framing construction fails before any line transition.
7. Runtime SLIP/COBS framing error returns partial structured result and leaves
   lines released.
8. NDJSON framing/parser and binary hex/base64 output reuse current behavior.
9. Silence timeout after banner; wall timeout during continuous output.
10. Disconnect returns partial capture with `connection_closed`.
11. Ring wrap reports `bytes_lost`; output bound/truncation is observable.
12. Concurrent `set_dtr_rts` cannot interleave between assert and release.
13. No reset config performs arm-only capture without touching lines.

At minimum tests 1–7, 9, 10, 12 are acceptance-critical. Split tests for clear
failure diagnosis.

### Existing read/transact regression

Run all existing ring, framing, match, timeout, and cursor tests after private
cursor extraction. Add focused unit test that private read leaves shared cursor
unchanged while shared wrapper still advances it.

### native_sim limitation and coverage

Zephyr `native_sim` PTY UART lacks modem-line callbacks, so it cannot observe
DTR/RTS and cannot prove reset pulse atomicity without a custom driver/supervisor
(explicitly out of scope). Do not fake that claim.

If practical, run arm-only capture against native_sim with an external command
emitted after capture starts to cover real byte pipeline. Atomic reset proof
must come from controlled backend above. Existing native_sim suites remain
mandatory regression if firmware asset is available; no firmware change is
required.

## Agent docs and evaluator

- Add `capture_boot` to server decision tree and README as boot/reset path.
- Tool description states private cursor, OS-input purge, optional line pulse,
  bounded in-memory result, and no file output.
- Update exact tool counts (27) in README/Cargo/server.json/tests/AGENTS.
- Update evaluator: `capture_boot` is implemented, not modeled. Keep Phase 4
  baseline historical. Run current evaluation with
  `--baseline docs/development/agent-interface-baseline.json` and record catalog
  delta in a Phase 5 note; do not rewrite Phase 4 baseline as if tool existed.
- Update evaluator fixtures/tests so no current report calls implemented
  `capture_boot` hypothetical.

## Expected files

- `src/rx_session.rs`
- `src/serial.rs`
- `src/tools/helpers.rs`
- `src/tools/io_ops.rs`
- `src/tools/control_ops.rs`
- `src/tools/types.rs`
- `src/server.rs`
- `src/tools/mod.rs`
- controlled backend test support + HTTP/PTY tests
- optional native_sim test only if honest on current driver
- evaluator modules/report note
- README, Cargo.toml, server.json, AGENTS.md, doc drift tests
- this handoff

## Verification

```bash
cargo test --test http_integration capture_boot --locked
cargo test --test serial_pty capture_boot --locked
cargo test --manifest-path xtask/Cargo.toml
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

If existing native_sim firmware asset is available, also run relevant ignored
suites. Report inability honestly rather than claiming DTR coverage.

## Commit requirements

Inspect status/diff/log, stage only Phase 5 files, commit conventional message,
return behavior/tests/evaluation delta/hash/deviations. Do not amend, push,
merge, open PR, add attribution, or implement Phase 6.
