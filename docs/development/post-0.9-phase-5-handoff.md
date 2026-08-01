# Post-0.9 Refinement — Phase 5 Handoff

## Role and delivery constraint

Implement Phase 5 on existing branch `refactor/post-0.9-refinement`. Follow
canonical plan and repository `AGENTS.md`. Commit completed work before
returning. Do not create/push another branch or PR, amend, or add attribution.

## Goal

Give raw `read` and `subscribe` one matcher-owned bounded-window policy while
preserving cross-chunk literal detection, global match indexes, context shaping,
regex/glob behavior within documented retained window, and per-frame matching.

## In scope

- Matcher-owned bounded push/retention API in `src/match_config.rs`.
- Use API for initial-history raw read, live raw read, and raw subscribe.
- Correct global indexes after front truncation.
- Bound and correctly shape literal pre-match context at truncation boundary.
- Characterization/unit/public-boundary tests for literal/regex/glob/context and
  read/subscribe parity.
- Remove completed matcher debt and document exact retained-window policy.
- Commit this handoff with Phase 5 work.

## Out of scope

- No wire fields, tools, resources, match request shapes, defaults, or limits.
- No change to framed semantics: each decoded frame resets matcher, and patterns
  never span frame boundaries.
- No promise that arbitrary unbounded regexes or arbitrarily long glob lines can
  match after more bytes than bounded retained policy allows.
- No encoding changes or module movement.

## Grounding evidence

- `Matcher` currently stores only a `Vec<u8>` and says callers own truncation.
- Raw subscribe manually truncates after `push` to
  `max(max_buffered_bytes, literal needle + 1 or 256)`; read never calls that
  policy directly.
- `truncate_front` does not track dropped prefix. A later `Found(index)` is
  window-local but callers treat it as stream-global, corrupting match indexes
  after subscription truncation.
- Subscribe's separate `accumulated` context buffer retains the first
  `max_buffered_bytes`, not the latest bytes. A later match can index outside it
  or return wrong context.
- Read inherently caps returned bytes at `max_bytes`, but Phase 5 still requires
  same matcher API at initial-history and live paths so policy cannot drift.
- `consume_frames` resets matcher per frame and must keep using frame-local
  unbounded `push` + reset behavior.

## Exact implementation decisions

### Matcher state and indexes

Track absolute bytes discarded from front of each matcher window (field name is
implementation choice). `Matcher::check`, `Matcher::push`, and bounded push must
return `MatchResult::Found` relative to total bytes fed since last
`reset_window`, not relative to truncated vector. `truncate_front` must advance
the base by exactly bytes removed. `reset_window` clears bytes and resets base to
zero so framed matches remain frame-local.

Update existing truncate test: after seven bytes, dropping four, then appending a
three-byte match, returned index is global 7, not local 3.

### One bounded-window policy API

Put policy in `match_config.rs`, not tools. Add APIs equivalent to:

```rust
pub fn retained_window_limit(&self, max_buffered_bytes: usize) -> usize;
pub fn push_bounded(
    &mut self,
    chunk: &[u8],
    max_buffered_bytes: usize,
) -> MatchResult;
```

`push_bounded` appends/checks combined data first (so boundary matches survive),
captures global result, then enforces retention before return. Retained length
after call must never exceed computed limit, including after `NoMatch`.

Policy:

- literal overlap allowance = `needle.len().saturating_sub(1)`;
- regex/glob conservative overlap allowance = 256 bytes (preserves existing
  subscribe heuristic; expose as named private or `pub(crate)` constant);
- retained limit = `max_buffered_bytes.saturating_add(overlap_allowance)`;
- use checked/saturating arithmetic; no allocation based on unbounded addition.

This gives one documented cap plus required cross-boundary overlap. A single
incoming ring chunk is already bounded by connection `max_buffered_bytes`.
Transient append may reach previous retained limit + one bounded chunk; retained
state after API return must meet limit.

