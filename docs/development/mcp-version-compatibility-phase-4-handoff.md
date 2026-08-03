# MCP Version Compatibility — Phase 4 Handoff

## Goal

Publish the durable MCP compatibility contract, add complete drift enforcement,
record older pre-`2025-11-25` protocol support as a demand-driven feature idea,
remove stale documentation, and run every repository/compatibility/Nix gate.

Implement this phase, run full verification, inspect diff/status/log, and commit
the complete phase. Do not push.

## Worktree and Starting Point

Work only in:

```text
/home/thomas-workstation/repos/serial-mcp-pr50-analysis
```

Starting detached HEAD: `51602016`. Worktree must start clean. Read repository
`AGENTS.md`, full compatibility plan, all Phase 1–3 commits/source, and current
docs before editing.

## Grounding

Shipped behavior now:

- `src/mcp_protocol.rs` is the exact two-row product policy, preferred
  `2026-07-28` first and permanent `2025-11-25` second. Unknown/future dates
  inherit no policy.
- `tests/protocol_compatibility.rs` has explicit
  `TestProtocol::{V2026_07_28,V2025_11_25}`, typed + independent raw-wire
  matrices, and an advertised-version coverage lock.
- `compat/rmcp-1-client/` is standalone, exact rmcp `1.7.0`, own lockfile, and
  proves real historical-client HTTP + stdio interoperability.
- `scripts/test-mcp-compat.sh` is the single executable local/CI gate and owns
  exact version-indexed scenario sets.
- CI delegates to that runner with exact Node `22.19.0`, Rust `1.97.1`,
  15-minute bound, and always-uploaded reports.
- Official conformance remains separate from Inspector interoperability.
- `2025-11-25` support is permanent product behavior.
- User requested older pre-`2025-11-25` protocol support be recorded only as a
  potential feature. Current supported set remains exactly two versions.

Known stale documentation to fix:

- `AGENTS.md` still names removed `LegacyClientHandler`,
  `connect_modern_client`/`connect_legacy_client`, date-range
  `modern_cache_fields`, `modern_read_result`, old manual conformance commands,
  and incorrectly mentions Inspector `--stored-auth-only`.
- `CHANGELOG.md` still calls cache policy a date-range
  `modern_cache_fields` gate.
- `README.md` mentions conformance generally but not the historical client or
  one-command runner.

## In Scope

### 1. Durable compatibility policy document

Add:

```text
docs/development/mcp-version-compatibility-policy.md
```

This is concise durable documentation, not another implementation plan. Include:

1. Exact support table:
   - `2026-07-28`: preferred, discovery/stateless, subscriptions enabled,
     immediate-private cache fields;
   - `2025-11-25`: permanent, initialize/session, subscriptions disabled,
     cache fields omitted.
2. Single-source rule: only exact `src/mcp_protocol.rs` policy rows are
   supported; rmcp known versions and date ordering do not imply support.
3. Compatibility directions:
   - required: current server with old clients;
   - out of product gate: new clients with historical server binaries, because
     serial-mcp ships a server and old binaries cannot regress from current
     source.
4. Required proof layers and their distinct purposes:
   policy unit tests, typed current-SDK cases, raw wire, stdio, actual rmcp 1.7
   client, official conformance, Inspector.
5. Permanent `2025-11-25` retention rule.
6. Future-version admission checklist from the plan. New versions add rows and
   tests; they never mutate/evict `2025-11-25`.
7. Exact local command:

   ```bash
   bash scripts/test-mcp-compat.sh
   ```

8. Exact pins, report path, expected-failure semantics, no `--suite all`, no
   fake production fixture endpoints.
9. Short industry rationale with official sources already researched:
   Docker Engine API negotiation/matrix, Kubernetes skew policy, Git protocol
   capability advertisement, Protobuf evolution, Connect conformance.

Add policy document to `docs/development/README.md` index.

### 2. User/contributor docs

