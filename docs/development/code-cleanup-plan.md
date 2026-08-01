# Code Cleanup Plan

## Status

Implementation complete on `refactor/code-cleanup` (branched from merged
`main` after PR #37). All six phases delivered and committed; the final
gate (fmt/build/test/clippy, xtask test-all, `nix flake check`,
agent-eval) passed. Awaiting delivery review of the single follow-up PR.

Consumed phase handoffs were deleted after each phase shipped; this plan
is retained as the single current record of the cleanup's scope,
invariants, and verification.

## Goal

Reduce duplicated transformation and result-building code, make long RX paths
easier to follow, consolidate test infrastructure, and replace historical
implementation commentary with names and structure that explain current
behavior.

This is a behavior-preserving refactor. Success means less code and lower drift
risk without changing MCP schemas, wire output, operation ordering, cursor
semantics, logging/accounting, persistence, cancellation, or platform support.

## Scope

- Share TX payload decoding, validation, and framing between `write` and
  `transact` without delegating one public tool handler to another.
- Centralize repeated post-read accounting used by `read`, `transact`, and
  `capture_boot`.
- Simplify `read_from_private_cursor` through explicit private state and pure
  result finalization while preserving every stop path.
- Reduce repeated profile-binding construction and simplify profile-preview
  control flow.
- Decompose subscription notification construction where failure semantics can
  remain explicit.
- Consolidate duplicated integration-test infrastructure, with platform gates
  and cross-platform compilation treated as acceptance criteria.
- Remove chronology-only comments and stale aliases while retaining comments
  that document non-obvious safety and compatibility invariants.

## Non-scope

- No new MCP tools, resources, prompts, fields, schema changes, or aliases.
- No intended changes to tool-result text, stop reasons, log event order,
  counters, timeout defaults, framing precedence, or read/write ordering.
- No change to `transact`'s default `from={"type":"now"}` behavior.
- No generic reset/BREAK cleanup guard.
- No generic SLIP/COBS decoder driver.
- No wrapper around `learning_lock`; lock scope stays visible.
- No automatic async client-cleanup macro or `Drop` wrapper in tests.
- No Windows virtual-serial E2E setup. Existing Windows compile/test coverage
  remains the supported gate.
- No dependency, Rust-version, package-version, or release-workflow changes.

## Load-bearing invariants

1. `transact` performs its TX half before RX matcher/framing validation, uses
   one looked-up connection, increments write/read counters only after each
   operation succeeds, and defaults its read cursor to the live edge.
2. Shared TX preparation may transform buffers only. Tool handlers retain
   session ownership, error mapping, logging, counter updates, and result
   assembly.
3. `read_from_private_cursor` never commits the shared cursor. Only
   `read_bytes_from_ring` applies its returned final cursor.
4. Match-context payload length may differ from consumed stream length. Cursor
   offsets always reflect consumed ring bytes, not shaped response bytes.
5. Runtime framing errors consume malformed bytes and return frames decoded
   before the error.
6. `read`, `transact`, and `capture_boot` preserve post-read accounting order:
   read operation, RX bytes, optional truncation, optional match.
7. Subscription frame, raw-chunk, partial-frame, and match-context encoding
   failures remain distinct. A successful hex fallback never counts as a drop.
8. Automatic profile bindings always report high confidence; explicit profile
   bindings report the matched port's confidence; generated-profile failure
   remains a transient open success carrying a persistence error.
9. Reset release, profile-store durability, matcher bounds, and decoder state
   comments remain where code alone cannot explain the safety contract.

## Platform constraints

- `tests/native_sim_validation.rs` selects `unix.rs` or `windows.rs` with
  target `cfg` attributes.
- `tests/native_sim_connection_lifecycle.rs` is compiled on every CI platform,
  even though its firmware tests are ignored during the general test job.
- Shared native_sim process code must therefore compile on Windows. Do not add
  a Unix-only API or gate the shared type away from the lifecycle test target.
- Actual native_sim runtime suites remain Linux-only in the dedicated CI job.
- Every test-infrastructure phase must pass all-target build and clippy before
  acceptance, then pass the GitHub matrix on Linux x64, Linux ARM64, macOS
  ARM64, and Windows before the final PR is accepted.

## Phase 1 — Shared TX preparation and read accounting

### Goal

Remove highest-risk simple duplication without changing handler sequencing or
user-visible behavior.

### Files

- `src/tools/io_ops.rs`
- `src/tools/control_ops.rs`
- Focused unit tests in `src/tools/io_ops.rs` or a small private tools module
- Existing public-boundary tests in `tests/serial_pty.rs`

### Changes

1. Extract a private TX preparation primitive used by `write` and `transact`.
   It owns string decoding, decoded-size validation, framing application, and
   framed-size validation. It returns prepared bytes, decoded byte count, and
   resolved encoding metadata. It performs no I/O, logging, counter mutation,
   connection lookup, or result construction.
2. Preserve each caller's current error text and field labels. If exact error
   preservation requires a small typed internal error, prefer that over policy
   callbacks or passing tool names into transformation code.
3. Extract post-read accounting into one private helper taking the connection,
   completed `ReadResult`, and optional match request. Use it from `read`,
   `transact`, and `capture_boot` only.
4. Keep `transact` as one connection-scoped operation. Do not call `write()` or
   `read()` from it.

### Verification

```bash
cargo fmt --all -- --check
cargo test --lib
cargo test --test serial_pty pty_transact -- --test-threads=1
cargo test --test serial_pty pty_write -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

Acceptance: existing write/transact bytes, errors, counters, framing, cursor,
and cancellation behavior remain unchanged; duplicated transformation and
accounting blocks are gone.

Status: complete.

## Phase 2 — Private read-loop state and finalization

### Goal

Make `read_from_private_cursor` understandable without flattening distinct RX
behaviors into unsafe generic control flow.

### Files

- `src/tools/read_loop.rs`
- Focused tests in `src/tools/read_loop.rs`
- `tests/serial_pty.rs` only when public-boundary coverage is missing

### Changes

1. Replace the large local result closure with a pure private finalizer.
2. Introduce a small private state type for consumed bytes, returned bytes,
   frame collection/counts, frame error, and cursor state when this removes
   repeated parameter lists.
3. Centralize cursor/result formulas only where formulas are identical.
4. Keep immediate cat, history match, initial framing, live framing, raw match,
   timeout, silence, cancellation, reconnect, and max-buffer exits visibly
   distinct.
5. Replace chronology comments with current cursor/offset invariants.

### Verification

```bash
cargo fmt --all -- --check
cargo test --lib tools::read_loop
cargo test --test serial_pty --locked -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

Acceptance: all existing offsets, bytes-lost values, context-shaped payloads,
framing-error tails, partial frames, and shared/private cursor behavior match
the pre-refactor tests.

Status: complete.

## Phase 3 — Profile and local production cleanup

### Goal

Replace repeated struct literals and nested result construction with small
domain-named helpers.

### Files

- `src/tools/port_ops.rs`
- `src/tools/helpers.rs`
- `src/match_config.rs`

### Changes

1. Add private factories for disabled, transient, and persistent profile
   bindings. Avoid one generic many-argument constructor.
2. Collapse duplicate automatic/explicit `mark_used` result construction while
   retaining their confidence and source differences.
3. Split weak, duplicate, empty, selected, and ambiguous profile-preview result
   construction into pure helpers if this shortens `compute_profile_matches`.
4. Remove the legacy `find_subslice` re-export and import
   `util::find_subsequence` directly.

### Verification

```bash
cargo fmt --all -- --check
cargo test --lib profiles
cargo test --lib profile_store
cargo test --test serial_pty --locked -- --test-threads=1
cargo test --test http_integration --locked -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

Acceptance: profile selection, candidate ordering, dirty/stale state, revision,
and partial-persistence behavior remain unchanged.

Status: complete.

## Phase 4 — Subscription readability

### Goal

Reduce `stream_rx_from_ring` size while preserving path-specific failure and
notification semantics.

### Files

- `src/tools/stream_ops.rs`
- `tests/http_integration.rs`
- `tests/serial_pty.rs` where real PTY behavior is required

### Changes

1. Add characterization coverage for matching-frame notification failure before
   changing `SubscribeFrameSink` behavior or comments.
2. Extract serialization and notification construction helpers where all
   callers share identical behavior.
3. Extract successful encoding-fallback warning mechanics only if it reduces
   duplication without swallowing path-specific true-failure handling.
4. Keep raw, framed, partial-frame, and final-stop accounting explicit.
5. Build final stop notification in a named helper or state method when doing
   so makes match-context shaping and encoding pairing clearer.

### Verification

```bash
cargo fmt --all -- --check
cargo test --lib tools::stream_ops
cargo test --test http_integration --locked -- --test-threads=1
cargo test --test serial_pty --locked -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

Acceptance: notification order, logger names, payload schemas, drop counters,
peer-disconnect stops, match reporting, and encoding fallback remain unchanged.

Status: complete.

## Phase 5 — Cross-platform test infrastructure cleanup

### Goal

Remove duplicated harness code without weakening test independence or platform
compilation.

### Files

- `tests/common/mod.rs`
- `tests/common/binaries.rs`
- `tests/common/firmware.rs`
- `tests/common/spawned.rs`
- `tests/native_sim_validation.rs`
- `tests/native_sim_validation/unix.rs`
- `tests/native_sim_validation/windows.rs`
- `tests/native_sim_connection_lifecycle.rs`
- `tests/http_integration.rs`
- `tests/stdio_integration.rs`
- `tests/blob_resources.rs`

### Changes

1. Move the duplicated `NativeSimFirmware` process harness into shared test
   support while keeping it compilable on Windows and retaining the exact PTY
   discovery timeout, stdout drain, and process cleanup.
2. Reuse the existing common `NotificationCollector` and `connect_to_url` from
   spawned-server tests.
3. Centralize workspace-root resolution and the explicit expected-tool list.
4. Remove duplicate stdio binary-build preludes.
5. Keep `TestServer::start()` and common manager shortcut. Introduce options for
   specialized provider/profile/security/capture-store combinations, then
   remove only wrappers made obsolete.
6. Keep suite-specific native_sim MCP helper functions local unless an exact
   duplicate has one clear behavior and signature.

### Verification

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test native_sim_validation -- --ignored --test-threads=1
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

Acceptance: all test targets compile locally, Linux native_sim runtime behavior
passes, no platform gate changes unintentionally, and final PR CI passes every
OS matrix entry. Local environment has no `rustup`, so Windows cross-compilation
cannot substitute for GitHub's Windows runner.

Status: complete.

## Phase 6 — Comment reconciliation and final gate

### Goal

Leave current code self-explaining and documentation synchronized after all
structural changes.

### Files

- Touched production/test files
- `docs/development/README.md`
- `AGENTS.md` only if executable architecture or test commands changed

### Changes

1. Remove or rewrite chronology-only `Phase N` comments in active production
   and ordinary test code.
2. Remove comments that restate names or syntax.
3. Retain and tighten comments that preserve safety, ordering, compatibility,
   protocol, persistence, and memory-bound invariants.
4. Update development index and repository guidance for any lasting module or
   helper changes.
5. Remove consumed phase handoff documents before final delivery; retain this
   plan only while it remains useful as active development documentation.

### Final verification

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo run --manifest-path xtask/Cargo.toml -- test-all
nix flake check --accept-flake-config --print-build-logs
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
git status --short
```

Status: complete — executed and passed in Phase 6.

Final PR acceptance also requires green GitHub CI for Linux x64, Linux ARM64,
macOS ARM64, Windows, native_sim, Nix, and CodeQL.

## Delivery

- One follow-up branch and one PR, separate from merged 0.9.1 PR #37.
- One reviewed commit per accepted phase where practical.
- Executor commits locally after each accepted implementation handoff.
- Orchestrator reviews actual diff and tests before advancing.
- No push, PR creation, merge, or version bump until explicitly requested.
