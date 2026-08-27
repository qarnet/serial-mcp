# CI runtime reduction plan

Status: implemented on `ci/runtime-reduction`; post-merge branch-protection update and Nix timing measurement pending.

Target baseline: `main` at `cbdcf8b61527f0dfced5f0fc0f71704774c72c71`.

## Scope

Reduce repeated required-CI work without removing platform, Linux real-PTY,
Nix packaging, schema-drift, or MCP compatibility coverage.

This plan changes CI orchestration only. It does not change product behavior,
the scheduled hardening workflow, release artifact builds, or test assertions.

## Baseline facts

- Before implementation, `ci.yml` ran `cargo fmt --check`, build, test, and Clippy in every matrix
  cell: Linux x86_64, Linux ARM64, macOS ARM64, and Windows.
- At baseline, `main` had no `native_sim`, NCS, firmware, or nrfutil CI path.
  Linux real-PTY Rust fixtures replaced that coverage.
- Ubuntu explicitly ran `device_fixture`, `device_command_parity`,
  `device_framing_parity`, `device_protocol_parity`, and the ignored
  100-iteration `device_parity_repeat` gate after broad `cargo test`.
  Keep this serial real-PTY evidence.
- Broad `cargo test --locked` already ran active
  `config_schema_validation` cases. The later named Ubuntu invocation repeated
  them.
- `schema-drift.yml` ran daily at 06:00 UTC and on manual dispatch. Its
  ignored `config_schema_validation` case fetched and validated current
  upstream schemas. Keep it unchanged.
- `flake.nix` ran `registry-manifest-builder-tests`. The Ubuntu CI Python
  invocation ran the same deterministic suite again.
- `release` waited for `nix-flake` and `build-test`, but not
  `mcp-conformance`.

Recent baseline main run
[`32989592995`](https://github.com/qarnet/serial-mcp/actions/runs/32989592995)
took about 9m31s for Nix, 5m42s for Windows, 4m56s for Ubuntu, 3m38s for
Linux ARM64, 2m13s for macOS, and 2m for MCP compatibility. There is no
native-sim job in that run.

## Decisions

### Add one format gate

Add a `format` job to `.github/workflows/ci.yml`.

- Run on Ubuntu.
- Checkout source and install Rust 1.97.1 with `rustfmt` only.
- Run `cargo fmt --all -- --check`.
- Do not install `libudev` or build the crate.
- Make `nix-flake`, `build-test`, and `mcp-conformance` depend on `format`.

This must be a job in `ci.yml`, not a separate workflow file. GitHub Actions
`needs` dependencies do not cross workflow boundaries.

Result:

- A formatting failure skips every expensive required job.
- A clean commit pays one short runner startup before matrix, Nix, and MCP
  jobs begin. This added green-run latency is accepted to avoid spending
  platform, Nix, and conformance minutes on a known formatting failure.
- Post-merge administrative follow-up: once CI workflow lands, add required
  `format` status check to branch protection. This branch does not change
  remote branch protection.

### Keep Clippy on every platform

Keep `cargo clippy --all-targets --locked -- -D warnings` in all four matrix
cells. Platform-conditional source and test code exists, so a single-host
lint run would reduce coverage.

Remove `rustfmt` from matrix toolchain components once the format job owns it.
Keep `clippy` there.

### Remove duplicate offline schema validation from required CI

Remove only this named Ubuntu step from `ci.yml`:

```text
Validate example configs against vendored schemas
```

The active vendored-schema checks remain part of broad `cargo test --locked`.
Do not change `.github/workflows/schema-drift.yml`; it owns the distinct
networked upstream-schema check.

### Keep registry-manifest builder tests in Nix

Remove the Ubuntu CI Python step:

```text
Run registry manifest builder tests
```

Keep `registry-manifest-builder-tests` in `flake.nix`.

That Nix check proves both behavior and that the Nix source filter retains the
`scripts/` tree the test suite needs. Moving it out would slightly shorten Nix
while weakening Nix packaging coverage. The standalone Ubuntu invocation adds
no meaningful second boundary.

### Make MCP compatibility a release prerequisite

Change `release.needs` to:

```yaml
needs: [nix-flake, build-test, mcp-conformance]
```

`format` does not need to appear directly because all three prerequisite jobs
already depend on it. A failed or skipped format job prevents release through
that dependency chain.

## Nix runtime strategy

Do not remove the Nix package build or registry-manifest check in this change.
They are the unique Nix boundary.

First measure cache effectiveness over several clean runs and runs with only
source changes:

1. Record cache restore/save duration and `nix flake check` duration.
2. Inspect whether the 8 GiB Nix-store cache retains dependency outputs or is
   repeatedly evicted under the repository cache quota.
3. Record cold versus warm time before changing cache keys, store limits, or
   source filtering.

Possible later work, only if measurements justify it:

- Evaluate a trusted signed binary cache for Nix dependencies and package
  outputs.
- Tighten source filtering only when an included path is not needed by the
  package or Nix checks.

Do not replace `nix flake check` with `--no-build`, drop the package check, or
move the builder test out of Nix merely to lower a timing number. Those changes
would remove coverage instead of removing duplicate work.

## Implementation order

1. Add `format` job and `needs: format` to required heavy jobs.
2. Remove matrix-local format step and `rustfmt` component.
3. Remove duplicate named vendored-schema and Python builder steps.
4. Add `mcp-conformance` to release prerequisites.
5. Update CI comments and contributor instructions that describe job order.
6. Collect post-change timing data before proposing further Nix or test-target
   changes.

## Validation

Run before merge:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
nix flake check --accept-flake-config --print-build-logs
bash scripts/test-mcp-compat.sh
```

Verify through a disposable pull request:

1. A formatted change runs format first, then Nix, matrix, and MCP jobs.
2. A temporary formatting failure skips Nix, all matrix cells, and MCP
   conformance.
3. `schema-drift.yml` still exposes its manual-dispatch upstream-schema check.
4. Linux still runs all four serial real-PTY fixture suites and the
   100-iteration repeat gate.
5. A main-push release cannot start until MCP conformance succeeds.
