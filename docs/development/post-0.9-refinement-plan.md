# Post-0.9 Refinement Plan

## Status and delivery model

This is one implementation plan on one branch:

```text
refactor/post-0.9-refinement
```

Phases may use separate checkpoint commits so review remains tractable, but
there will be no phase branches and no intermediate pull requests. The entire
plan ends in one pull request to `main` and one patch-level release bump from
0.9.0 to 0.9.1.

## Goals

- Close the current high-severity locked dependency advisory.
- Make every vendored configuration-schema test mandatory and hermetic.
- Turn release and documentation consistency into explicit CI contracts.
- Preserve serial bytes consistently across `read` and `subscribe` encoding
  failures.
- Give read and subscribe one bounded matcher-window policy.
- Reduce the largest internal module boundaries without changing the MCP
  surface.
- Add bounded, scheduled fuzz and mutation checks.
- Leave the repository green, documented, and ready for one 0.9.1 PR.

## Non-goals

- No new MCP tools or resources.
- No shorthand wire forms, recipes, or versioned facade.
- No continuous-capture implementation.
- No Rust-version bump beyond 1.88.0.
- No 1.0 compatibility declaration.
- No privileged Windows virtual-port driver installation without a separate,
  evidence-backed design decision.

## Invariants across every phase

- Tool count stays 27.
- Existing tool names and tagged wire forms stay stable.
- Operational failures remain MCP tool results where current handlers use tool
  errors; malformed requests remain protocol-level errors.
- Unsigned schema fields must not regain non-standard `uintN` formats.
- `read` keeps the shared cursor; `subscribe` and `capture_boot` keep private
  cursors.
- Framed matching stays per-frame; patterns do not span frame boundaries.
- Profile durability, CAS, cancellation, and capture no-clobber contracts stay
  unchanged.
- Every phase must be green before the next handoff starts. Fixes stay on the
  same branch and within the same eventual PR.

## Phase 0 — Baseline and branch setup

Capture pre-refinement truth before behavior or module movement:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/pre-refinement-agent-eval
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
nix flake check --accept-flake-config
```

The generated baseline under `target/` is not committed. The historical Phase
4 baseline remains unchanged.

## Phase 1 — Security and dependency foundations

### Scope

- Update locked `quinn-proto` 0.11.14 to 0.11.15, which fixes
  RUSTSEC-2026-0185 / CVE-2026-25800. `quinn` 0.11.9 accepts
  `quinn-proto ^0.11.12`, so no direct dependency or feature change is needed.
- Add monthly Dependabot configuration for Cargo and GitHub Actions.
- Make `rust-toolchain.toml`'s 1.88.0 pin explicit across CI, release,
  schema-drift, and Nix workflows; print the resolved compiler version in CI.
- Do not add an unpinned installer or advisory scanner.

### Files

- `Cargo.lock`
- `.github/dependabot.yml`
- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.github/workflows/schema-drift.yml`
- `rust-toolchain.toml`
- `flake.nix` only if alignment requires a comment or assertion change
- `docs/development/FEATURES.md`

### Verification

```bash
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
nix flake check --accept-flake-config
```

After the final PR merges, Dependabot alert #1 must close.

## Phase 2 — Fail-closed, hermetic schema validation

### Current defect

`tests/config_schema_validation.rs::load_json_file` returns `None` for missing
files, so missing schemas and example configs silently skip. In addition,
`schemas/opencode.schema.json` has four references to
`https://models.dev/model-schema.json#/$defs/Model`; Nix excludes the opencode
schema because `jsonschema` resolves that URI eagerly in a networkless sandbox.

### Decisions

- Vendor the models.dev schema exactly; record provenance separately rather
  than editing the upstream blob.
- Keep opencode's original external URI unchanged and register the vendored
  schema in memory using verified `jsonschema 0.26.2` APIs:
  `Resource::from_contents` and `ValidationOptions::with_resource`.
- Missing fixture files are failures. No skip path remains in the vendored
  test.
- Normal CI and Nix validate exactly three cases: Claude Code, Codex, and
  opencode. The scheduled upstream-drift test remains networked.

### Files

- `schemas/models-dev-model.schema.json` (new)
- `schemas/README.md` (new provenance/update instructions)
- `tests/config_schema_validation.rs`
- `flake.nix`
- `AGENTS.md`
- `docs/development/FEATURES.md`

### Behavior proofs

- Missing schema path returns an explicit error.
- Missing instance path returns an explicit error.
- Unregistered models.dev resource makes opencode compilation fail.
- Registered vendored resource makes opencode compilation and validation pass.
- All three real example configurations validate offline.
- `nix flake check` includes all schemas and does no schema-network access.

### Verification

```bash
cargo test --locked --test config_schema_validation
nix flake check --accept-flake-config --print-build-logs
```

## Phase 3 — Release and documentation guards

### Scope

Add a hermetic `doc_drift` test that requires:

- a changelog table entry for the current Cargo package version;
- a matching `## [x.y.z]` body heading;
- an `## [Unreleased]` heading before the current release heading;
- existing Cargo/server.json version equality.

Add an explicit named Ubuntu CI step for `tests/doc_drift.rs`. Keep the existing
named vendored-schema step. No git tags or network are needed for these checks.

### Files

- `tests/doc_drift.rs`
- `.github/workflows/ci.yml`
- `docs/development/FEATURES.md`
- `AGENTS.md` if the named gate changes contributor instructions

### Verification

```bash
cargo test --locked --test doc_drift
```