#### `README.md`

- Keep exact two-version compliance claim.
- Add that backward compatibility is continuously tested with actual rmcp 1.7
  over HTTP and stdio, not only current SDK compatibility mode.
- Under Development, use locked commands and document
  `bash scripts/test-mcp-compat.sh` as the one complete MCP version gate.
- Link durable compatibility policy from Documentation.
- Do not overexpose internal implementation detail in top-level prose.

#### `AGENTS.md`

Update executable truth comprehensively:

- Fast truth: `src/mcp_protocol.rs` exact policy table, permanent
  `2025-11-25`, exact-match cache behavior, no future date inheritance.
- Commands: replace manual one-scenario server/npx sequence with
  `bash scripts/test-mcp-compat.sh`; include standalone fixture fmt/clippy only
  if useful.
- CI summary: shared runner includes current typed/raw/stdio/subscription tests,
  actual rmcp 1.7 HTTP+stdio, both official conformance sets, and Inspector.
- Test map: exact-version `TestProtocol`, `VersionedClientHandler`, current
  helper names, coverage lock, historical fixture, runner, report locations.
- Dual lifecycle section: remove obsolete helper/type names.
- Cache section: exact `cache_fields_for` in `src/mcp_protocol.rs`,
  `read_result_with_cache_fields`; only explicit `2026-07-28` policy receives
  fields, not `2026-07-28+` by date.
- Conformance section: scenario ownership moved to shared runner; exact
  version-suffixed report dirs; `set -euo pipefail` + timeout semantics.
- Inspector: remove false `--stored-auth-only` claim; preserve actual
  `MCP_AUTO_OPEN_ENABLED=false`, non-TTY, connect timeout behavior.
- Historical fixture exact pin/checksum and what it proves.
- Future protocol admission invariant and permanent legacy rule.

Search entire AGENTS file for old names and stale claims after editing.

#### `CHANGELOG.md`

Add an Unreleased subsection describing:

- centralized exact policy table/no date inference;
- explicit version-indexed typed/raw/stdio matrix + coverage lock;
- actual rmcp 1.7 HTTP/stdin-stdout interoperability fixture;
- shared local/CI compatibility runner and exact proof layers;
- permanent `2025-11-25` contract.

Correct stale references to removed `modern_cache_fields` and range semantics.
Do not rewrite historical released sections.

#### `docs/development/FEATURES.md`

Under **Wish**, add:

```text
Earlier MCP protocol revisions (pre-2025-11-25)
```

State:

- current supported set remains exactly `2026-07-28` and permanent
  `2025-11-25`;
- possible older candidates are `2025-06-18`, `2025-03-26`, and
  `2024-11-05`;
- implement only with concrete user/client demand;
- each requires explicit policy row, lifecycle/capability/cache review,
  raw-wire tests, official conformance support where available, and a real
  historical client fixture;
- never enable merely because rmcp lists it in `KNOWN_VERSIONS`.

This is a potential feature, not current support and not near-term commitment.

### 3. Complete drift enforcement

Extend `tests/doc_drift.rs` with behavior-oriented cross-file guards.

#### Exact shell scenario parser

Replace loose `script.contains(sc)` scenario checks with a small parser for
quoted shell assignments. Extract whitespace-separated words from exactly:

```text
SCENARIOS_2025_11_25="..."
SCENARIOS_2026_07_28="..."
```

Assert exact ordered arrays:

```text
2025-11-25:
server-initialize ping completion-complete tools-list resources-list prompts-list

2026-07-28:
server-stateless completion-complete tools-list resources-list prompts-list caching sep-2164-resource-not-found
```

Assert both exact `--spec-version` values and exact report suffixes
`-2025-11-25` / `-2026-07-28`. Keep `server-session-lifecycle` forbidden.
Add synthetic negative proof(s) for parser/check where practical, consistent
with existing doc-drift style.

#### Historical fixture pin

