# PLAN 5 - Add Event Context

## Goal

Separate search buffering from returned event context so pattern-based tools can return the most useful bytes around a match.

## Initial Scope

Primary tool:
- `wait_for`

Shared internal design should anticipate:
- future `read_until`

## Proposed Request Fields

- `match_context_mode`
  - `before_match`
  - `after_match`
  - `full_match_context`
- `context_amount_of_matched_bytes`

## Semantics

- `max_buffered_bytes` controls how much data tool may scan/buffer in total
- `context_amount_of_matched_bytes` controls how much surrounding data gets returned
- return shaping should be deterministic and documented

## Scope Boundaries

In scope:
- request/response schema
- return slicing rules for literal substring matching
- shared internal helper structure for future pattern/event tools

Out of scope:
- regex-aware slicing
- glob-aware slicing
- ring-buffer defaults

## Documentation Phase

Explain clearly:
- search budget vs returned context budget
- meaning of each context mode
- truncation behavior when context larger than available budget

## Testing Phase

Cover:
- `before_match`
- `after_match`
- `full_match_context`
- context truncation at beginning/end
- interaction with `max_buffered_bytes`
