# GitHub Actions dependencies

Every workflow is onboarded to `.github/workflows/actions.lock`. The lockfile
records the exact commits and repository identities for direct and transitive
actions. Workflow `uses:` references show release tags where available; the
runner executes the locked commits. The migration preserves the previous direct
action commits. `dtolnay/rust-toolchain` and `Swatinem/rust-cache` retain bare SHA
references because those commits have no matching release tag.

## Updating actions

Install the generator version used by the CI verification job:

```bash
gh extension install github/gh-actions-lock --pin v0.4.1
```

If already installed, check `gh actions-lock --version` and use
`gh extension upgrade gh-actions-lock` only when deliberately updating the tool.
Keep the CI installation pin and this procedure in sync.

After adding a workflow or changing its action dependencies:

```bash
gh actions-lock
gh actions-lock --verify
```

Review and commit workflow changes and `actions.lock` together. Never edit the
lockfile manually. Check changed commits as dependency upgrades, and investigate
any changed repository or owner identity. The verifier uses authenticated,
read-only GitHub API access; CI passes its existing read-only `GITHUB_TOKEN`.
`gh actions-lock --verify-local` is a useful offline coverage check, but does not
verify upstream provenance.

Monthly Dependabot Actions updates regenerate matching entries for onboarded
workflows without extra configuration. The existing patch/minor grouping stays
in place. After this migration merges, existing Actions PRs need to be rebased or
recreated so their diffs include the lockfile. Keep major upgrades separate.

For a moved branch or partial tag, normal generation preserves its locked commit.
Use `--relock` only after reviewing the intended upstream changes. Rewritten
upstream history needs investigation; never put `--accept-moved` in automation.

## Coverage and release gating

The CI `verify actions lockfile` job runs `gh actions-lock --verify` on pull
requests and main pushes. Build, Nix and conformance jobs depend on it, so a failed
verification also blocks the existing trusted release gate. The verifier has a
five-minute timeout and does not write repository content.

Reusable workflow calls are not themselves action lock entries. Our release,
registry, dry-run and backfill calls use same-repository paths tied to the workflow
commit, and the called workflow files are onboarded independently. Any future
external reusable workflow must use an exact commit and manage its own actions
lockfile. The lockfile does not pin runner images, downloaded tools or containers.

See GitHub's [lockfile guide](https://docs.github.com/en/actions/how-tos/secure-your-work/lock-workflow-dependencies)
and [generator documentation](https://github.com/github/gh-actions-lock).
