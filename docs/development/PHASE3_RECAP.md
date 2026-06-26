# Phase 3 Recap — SLIP framing (TX + RX)

**Date:** 2026-06-26
**Source plan:** `docs/development/PHASE3_SLIP_HANDOFF.md`
**Status:** Complete

---

## Summary

Added SLIP (RFC 1055) as a symmetric framing mode on both `write` (TX) and
`read`/`subscribe` (RX). Byte-stuffs payloads between END markers so binary
payloads can be transported without ambiguity. Introduced the first runtime
decode error path (malformed escape → `FrameDecodeError`), a new
`framing_error` stop reason, and changed `FrameDecoder::push()` signature from
`Vec<Frame>` to `Result<Vec<Frame>, FrameDecodeError>`.

---

## SLIP constants and codec

Defined in `src/framing.rs`:

| Constant | Value | Meaning |
|----------|-------|---------|
| `SLIP_END` | `0xC0` | Frame delimiter |
| `SLIP_ESC` | `0xDB` | Escape introducer |
| `SLIP_ESC_END` | `0xDC` | Escaped END |
| `SLIP_ESC_ESC` | `0xDD` | Escaped ESC |

- `slip_stuff(payload)`: replaces `0xC0` → `ESC ESC_END`, `0xDB` → `ESC ESC_ESC`.
  Used by TX encode; payload bytes pass through unchanged.
- RX unstuffing is streaming (inside `slip_decode` free function), not
  whole-payload.

---

## TX framing

`TxFramingMode::Slip` (unit variant, `{"type": "slip"}`). Encode wraps payload
as `END + stuffed(payload) + END`. No configurable parameters.

---

## RX framing — SlipState decoder

`RxFramingMode::Slip` → `DecoderMode::Slip { state: SlipState }`.

State machine:

| State | Behavior |
|-------|----------|
| `BeforeFirstEnd` | Discard bytes until END seen, then → `InFrame` |
| `InFrame { buf, escaped }` | Accumulate decoded payload. `END` → emit frame. `ESC` → set `escaped`. `ESC`+`ESC_END` → push `0xC0`. `ESC`+`ESC_ESC` → push `0xDB`. `ESC`+other → `Err(FrameDecodeError::SlipInvalidEscape(b))`. Normal byte → push to `buf` |
| Truncated ESC | `ESC` as last byte sets `escaped = true`. Next chunk resolves. `flush_partial` emits `buf` as a raw partial frame |

`flush_partial` updated to drain the in-frame buffer for SLIP (other decoders
use `self.buf`).

---

## push() signature change

`pub fn push(&mut self, chunk: &[u8]) -> Vec<Frame>` →
`pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, FrameDecodeError>`

All non-SLIP decoders return `Ok(frames)` (no behavior change). SLIP can return
`Err`. Blast radius:

| File | Change |
|------|--------|
| `src/framing.rs` | Signature + `Ok(frames)` wrap + SLIP arm |
| `src/tools/rx_consume.rs` | `consume_frames` unwraps `push()` result; new `FrameOutcome::DecodeError` variant |
| `src/tools/helpers.rs` | `read_bytes_via_session` handles `DecodeError` → `return Err(error_message)` |
| `src/tools/stream_ops.rs` | `stream_rx_via_session` handles `DecodeError` → sets `stop_outcome` with `framing_error` + saves error message for stop notification |
| All framing tests | `dec.push(...)` → `dec.push(...).unwrap()` (69 call sites) |

---

## New stop reason: `framing_error`

- `RxStopReason::FramingError` added to `src/rx_metadata.rs`.
- `RxStopMetadata::framing_error(bytes_observed)` constructor.
- `RxStopController::framing_error(error)` outcome builder.
- `is_normal_stop(RxStopReason::FramingError)` → `false` (it is an error, not a normal stop).
- `ReadResult.stop_reason` doc comment updated to list `framing_error`.
- Read path: `DecodeError` → `Err` from `read_bytes_via_session` → `io_ops::read` returns tool result with `is_error: true`.
- Subscribe path: `DecodeError` → final notification with `stop_reason: "framing_error"` and `error: "SLIP framing error: invalid escape byte 0x41"`.

