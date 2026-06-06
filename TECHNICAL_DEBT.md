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
