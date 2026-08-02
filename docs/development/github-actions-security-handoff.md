# GitHub Actions security remediation handoff

## Goal

Remove all eight open GitHub code-scanning findings (#7–#14) by making trust
boundaries structural, not by suppressing CodeQL or adding event-name checks
around privileged `workflow_run` checkouts.

## Security design

Use one pipeline rooted in `CI`, whose existing triggers are `push` and
`pull_request` for `main`:

1. Normal CI jobs run for both events with `contents: read` only.
2. Release runs as a local reusable workflow only from a final CI job whose
   condition is exactly trusted `push` to `refs/heads/main` and whose needs are
   every required CI job.
3. MCP Registry publication runs as another local reusable workflow after the
   release call, under the same trusted CI run.
4. Pull requests can define repository content tested by CI, but cannot reach
   either privileged call because both caller jobs are event/ref gated.
5. Remove every `workflow_run` trigger. No privileged workflow may check out
   `github.event.workflow_run.head_sha` or other downstream-event content.
6. Keep manual release testing in a separate, read-only dry-run caller. It may
   accept a branch/tag/SHA and execute that content, but receives only
   `contents: read`, receives no repository or crates.io write credential, and
   hardcodes release mode to `dry-run`.
7. Reusable workflow token permissions may only be maintained or reduced by
   callers. CI release caller grants `contents: write`; dry-run caller grants
   only `contents: read`. Pass `CARGO_REGISTRY_TOKEN` explicitly only from
   trusted CI release caller. Registry caller grants only `contents: read` and
   `id-token: write`.

This follows GitHub reusable-workflow permission semantics: called workflows
cannot elevate caller token permissions. It also avoids GitHub's documented
unsafe shape of checking out and executing event-derived code from privileged
`workflow_run` contexts.

## In scope

- `.github/workflows/ci.yml`
- `.github/workflows/release.yml`
- `.github/workflows/publish-mcp-registry.yml`
- New `.github/workflows/release-dry-run.yml`
- `tests/doc_drift.rs` security regression tests
- `AGENTS.md` release-workflow truth
- Comments in touched workflow files that describe old orchestration

## Out of scope

- Pinning third-party actions from version tags to commit SHAs. Valuable future
  hardening, but unrelated to alerts #7–#14.
- Changing release artifacts, platforms, Cargo version policy, immutable
  release behavior, registry package generation, or crates.io idempotency.
- Dismissing CodeQL findings or adding CodeQL suppression comments.
- Pushing branch, opening PR, or changing repository settings.

## Exact implementation

### `.github/workflows/ci.yml`

- Add top-level `permissions: { contents: read }` (expanded YAML form is fine).
- Preserve existing three CI jobs and triggers.
- Add final reusable-workflow job `release`:
  - `needs: [nix-flake, build-test, native-sim]`.
  - `if: github.event_name == 'push' && github.ref == 'refs/heads/main'`.
  - `permissions: contents: write`.
  - `uses: ./.github/workflows/release.yml`.
  - pass `mode: release` and immutable `ref: ${{ github.sha }}` as inputs.
  - pass only `CARGO_REGISTRY_TOKEN` explicitly. Do not use `secrets: inherit`.
- Add final reusable-workflow job `publish-mcp-registry`:
  - `needs: release`.
  - same exact trusted push/ref condition.
  - `permissions: contents: read` and `id-token: write`.
  - `uses: ./.github/workflows/publish-mcp-registry.yml`.
  - pass immutable `ref: ${{ github.sha }}` and `version: ${{ needs.release.outputs.version }}`
    (the release `workflow_call` exposes the prepared version, so an
    already-published release skip still reaches the registry — this is how
    v0.9.1 recovers without a version bump). No secrets.
- Add final job `notify-release-failure`:
  - `needs: [release, publish-mcp-registry]`.
  - `if: always() && github.event_name == 'push' && github.ref == 'refs/heads/main' &&
    (needs.release.result == 'failure' || needs.publish-mcp-registry.result == 'failure')`.
  - `permissions: issues: write` only; no checkout, no project execution.
  - Creates or comments on one `[automation] Release pipeline failed` issue with
    run URL/SHA and per-job results; all dynamic values passed via env, never
    interpolated into shell source.

### `.github/workflows/release.yml`

- Convert triggers to `on: workflow_call` only.
- Declare required string inputs `mode` and `ref`; declare
  `CARGO_REGISTRY_TOKEN` as an optional workflow secret so the no-secret
  dry-run caller can invoke the same workflow; only the release-mode publish
  step references it.
- Validate `mode` at start: only `release` and `dry-run` accepted. This is
  defense in depth against future callers. Invalid mode fails before mutation.
- In release mode (when the run will actually proceed), validate that
  `CARGO_REGISTRY_TOKEN` is non-empty early in `prepare`, before any release
  mutation, via env indirection — never print the token. Dry-run runs with no
  token.
- Replace event-derived mode/ref expressions with inputs. `prepare` resolves
  `inputs.ref` once; every later checkout uses the immutable resolved
  `${{ needs.prepare.outputs.sha }}`.
- Split GitHub release mutations from code execution. `prepare`, `build`, and
  `publish-crate` check out and execute repository code and carry
  `contents: read`. The two write-only jobs never check out or execute project
  code: `create-draft` creates/reuses the draft (no checkout), and
  `publish-release` downloads the exact four named Actions artifacts, uploads
  them to the draft, then seals it — it never executes the downloaded binaries
  or repository scripts. Build matrix jobs always upload named Actions
  artifacts (dry-run artifacts remain user-visible); four-platform completeness
  flows through `needs`.
- Preserve current behavior:
  - release mode skips an already-published GitHub release;
  - leftover draft remains retryable;
  - release mode creates/reuses draft, builds four targets, uploads assets,
    publishes release, and publishes crate idempotently;
  - dry-run never creates/edits release, uploads release assets, or publishes
    crate; it builds four targets and uploads Actions artifacts.
- Set crates token from explicitly declared workflow secret, not ambient secret
  inheritance.

### `.github/workflows/release-dry-run.yml`

- `name: Release dry run`.
- Trigger only `workflow_dispatch` with current optional `ref` input semantics
  and default `main`.
- Explicit top-level or job `permissions: contents: read`.
- One reusable-workflow call to `./.github/workflows/release.yml`, passing
  `mode: dry-run` and selected ref. Pass no secrets. Grant no write permission.

### `.github/workflows/publish-mcp-registry.yml`

- Convert trigger to `on: workflow_call` only with required `ref` input.
- Convert trigger to `on: workflow_call` only with required `ref` and
  `version` inputs.
- Remove `workflow_run` condition and event-derived SHA; the caller (trusted CI
  or backfill) supplies the immutable ref and the strict SemVer version.
- Strictly validate `version` before it is used in any tag/path argument.
- Keep explicit least privilege: `contents: read`, `id-token: write`.
- Checkout the current trusted `inputs.ref` with `fetch-depth: 0`; read the
  historical template ONLY as data via `git show v<version>:server.json` —
  never execute historical code.
- Verify the published GitHub release/tag exists (`gh release view`); keep the
  exact registry-version idempotency gate.
- Download each exact asset fail-closed with `gh release download` into clean
  staging (no curl-to-hash pipelines); fetch GitHub release asset metadata
  (size + sha256 digest); build the manifest with
  `scripts/build_registry_manifest.py`, which fails before output commit on
  invalid version, template version mismatch, pre-existing packages, tag
  mismatch, missing/duplicate/unexpected metadata, missing/non-regular/empty
  local assets, size mismatch, or digest missing/mismatched.
- Validate the generated manifest schema with the independent
  `nix shell .#jsonschema-cli` package — never `nix develop`, never building
  serial-mcp. Publish the staged manifest with mcp-publisher OIDC.
- Add `publish-mcp-registry-backfill.yml` (`workflow_dispatch`, strict version
  input, `contents: read` + `id-token: write`, no secrets): calls the current
  local publisher with `ref: ${{ github.sha }}` so already-released versions
  (e.g. 0.9.0/0.9.1) can be published without checking out historical code.

### `.github/workflows/schema-drift.yml`

- Add explicit `permissions: contents: read`.

### `scripts/build_registry_manifest.py` + `scripts/tests/`

- Standard-library, offline, fail-closed manifest builder. Inputs: historical
  template server.json, strict SemVer version, release metadata JSON (tag +
  assets with size + sha256 digest), asset directory, output path. Fixed
  expected assets: serial-mcp-x86_64-linux, serial-mcp-aarch64-linux,
  serial-mcp-aarch64-macos, serial-mcp-x86_64-windows.exe. Fails before output
  commit on: invalid version; template version mismatch; template already has
  packages; release/tag mismatch; missing/duplicate/unexpected metadata;
  missing/not-regular/empty local file; local size differs from GitHub
  metadata; digest missing/not-sha256/mismatched. Builds package URLs for
  v<version>, hashes local files, preserves template fields, writes atomically
  (temp + rename).
- `scripts/tests/test_build_registry_manifest.py` (stdlib unittest) covers the
  happy path and every failure class, deterministic and offline. Run by Ubuntu
  CI and by the `registry-manifest-builder-tests` flake check — never left
  unused.

### `flake.nix`

- Extend the source filter to include the complete `.github/workflows/` and
  `scripts/` trees (doc_drift and the builder tests read them at runtime).
- Export `jsonschema-cli` as an independent flake package (no dependency on
  serial-mcp) so the publisher can validate the manifest schema without
  building the project.
- Add `workflow-fixtures-present` and `registry-manifest-builder-tests` checks:
  `nix flake check` proves the filtered source ships every workflow fixture
  and that the offline unittest suite passes.

### Regression tests in `tests/doc_drift.rs`

Add public-boundary static contract tests over committed workflow text. Tests
must fail if future edits recreate vulnerable orchestration. At minimum prove:

- No workflow file contains a `workflow_run:` trigger.
- CI and schema-drift have explicit read-only top-level permissions.
- CI release and registry caller jobs contain trusted `push` +
  `refs/heads/main` gates, required dependency ordering, and narrow permissions.
- CI passes only named crates secret; `secrets: inherit` is absent from all
  workflow files.
- Release and registry workflows are reusable (`workflow_call`) rather than
  independently privileged event handlers.
- Dry-run caller is `workflow_dispatch`, read-only, hardcodes `mode: dry-run`,
  and passes no secrets.
- No workflow contains `github.event.workflow_run.head_sha`.
- Code-executing jobs (`prepare`/`build`/`publish-crate`) are `contents: read`
  with no `contents: write`; the write jobs (`create-draft`/`publish-release`)
  contain no checkout/cargo/nix and never execute project code; later
  checkouts use the resolved immutable SHA rather than mutable `inputs.ref`.
- Workflow parsers normalize CRLF (Windows checkouts) and are proven against a
  CRLF fixture.
- The flake source filter ships `.github/workflows/` and `scripts/`.
- The publisher workflow never uses `nix develop`, uses the offline manifest
  builder + `gh release download` + `git show` data extraction, and declares
  required `ref` + `version` inputs.
- The backfill workflow is dispatch-only, read-only, passes `version` +
  `github.sha`, and carries no secrets/checkout.
- The failure notifier is push-main gated with `always()`, depends on both the
  release and registry jobs, carries `issues: write` only, and has no
  checkout/project execution.

Keep helpers understandable. Avoid brittle exact-whole-file snapshots. Tests
should inspect relevant bounded job sections or normalized text and include
clear failure messages.

### `AGENTS.md`

Update release workflow section to state executable truth: trusted main push CI
calls reusable release after all required CI jobs, registry call follows
release, and manual dry-run is separate/read-only. Preserve artifact and
version-bump facts.

## Verification

Run:

```bash
cargo fmt --all -- --check
cargo test --locked --test doc_drift
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Also inspect all workflow files for forbidden privileged trigger/data patterns:

```bash
rg -n 'workflow_run|pull_request_target|secrets:\s*inherit|github\.event\.workflow_run' .github/workflows
```

Expected: no matches. Validate git diff and status. Do not commit, push, merge,
open PR, dismiss alerts, or weaken tests. Return files changed, behavior change,
commands/results, deviations, and blockers.

## Escalation

Stop and report rather than inventing architecture if reusable-workflow syntax
cannot express job-level permissions/secret flow, GitHub validation contradicts
this design, or preserving current release semantics requires a new privileged
event path. Include exact evidence and smallest focused question.
