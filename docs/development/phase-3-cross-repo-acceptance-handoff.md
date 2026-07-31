# Phase 3 Handoff — Cross-Repository Acceptance

## Goal

Prove the published `nix-nrf-dev` revision and migrated `serial-mcp` shell work
across real consumers, verify no compatibility workaround remains, and record
final acceptance without changing `le-audio-receiver`'s pinned dependency.

## Scope

Validate three repositories:

- `/home/thomas-workstation/repos/nix-nrf-dev`
- `/home/thomas-workstation/repos/serial-mcp`
- `/home/thomas-workstation/repos/le-audio-receiver`

Create a concise acceptance-results document in
`serial-mcp/docs/development/` containing exact commands, revisions, and
results. Commit only that results document and this handoff in `serial-mcp`.

## Fixed decisions

- Reviewed library revision: `70b469320c8785b7b86ec25410e797e2ac3b0333`.
- `serial-mcp` migration commit: `e43ff05`.
- `le-audio-receiver` currently pins older library revision `e9d7736`.
  Do not update its lock file during acceptance. Use a temporary Nix input
  override so the test proves the new revision is backward-compatible:

  ```bash
  --override-input nix-nrf-dev github:qarnet/nix-nrf-dev/70b469320c8785b7b86ec25410e797e2ac3b0333
  ```

- No hardware flashing or serial interaction is required.
- Both receiver firmware build targets should run because they exercise the
  shared scoped `west` path against distinct NCS build shapes.

## Validation

### 1. nix-nrf-dev

Preserve pre-existing untracked `goals.md`. Verify branch tracks published
main and run:

```bash
nix flake check -L
nix develop .#clean-env-test --command sh -ceu '
  case "${LD_LIBRARY_PATH:-}" in *ncs/toolchains*) exit 1;; esac
  case "${PYTHONPATH:-}" in *ncs/toolchains*) exit 1;; esac
  case "${GIT_EXEC_PATH:-}" in *ncs/toolchains*) exit 1;; esac
  test -z "${PYTHONHOME:-}"
  nix --version
  node --version
  git --version
  python3 -c "import json"
'
```

Sanitize only outer command invocation if the old parent environment remains
poisoned. Do not weaken assertions inside the new shell.

### 2. serial-mcp

Verify clean worktree before adding Phase 3 docs and confirm lock pin. Run:

```bash
direnv exec . nix flake check -L
direnv exec . sh -ceu '
  case "${LD_LIBRARY_PATH:-}" in *ncs/toolchains*) exit 1;; esac
  test -z "${PYTHONHOME:-}"
  node --version
  serial-mcp-dev --version
'
```

Repeat timeout-guarded stdio initialize handshakes for:

- `serial-mcp-dev --allowlist=/dev/pts/*`
- `npx -y @brave/brave-search-mcp-server`

Do not print API keys.

### 3. le-audio-receiver with new library override

First verify its worktree is clean. Enter the overridden shell and prove
external-tool cleanliness plus tool availability:

```bash
nix develop \
  --override-input nix-nrf-dev github:qarnet/nix-nrf-dev/70b469320c8785b7b86ec25410e797e2ac3b0333 \
  --command sh -ceu '
    case "${LD_LIBRARY_PATH:-}" in *ncs/toolchains*) exit 1;; esac
    case "${PYTHONPATH:-}" in *ncs/toolchains*) exit 1;; esac
    case "${GIT_EXEC_PATH:-}" in *ncs/toolchains*) exit 1;; esac
    test -z "${PYTHONHOME:-}"
    nix --version
    git --version
    python3 -c "import dbus, gi"
    command -v west
    command -v nrfutil
    command -v openocd
    command -v nrf-probes
  '
```

Build both targets through the overridden shell:

```bash
nix develop \
  --override-input nix-nrf-dev github:qarnet/nix-nrf-dev/70b469320c8785b7b86ec25410e797e2ac3b0333 \
  --command fw-build-5340

nix develop \
  --override-input nix-nrf-dev github:qarnet/nix-nrf-dev/70b469320c8785b7b86ec25410e797e2ac3b0333 \
  --command fw-build-54l15
```

Do not modify or commit `le-audio-receiver/flake.lock`. Verify status after
build; generated build outputs are ignored and tracked files must remain
unchanged.

### 4. Cleanup audit

In `serial-mcp`, search current config/source/docs for live references to:

- `scripts/opencode-mcp-clean-env`
- global shell-hook evaluation of `nrfutil sdk-manager toolchain env`
- local `nrfutilCoreSrc` / `nrfutil-core` derivation

References inside migration plan/handoff history are acceptable. Runtime
config, current AGENTS guidance, and `flake.nix` must reflect scoped ownership.

## Results document

Add:

```text
docs/development/nix-nrf-dev-migration-acceptance.md
```

Include:

- accepted revisions
- tool versions
- clean-variable results
- each command and pass/fail status
- receiver build result summaries
- MCP serverInfo results
- final worktree status for all three repositories
- explicit statement that no hardware was flashed
- any deviation or residual follow-up

Keep it factual and concise. Do not claim tests not run.

## Constraints

- Do not modify `nix-nrf-dev` tracked files.
- Preserve its pre-existing untracked `goals.md`.
- Do not modify `le-audio-receiver` tracked files or dependency lock.
- Do not alter serial runtime, firmware, flake, OpenCode config, or Phase 2
  implementation unless validation finds a real defect. If a defect appears,
  stop before broad changes and report exact evidence.
- Do not push, merge, open a PR, amend, force-push, or add attribution footers.

## Commit

In `serial-mcp`, inspect `git status`, `git diff`, and
`git log --oneline -10`. Stage only:

- this handoff
- acceptance results document

Commit with:

```text
docs(nix): record scoped shell acceptance
```

## Return recap

Return command results, build summaries, repository statuses, files committed,
commit hash/message, defects, deviations, and recommendation on pushing
`serial-mcp` commits.
