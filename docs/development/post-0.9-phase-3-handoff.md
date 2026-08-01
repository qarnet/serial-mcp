# Post-0.9 Refinement — Phase 3 Handoff

## Role and delivery constraint

Implement Phase 3 on existing branch `refactor/post-0.9-refinement`. Follow
`docs/development/post-0.9-refinement-plan.md` and `AGENTS.md`. Do not create or
push another branch or PR. Commit completed work before returning. Do not amend
prior commits or add attribution.

## Goal

Turn release-document consistency into a hermetic regression contract and make
its CI execution visible as a named Ubuntu step.

## In scope

- Guard current Cargo package version against CHANGELOG table and body shape.
- Preserve and incorporate existing Cargo/server.json version equality guard.
- Add focused negative tests proving each missing/drifted element fails for its
  intended reason.
- Add explicit named Ubuntu CI step for `tests/doc_drift.rs`.
- Remove completed release/doc-drift debt from FEATURES and update concise
  contributor truth in AGENTS.md.
- Commit this handoff with Phase 3 work.

## Out of scope

- No package version bump or 0.9.1 changelog content yet.
- No git tag, GitHub release, network, or previous-tag comparison.
- No tool/schema/runtime behavior changes.
- No RX encoding or matcher work.
- No changes to existing vendored-schema CI step.

## Grounding evidence

- `Cargo.toml` package version is `0.9.0`; `server.json` version is `0.9.0`.
- Existing `server_json_versions_match_cargo_toml` recursively checks every
  `version` field in committed registry template.
- CHANGELOG release table uses `| [0.9.0](#090) | ...`; body contains
  `## [Unreleased]` before `## [0.9.0]`.
- `tests/doc_drift.rs` already anchors repo paths at `CARGO_MANIFEST_DIR` and
  contains `cargo_toml_version()`.
- CI matrix runs `cargo test --locked` everywhere and a separately named
  vendored-schema step only on `ubuntu-latest`; doc drift currently lacks its
  own visible step.

## Exact implementation decisions

### Changelog contract

Add a small pure contract helper in `tests/doc_drift.rs` that checks supplied
CHANGELOG text for supplied package version and returns a descriptive error (or
equivalent inspectable failure) for each rule:

1. release table contains a row beginning with exact current version link and
   anchor: `| [x.y.z](#xyz) |`, where anchor removes dots from version;
2. body contains exact heading `## [x.y.z]`;
3. body contains exact `## [Unreleased]` heading;
4. Unreleased heading occurs before current release heading.

Use line-based exact matching for headings and row prefix so prose mentions do
not satisfy contract. Do not parse dates, highlights, or historical versions.
One public regression test must read real Cargo.toml + CHANGELOG and apply this
contract.

Preserve existing server.json recursive version check. If useful, extract its
comparison into a pure helper, but do not weaken recursive behavior or
`server_json_omits_packages`.

### Negative/mutation proofs

Add focused tests against synthetic/modified strings (not private call counts)
that prove independent failures for:

- missing current-version table row;
- missing current-version body heading;
- missing Unreleased heading;
- Unreleased heading after current release;
- mismatched server.json version versus Cargo package version.

Assertions must inspect descriptive error reason, preventing one earlier check
from accidentally masking every later rule. These permanent tests satisfy plan
mutation-check requirement without dirtying repository files during test runs.

### CI and docs

In `.github/workflows/ci.yml`, add immediately near vendored-schema step:

```yaml
- name: Validate release and documentation consistency
  if: matrix.os == 'ubuntu-latest'
  run: cargo test --locked --test doc_drift
```

Keep general `cargo test --locked`, existing schema step, matrix, toolchain, and
job semantics unchanged.

Remove completed `Release-flow guard: version bump ⇒ CHANGELOG roll` and
`Explicit doc_drift gate in CI` sections from FEATURES. Update AGENTS command/CI
truth to state Ubuntu has explicit config-schema and doc-drift gates. No other
roadmap cleanup.

## Required verification

Run:

```bash
cargo fmt --all -- --check
cargo test --locked --test doc_drift
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

Inspect:

```bash
git status --short
git diff -- tests/doc_drift.rs .github/workflows/ci.yml AGENTS.md docs/development
git log --oneline -10
```

Confirm package version remains 0.9.0, tool count remains 27, no network/git-tag
dependency was added, every negative proof fails for its own named reason, and
all existing doc guards remain present.

## Commit and recap

Stage only Phase 3 files and this handoff. Commit with:

```text
test: enforce release and documentation consistency
```

Return files changed, exact guards added, tests/commands and results, commit
hash/message, blockers, deviations, and Phase 4 follow-up. Do not push, merge,
open a PR, or amend.
