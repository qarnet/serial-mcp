# Feature Ideas

## Richer matching modes

- Add optional regex matching for `read` and `subscribe` match config.
- Add optional glob-style matching for `read` and `subscribe` match config.
- Keep current default behavior as literal byte-substring matching for backward compatibility.
- Likely shape: add `match_mode` with values like `substring` (default), `regex`, `glob`.
- Add smarter context slicing around regex/glob matches once richer matching exists.

## Multiple public subscriptions per connection

- Consider allowing more than one public `subscribe` on the same connection.
- Possible use cases:
  - one raw log stream plus one match watcher
  - multiple different match patterns at once
  - long-lived monitor plus short-lived diagnostic stream
- Likely requires:
  - subscription IDs
  - unsubscribe by subscription ID
  - clear budget accounting per subscription
  - defined fanout/duplication semantics
- Simpler alternative to evaluate later:
  - keep one public subscription per connection but allow reconfiguration/update
    semantics instead of full replacement

## native_sim test firmware diagnostics

- Keep and evolve the test firmware features that make TX/RX behavior
  observable from a software-only test harness (native_sim + USB/IP).
- Existing useful diagnostics now include:
  - `framing on|off` for committed line visibility
  - `trace on|off` for per-byte RX ordering visibility
  - `write cmd <id> <rest>` for command-ID based ordering/drop checks
  - `arm_cmd <delay_ms>` and `slow on|off [<us>]` for timing and backpressure experiments
- These firmware-side features let software tests prove split-write
  ordering and exact line assembly, not just successful command
  execution.

## Stronger live TX flush testing support

- Add firmware support for true RX holdoff or throttle so host-side queued TX can remain undelivered long enough to test `flush(target="output")` on real hardware.
- Candidate firmware capabilities:
  - `rx_throttle on|off`
  - explicit UART FIFO drain pause/resume
  - host-visible counters for delivered RX bytes vs buffered-but-uncommitted bytes
- Goal: enable hardware tests that distinguish:
  - fully delivered TX
  - partially delivered TX
  - flushed-before-delivery TX
  - exact delivered prefix after close or flush
