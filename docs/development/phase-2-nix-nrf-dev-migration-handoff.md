# Phase 2 Handoff — Migrate serial-mcp to nix-nrf-dev

## Goal

Replace `serial-mcp`'s globally evaluated Nordic sdk-manager environment with
the reviewed `nix-nrf-dev` scoped shell. Keep Rust and native_sim development
working, make external tools safe, and start the project MCP server directly
from a source-matched executable already built during shell activation.

## Grounding evidence

- `flake.nix:260-296` currently evaluates
  `nrfutil sdk-manager toolchain env` into the whole shell. This exports
  Nordic `PYTHONHOME`, `PYTHONPATH`, `LD_LIBRARY_PATH`, and `GIT_EXEC_PATH`.
- Reproduced failure: Nordic Brotli 1.0.7 shadows Nix's Brotli and Node 24
  exits with `undefined symbol: BrotliEncoderAttachPreparedDictionary`.
- `nix-nrf-dev` commits `687e4d0` and `70b4693` are published on
  `qarnet/nix-nrf-dev/main`. They add `mkNrfShell.inputsFrom`, keep sdk-manager
  variables scoped to the `west` wrapper, and test Nix/Node 24/Git/Python.
- `firmware/bin/fw-build-native:28-29` invokes `west build`; it will therefore
  enter the scoped Nordic environment without changing the helper.
- `firmware/bin/fw-common.sh:23-53` only requires `west` and `ZEPHYR_BASE`,
  both provided by `mkNrfShell`.
- `opencode.json:28-37` currently starts the server through
  `nix run .#serial-mcp-dev`. A cold package build exceeded OpenCode's MCP
  initialization deadline. A shell package referencing the exact server
  derivation shifts this cost to visible direnv activation.
- `clangd` resolves independently to `/run/current-system/sw/bin/clangd`
  version 21.1.8, so removing Nordic PATH pollution does not remove the LSP.

## Exact implementation

### 1. Consume nix-nrf-dev

Edit `flake.nix` inputs:

```nix
nix-nrf-dev = {
  url = "github:qarnet/nix-nrf-dev";
  inputs.nixpkgs.follows = "nixpkgs";
};
```

Add `nix-nrf-dev` to the `outputs` argument set. Update `flake.lock` through
normal Nix commands and verify it pins a revision containing `70b4693`.

### 2. Remove duplicated Nordic shell implementation

Delete from `flake.nix`:

- local `nrfutilCoreSrc`
- local `nrfutil-core` derivation
- global sdk-manager `eval` shell-hook block
- duplicate `ZEPHYR_BASE` derivation
- duplicate multilib package/path setup supplied by `mkNrfShell`

Keep local `mcp-publisher`, crane package builds, cross-compilation outputs,
apps, checks, and release behavior unchanged.

### 3. Compose the hybrid shell

Replace `craneLib.devShell` with
`nix-nrf-dev.lib.${system}.mkNrfShell` using this shape:

- `name = "serial-mcp"`
- `ncsVersion = "v3.3.0"`
- `withMultilib = true`
- `inputsFrom = [ serial-mcp ]`
- explicit `rustToolchain` in `packages`, because `craneLib.devShell` no
  longer injects it
- retain `cargo-watch`, `cargo-edit`, `cargo-nextest`, `jsonschema-cli`, and
  local `mcp-publisher`
- retain project helper PATH entries for `scripts/` and `firmware/bin/` via
  `extraShellHook`

Do not globally set Nordic Python or library variables. Do not duplicate
`nix-nrf-dev`'s `west`, nrfutil, OpenOCD, probes, or multilib packages.

### 4. Add source-matched MCP executable to shell

Create an internal Nix wrapper package using `pkgs.writeShellScriptBin`:

```nix
serial-mcp-dev = pkgs.writeShellScriptBin "serial-mcp-dev" ''
  exec ${serial-mcp}/bin/serial-mcp "$@"
'';
```

