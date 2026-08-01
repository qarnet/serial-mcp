# Post-0.9 Refinement — Phase 4 Handoff

## Role and delivery constraint

Implement Phase 4 on existing branch `refactor/post-0.9-refinement`. Follow
canonical plan and repository `AGENTS.md`. Do not create or push another branch
or PR. Commit completed work before returning. Do not amend prior commits or add
attribution.

## Goal

Make every RX payload lossless across `read`, `subscribe`, and `capture_boot`:
try requested encoding, fall back to exact lowercase spaced hex when requested
encoding cannot represent bytes, and report effective encoding. Successful
fallback warns but is never counted as a dropped notification/frame.

## In scope

- Shared pure encode-or-hex primitive in `src/codec.rs`.
- Read top-level data and each decoded frame.
- Subscribe raw chunks, decoded frames, partial-frame flushes, and matched
  context in final stop notifications.
- Effective-encoding wire documentation and optional stop-context encoding.
- Public-boundary tests over HTTP loopback/controlled backend and real PTY.
- Update current README, protocol guide, AGENTS, and remaining FEATURES debt.
- Commit this handoff with Phase 4 work.

## Out of scope

- No matcher-window changes (Phase 5).
- No module movement (Phase 6).
- No lossy UTF-8, replacement characters, base64 fallback, new user option, or
  new MCP tool/resource.
- No stop-reason, cursor, framing, parser, checksum, or match semantic change.
- No removal of `SubscribeEncodingErrorNotification` wire type.

## Grounding evidence

- `codec::encode(Utf8, bytes)` rejects invalid UTF-8; hex/base64 encode paths
  are representationally total today.
- `build_read_result` falls back only when `outcome.error.is_some()`. Ordinary
  raw invalid UTF-8 returns a tool error. It also propagates top-level effective
  hex to every frame, so a valid UTF-8 frame before binary framing error becomes
  hex unnecessarily.
- `SubscribeFrameSink` drops unencodable frames and increments
  `notification_drop_count`.
- Raw subscribe emits `SubscribeEncodingErrorNotification`, increments drop
  count, then `continue`s before private cursor advance; binary UTF-8 chunks can
  repeat until timeout.
- Partial-frame flush silently omits an unencodable payload. Match-context data
  is omitted on encoding error and final stop notification has no companion
  effective-encoding field.
- Existing controlled capture test expects valid `OK` frame as hex only because
  top-level framing-error encoding leaked into per-frame encoding; new
  per-payload rule should keep that frame UTF-8 while raw malformed bytes use
  hex.

## Exact implementation decisions

### Shared primitive

In `src/codec.rs`, add a small result type such as:

```rust
pub struct EncodedPayload {
    pub data: String,
    pub encoding: Encoding,
    pub fallback_reason: Option<String>,
}
```

and:

```rust
pub fn encode_or_hex(
    requested: Encoding,
    bytes: &[u8],
) -> Result<EncodedPayload, CodecError>
```

Behavior:

1. call existing `encode(requested, bytes)`;
2. success returns requested encoding and no fallback reason;
3. failure calls existing `encode(Encoding::Hex, bytes)` and returns effective
   `Hex` plus original error text;
4. only true hex-fallback failure returns `Err`.

Keep primitive pure; callers own warnings/log/counter semantics. Do not duplicate
hex formatting or change existing `encode`/`decode` behavior. Add pure tests for
valid UTF-8, invalid UTF-8 exact-byte hex fallback, requested hex, requested
base64, empty bytes, and round-trip recovery of fallback bytes.

### Read and capture result construction

Use `encode_or_hex` for top-level `outcome.bytes` on every stop reason, not only
framing errors. Warn on fallback and set `ReadResult.encoding` to effective
encoding. A true fallback failure remains a tool-result construction error.

Encode each `FrameResult` independently from requested encoding with
`encode_or_hex`; do not inherit top-level effective encoding. Set each frame's
own effective `encoding`. Warn on successful frame fallback but do not increment
`frames_dropped`. Increment `frames_dropped` only on checksum validation drops
or true encode+hex failure. Thus a valid UTF-8 frame preceding malformed binary
SLIP remains UTF-8 while top-level raw data may be hex.

