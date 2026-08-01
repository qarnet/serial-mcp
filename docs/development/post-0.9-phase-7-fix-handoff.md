# Post-0.9 Refinement — Phase 7 Review Fix Handoff

## Goal

Align local fuzz runner instructions with pinned scheduled workflow and strengthen
Windows decision provenance.

## Changes

### `fuzz/run.sh`

- Replace floating `rustup toolchain install nightly` guidance with pinned
  `nightly-2026-07-15`.
- Replace unpinned cargo-fuzz install guidance with
  `cargo install cargo-fuzz --locked --version 0.13.2`.
- Invoke fuzz target through pinned toolchain explicitly:
  `cargo +nightly-2026-07-15 fuzz run ...`.
- Preserve target list, duration argument, fail-fast shell behavior, and local
  Nix environment notes otherwise.

### Windows investigation

Add direct com0com README/source evidence URL for kernel-mode/test-signing claims
(SourceForge code README or faithful project mirror), while retaining official
GitHub hosted-runner docs link. Do not cite community signed distributions as
official Microsoft support.

## Verification

```bash
bash -n fuzz/run.sh
cargo check --manifest-path fuzz/Cargo.toml --bins
cargo test --locked --test doc_drift
git diff --check
```

Commit handoff + fix with:

```text
docs: pin local fuzz tooling guidance
```

No push, merge, PR, amend, attribution, tool installation, or other changes.
