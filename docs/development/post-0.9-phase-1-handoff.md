# Post-0.9 Refinement — Phase 0/1 Handoff

## Role and delivery constraint

Implement Phase 0/1 on the existing branch
`refactor/post-0.9-refinement`. This branch will eventually contain every phase
from `docs/development/post-0.9-refinement-plan.md` and produce one final PR.
Do not create or push a phase branch or PR.

Commit the completed Phase 0/1 work before returning. Do not amend, push,
merge, open a PR, or add attribution/co-author footers.

## Goal

Capture the pre-refinement baseline, clear the current locked Quinn advisory,
pin GitHub workflows to the repository's Rust 1.88.0 policy, add bounded monthly
dependency updates, and commit the active refinement plan/index with the phase
changes.

## In scope

1. Run and record the Phase 0 baseline before modifying dependencies or
   workflows.
2. Update locked `quinn-proto` 0.11.14 to 0.11.15.
3. Pin every `dtolnay/rust-toolchain` workflow use to `@1.88.0` while
   preserving existing components and targets.
4. Add visible `rustc --version --verbose` output after each workflow toolchain
   installation.
5. Add monthly Dependabot updates for root Cargo dependencies and GitHub
   Actions.
6. Remove the now-completed Dependabot/toolchain debt entries from FEATURES.
7. Update AGENTS.md only with high-signal resulting truth.
8. Include the already-written active plan and development index update in the
   phase commit.

## Out of scope

- No schema-vendoring or schema-test changes (Phase 2).
- No release/doc-drift guards (Phase 3).
- No RX behavior, matcher, framing, serial, or tool-helper changes.
- No Rust version bump beyond 1.88.0.
- No Cargo dependency feature changes or new direct Quinn dependency.
- No cargo-audit/cargo-deny installer or new advisory workflow.
- No package version/changelog/server.json bump.

## Grounding evidence

- `rust-toolchain.toml` pins channel `1.88.0` and declares rustfmt, clippy,
  rust-src, rust-analyzer, and the aarch64 GNU target.
- `flake.nix:39-43` already uses
  `pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml`; Nix is already
  pinned and should not be reworked.
- Workflow toolchain uses currently float at `dtolnay/rust-toolchain@stable`:
  - `.github/workflows/ci.yml` build/test matrix and native_sim job;
  - `.github/workflows/release.yml` build matrix and crate-publish job;
  - `.github/workflows/schema-drift.yml` schema job.
- The official dtolnay action selects the toolchain from its requested action
  revision; `dtolnay/rust-toolchain@1.88.0` installs Rust 1.88.0. Existing
  `components:` and `targets:` inputs remain valid.
- `Cargo.lock` contains `quinn-proto 0.11.14`; RUSTSEC-2026-0185 is fixed in
  0.11.15. The locked `quinn 0.11.9` declares `quinn-proto` version
  `0.11.12` (caret range), so 0.11.15 is compatible.
- GitHub Dependabot currently reports alert #1 against Cargo.lock. The package
  is not in the active `cargo tree` feature graph, but a vulnerable locked
  package still needs removal.

## Phase 0 baseline — run before edits

Run:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --output-dir target/pre-refinement-agent-eval
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
nix flake check --accept-flake-config
```

Do not commit anything under `target/`. Record in the recap:

- tool count;
- aggregate catalog bytes;
- evaluator status;
- all gate results.

If baseline fails, stop without implementation or commit and report the exact
failure.

## Exact implementation decisions

### Cargo.lock

Run:

```bash
cargo update -p quinn-proto --precise 0.11.15
```

Inspect the lock diff. Expected change is the quinn-proto version/checksum (and
only resolver-required lock changes). Do not add Quinn to Cargo.toml.

### GitHub workflows

In every existing workflow occurrence, replace:

```yaml
uses: dtolnay/rust-toolchain@stable
```

with:

```yaml
uses: dtolnay/rust-toolchain@1.88.0
```

Preserve all existing `components:` and `targets:` fields. Immediately after
each install step add a shell-neutral command step:

```yaml
- name: Report Rust toolchain
  run: rustc --version --verbose
```

Apply this to:

- CI build/test matrix;
- CI native_sim job;
- release build matrix;
- release crate-publish job;
- schema-drift job.

Do not alter firmware's NCS toolchain setup or release behavior.

### Dependabot

Create `.github/dependabot.yml` using version 2. Add two monthly entries rooted
at `/`:

1. `package-ecosystem: cargo`
2. `package-ecosystem: github-actions`

Use a low open-PR limit (5). Group patch/minor updates within each ecosystem so
routine maintenance does not create one PR per transitive update. Do not group
major updates. Use valid Dependabot v2 keys only.

### Documentation

- `docs/development/FEATURES.md`: remove completed “Proper dependabot /
  renovate setup” and “Toolchain single source of truth” items. Do not remove
  unrelated future work.
- `AGENTS.md`: state that CI/release/schema workflows install Rust 1.88.0 and
  Nix reads the same version from rust-toolchain.toml. Keep this concise.
- Preserve and include:
  - `docs/development/post-0.9-refinement-plan.md`
  - `docs/development/README.md` index update
  - this handoff document (it remains active until final documentation cleanup).

## Validation after implementation

Run:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
nix flake check --accept-flake-config
git diff --check
```

Also inspect:

```bash
git status --short
git diff -- Cargo.lock .github rust-toolchain.toml AGENTS.md docs/development
git log --oneline -10
```

Confirm:

- `Cargo.lock` has `quinn-proto >=0.11.15` and no older quinn-proto entry;
- no `dtolnay/rust-toolchain@stable` remains in workflows;
- every workflow install has a following version-report step;
- flake still derives Rust from rust-toolchain.toml;
- no target evaluator output is staged;
- package version remains 0.9.0.

## Commit and recap

Stage only intended Phase 0/1 files and commit with:

```text
chore: harden dependency and toolchain maintenance
```

Return:

- files changed;
- dependency/workflow behavior changed;
- baseline metrics;
- every command run and result;
- commit hash/message;
- blockers;
- deviations from this handoff;
- suggested Phase 2 follow-up.