Parse `compat/rmcp-1-client/Cargo.toml` and `Cargo.lock` using existing TOML
dependency. Assert:

- manifest uses exact `=1.7.0` and `default-features = false`;
- required three rmcp client/transport features remain present;
- lock contains exactly one rmcp package entry at `1.7.0`;
- checksum exactly
  `0810a9f717d9828f475fe1f629f4c305c8464b7f496c3a854b58d29e65f4058e`.

Malformed/missing fixture files already fail through `repo_file`; add clear
assertion messages. Do not invoke Cargo/network from drift tests.

#### Contract/docs wiring

Assert:

- README names both supported versions and shared runner command;
- policy doc names both versions in preferred order, permanent
  `2025-11-25`, exact runner, historical fixture, no implicit known-version
  support;
- FEATURES contains pre-`2025-11-25` item and labels it non-current/demand
  driven;
- CI invokes shared runner once and does not duplicate scenario loops;
- runner pins conformance, uses exact expected-failure path, invokes Inspector,
  and contains fixture HTTP + stdio modes;
- exactly four expected failures remain.

Avoid brittle whole-prose snapshots. Check contract-bearing anchors and exact
data lists.

If Nix source filtering omits `compat/rmcp-1-client/Cargo.toml` or lock during
`nix flake check`, update `flake.nix` source filter to admit `/compat` and update
its explanatory comment. This is fixture inclusion only, not dependency change.

### 4. Complete plan status

Update `docs/development/mcp-version-compatibility-plan.md`:

- status = implemented;
- record Phase 1–3 commit hashes and Phase 2/3 review-fix commits;
- state Phase 4 completes durable docs/drift/full verification without trying
  to predict its own hash;
- reflow line 11's overlong sentence;
- keep research and design record intact.

Add this handoff document to the phase commit.

## Out of Scope

- Supporting pre-`2025-11-25` versions now.
- Removing or deprecating `2025-11-25`.
- New-client/old-server gate.
- Production protocol/tool/resource behavior changes.
- Changing expected-failure IDs.
- Floating dependencies.
- Package/release version bump.
- Push, merge, PR operations.

## Verification

Run focused docs/compatibility checks first:

```bash
cargo fmt --all -- --check
cargo fmt --manifest-path compat/rmcp-1-client/Cargo.toml --all -- --check
cargo test --locked --test doc_drift
bash scripts/test-mcp-compat.sh
```

Then full repository gates:

```bash
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --locked --manifest-path compat/rmcp-1-client/Cargo.toml --all-targets --target-dir target/mcp-compat-rmcp-1 -- -D warnings
cargo test --locked --test config_schema_validation
cargo test --locked --test doc_drift
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
nix flake check
```

Also run targeted searches for obsolete names/claims:

```text
LegacyClientHandler
connect_modern_client
connect_legacy_client
modern_cache_fields
modern_read_result
2026-07-28+
--stored-auth-only
```

Classify legitimate historical-plan mentions; no stale current-truth mentions
may remain in README/AGENTS/CHANGELOG/current policy docs.

Every required command must pass. If Nix or native_sim exposes a real defect,
fix it within this phase when directly caused by compatibility changes. Do not
normalize, skip, or weaken failures.

## Commit and Recap

Stage only Phase 4 files and this handoff. Commit:

```text
docs: define MCP version compatibility policy
```

No attribution footer. Do not push, merge, amend, or open a PR.

Return recap containing:

1. Files changed.
2. Durable support contract and feature idea.
3. Drift tests added and synthetic negative proofs.
4. Compatibility runner outcomes by version and historical transport.
5. Full gate command results, including native_sim and Nix.
6. Commit hash/message.
7. Git status.
8. Deviations, blockers, follow-up.

Stop and escalate before committing if full gates cannot pass after two
materially different attempts, source filtering contradicts fixture inclusion,
docs conflict with executable behavior, a test would need weakening, or scope
must expand. Preserve exact evidence and partial state.
