# Cleanup

## Goal

Clean up technical debt created while landing the RX redesign and follow-up
features.

This plan is not for new features. It is for:
- fixing memory/accounting mismatches
- removing obsolete compatibility code where safe
- aligning schemas/docs/tests with the final RX model
- carrying forward unfinished cleanup work from the old phased plans

## Current Public RX Reality

- `read` is the main blocking RX tool
- `subscribe` is the main background RX tool
- `wait_for` is deprecated compatibility surface and should be removed
- matching, stop-reasons, buffer budgets, silence timeout, and simple pre-match
  context already exist

## Current Subscription Semantics

- Internally, `RxSession` can support multiple streaming consumers.
- Publicly, the server currently allows only **one active `subscribe` per
  connection**, because `StreamRegistry` is keyed by `connection_id` and a new
  subscribe replaces the old one.
- This cleanup plan should preserve or intentionally revise that rule explicitly.

## Highest Priority Findings

### 1. Fix `subscribe(match=...)` memory growth beyond reserved budget

Problem:
- `subscribe(match=...)` keeps a bounded `accumulated` buffer, but matcher state can
  still grow unbounded if never pruned.
- Actual memory use can exceed reserved `max_buffered_bytes`.

Likely fix:
- ensure matcher keeps only the minimal tail needed for future match detection
- keep context payload accumulation separately bounded
- make memory/accounting match the reserved budget

### 2. Fix subscription replacement budget ordering

Problem:
- replacing an active subscribe can fail spuriously because new reservation is
  attempted before old reservation is released.

Chosen direction:
- drop old subscription first
- release old reservation first
- then reserve/start new one

Reason:
- deterministic budget behavior
- no spurious exhaustion when replacing same connection
- tiny no-subscription gap is acceptable

### 3. Fix subscribe final stop metadata counters

Problem:
- final subscribe notification may report inconsistent `bytes_read`,
  `bytes_observed`, and `bytes_returned`.

Likely fix:
- decide canonical field semantics
- make final payload counts internally consistent

## Medium Priority Findings

### 4. Align schema with runtime for `no_new_rx_timeout_ms`

Problem:
- schema appears to allow `0`
- runtime rejects `0`

Likely fix:
- update schema helper or field schema so generated schemas match runtime truth

### 5. Remove deprecated `wait_for` completely

Problem:
- `wait_for` wrapper still duplicates some validation/mapping and intentionally
  preserves old timeout-as-error behavior
- this is drift-prone and no longer needed as primary API

Likely fix:
- remove deprecated tool surface
- remove deprecated `WaitForResult` / wrapper-specific code where possible
- migrate any remaining tests/docs/examples fully to `read(match=...)` and
  `subscribe(match=...)`

### 6. Remove obsolete RX helpers

Problem:
- old direct-read / old wait / old stream helper functions remain in tree
- tool paths no longer use them

Likely fix:
- delete dead helpers
- keep only session-based helpers and their tests

### 7. Simplify subscribe public semantics if needed

Problem:
- current public API only allows one active subscribe per connection even though
  internals could support more than one consumer

Need explicit decision:
- keep one-subscription-per-connection public rule, or
- expose multiple concurrent public subscriptions on the same connection later

For cleanup now, recommendation:
- keep one-subscription-per-connection public rule
- document it clearly
- do not widen public semantics during cleanup

## Lower Priority Cleanup

### 8. Update public docs to final RX semantics

Examples:
- README still over-emphasizes `wait_for`
- matching docs should prefer `read(match=...)` / `subscribe(match=...)`
- explain `max_buffered_bytes`, `stop_reason`, `matched`, `match_index`,
  `context_amount_of_matched_bytes`, and `no_new_rx_timeout_ms`

### 9. Tighten tests to current semantics

Examples:
- proptest coverage should include newer stop reasons like `no_new_rx_timeout`
- tests should not keep validating old impossible inline subscribe result shapes
- wrapper-specific tests should be isolated or removed once wrapper goes away

### 10. Revisit duplicate payload counters

Examples:
- subscribe final notification currently mixes overlapping counters
- decide whether `bytes_read` should remain at all once `bytes_observed` and
  `bytes_returned` are stable

### 11. Align plan/docs naming with final state

Examples:
- remove references to migration phases now that core plans are finished
- make docs describe final RX model instead of transition history

## Scope Boundaries

In scope:
- bug fixes
- debt removal
- schema/doc/test alignment
- deprecated API removal

Out of scope:
- new match modes
- new event-context modes
- new RX tool types
- new budgeting features
- widening public subscribe semantics to multiple concurrent subscriptions

## Open Cleanup Questions

### Subscribe matcher memory shape

Need to decide whether subscribe matching should keep:
- one bounded accumulated payload buffer
- plus one minimal matcher tail buffer

Recommendation:
- yes
- accumulated payload buffer should stay bounded by reservation/context rules
- matcher tail should keep only what is needed to detect matches across chunk
  boundaries

### Subscribe final payload counters

Need to decide canonical final payload counters.

Recommendation:
- keep only one clear semantic for each counter
- prefer `bytes_observed` and `bytes_returned`
- drop `bytes_read` later if it is only confusing duplicate meaning

## Documentation Phase

At end of cleanup, docs should describe the final RX model rather than the
migration journey.

## Testing Phase

Cover:
- no matcher memory growth beyond budget
- replacement subscribe does not fail spuriously
- final subscribe counters are internally consistent
- schemas match runtime validation
- obsolete helper removal does not reduce coverage
- deprecated `wait_for` removal does not leave stale docs/tests behind
