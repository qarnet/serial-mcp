# Technical Debt

## RX cleanup after PLAN 1 and PLAN 2

### 1. Remove or refactor obsolete direct-read helpers

Current issue:
- Old direct-read helpers still exist in `src/tools/helpers.rs`.
- Public RX tools no longer use them.
- Their semantics now diverge from unified `RxSession`-based RX behavior.

Why it matters:
- Confusing for future maintainers.
- Easy source of accidental regressions if reused later.

Likely follow-up:
- delete unused helpers, or
- rewrite them as thin wrappers over session-based helpers if tests still need shared logic.

### 2. Simplify subscribe final-stop payload fields

Current issue:
- Final subscribe stop payload includes overlapping counters such as `bytes_read`,
  `bytes_observed`, and `bytes_returned`.

Why it matters:
- Redundant fields make agent behavior and docs harder to reason about.
- Future plans may add more metadata and increase confusion.

Likely follow-up:
- decide canonical field set for subscribe stop notifications,
- remove or deprecate overlapping counters once compatibility plan is clear.

### 3. Update docs to match unified RX + metadata model

Current issue:
- README, tool descriptions, and related docs may not fully explain:
  - unified `RxSession` RX model
  - future-only semantics
  - `stop_reason`
  - `truncated`
  - `bytes_observed` / `bytes_returned`

Why it matters:
- Agents depend on precise semantics.
- Stale docs cause wrong tool usage patterns.

Likely follow-up:
- full documentation pass after PLAN 3/4 settle naming and semantics.

### 4. Reduce duplicated `tx_session` test coverage

Current issue:
- `src/tx_session.rs` unit tests and `tests/tx_session.rs` integration tests cover many same cases.
- Coverage overlap includes write ordering, close behavior, idempotent session creation, and active RX pump coexistence.

Why it matters:
- Duplicate assertions increase maintenance cost when TX behavior changes.
- Similar failures in two layers can add noise without adding much diagnostic value.

Likely follow-up:
- keep low-level behavior checks close to `src/tx_session.rs`,
- keep only tool-surface or cross-module wiring coverage in `tests/tx_session.rs`.

### 5. Add better live coverage for TX output flush semantics

Current issue:
- PTY and unit tests cover TX ordering and basic output flush behavior.
- The current `native_sim` firmware does not expose command-processing states (e.g. partial-line buffering or delayed command commit) that make `flush(target="output")` semantics fully testable in software.

Why it matters:
- New `TxSession` architecture now owns serialized writes and output-side flush behavior.
- Tests still do not directly prove what happens to queued partial TX data on a serial device.

Likely follow-up:
- extend test firmware with commands that expose partial-line buffering or delayed command commit,
- add tests that can distinguish fully delivered TX, partially queued TX, and flushed-before-delivery TX.
