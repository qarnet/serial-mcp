# rmcp 3 Phase 4 Handoff: Cache, Conformance, and Final Gates

## Goal

Finish migration: emit required modern non-cacheable fields without leaking
them to legacy peers, add pinned official conformance and Inspector gates, run
all software-only repository gates, refresh measured docs, and commit complete
Phase 4.

Work only in `/home/thomas-workstation/repos/serial-mcp-pr50-analysis` at Phase
3 HEAD `c36f82cf`. Planning edits to `rmcp-3-migration-plan.md` and
`FEATURES.md` are intentionally uncommitted; include them. Never touch primary
checkout `/home/thomas-workstation/repos/serial-mcp` (known only
`M src/tools/helpers.rs`). Do not use Worker subagents.

User approved downloading/executing these exact pinned official packages:

```text
@modelcontextprotocol/conformance@0.2.0-alpha.10
@modelcontextprotocol/inspector@2.0.0
Node 22.19.0
```

No floating tags or versions.

## 1. Version-Correct Cache Fields

Pinned rmcp facts:

- paginated results and `ReadResourceResult` expose
  `.with_ttl_ms(0).with_cache_scope(CacheScope::Private)`;
- constructors set `resultType: complete`;
- rmcp strips `resultType` for legacy but does **not** strip cache fields.

Therefore set cache fields only when
`RequestContext::protocol_version()` is `>= V_2026_07_28`. Add small pure
helper(s), not repeated ad-hoc comparisons.

Modern `ttlMs: 0`, `cacheScope: "private"` required on:

- `tools/list`;
- `resources/list`;
- `resources/templates/list`;
- `resources/read` complete results for every URI kind;
- `prompts/list`.

Legacy results omit both fields. Discovery keeps rmcp's existing required
zero/private fields. Tool calls, prompt get, and completion have no applicable
cache fields; do not invent them.

The `#[tool_handler]`/`#[prompt_handler]` generated list methods currently leave
cache fields unset. Add explicit `list_tools`/`list_prompts` handlers using the
existing routers, preserving exact deterministic catalog, pagination, titles,
schemas, and prompt definitions. Resolve macro integration from pinned rmcp
source; do not duplicate tool definitions manually or alter tool count.

Extend raw `tests/protocol_compatibility.rs`:

- modern assertions for every cacheable family above;
- equivalent legacy assertions prove both fields absent;
- retain `resultType` modern-present/legacy-absent assertions;
- test cursor pages if manual list handlers touch pagination.

Add pure helper tests for exact boundary behavior.

## 2. Official Conformance Gate

Add `conformance/expected-failures.yaml` containing only these four documented
fixture-dependent checks:

```yaml
server:
  - server-stateless:sep-2575-server-rejects-undeclared-capability
  - server-stateless:sep-2575-missing-capability-http-400
  - server-stateless:sep-2575-http-server-no-independent-requests-on-stream
  - server-stateless:sep-2575-server-no-log-without-loglevel
```

Use syntax required by pinned runner; if package expects a different structural
shape, change representation only, never IDs or scope.

Add bounded Ubuntu `mcp-conformance` CI job in `.github/workflows/ci.yml`:

- `permissions: contents: read`, timeout (15 minutes max);
- checkout; Rust `1.97.1` + version report; install `libudev-dev pkg-config`;
- `actions/setup-node@v4` with exact `22.19.0`;
- build locked binary;
- create temporary isolated profiles path;
- start HTTP server on loopback, bounded readiness probe, save PID;
- always stop process;
- run only planned scenario sets, each exact protocol version:
  - legacy: `server-initialize`, `server-session-lifecycle`, `ping`,
    `completion-complete`, `tools-list`, `resources-list`, `prompts-list`;
  - modern: `server-stateless`, `completion-complete`, `tools-list`,
    `resources-list`, `prompts-list`, `caching`,
    `sep-2164-resource-not-found`;
- apply expected failures only where runner supports relevant check IDs;
- never run `--suite all`; never add fixture endpoints to serial-mcp;
- write reports under stable `target/conformance-results/` paths;
- upload reports on success/failure with `actions/upload-artifact@v7`,
  `if-no-files-found: warn`, bounded retention;