For glob truncation, avoid false whole-line matches from a retained suffix that
starts mid-line. Track whether first retained line is partial (or truncate to a
safe newline boundary) and do not treat an incomplete prefix as a complete line.
Keep CRLF stripping and complete/last-current-line behavior unchanged when no
truncation occurred.

### Context shaping

Delete raw subscribe's first-bytes `accumulated` strategy. Add matcher-owned
literal context shaping over current retained window, equivalent to:

```rust
pub fn shape_literal_match_context(
    &self,
    global_match_index: usize,
) -> Option<ShapedMatchPayload>;
```

It returns `Some` only when literal matcher has configured context and index
matches most recent bounded `Found`. Implement this mechanically by storing an
optional `last_bounded_match_context` in literal matcher state. Each bounded
push clears old saved context; on `Found`, translate global index through window
base and compute/save shaped payload **before** enforcing retention. Then enforce
retention and return global `Found`. Accessor returns clone of saved payload for
matching index. Clear it on reset. Regex/glob store no shaped context.

Return up to requested context actually retained; request values are not
rewritten. Because retained limit includes `max_buffered_bytes` plus literal
overlap, normal match after bounded pushes retains at least
`min(requested context, max_buffered_bytes)` immediately preceding bytes plus
matched literal when available.

Use matcher-owned shaping for raw subscribe final matched context. For raw read:

- initial-history context remains exact;
- live match path must apply same context shaping (currently it returns whole
  buffered result despite context request);
- preserve cursor consumption/offset semantics: this phase changes returned
  shaping and relative `match_index`, not already-consumed ring bytes.

Regex/glob context behavior stays unchanged (no fixed match length API today).
Framed read/subscribe behavior stays frame-local and unchanged.

### Tool integration

Replace raw calls as follows:

- initial-history read: `push_bounded(hist, max_bytes)`;
- live raw read: `push_bounded(chunk, max_bytes)`;
- raw subscribe: `push_bounded(chunk, max_buffered_bytes)` and remove manual
  `needle_len/256`, `cap`, and `truncate_front` block.

Do not use bounded push in `rx_consume.rs`; keep per-frame push/reset.

### Tests

Unit tests in `match_config.rs` must prove:

- literal spanning two chunks at retention boundary matches global index;
- repeated no-match pushes leave `len() <= retained_window_limit`;
- truncation advances global index correctly;
- zero/one-byte literal overlap has no underflow;
- context at exact truncation boundary returns exact bytes and relative index;
- regex and glob still match across normal chunk splits within retained cap;
- glob truncated mid-line does not create false whole-line match;
- reset restores frame-local index zero.

Public tests in HTTP + PTY must prove:

- raw read and raw subscribe find same literal over same chunked sequence and
  report same match/no-match outcome and correct index;
- literal cross-chunk match survives boundary;
- context returns exactly requested retained bytes plus match with relative
  `match_index`;
- regex and glob existing behavior remains green.

Keep tests bounded. Use small live connection `max_buffered_bytes` through
connection-mode `configure` or existing test builder when possible; do not add a
wire option removed from read/subscribe.

## Documentation

Update Matcher module docs and AGENTS with exact cap/overlap/global-index policy.
Remove completed matcher-only parity debt from FEATURES. README changes only if
existing match teaching would otherwise be misleading. No user-facing claim of
unbounded regex/glob history.

## Required verification

Run:

```bash
cargo fmt --all -- --check
cargo test --locked --lib match_config
cargo test --locked --test http_integration
cargo test --locked --test serial_pty
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/phase-5-agent-eval
git diff --check
```

Compare evaluator to Phase 4: 27 tools, no description or input/output schema
change. Do not commit evaluator output. Inspect status/diff/log and confirm no
matcher path outside framed `rx_consume` uses ad hoc truncation.

## Commit and recap

Stage only Phase 5 files and this handoff. Commit with:

```text
fix: unify read and subscribe matcher bounds
```

Return exact policy/API, files and behavior, tests/evaluator metrics, commit
hash/message, blockers, deviations, and Phase 6 follow-up. Do not push, merge,
open a PR, or amend.
