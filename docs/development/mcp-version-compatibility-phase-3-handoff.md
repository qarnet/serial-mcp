# MCP Version Compatibility — Phase 3 Handoff

## Goal

Add an actual historical rmcp `1.7.0` client interoperability fixture for MCP
`2025-11-25`, then make one bounded script the executable local/CI compatibility
gate for both supported protocol versions.

Implement this phase, run verification, inspect diff/status/log, and commit the
complete phase. Do not push.

## Worktree and Starting Point

Work only in:

```text
/home/thomas-workstation/repos/serial-mcp-pr50-analysis
```

Starting detached HEAD: `ae466564`. Worktree must start clean. Read repository
`AGENTS.md`, full compatibility plan, and prior phase source before editing.

Disk pressure was resolved with user-approved cleanup; about 80 GB is free.

## Grounding

- Current root package resolves rmcp `3.0.1`; its legacy typed tests prove rmcp
  3 compatibility mode, not an old client implementation.
- `origin/main` before migration resolved rmcp `1.7.0`, checksum
  `0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e`.
- Local pinned rmcp 1.7 source confirms:
  - `ProtocolVersion::LATEST == V_2025_11_25`;
  - `().serve(StreamableHttpClientTransport::from_uri(url))` performs the
    initialize/session lifecycle;
  - `TokioChildProcess::new(tokio::process::Command)` supports stdio;
  - `RunningService<RoleClient, ()>` exposes `peer_info`, list methods,
    `call_tool`, and `cancel`;
  - `CallToolRequestParams::new(...).with_arguments(...)` is available.
- Historical source paths for load-bearing APIs:
  `/home/thomas-workstation/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/rmcp-1.7.0/src/{model.rs,service/client.rs,transport/child_process.rs,transport/common/reqwest/streamable_http_client.rs}`.
- Existing official conformance and Inspector commands live in
  `.github/workflows/ci.yml`; exact pins and scenario lists are guarded by
  `tests/doc_drift.rs`.
- Existing Inspector driver remains `scripts/inspector-smoke.mjs`; do not
  duplicate its assertions or package-resolution logic.
- No conformance-only production behavior may be added.

## In Scope

### 1. Standalone historical client package

Add:

```text
compat/rmcp-1-client/Cargo.toml
compat/rmcp-1-client/Cargo.lock
compat/rmcp-1-client/src/main.rs
```

This package is standalone, not a root workspace member, and has
`publish = false`. Use edition 2021 and the repository Rust toolchain.

Dependencies must be minimal and exact where compatibility depends on them:

```toml
rmcp = { version = "=1.7.0", default-features = false, features = [
  "client",
  "transport-child-process",
  "transport-streamable-http-client-reqwest",
] }
```

Add only needed `tokio`, `serde_json`, `anyhow`, and `tempfile` dependencies.
Commit the generated nested `Cargo.lock`; build it with `--locked`. Confirm its
rmcp package entry is exactly 1.7.0 and carries the historical checksum above.

#### CLI contract

Use a small manual CLI with exactly two modes:

```text
rmcp-1-client http <server-url>
rmcp-1-client stdio <absolute-serial-mcp-binary>
```

Invalid/missing arguments print concise usage to stderr and exit nonzero.
Wrap each complete mode in a 30-second Tokio timeout. Errors carry operation
context and exit nonzero; no panic-based runtime validation.

HTTP mode uses rmcp 1.7's reqwest Streamable HTTP transport against the passed
URL. Stdio mode:

- creates a temporary directory;
- passes `--profiles-path <temp>/profiles.toml` and `RUST_LOG=off` to the
  current binary;
- uses rmcp 1.7 `TokioChildProcess`;
- keeps temp directory alive through client shutdown.

Both modes run the same public-behavior verifier and assert:

1. peer info exists;
2. negotiated protocol is exactly `V_2025_11_25`;
3. server implementation name is `serial-mcp`;
4. exact 25-tool name set equals an independent fixture-local constant;
5. static resource URIs are exactly `serial://ports` and
   `serial://connections`;
6. resource template URIs are exactly connection detail/raw/log templates;
7. prompt names are exactly `diagnose_port` and `interactive_terminal`;
8. `compute_checksum` with
   `{"algorithm":"xor","data":"$GPGGA,1","encoding":"utf8"}` succeeds
   and structured result contains integer `checksum=111` and string
   `checksum_hex="6F"`.

Use all-page rmcp 1.7 helpers where available so pagination cannot hide items.
Always cancel/shut down cleanly after verification, including stdio child
cleanup. A verification failure must still attempt client cancellation before
returning the error.

On success print one JSON line to stdout containing at least:

```json
{"mode":"http|stdio","protocolVersion":"2025-11-25","tools":25,"status":"ok"}
```

Do not depend on serial-mcp library internals or import current test constants.
This fixture is independent implementation evidence.

### 2. Shared compatibility runner

Add executable:

```text
scripts/test-mcp-compat.sh
```

Linux/CI scope is explicit. Use `#!/usr/bin/env bash`, `set -euo pipefail`,
repository-root resolution from script path, quoted paths, and a trap that
always stops/waits for the HTTP server and removes only its own `mktemp -d`
directory.

Supported environment overrides:

```text
SERIAL_MCP_BIN             default: target/debug/serial-mcp
MCP_COMPAT_PORT            default: 8931
MCP_COMPAT_REPORT_DIR      default: target/conformance-results
MCP_COMPAT_CARGO_TARGET    default: target/mcp-compat-rmcp-1
```

Overrides may select paths/port only. Protocol versions, package pins,
scenario lists, expected-failure path, and assertions remain fixed in source.
Do not delete an arbitrary override directory. Create report directory and
write/replace only known files owned by this run (for example `server.log` and
`server.pid`); conformance scenario output directories may be reused as the
pinned runner supports.