### Subscribe payloads

Use the same primitive in all four payload paths:

1. raw chunk notification;
2. decoded frame notification;
3. decoder `flush_partial()` notification;
4. shaped match-context `data` in final stop notification.

On successful fallback:

- emit normal chunk/frame/partial/stop-context payload;
- set that payload's `encoding` to `"hex"`;
- emit `tracing::warn!` naming connection, requested encoding, and original
  reason;
- count represented raw bytes in `total_returned` exactly as normal success;
- do not call `record_notification_drop`, do not write a
  `notification_dropped` log event, do not increment `frames_dropped`, and do
  not emit `SubscribeEncodingErrorNotification`.

For final stop match context, add
`encoding: Option<String>` to `SubscribeStopNotification`, serialized only when
`data` is present. Set `data` and `encoding` together to the shaped context's
effective values. This is a minimal additive notification field needed to make
hex data decodable; it is not a tool input/output schema change. Update
serialization/schema tests.

If encode+hex truly fails, preserve existing failure semantics as closely as
possible: raw path may emit existing `SubscribeEncodingErrorNotification` and
count a drop; frame/partial/context loss must warn and be counted rather than
silently presented as success. Keep the error-notification struct and document
that successful hex fallback does not use it.

Do not use event-log `notification_dropped` as the warning channel for a
successful fallback; that would corrupt observable drop accounting.

### Type/docs corrections

Document every `encoding` field as effective encoding: requested value on direct
success, `hex` after fallback. Correct `ReadResult.frames_dropped` wording so
successful fallback is not described as a drop. Document stop `encoding`/`data`
pair and retained true-failure error notification.

README must state lossless RX fallback in one discoverable location. Update
`docs/protocols.md` field reference: successful per-frame encoding fallback is
emitted and not counted; only fallback failure can count as encoding drop.
Update AGENTS framing/pipeline truth for read+subscribe parity. In FEATURES,
replace combined encoding/matcher debt section with matcher-only debt; do not
remove matcher work before Phase 5.

## Required public behavior proofs

Add/extend tests proving:

1. raw `read` requested as UTF-8 returns exact spaced hex and
   `encoding="hex"` for invalid bytes;
2. raw `subscribe` emits exact spaced hex and `encoding="hex"`, advances rather
   than repeating chunk, and leaves `notification_drop_count == 0`;
3. binary framed subscribe emits normal frame notification with exact hex data,
   effective hex encoding, and no drop count;
4. binary partial-frame flush emits exact hex with effective encoding;
5. framing-error read/capture preserves raw malformed bytes as hex and diagnostic
   while independently valid prior frame stays in requested UTF-8;
6. matched binary context reports both hex `data` and `encoding="hex"`;
7. successful fallback does not increment `frames_dropped` or notification-drop
   count.

Use smallest existing HTTP loopback/controlled harness and PTY setup. Include
tests in both `tests/http_integration.rs` and `tests/serial_pty.rs`; assert wire
payloads, not private helper calls. Keep tests bounded and unsubscribe/cancel
cleanly.

## Required verification

Run:

```bash
cargo fmt --all -- --check
cargo test --locked --lib codec
cargo test --locked --test http_integration
cargo test --locked --test serial_pty
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/phase-4-agent-eval
git diff --check
```

Inspect evaluator against `target/pre-refinement-agent-eval`: tool count must
remain 27 and no tool input/output schema change may result from notification
types. Do not commit evaluator output.

Inspect status/diff/log. Confirm no `from_utf8_lossy` was added in RX encoding
paths, package version remains 0.9.0, and only successful emitted bytes contribute
to returned counters.

## Commit and recap

Stage only Phase 4 files and this handoff. Commit with:

```text
fix: preserve RX bytes with shared encoding fallback
```

Return files/behavior changed, public wire impact, tests and evaluator metrics,
commit hash/message, blockers, deviations, and Phase 5 follow-up. Do not push,
merge, open a PR, or amend.
