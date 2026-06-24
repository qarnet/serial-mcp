# Technical Debt

No open items.

## Resolved

### TX output flush semantics — closed 2026-06-24

All three TX flush cases from the original debt item are now covered:

- **fully delivered TX** —
  `tests/native_sim_validation.rs::native_flush_output_after_full_delivery_is_safe`
  verifies a completed ping→pong exchange is unaffected by a later
  `flush(target="output")`, and that a subsequent independent write still
  lands.
- **partially queued TX** —
  `tests/native_sim_validation.rs::native_partial_line_buffered_then_completed`
  verifies a command written without a line terminator is held in firmware's
  `cmd_buf` (no execution), and that writing the remainder plus terminator
  drives the assembled command to completion.
- **flushed-before-delivery TX** —
  `src/tx_session.rs` unit tests `tx_flush_output_drops_undrained_queued_tx`,
  `tx_flush_output_on_empty_queue_does_not_drop_later_write`, and
  `tx_flush_output_after_drain_does_not_recall_delivered` cover this via a
  new `QueuedTxIo` mock `SerialIo` backend in `src/serial.rs::test_support`.
  The mock models an OS transmit queue: written bytes land in the queue, and
  `clear_os_buffers(Output)` (the backend behind `flush(target=output)`)
  discards bytes still queued before the device drains them. This is the
  state a `native_sim` PTY cannot reproduce because a PTY delivers every
  `write()` byte into its kernel buffer immediately.