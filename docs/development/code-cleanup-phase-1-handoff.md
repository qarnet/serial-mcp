# Code Cleanup Phase 1 Handoff

## Goal

Remove duplicated TX buffer preparation from `write` and `transact`, and
centralize identical post-read accounting across `read`, `transact`, and
`capture_boot`, without changing observable behavior or operation ordering.

Implement this phase only. Commit completed work before returning.

## In scope

- `src/tools/io_ops.rs`
- `src/tools/result_builders.rs`
- `src/tools/control_ops.rs`
- Focused private/unit tests for newly extracted pure TX helpers
- Existing `tests/serial_pty.rs` tests only if a public-boundary regression gap
  must be filled
- The active plan/index files already added by the orchestrator:
  - `docs/development/code-cleanup-plan.md`
  - `docs/development/README.md`

## Out of scope

- Do not make `transact` call `write()` or `read()`.
- Do not change `TransactArgs`, `WriteArgs`, `ReadArgs`, result types, schemas,
  tool descriptions, or tool count.
- Do not change `transact`'s default `from={"type":"now"}` behavior.
- Do not reorder connection lookup, decode, framing resolution, session I/O,
  matcher setup, budget reservation, or counter/log operations.
- Do not normalize `write` and `transact` error strings.
- Do not refactor `read_from_private_cursor`, subscription code, profile code,
  test harnesses, comments outside touched blocks, or any platform-specific
  test structure in this phase.
- Do not update dependencies or package version.

## Current behavior and grounding

### TX ordering

`src/tools/io_ops.rs` currently performs these steps in both handlers:

1. Parse encoding before connection lookup.
2. Look up one connection.
3. Decode tool string after connection lookup.
4. Validate decoded size with a tool-specific field label.
5. Resolve TX framing through four-layer precedence.
6. Apply framing and validate framed size.
7. Get/create TX session, write, then increment write counter.
8. Build the tool-specific result.

Preserve this order. In particular:

- Unknown encoding currently wins before a missing-connection error.
- Malformed encoded data currently loses to a missing-connection error because
  data decoding happens after lookup.
- `write` and `transact` currently expose different framing/write error text.
- `transact` keeps the same `Arc<SerialConnection>` through both halves.

### Current error text

Preserve these caller-specific forms:

- Both decode failures: `Data decoding failed - {e}`.
- `write` framing failure: routed through `log_tool_err`, context
  `TX framing failed on {connection_id}: {e}`.
- `transact` framing failure: `TX framing failed: {e}`.
- `write` send failure: routed through `log_tool_err`, context
  `Data sending failed on {connection_id}`.
- `transact` send failure: `Write failed: {e}`.
- Existing validation field labels remain `write.data.len()`,
  `write.framed_len()`, `transact.data.len()`, and
  `transact.framed_len()`.

### Post-read accounting

Exact blocks currently exist at:

- `src/tools/io_ops.rs` after `read` result construction.
- `src/tools/io_ops.rs` after `transact` read-result construction.
- `src/tools/control_ops.rs` after `capture_boot` read-result construction.

Required order:

1. `connection.record_read_op()`.
2. `connection.log().rx_data(result.bytes_read)`.
3. If truncated: `record_truncation()`, then `log.truncated(...)`.
4. If matched and request exists: `log.match_found(...)`.

`subscribe` is intentionally excluded because its accounting lifecycle differs.

## Exact implementation shape

### 1. Private decoded/prepared TX values

Keep helpers private to `src/tools/io_ops.rs`; do not create a new module for
this small extraction.

Use two stages so call ordering and caller-specific framing errors remain
visible:

1. `decode_tx_payload(encoding, input, decoded_limit_field)`
   - Calls `codec::decode`.
   - Maps decode failure to the existing exact `Data decoding failed - {e}`.
   - Calls `clamp_or_err` with the supplied current field label and
     `MAX_WRITE_BYTES`.
   - Returns a private value containing decoded `Vec<u8>`, decoded byte count,
     and encoding.

