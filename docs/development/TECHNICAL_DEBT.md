# Technical Debt

## Testing

### TX output flush semantics — remaining hardware-only gap

Current issue:
- Two of the three TX flush cases named in the original debt item are now
  covered by `tests/native_sim_validation.rs`:
  - **fully delivered TX** — `native_flush_output_after_full_delivery_is_safe`
    verifies a completed ping→pong exchange is unaffected by a later
    `flush(target="output")`, and that a subsequent independent write still
    lands.
  - **partially queued TX** — `native_partial_line_buffered_then_completed`
    verifies a command written without a line terminator is held in firmware's
    `cmd_buf` (no execution), and that writing the remainder plus terminator
    drives the assembled command to completion.
- The third case, **flushed-before-delivery TX**, is not coverable on
  `native_sim` and remains a hardware-only gap. A PTY delivers every
  `write()` byte into its kernel buffer immediately, so the host's
  `tcflush(TCOFLUSH)` (which is what `flush(target="output")` calls on Linux
  via `serialport`) cannot recall bytes that have already left serialport's
  output buffer. Simulating this case requires real hardware with flow
  control (deassert RTS so the device stops reading, then flush).

Why it matters:
- The `native_sim` PTY cannot reproduce the "undrained TX in the device-side
  buffer" state that `flush(output)` is meant to discard.
- A hardware-backed test or a mock `SerialIo` that models an OS TX queue
  would be needed to close the gap.

Likely follow-up (out of scope for native_sim):
- add a hardware-only test (gated by a real-port feature flag) that uses RTS
  flow control to hold device RX, issues `flush(target="output")`, then
  releases RTS and asserts the flushed command never executed; or
- introduce a mock `SerialIo` backend with a configurable OS-output buffer
  that `clear_os_buffers(Output)` actually drains, and unit-test the
  flush-before-delivery path against that mock.