Runner sequence:

1. `cargo build --locked --bin serial-mcp` unless `SERIAL_MCP_BIN` already
   names an executable; validate executable before use.
2. Run focused Rust gates:
   - `cargo test --locked --test protocol_compatibility`
   - `cargo test --locked --test stdio_integration`
   - `cargo test --locked --test resource_subscriptions`
3. Build historical fixture with its own manifest and lock into the stable
   compatibility target directory.
4. Run historical fixture stdio mode.
5. Start current HTTP binary on `127.0.0.1:$MCP_COMPAT_PORT` with an isolated
   temporary profile path; redirect logs to known report file.
6. Use the existing bounded raw `server/discover` curl readiness probe for
   `2026-07-28`; fail with server log if readiness never reaches HTTP 200.
7. Run historical fixture HTTP mode against `/mcp`.
8. Run exact official legacy conformance scenarios with
   `@modelcontextprotocol/conformance@0.2.0-alpha.10` and
   `--spec-version 2025-11-25`:
   `server-initialize ping completion-complete tools-list resources-list prompts-list`.
9. Run exact official current conformance scenarios with same package and
   `--spec-version 2026-07-28`:
   `server-stateless completion-complete tools-list resources-list prompts-list caching sep-2164-resource-not-found`.
10. Every scenario uses
    `--expected-failures conformance/expected-failures.yaml` and writes a
    stable version-suffixed report directory.
11. Run `node scripts/inspector-smoke.mjs <url>`; that script owns exact
    `@modelcontextprotocol/inspector@2.0.0` fallback.
12. Print concise success summary. Trap handles server cleanup on success,
    failure, signal, or timeout.

Use GNU `timeout` around historical fixture invocations and each conformance
scenario (180 seconds each is acceptable) so local execution is bounded in
addition to CI's 15-minute job timeout. Never suppress an exit code, use
`|| true` around a gate, or run `--suite all`. The readiness probe may retain
its existing `curl ... || true` because it explicitly polls and validates the
HTTP status.

### 3. CI delegates to shared runner

Update `.github/workflows/ci.yml` `mcp-conformance` job:

- keep exact Node `22.19.0`, Rust `1.97.1`, system packages, cache, 15-minute
  timeout, `contents: read`, and always-uploaded v7 artifact with seven-day
  retention;
- replace duplicated build/start/readiness/scenario/Inspector/stop shell steps
  with one named step running `bash scripts/test-mcp-compat.sh`;
- keep comments explaining exact versions, expected-failure semantics, no
  fixture endpoints, and Inspector-vs-conformance distinction;
- artifact path stays `target/conformance-results/`.

### 4. Minimum drift-test migration required now

Because root CI runs `cargo test --locked`, update `tests/doc_drift.rs` in this
phase so moving scenario ownership does not leave failing or false tests.

- `ci_conformance_job_pins_packages_and_never_runs_suite_all` should assert CI
  pins Node/job bounds/artifact handling and invokes
  `bash scripts/test-mcp-compat.sh`.
- Add/read shared-runner assertions proving exact conformance package,
  Inspector script invocation, expected-failure argument, `set -euo pipefail`,
  and no non-comment `--suite all` usage.
- `ci_scenario_lists_match_pinned_runner_scenarios` should inspect
  `scripts/test-mcp-compat.sh` instead of CI YAML, preserving exact existing
  scenario expectations and prohibition of `server-session-lifecycle`.
- Preserve existing expected-failure and Inspector-script tests unchanged.
- Phase 4 will add broader policy/README/historical-lock drift checks; do not
  preempt those except where needed for executable truth now.

If Nix source filtering omits `compat/` files needed by tests, update
`flake.nix` minimally and explain why. Do not change Nix dependencies.

## Out of Scope

- Production server behavior.
- Additional MCP versions or pre-`2025-11-25` support.
- New-client/old-server testing.
- Fake conformance tools/resources/capabilities.
- Changes to four expected-failure IDs.
- Floating npm/Cargo versions.
- README, CHANGELOG, AGENTS, FEATURES, or durable policy doc; Phase 4 owns them.
- Full repository/native_sim/Nix gate; Phase 4 owns final gate.

## Verification

Run:

```bash
cargo fmt --manifest-path compat/rmcp-1-client/Cargo.toml --all -- --check
cargo build --locked --manifest-path compat/rmcp-1-client/Cargo.toml --target-dir target/mcp-compat-rmcp-1
cargo clippy --locked --manifest-path compat/rmcp-1-client/Cargo.toml --all-targets --target-dir target/mcp-compat-rmcp-1 -- -D warnings
cargo test --locked --test doc_drift
bash scripts/test-mcp-compat.sh
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
```

Inspect fixture output for both `stdio` and `http` success summaries. Inspect
`target/conformance-results/` for server log and all version-suffixed scenario
reports. Confirm exact four expected failures and no unexpected/stale failures.

## Commit and Recap

Stage only Phase 3 files and this handoff. Commit:

```text
test: add historical MCP client compatibility gate
```

No attribution footer. Do not push, merge, amend, or open a PR.

Return recap with files, fixture dependency/lock proof, HTTP+stdio observations,
conformance results by version, Inspector result, all commands/results, commit
hash/message, git status, deviations, blockers, and follow-up.

Stop and escalate before committing if rmcp 1.7 cannot interoperate without
production changes, the official runner behavior contradicts accepted
expected-failure semantics, cleanup is unreliable, tests would need weakening,
or scope must expand. Preserve exact command/errors/logs and partial state.