2. `apply_tx_framing(decoded, framing, framed_limit_field)`
   - Applies `TxFramingConfig::mode.encode` when framing exists; otherwise
     retains decoded bytes without a needless semantic conversion.
   - Validates final length with the supplied current field label and
     `MAX_WRITE_BYTES`.
   - Returns a private prepared value containing `Arc<[u8]>`, decoded byte
     count, and encoding.
   - Distinguishes framing failure from size-validation failure with a small
     private enum, so each caller can preserve its exact framing error mapping
     while validation errors pass through unchanged.

Names may vary slightly if a clearer equivalent preserves this structure. Do
not use callbacks, boxed policy objects, tool-name switches, or public APIs.

Resolve four-layer TX framing in each caller after decode-size validation, as
today. Then pass the resolved framing by reference/value into the shared framing
stage. Session lookup, write, error mapping, write counter, and result assembly
stay in each handler.

### 2. Shared post-read accounting

Add a crate-private helper in `src/tools/result_builders.rs`, beside read-result
construction:

```rust
pub(crate) fn record_read_completion(
    connection: &crate::serial::SerialConnection,
    result: &crate::tools::types::ReadResult,
    match_request: Option<&crate::match_config::MatchRequest>,
)
```

Equivalent naming is fine. Helper must perform exact four-step order listed
above. It must not build results, return errors, or own tool-specific logging.

Call it from `read`, `transact`, and `capture_boot` immediately after
`build_read_result`, before each caller's subsequent result/info handling.

## Tests

Add focused pure tests for extraction where they provide meaningful behavior:

- UTF-8, hex, and base64 decode preserve encoding and decoded length.
- Unframed payload remains byte-identical.
- Representative framing produces exact bytes while decoded length remains
  pre-framing length.
- Framing errors remain distinguishable from size validation errors.

Do not duplicate exhaustive framing-codec tests already in
`src/framing/codecs.rs`.

Existing public-boundary coverage that must remain green:

- `tests/serial_pty.rs::pty_transact_writes_then_reads_response`
- `tests/serial_pty.rs::pty_transact_from_now_skips_pre_write_buffer`
- `tests/serial_pty.rs::pty_transact_from_cursor_includes_pre_write_buffer`
- `tests/serial_pty.rs::pty_transact_with_protocol_applies_both_directions`
- `tests/serial_pty.rs::pty_transact_cancellation_aborts_read`

Add a new public-boundary test only if implementation exposes a gap not covered
by those tests. Tests must assert externally observable bytes/results, not
helper call counts or private object identity.

## Verification

Run:

```bash
cargo fmt --all -- --check
cargo test --lib
cargo test --test serial_pty pty_transact -- --test-threads=1
cargo test --test serial_pty pty_write -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

If test-name filtering reports zero tests for `pty_write`, run the complete
serial PTY suite instead:

```bash
cargo test --test serial_pty --locked -- --test-threads=1
```

## Acceptance criteria

- One shared decode/validate/framing implementation serves both `write` and
  `transact`.
- Public handlers retain existing lookup/I/O/error/counter/result policy.
- No public schema or wire change.
- Three post-read accounting copies become one helper and three calls.
- All requested checks pass with no warnings.
- Diff contains no unrelated cleanup.
- Working tree is clean after commit.

## Commit and recap

Before returning:

1. Inspect `git status`, `git diff`, and recent log.
2. Stage only this phase's intended files, including active plan/index/handoff
   documents already present in the worktree.
3. Commit with conventional message, suggested:

   `refactor: share serial I/O preparation`

4. Do not push, merge, open a PR, amend, force-push, or add attribution.
5. Return concise recap containing:
   - Files changed.
   - Exact behavior preserved/refactored.
   - Tests and commands run with results.
   - Commit hash and message.
   - Deviations, blockers, or follow-up concerns.
