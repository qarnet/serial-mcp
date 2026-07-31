# Clean Nordic Development Shell Migration Plan

## Goal

Make `nix-nrf-dev` the single reusable owner of Nordic development-shell
behavior, then migrate `serial-mcp` to it. Nordic's sdk-manager environment
must remain scoped to firmware commands so unrelated programs launched from
the shell—OpenCode, MCP servers, Nix, Node, Git, and project Python—use their
own compatible libraries.

## Current state and evidence

- `serial-mcp/flake.nix` does not consume `nix-nrf-dev`. Its shell hook runs
  `eval "$(nrfutil sdk-manager toolchain env ...)"` globally.
- That sdk-manager environment adds
  `~/ncs/toolchains/911f4c5c26/usr/lib/x86_64-linux-gnu` to
  `LD_LIBRARY_PATH`.
- The bundled Brotli 1.0.7 library lacks
  `BrotliEncoderAttachPreparedDictionary`; Node 24 therefore fails before
  Brave Search MCP can start.
- The same directory causes the system `nix` executable to fail to resolve
  `libcom_err.so.2`.
- `nix-nrf-dev/nix/mk-nrf-shell.nix` already implements the desired boundary:
  shell stays clean and a `west` wrapper evaluates the Nordic environment only
  inside the firmware command process tree.
- `le-audio-receiver/flake.nix` is a working consumer of that API.
- `serial-mcp` currently starts its development MCP through
  `nix run .#serial-mcp-dev`. When its derivation was absent from the store,
  OpenCode hit its approximately 30-second MCP initialization deadline during
  the cold build. Once built, the same command started in about 0.17 seconds.

## Design decisions

1. Fix reusable shell composition in `nix-nrf-dev`; do not add more
   application-specific environment-cleaning wrappers.
2. Keep Nordic toolchain variables scoped to `west` and descendants.
3. Add `inputsFrom` to `mkNrfShell` so hybrid consumers can inherit build
   inputs from their own derivations.
4. Add executable clean-shell regression coverage, including Node startup,
   because inspecting variable names alone would not catch dynamic-linker
   regressions.
5. During `serial-mcp` migration, put a source-matched development server
   executable on the dev-shell PATH. OpenCode must start that executable
   directly rather than invoking a potentially cold Nix build.
6. Preserve `nix-nrf-dev`'s existing scoped `west` behavior and
   `serial-mcp`'s native_sim multilib support.

## Phase 1 — Harden `nix-nrf-dev`

### Scope

- Add `inputsFrom ? []` to `mkNrfShell` and pass it to `pkgs.mkShell`.
- Add a test-only dev shell containing Node, Git, and Python.
- Add CI validation that the shell does not expose Nordic toolchain paths in
  toxic variables and that Nix, Node, Git, and Python execute.
- Document `inputsFrom` and the clean-shell contract.

### Exclusions

- No hermetic packaging of the full NCS toolchain.
- No generic `nrf-env` command unless a concrete non-`west` consumer requires
  one during migration.
- No OpenOCD, probe, flashing, or hardware behavior changes.
- No `serial-mcp` edits.

### Acceptance

```bash
nix fmt
nix flake check -L
pre-commit run --all-files
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

## Phase 2 — Migrate `serial-mcp`

### Scope

- Add `nix-nrf-dev` as a flake input following `serial-mcp`'s `nixpkgs`.
- Compose `mkNrfShell` with the existing crane/Rust inputs and project tools.
- Remove the global sdk-manager environment evaluation and duplicate local
  nrfutil packaging that the library now owns.
- Preserve Rust, schema, release, native_sim, clangd, and firmware helper
  behavior.
- Add a shell-provided `serial-mcp-dev` executable tied to the current source
  derivation.
- Change project OpenCode config to start `serial-mcp-dev` directly.
- Remove `scripts/opencode-mcp-clean-env` after all clean-shell and MCP tests
  pass.
- Update `AGENTS.md` and relevant development docs to describe the new source
  of shell behavior.

### Exclusions

- No serial protocol, MCP schema, runtime, or firmware behavior changes.
- No global OpenCode configuration edits as the primary fix.
- No release version bump.

### Acceptance

```bash
direnv reload
direnv exec . nix --version
direnv exec . node --version
direnv exec . git --version
direnv exec . cargo fmt --all -- --check
direnv exec . cargo build --all-targets --locked
direnv exec . cargo test --locked
direnv exec . cargo clippy --all-targets --locked -- -D warnings
direnv exec . nix flake check
direnv exec . fw-build-native
```

Also perform direct MCP initialize handshakes for Brave Search and
`serial-mcp`, then fully restart OpenCode and confirm both servers load.

## Phase 3 — Cross-repository acceptance and cleanup

- Pin `serial-mcp` to the reviewed `nix-nrf-dev` revision.
- Re-run both repositories' full gates.
- Verify `le-audio-receiver` still enters its shell and builds through the
  unchanged scoped `west` path.
- Remove any migration-only compatibility code left from Phase 2.
- Record optional follow-up separately: generic `nrf-env`, version tags, and
  hermetic NCS packaging.

## Rollout order

1. Implement, commit, and review Phase 1 in `nix-nrf-dev`.
2. Push/release availability is a separate user-controlled action; for local
   migration testing, use a temporary local flake input override if needed.
3. Implement, commit, and review Phase 2 in `serial-mcp`.
4. Run Phase 3 acceptance before declaring migration complete.