Mutation checks during implementation must show that a missing version table
row, version heading, Unreleased heading, or server.json match fails for the
intended reason.

## Phase 4 — Lossless RX encoding parity

### Contract

For both `read` and `subscribe`:

1. Try the requested encoding.
2. If it cannot represent the bytes, encode the same bytes as hex.
3. Report the effective encoding as `hex`.
4. Warn, but do not increment notification-drop counters when fallback works.
5. Never use lossy UTF-8.

The rule applies to raw reads, framed reads, raw subscription chunks, framed
subscription frames, and partial-frame notifications. Existing stop reasons,
framing diagnostics, offsets, cursors, and match semantics remain unchanged.
`SubscribeEncodingErrorNotification` remains available for a true fallback or
serialization failure; it is not removed from the wire surface.

### Files

- `src/codec.rs` — shared encode-or-hex primitive and pure tests
- `src/tools/helpers.rs` — read result construction
- `src/tools/stream_ops.rs` — chunk/frame/partial notifications
- `src/tools/types.rs` — effective-encoding documentation
- `tests/http_integration.rs`
- `tests/serial_pty.rs`
- `README.md`
- `AGENTS.md`

### Public behavior proofs

- Invalid UTF-8 raw read returns exact hex bytes.
- Invalid UTF-8 raw subscription emits exact hex bytes.
- Binary framed subscription emits an exact hex frame.
- Partial framing-error bytes survive alongside the diagnostic.
- Successful fallback does not count as a dropped notification.

## Phase 5 — Matcher memory-bound parity

### Decisions

- Characterize literal, regex, glob, context, initial-history, and cross-chunk
  behavior before changing the matcher.
- Add one matcher-window limit API in `src/match_config.rs`.
- Use it from initial-history read, live raw read, and subscribe.
- Preserve enough overlap and requested context for supported match modes.
- Keep framed matching per-frame.

### Files

- `src/match_config.rs`
- `src/tools/helpers.rs`
- `src/tools/stream_ops.rs`
- matcher unit tests
- `tests/http_integration.rs`
- `tests/serial_pty.rs`

### Behavior proofs

- Literal matches spanning chunks remain detectable at the correct index.
- Context shaping remains correct at the truncation boundary.
- Regex and glob behavior remains stable.
- Read and subscribe produce the same match/no-match outcome over the same
  byte sequence.
- Retained matcher memory never exceeds the documented cap plus required
  overlap allowance.

## Phase 6 — Mechanical module refinement

Only begin after Phases 4 and 5 are accepted so behavior changes do not mix
with file movement.

### Framing split

```text
src/framing/
  mod.rs
  config.rs
  decoder.rs
  codecs.rs
  parsers/
```

### Serial split

```text
src/serial/
  mod.rs
  config.rs
  connection.rs
  manager.rs
  port_info.rs
  test_support.rs
```

### Tool-helper split

```text
src/tools/
  rx_validate.rs
  read_loop.rs
  result_builders.rs
```

Each split gets a checkpoint commit but not a PR. No public symbol, schema,
tool-catalog, lifecycle, precedence, or stop-reason change is allowed from file
movement. Run focused tests after each split and compare the live evaluator to
the uncommitted Phase 0 output.

## Phase 7 — Scheduled hardening

Add one scheduled/manual workflow with bounded jobs:

- a five-minute fuzz smoke using the existing fuzz targets;
- focused mutation testing for checksums and parser code;
- explicit wall-clock limits;
- failure artifact/corpus upload.

These jobs are scheduled/manual initially, not required on every PR. Document
the Windows serial-e2e investigation outcome, but do not install privileged
virtual-port drivers without a separately approved design.

Files:

- `.github/workflows/hardening.yml` (new)
- existing fuzz configuration only where needed
- `docs/development/FEATURES.md`
- `AGENTS.md`

## Phase 8 — Documentation reconciliation

Refresh current truth in:

- `AGENTS.md`
- `README.md`
- `docs/development/README.md`
- `docs/development/FEATURES.md`
- protocol/behavior docs affected by module or encoding changes

Remove completed debt items and re-scan renamed paths. Continuous capture and
the `UInt`/schemars redesign remain deferred.

## Phase 9 — Final acceptance and 0.9.1 release prep

Generate a post-refinement evaluator report and compare it to Phase 0. Expected
result: 27 tools, no unintended input/output schema changes, and only approved
description changes.

Run the complete gate:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/post-refinement-agent-eval
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test config_schema_validation
cargo test --locked --test doc_drift
cargo run --manifest-path xtask/Cargo.toml -- test-all
nix flake check --accept-flake-config --print-build-logs
```

Only after the gate passes:

- bump `Cargo.toml` from 0.9.0 to 0.9.1;
- update `Cargo.lock` and `server.json`;
- roll a complete 0.9.1 changelog entry for the entire branch;
- rerun the complete gate;
- commit release prep separately.

Finally push `refactor/post-0.9-refinement`, open one PR to `main`, and watch
all CI checks to completion. Do not merge automatically.

## Expected checkpoint commits

Commit boundaries may be adjusted during review, but remain on the same branch:

```text
chore: harden dependency and toolchain maintenance
test: make config schema validation hermetic
test: enforce release and documentation consistency
fix: preserve RX bytes with shared encoding fallback
fix: unify read and subscribe matcher bounds
refactor: split framing internals into focused modules
refactor: split serial connection internals
refactor: split RX tool helpers
ci: add bounded fuzz and mutation hardening
docs: refresh refinement guidance and roadmap
chore: bump version to 0.9.1
```
