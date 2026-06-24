# Technical Debt

## Testing

### Add better live coverage for TX output flush semantics

Current issue:
- PTY and unit tests cover TX ordering and basic output flush behavior.
- The current `native_sim` firmware does not expose command-processing states (e.g. partial-line buffering or delayed command commit) that make `flush(target="output")` semantics fully testable in software.

Why it matters:
- New `TxSession` architecture now owns serialized writes and output-side flush behavior.
- Tests still do not directly prove what happens to queued partial TX data on a serial device.

Likely follow-up:
- extend test firmware with commands that expose partial-line buffering or delayed command commit,
- add tests that can distinguish fully delivered TX, partially queued TX, and flushed-before-delivery TX.