- no suppressed runner exit status.

Run pinned package locally now (explicit approval given) against built server.
If scenario names/options differ, inspect `npx ... list`/`--help` from exact
package and update commands while preserving targeted intent. A baseline entry
that unexpectedly passes must fail as stale; do not broaden baseline.

## 3. Inspector 2.0.0 Interoperability Smoke

Add `scripts/inspector-smoke.mjs` using Node standard library only. Script takes
server URL, invokes exact installed/passed Inspector CLI binary or exact pinned
`npx` package, enforces per-command timeout, parses `--format json`, and asserts
semantics without `jq` or prose snapshots.

Against Streamable HTTP (`--transport http`) prove:

- `initialize`: server name `serial-mcp`, selected modern `2026-07-28`;
- `tools/list`: exactly 25 unique tools and `compute_checksum` present;
- `resources/list`: `serial://ports` and `serial://connections` present;
- `prompts/list`: `diagnose_port` and `interactive_terminal` present;
- `tools/call compute_checksum` with
  `{"algorithm":"xor","data":"$GPGGA,1","encoding":"utf8"}` returns
  raw `111` / hex `6F` (respect actual JSON envelope).

Set noninteractive behavior (`MCP_AUTO_OPEN_ENABLED=false`), bounded connect
timeout, and no auth/browser flow. CLI exit nonzero fails script. Do not test
Web/TUI/Playwright. Add focused Node script self-test only if parsing helpers
need it; keep script small.

Run in same CI job/server after conformance with exact package
`@modelcontextprotocol/inspector@2.0.0`. Keep Inspector named separately in logs
and docs: interoperability smoke, not conformance.

## 4. Docs, Drift, and Evaluator

Update:

- `AGENTS.md`: exact pins, scenarios, cache rules, Inspector purpose/command,
  Phase 4 gates, current architecture truth;
- `CHANGELOG.md` Unreleased: cache compliance, official pinned conformance,
  Inspector smoke;
- `README.md` only if user-facing protocol compatibility/testing claims need it;
- `docs/development/agent-interface-evaluation.md` from actual evaluator output;
- `docs/development/README.md` to list Phase handoffs if its index requires it;
- `tests/doc_drift.rs` with narrow executable guards for exact package pins,
  no `--suite all`, expected-failure IDs, Node pin, and Inspector smoke wiring.

`FEATURES.md` planning edit adds future MCPB distribution and removes shipped
hotplug wishlist. Keep it. No MCPB implementation now.

Run evaluator; update actual current bytes/largest values, never historical
baseline JSON:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```

## 5. Complete Verification

Run:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test config_schema_validation --locked
cargo test --test doc_drift --locked
cargo test --test protocol_compatibility --locked
cargo test --test resource_subscriptions --locked
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
nix flake check
```

Also run exact local conformance scenarios and Inspector smoke against built
HTTP binary. Preserve reports. Explain any environmental failure; do not call
phase complete while required product/test criteria fail.

Inspect status, full diff, `git diff --check`, recent log, and ensure no secrets,
capture artifacts, node_modules, or generated reports are staged.

## Out of Scope

- positive cache TTL policy;
- MCPB implementation;
- Inspector Web/TUI;
- full conformance suite or product fixture endpoints;
- new expected failures;
- tasks/MRTR/promoted parameter headers;
- tool count changes, dependency bumps unrelated to migration;
- push/merge/PR/amend/version bump.

## Commit and Recap

Commit all Phase 4 work plus planning/handoff docs as one new commit:

`test: add pinned MCP conformance gates`

Do not push. Recap exact local scenario outcomes, expected failures, Inspector
outputs, cache wire proofs, all repository gates, evaluator numbers, files,
hash, deviations, and primary status read-only.

Escalate before commit on package/scenario contradiction, extra conformance
failure, stale baseline, Inspector incompatibility, cache leakage to legacy,
Nix/native asset blocker requiring scope change, flaky/hanging process cleanup,
test weakening, or two failed approaches.
