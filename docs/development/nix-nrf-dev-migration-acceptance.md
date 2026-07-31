# nix-nrf-dev Migration — Cross-Repository Acceptance (Phase 3)

Date: 2026-08-01
Scope: validate published `nix-nrf-dev` revision across `nix-nrf-dev`,
`serial-mcp`, and `le-audio-receiver`; confirm no compatibility workaround
remains; record results without touching `le-audio-receiver`'s pinned lock.

## Accepted revisions

| Repository | Revision | Note |
|---|---|---|
| nix-nrf-dev (reviewed) | `70b469320c8785b7b86ec25410e797e2ac3b0333` | local `main` == `origin/main`, 0 ahead / 0 behind |
| serial-mcp (migration commit) | `e43ff05` "fix(nix): scope Nordic toolchain environment" | 1 commit ahead of `origin/main` (unpushed) |
| le-audio-receiver (pinned) | `e9d77367e334213417c10fa7a7ded3e9195f78c0` | unchanged; new revision exercised via `--override-input` only |

No tracked file or lock file in `le-audio-receiver` was modified.

## Tool versions (inside clean shells)

| Tool | Version |
|---|---|
| nix | 2.34.7 |
| node | v24.18.0 (nix-nrf-dev clean shell); v24.16.0 (serial-mcp dev shell) |
| git | 2.51.2 (nix-nrf-dev clean shell); 2.52.0 (le-audio-receiver shell) |
| python3 | `import json` OK (nix-nrf-dev); `import dbus, gi` OK (le-audio-receiver) |
| west | v1.5.0 |
| nrfutil | 8.1.1 (`nrfutil-core-8.1.1` on PATH) |
| openocd / nrf-probes | present on PATH |
| serial-mcp | 0.8.1 |
| rustc | 1.88.0 (serial-mcp dev shell banner) |

## Environment note (outer shell)

The outer environment was still poisoned by the sdk-manager parent shell
(`LD_LIBRARY_PATH`, `PYTHONPATH`, `GIT_EXEC_PATH`, `PYTHONHOME` all pointing
into `~/ncs/toolchains/911f4c5c26`; `nix` and `node` broken under it). Outer
invocations were sanitized with `env -u LD_LIBRARY_PATH -u PYTHONPATH
-u GIT_EXEC_PATH -u PYTHONHOME`. The assertion scripts inside every clean
shell were run verbatim and unchanged.

## Clean-variable assertions (all shells)

`LD_LIBRARY_PATH` / `PYTHONPATH` / `GIT_EXEC_PATH` contain no
`ncs/toolchains`; `PYTHONHOME` empty — verified in:

- nix-nrf-dev `.#clean-env-test` shell
- serial-mcp dev shell (via `direnv exec .`)
- le-audio-receiver shell with `--override-input`

## Command results

### 1. nix-nrf-dev

| Command | Result |
|---|---|
| `nix flake check -L` | PASS — all checks passed (lib, packages incl. openocd-master / nrfutil-core / nrf-probes, formatter, pre-commit, formatting, devShells incl. clean-env-test, templates) |
| `nix develop .#clean-env-test` assertions | PASS — clean vars, nix 2.34.7, node v24.18.0, git 2.51.2, python3 json import |

`goals.md` (pre-existing untracked) preserved; no tracked file modified.

### 2. serial-mcp

| Command | Result |
|---|---|
| `direnv exec . nix flake check -L` | PASS — all checks passed. Non-fatal eval warnings: `serial-mcp-dev` app lacks `meta`; crane cross-compile overrideToolchain splice warning |
| `direnv exec .` shell assertions | PASS — clean vars, node v24.16.0, `serial-mcp-dev --version` → `serial-mcp 0.8.1` |

Lock pin confirmed: `flake.lock` pins nix-nrf-dev `70b469320c8785b7b86ec25410e797e2ac3b0333`.

### 3. MCP stdio initialize handshakes (timeout-guarded, protocolVersion 2025-03-26)

| Server | serverInfo | Result |
|---|---|---|
| `serial-mcp-dev --allowlist=/dev/pts/*` | `{"name":"serial-mcp","version":"0.8.1"}` | PASS |
| `npx -y @brave/brave-search-mcp-server` | `{"name":"brave-search-mcp-server","version":"2.1.0","title":"Brave Search MCP Server"}` | PASS |

No API keys printed.

### 4. le-audio-receiver with `--override-input` (`70b4693`)

| Command | Result |
|---|---|
| override shell assertions | PASS — clean vars; `west`, `nrfutil`, `openocd`, `nrf-probes` on PATH; python `dbus` + `gi` import |
| `nix develop --override-input ... --command fw-build-5340` | PASS — sysbuild app + hci_ipc (net core) + `merged.hex` / `merged_CPUNET.hex` |
| `nix develop --override-input ... --command fw-build-54l15` | PASS — sysbuild incl. FLPR subimage |

Nix reported "not writing modified lock file" (override only). After builds,
`git status --porcelain` empty; `flake.lock` still pins `e9d7736`; no diff
against HEAD.

#### Build summaries

**fw-build-5340** (board `ebyte_e83_nrf5340/nrf5340/cpuapp`, sysbuild):

- App core: FLASH 146780 B / 256 KB (55.99%), RAM 40512 B / 64 KB (61.82%)
- Net core (hci_ipc) built; `merged.hex` + `merged_CPUNET.hex` generated
- Exit 0

**fw-build-54l15** (board `nrf54l15dk/nrf54l15/cpuapp`, sysbuild):

- FLASH 505368 B / 1428 KB (34.56%), RAM 159988 B / 160 KB (97.65%)
- FLPR subimage completed
- Exit 0

Both targets exercise the shared scoped `west` wrapper against distinct NCS
build shapes (dual-core 5340 sysbuild vs. single-core 54L15 with FLPR).

## Cleanup audit (serial-mcp)

| Item | Result |
|---|---|
| `scripts/opencode-mcp-clean-env` | Removed — no such file. References remain only in migration plan / phase-2 / phase-3 handoff history docs |
| global shell-hook eval of `nrfutil sdk-manager toolchain env` | Gone. Only occurrence outside doc history is `.github/workflows/ci.yml` step "Build native_sim firmware", where the eval is scoped to that single step (CI has no Nix); AGENTS.md documents per-command `west` wrapper ownership |
| local `nrfutilCoreSrc` / `nrfutil-core` derivation | No reference in `flake.nix` or runtime config; only doc history mentions |

`flake.nix` devShell comment reflects scoped ownership ("sdk-manager
variables scoped to the west wrapper"). No compatibility workaround remains.

## Final worktree status

- **nix-nrf-dev**: clean tracked files; untracked `goals.md` preserved; `main` == `origin/main` == `70b4693`.
- **serial-mcp**: clean tracked files; 1 commit ahead of `origin/main` (`e43ff05`); staged for this record: handoff + this document (commit `docs(nix): record scoped shell acceptance`).
- **le-audio-receiver**: clean; `flake.lock` pins `e9d7736` (unchanged); generated `build/` outputs ignored.

## Hardware

No hardware was flashed, probed, or interacted with. Build-only acceptance.

## Deviations / residual follow-up

- Deviation: outer invocation sanitized with `env -u ...` due to stale
  sdk-manager pollution in the parent environment; assertions unchanged.
- `nix flake check` emitted non-fatal eval warnings (app `meta` missing,
  crane cross-compile splice) — no failure.
- Follow-up: `serial-mcp` `main` carries two unpushed commits (`e43ff05`,
  this record). Recommend push once the migration record is reviewed.
