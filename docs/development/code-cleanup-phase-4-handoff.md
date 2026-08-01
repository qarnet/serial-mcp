# Code Cleanup Phase 4 Handoff

## Goal

Shorten `stream_rx_from_ring` by moving complete notification-delivery units
into named private helpers. Preserve path-specific encoding failures,
notification drops, peer-disconnect handling, cursor movement, match behavior,
and stop payloads exactly.

Implement Phase 4 only. Commit completed work before returning.

## In scope

- `src/tools/stream_ops.rs`
- Existing stream/PTY/HTTP tests
- Focused pure serialization tests only if helper behavior is not covered
- This handoff document

## Out of scope

- No changes to `subscribe` request validation, framing construction,
  replacement ordering, budget lifetime, task registration, or cursor default.
- No changes to notification structs, schemas, logger names, levels, payload
  fields, stop reasons, offsets, counters, or warning/error text.
- Do not create one generic encoding/accounting helper for all frame/chunk/
  partial/context paths; true-failure behavior differs.
- Do not change `SubscribeFrameSink` matching-frame peer-failure behavior in
  this phase. Matching frame still reports match even if emitting that frame
  fails; nonmatching frame failure still stops as `peer_disconnected`.
- Do not add a complex fake `Peer` harness solely to test that existing quirk.
- Do not alter `RxStopController`, `consume_frames`, matcher, decoder, or read
  code.
- Do not perform test-harness or broad comment cleanup.

## Grounding and required behavior

`stream_rx_from_ring` currently mixes ring control with three complete delivery
units:

1. Raw chunk or raw encoding-error notification.
2. Partial-frame flush notification.
3. Optional shaped match-context encoding.

Each unit has distinct true-failure behavior.

### Raw chunk

- Successful direct encoding or hex fallback emits
  `SubscribeChunkNotification` at `Info` and increments `total_returned` by raw
  chunk length only after successful peer delivery.
- Successful hex fallback warns but never records a drop.
- True encode+hex failure records one notification drop, logs it, emits
  `SubscribeEncodingErrorNotification` at `Warning`, then continues without
  advancing the private cursor. Preserve this currently unreachable-but-shipped
  control flow.
- Peer failure while emitting error/chunk notification stops as
  `peer_disconnected`; normal chunk peer failure also records/logs a drop.

### Partial frame

- Flush happens after loop stop when decoder returns a partial frame.
- Encoding fallback warns but is not a drop.
- True encoding failure records/logs a drop and emits nothing.
- Peer delivery failure records/logs a drop and contributes zero returned
  bytes.
- Success contributes raw partial-frame byte length to `total_returned`.

### Match context

- Shaping happens before encoding.
- Successful fallback pairs `data` with effective `encoding`.
- True failure records/logs one drop and returns `(None, None)`.
- No shaped data also returns `(None, None)` without accounting.

### Serialization

Every notification currently uses `serde_json::to_value`, warns on failure,
and substitutes `{}`. Preserve this behavior and each current warning context.

## Exact implementation shape

### 1. Shared serialization-to-logging-param helper

Add one private generic helper near `SubscribeFrameSink`:

```rust
fn logging_notification<T: serde::Serialize>(
    notification: &T,
    level: LoggingLevel,
    logger: &str,
    serialization_context: &str,
    connection_id: &str,
) -> LoggingMessageNotificationParam
```

It serializes with `serde_json::to_value`; on failure emits:

```text
{serialization_context} serialization error on {connection_id}: {error}
```

and uses `json!({})`. It sets supplied level, `Some(logger.to_string())`, and
payload. Use exact existing context strings at each call so log text remains
unchanged:

- `RX frame notification`
- `SubscribeEncodingErrorNotification`
- `SubscribeChunkNotification`
- `SubscribePartialFrameNotification`
- `SubscribeStopNotification`

Use helper in `SubscribeFrameSink` and final stop notification as well as new
delivery helpers. It performs no peer send and no accounting.

### 2. Raw delivery helper

Extract raw encoding + notification send into one private async function. Use a
small private result enum with three outcomes:

```rust
enum RawChunkDelivery {
    Sent,
    EncodingDropped,
    PeerDisconnected,
}
```

Helper receives peer, connection, logger/id, requested encoding, chunk bytes,
and current-slice `bytes_lost`. It owns existing encoding warnings, drop/log
accounting, error notification, chunk notification, and peer send.

Caller behavior must remain:

- `Sent`: add `n` to `total_returned`, then process existing stop outcome.
- `EncodingDropped`: execute current `continue` before private-cursor advance.
- `PeerDisconnected`: set `stop_outcome = Some(ctrl.peer_disconnected())` and
  break.

Do not pass `RxStopController` or mutable loop state into helper.

### 3. Partial-frame helper

Extract partial encoding + notification delivery into a private async function
returning emitted raw byte count (`usize`, zero on encode/send failure). It
receives peer, connection, logger/id, encoding, and owned/borrowed partial
frame. Preserve exact notification, warning, drop/log, and peer-failure text.

Caller increments `frames_emitted` exactly where it does today, then adds
returned count to `total_returned`.

### 4. Match-context helper

Extract shaped-data encoding into a private synchronous function returning
`(Option<String>, Option<String>)`. It receives connection, id, requested
encoding, and `Option<&[u8]>`. Preserve fallback warning and true-failure
drop/log behavior.

### 5. Keep frame sink behavior explicit

Use shared serialization helper for frame notification parameter construction,
but leave branch order and peer-error policy inside `SubscribeFrameSink`.
Do not alter or remove matching-frame failure comment in this phase.

### 6. Keep final stop assembly visible

Leave `SubscribeStopNotification` field construction in
`stream_rx_from_ring`; it documents final wire state. Use only shared
serialization helper for payload/parameter creation.

## Tests

Existing authoritative behavior includes:

- PTY raw and framed match/context flows.
- Framed per-frame and partial-frame notifications.
- HTTP binary fallback for frame, partial, and match context.
- Notification-drop count remains zero after successful fallback.
- Runtime framing-error and peer lifecycle tests.

Do not add tests that assert helper calls. Add a helper unit test only if needed
for serialized output equivalence.

## Verification

Run:

```bash
cargo fmt --all -- --check
cargo test --lib tools::stream_ops --locked
cargo test --test serial_pty --locked -- --test-threads=1
cargo test --test http_integration subscribe_binary_framed_frame_emits_hex_without_drop --locked -- --test-threads=1
cargo test --test http_integration subscribe_binary_partial_flush_emits_hex_with_effective_encoding --locked -- --test-threads=1
cargo test --test http_integration subscribe_matched_binary_context_reports_hex_data_and_encoding --locked -- --test-threads=1
cargo test --test http_integration read_and_subscribe_same_literal_match_index_over_chunked_stream --locked -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

## Acceptance criteria

- Raw chunk and partial-frame delivery no longer dominate ring-loop control
  flow.
- Serialization boilerplate exists once.
- Path-specific true-failure and peer-failure behavior remains explicit and
  unchanged.
- Final stop fields remain visible in main stream function.
- No public schema/wire/log/counter change.
- Requested tests pass with no warnings.
- Diff contains no unrelated cleanup.
- Working tree clean after commit.

## Commit and recap

Before returning:

1. Inspect status, diff, and recent log.
2. Stage only `src/tools/stream_ops.rs` and this handoff.
3. Commit with suggested message:

   `refactor: simplify subscription delivery`

4. Do not push, merge, open PR, amend, force-push, or add attribution.
5. Return files, behavior preserved, tests/results, commit hash/message,
   deviations, blockers, and follow-up concerns.
