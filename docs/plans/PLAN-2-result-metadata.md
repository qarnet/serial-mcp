# PLAN 2 - Add Result Metadata

## Goal

Give agents clear structured metadata explaining why RX operations stopped and whether results were truncated.

## Scope

Tools:
- `read`
- `wait_for`
- `subscribe` start ack and final stop info where possible

Likely metadata fields:
- `stop_reason`
- `truncated`
- `bytes_observed`
- `bytes_returned`
- `buffer_limit_hit`
- `match_found`
- `match_index`
- `timeout_ms`
- `no_new_rx_timeout_ms`

## Scope Boundaries

In scope:
- response schema additions
- deterministic stop-reason mapping
- docs/tests

Out of scope:
- implementing buffer pool itself
- regex matching
- event-context return shaping

## Documentation Phase

Document stop reasons and truncation semantics in:
- README
- tool descriptions
- schemas/comments in `src/tools/types.rs`

## Testing Phase

Cover:
- timeout stop reason
- max-buffer stop reason
- match-found stop reason
- close/cancel stop reason
- truncation flags accurate

## Notes

- subscribe final metadata may arrive through final notification, not tool result, if PLAN 1 lands first