Include it in dev-shell `packages`. Its reference to `serial-mcp` must cause
the current source derivation to be built during shell realization. Keep the
existing flake app `apps.serial-mcp-dev` for compatibility; only OpenCode
startup changes.

### 5. Simplify OpenCode MCP commands

Edit `opencode.json`:

- `serial-mcp.command` becomes:

  ```json
  [
    "serial-mcp-dev",
    "--allowlist=/dev/ttyACM*,/dev/ttyUSB*,/dev/pts/*"
  ]
  ```

- `searxng.command` becomes plain:

  ```json
  ["npx", "-y", "mcp-searxng@v1.0.5"]
  ```

Global Brave Search config remains unchanged. It should work because OpenCode
now inherits a clean project shell.

Delete `scripts/opencode-mcp-clean-env` after direct external-tool and MCP
handshakes pass. Ensure no live references remain outside historical migration
documentation.

### 6. Documentation

- Update root `AGENTS.md` firmware/Nix section: `nix-nrf-dev` owns scoped
  Nordic environment; normal shell remains clean; `west` loads it per command.
- Update other current docs only where statements become false.
- Keep and stage:
  - `docs/development/nix-nrf-dev-migration-plan.md`
  - this handoff document
- Historical changelog entries stay unchanged.

## Invariants

- No runtime Rust, MCP schema, serial protocol, or firmware source changes.
- No release version bump.
- No global OpenCode configuration edits.
- No `unwrap`/`expect`/`println!` production-code changes are relevant.
- Preserve app/package names and release outputs.
- Preserve `native_sim` build directory and LSP compile database behavior.
- Preserve firmware helper names and `west` command path.
- Preserve pre-existing clean worktree except the two untracked migration docs
  created by the orchestrator; include those docs in this phase commit.
- Do not push, merge, open a PR, amend, force-push, or add attribution footers.

## Validation order

Outer shell is currently poisoned. Before first flake update, invoke Nix with
`LD_LIBRARY_PATH`, `PYTHONHOME`, `PYTHONPATH`, and `GIT_EXEC_PATH` unset as
needed. After migration and `direnv reload`, ordinary `direnv exec .` commands
must work without cleanup.

### Flake and clean-shell checks

```bash
direnv reload
direnv exec . sh -ceu '
  case "${LD_LIBRARY_PATH:-}" in *ncs/toolchains*) exit 1;; esac
  case "${PYTHONPATH:-}" in *ncs/toolchains*) exit 1;; esac
  case "${GIT_EXEC_PATH:-}" in *ncs/toolchains*) exit 1;; esac
  test -z "${PYTHONHOME:-}"
  nix --version
  node --version
  git --version
  rustc --version
  cargo --version
  command -v west
  command -v nrfutil
  command -v serial-mcp-dev
'
nix flake check -L
```

Confirm `command -v serial-mcp-dev` resolves to a Nix store wrapper from this
shell, and `serial-mcp-dev --version` reports repository version 0.8.1.

### Project gates

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
fw-build-native
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

### MCP handshakes

Use JSON-RPC initialize requests over stdio and require successful
`serverInfo` responses:

1. `serial-mcp-dev --allowlist=/dev/pts/*`
2. `npx -y @brave/brave-search-mcp-server` with existing `BRAVE_API_KEY`
3. `npx -y mcp-searxng@v1.0.5` with existing `SEARXNG_URL`

Use `timeout` so a server cannot hang the validation. Never print API keys.

### Final inspection and commit

```bash
git status
git diff
git log --oneline -10
```

Stage only intended migration files. Commit completed work before returning.
Suggested message:

```text
fix(nix): scope Nordic toolchain environment
```

## Return recap

Return:

1. Files changed/deleted.
2. Final flake composition and pinned `nix-nrf-dev` revision.
3. Clean-shell variable and tool results.
4. Every project gate and MCP handshake result.
5. Commit hash/message.
6. Blockers or deviations with exact evidence.
