# Post-0.9 Refinement — Phase 6C RX Tool Split Handoff

## Role and delivery constraint

Perform final mechanical Phase 6 split on current branch. No behavior, wire,
schema, tool-catalog, error, cursor, matcher, framing, or encoding change.
Commit before returning. No push, merge, PR, amend, or attribution.

## Goal

Move RX validation, ring read loop, and read result construction out of
`src/tools/helpers.rs` into focused modules while keeping general validation,
connection/open settings, parser helpers, and shared limits in helpers.

## Target files

```text
src/tools/
  helpers.rs
  rx_validate.rs
  read_loop.rs
  result_builders.rs
```

Declare three new modules in `src/tools/mod.rs`.

## Exact ownership

### `rx_validate.rs`

Move exactly:

- `RxLimits`;
- `ResolvedRxArgs`;
- `RxRequestArgs` trait + ReadArgs/SubscribeArgs impls;
- `validate_rx_request`;
- its dedicated test args/helpers and seven validation tests.

Import shared primitives (`clamp_or_err`, `require_min_or_err`, timeout clamp,
`lookup_connection`, `parse_encoding`, limits) from helpers. Update `io_ops.rs`
and `stream_ops.rs` to import these RX-specific symbols directly from
`rx_validate`, not via a compatibility facade.

### `read_loop.rs`

Move exactly:

- `ReadOutcome`;
- private `ReadFrameSink` + trait impl;
- `advance_private_cursor`;
- `read_from_private_cursor`;
- `read_bytes_from_ring` shared-cursor wrapper;
- cancellation/private-cursor tests and their local fixtures/imports.

Preserve visibility: `ReadOutcome` and shared wrapper usable by sibling tools;
private cursor core remains `pub(crate)` because `capture_boot` calls it. Update
`io_ops.rs` and `control_ops.rs` to direct imports from `read_loop`.

Keep `#[allow(clippy::too_many_arguments)]`, cancellation flow, cursor updates,
frame sink behavior, matcher policy, all return branches, and test bodies
unchanged except imports.

### `result_builders.rs`

Move `build_read_result` and its five focused tests. Import `ReadOutcome` from
`read_loop`, codec/types directly, and shared timeout constant from helpers.
Update `io_ops.rs` and `control_ops.rs` to import builder directly.

### What remains in `helpers.rs`

- shared limit re-exports and `DEFAULT_READ_TIMEOUT_MS`;
- clamp/min/poll/timeout helpers;
- budget-error mapping;
- connection lookup;
- `find_subslice` util alias used by matcher;
- encoding parser;
- `OpenOverlay`, `ResolvedOpenSettings`, open-arg parsing/validation;
- generic tool error formatter;
- tests for those remaining symbols.

Do not re-export moved symbols back through helpers unless an external public
API requires it (none should; `tools` is crate-internal organization). Update
all crate call sites to direct owner modules so boundaries are real and no
helpers↔new-module facade cycle remains.

## Tests and path comments

- Preserve every pre-split helpers `#[test]`/`#[tokio::test]` exactly once.
  Compare names/count before and after; no duplicate/omission.
- Update active references:
  - AGENTS private-cursor/cancellation paths → `src/tools/read_loop.rs`;
  - `docs/development/agent-interface-evaluation.md` unit-test path;
  - `src/tools/rx_consume.rs` shared read-loop comment;
  - HTTP integration comment pointing at helper unit test;
  - server extraction header should describe split tool modules accurately.
- Replace completed helper-split FEATURES debt with narrower future debt for
  decomposing long `read_from_private_cursor` (`read_loop.rs`) and
  `stream_rx_from_ring` (`stream_ops.rs`). Do not perform that decomposition in
  this mechanical commit.
- Canonical plan/historical handoffs and CHANGELOG may retain old path context.

## Mechanical constraints

- Move bodies/tests verbatim except imports/module docs/path comments.
- No compatibility re-export layer from helpers unless compilation proves a
  consumer outside crate requires it; integration tests use public MCP surface,
  not helper paths.
- No algorithm cleanup or changed visibility beyond sibling access required.
- No unrelated formatting churn.

## Verification

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked --lib
cargo test --locked --test http_integration
cargo test --locked --test serial_pty
cargo test --locked --test stdio_integration
cargo test --locked --test doc_drift
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/phase-6c-agent-eval
git diff --check
```

Compare evaluator byte-for-byte to Phase 6B: 27 tools and identical catalog.
Do not commit evaluator output. Inspect direct imports, status/diff/log, exact
test-set comparison, and remaining active `helpers.rs` references.

## Commit and recap

Commit scoped split and handoff as:

```text
refactor: split RX tool helpers
```

Return ownership/import/visibility map, exact test comparison, files,
commands/results, evaluator comparison, commit, blockers, deviations, and Phase
7 follow-up.