---

## Files changed

| File | Changes |
|------|---------|
| `src/framing.rs` | SLIP constants, `slip_stuff`, `FrameDecodeError`, `SlipState`, `DecoderMode::Slip`, `slip_decode` free function, SLIP arms in `new()`/`push()`/`encode()`/`frame_type_str()`/`flush_partial()`, push() → Result |
| `src/tools/rx_consume.rs` | `FrameOutcome::DecodeError`, `consume_frames` unwraps push() |
| `src/tools/helpers.rs` | Handle `DecodeError` in read loop, SLIP read integration tests |
| `src/tools/stream_ops.rs` | Handle `DecodeError` in subscribe loop, `error` field in stop notification |
| `src/rx_metadata.rs` | `RxStopReason::FramingError`, `framing_error()` constructor |
| `src/stop_controller.rs` | `framing_error()` outcome builder, `is_normal_stop` test |
| `src/tools/types.rs` | `stop_reason` doc comment updated |
| `src/server.rs` | Tool descriptions updated to mention SLIP and `framing_error` |
| `tests/proptest.rs` | SLIP added to `rx_framing_config_roundtrip_all_modes` and `write_args_with_tx_framing_roundtrip` |

---

## Tests added

- **15 SLIP unit tests** in `src/framing.rs`: `slip_stuff_replaces_end_and_esc`,
  `tx_slip_encodes_end_end`, `tx_slip_stuffs_payload_with_end`,
  `tx_slip_stuffs_payload_with_esc`, `rx_slip_skips_to_first_end`,
  `rx_slip_decodes_basic_frame`, `rx_slip_decodes_esc_end`,
  `rx_slip_decodes_esc_esc`, `rx_slip_malformed_escape_returns_err`,
  `rx_slip_two_frames_in_one_chunk`, `rx_slip_cross_chunk_frame`,
  `rx_slip_truncated_escape_holds_pending`, `rx_slip_flush_partial_emits_pending`,
  `roundtrip_slip_arbitrary_binary`, `roundtrip_slip_empty_payload`.
- **2 read integration tests** in `src/tools/helpers.rs`:
  `char_framing_slip_decode_success`, `char_framing_slip_malformed_surfaces_error`.
- **1 stop_controller test** in `src/stop_controller.rs`:
  `framing_error_outcome_not_normal_stop`.
- **Proptest updates**: SLIP added to both `rx_framing_config_roundtrip_all_modes`
  and `write_args_with_tx_framing_roundtrip`.

---

## Gate results

| Command | Result |
|---------|--------|
| `cargo fmt --all -- --check` | PASS |
| `cargo build --all-targets --locked` | PASS |
| `cargo test --locked` | PASS (368 lib + all integration) |
| `cargo clippy --all-targets --locked -- -D warnings` | PASS |

---

## Scope decisions

- **`slip_decode` is a free function** (not a method on `FrameDecoder`) to avoid
  borrow conflicts: `push()` borrows `self.mode` mutably, and the SLIP decoder
  needs access to `self.buf`, `self.frame_count`, and `self.parser`.
  `slip_decode(buf, frame_count, parser, mode)` borrows each field separately.
- **`flush_partial` updated for SLIP.** Drains the in-frame buffer from
  `SlipState::InFrame` rather than `self.buf`. Other decoders unchanged.
- **No subscribe integration test (PTY-level)**. The SLIP PTY test is deferred
  — the read integration tests cover the `consume_frames` → `DecodeError` path,
  and the subscribe loop's `DecodeError` handling follows the same
  `FrameOutcome` variant.
- **Malformed escape discards frame, resyncs for safety.** After error, state
  transitions to `BeforeFirstEnd` (skips to next END). Phase 3 stops on first
  error, but if the caller ever continues, the decoder is in a clean state.

---

## Out of scope

- SLIP variants other than RFC 1055 classic.
- SLIP checksum/CRC extension.
- Compressed SLIP (CSLIP).
- Automatic resume-on-error (stops on first error; resync state is defensive).
- Profile defaults for SLIP (Phase 5).
