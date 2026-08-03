# rmcp 3 Phase 2 Handoff: Dual Lifecycle and Discovery

## Goal

Make serial-mcp explicitly support modern MCP `2026-07-28` discovery/stateless
requests and legacy MCP `2025-11-25` initialize/session requests, with strict
typed and raw-wire proofs. Keep resource subscriptions disabled until Phase 3.

Work only in:

`/home/thomas-workstation/repos/serial-mcp-pr50-analysis`

Current accepted Phase 1 HEAD is `03a14321`. Primary checkout
`/home/thomas-workstation/repos/serial-mcp` has one pre-existing modified file,
`src/tools/helpers.rs`; do not run any command or edit there.

## Grounding

- `rmcp 3.0.1` source:
  `/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/rust-sdk-rmcp-v3.0.1`
- `crates/rmcp/src/handler/server.rs:327-339` exposes
  `supported_protocol_versions()` and `discover()`; default discovery builds
  from supported versions plus `get_info()`.
- `crates/rmcp/src/handler/server.rs:50-272` performs protocol routing,
  mandatory modern `_meta` validation, modern ping/listen gates,
  `resultType` stripping for legacy peers, and modern resource-not-found
  remapping to `INVALID_PARAMS`.
- `crates/rmcp/src/service/client.rs:559-729` exposes
  `ClientLifecycleMode::{Initialize,Discover}` and
  `ClientServiceExt::serve_with_lifecycle`.
- `ProtocolVersion::LATEST` is still `2025-11-25`; modern selection therefore
  must be explicit. `ProtocolVersion::KNOWN_VERSIONS` is broader than product
  support and must not be advertised wholesale.
- Modern required request `_meta` keys are:
  - `io.modelcontextprotocol/protocolVersion`
  - `io.modelcontextprotocol/clientCapabilities`
  (`clientInfo` optional). See `crates/rmcp/src/model/meta.rs:368-529`.
- `StreamableHttpServerConfig::default()` keeps legacy sessions enabled, while
  rmcp serves `2026-07-28` requests statelessly regardless. Keep this default;
  do not call `with_legacy_session_mode(false)`.
- Phase 1 server currently advertises only tools/resources/prompts/completions,
  no logging, list-change, or resource-subscription capability.
- Constructors already set `resultType: "complete"`; rmcp strips it for legacy
  peers. Do not hand-strip or duplicate this logic.
- `McpError::resource_not_found` is already transformed by rmcp to modern
  `-32602`, while legacy stays `-32002`. Preserve handler errors and test wire.

## Exact Server Shape

### Supported versions

In `src/server.rs`, define one product-owned ordered supported-version slice:

```rust
[
    ProtocolVersion::V_2026_07_28,
    ProtocolVersion::V_2025_11_25,
]
```

Override `ServerHandler::supported_protocol_versions()` to return exactly this
order using `Cow<'static, [ProtocolVersion]>`.

### Capability views

Use small pure helpers so unit tests can assert behavior without transport:

- common capability builder: tools, resources, prompts, completions;
- modern discovery info/capabilities: same common set in Phase 2;
- legacy initialize info/capabilities: same common set in Phase 2.

Both views must omit:

- MCP logging;
- resource subscriptions;
- tool/resource/prompt list-change flags;
- tasks and unrelated capabilities.

Keep views separate even though equal now. Phase 3 will add modern resource
subscription capability atomically with `accepted_subscription_filter` and
`listen`, while legacy remains unchanged.

### Lifecycle methods

- `get_info()` returns legacy initialization info with protocol version
  `2025-11-25`; this keeps default `ServiceExt::serve` clients stable.
- `discover()` returns `DiscoverResult::from_server_info` with exact ordered
  supported versions and modern discovery info.
- `initialize()` must preserve rmcp peer bookkeeping by calling
  `context.peer.set_peer_info(request.clone())`, then return legacy info with
  selected version `2025-11-25` for explicit legacy initialization.
- Do not add a `ping` override. rmcp accepts ping only on legacy lifecycle and
  rejects it on modern lifecycle.
- Do not add `accepted_subscription_filter` or `listen` yet. Modern and legacy
  `subscriptions/listen` must remain method-not-found in Phase 2.

If rmcp transport routing prevents the handler from seeing a modern
`initialize`, rely on that routing. Do not invent duplicate HTTP method gates.

## Typed Test Harness

In `tests/common/mod.rs` add:

```rust
pub enum TestProtocol { Modern, Legacy }
```

Add explicit HTTP helpers based on existing `TestClientHandler`:

- modern: `serve_with_lifecycle(..., ClientLifecycleMode::Discover {
  preferred_versions: vec![ProtocolVersion::V_2026_07_28] })`;
- legacy: `serve_with_lifecycle(..., ClientLifecycleMode::Initialize)` using a
  client handler whose `ClientInfo.protocol_version` is exactly
  `V_2025_11_25` if default handler metadata does not already prove this
  explicitly.

Return same `(RunningService<...>, ())` shape as current helpers. Existing
`connect_client` remains legacy-compatible so unrelated tests need no churn.
Mirror protocol-selecting URL/spawned helpers in `tests/common/spawned.rs`.

For stdio, add local explicit lifecycle startup helpers in
`tests/stdio_integration.rs` using `TokioChildProcess`; do not force HTTP
helpers onto child-process transport.

## New `tests/protocol_compatibility.rs`

Add one focused integration file. Use in-process or spawned server as noted.

### Typed matrix

Parameterize or duplicate clearly for modern and legacy:

1. Lifecycle selects exact version (`peer_info().protocol_version`).
2. `tools/list` returns exact 25 names (`EXPECTED_TOOLS`).
3. `resources/list` returns two static resources.
4. `resources/templates/list` returns three templates.
5. `prompts/list` returns two prompts.
6. `compute_checksum` succeeds without hardware and returns known XOR result.
7. `resources/read` for `serial://ports` succeeds.
8. Capabilities contain tools/resources/prompts/completions and omit logging,
   list-change, tasks, and resource subscriptions.

### Raw HTTP helper

Use existing dev `reqwest` directly. Add test-local helpers that:

- POST JSON-RPC to spawned server URL;
- set `Accept: application/json, text/event-stream` and
  `Content-Type: application/json`;
- optionally set `MCP-Protocol-Version` and `Mcp-Session-Id`;
- parse either direct JSON or SSE `data:` JSON payload without typed rmcp
  result deserialization;
- return HTTP status, headers, and raw `serde_json::Value` response.

Modern request params carry:

```json
"_meta": {
  "io.modelcontextprotocol/protocolVersion": "2026-07-28",
  "io.modelcontextprotocol/clientInfo": {"name":"serial-mcp-test","version":"1"},
  "io.modelcontextprotocol/clientCapabilities": {}
}
```

Use matching `MCP-Protocol-Version: 2026-07-28` header after discovery.

Legacy flow sends initialize with exact `2025-11-25`, captures
`Mcp-Session-Id` response header, sends `notifications/initialized`, then uses
that session header plus `MCP-Protocol-Version: 2025-11-25` for requests.

### Raw-wire assertions

Prove at minimum:

1. `server/discover` succeeds without initialize/session ID; ordered
   `supportedVersions` is exactly modern then legacy; no session header is
   required; capabilities match modern Phase 2 view.
2. Modern `tools/list`, `resources/list`, `resources/read(serial://ports)`,
   `prompts/list`, `prompts/get`, `completion/complete`, and
   `tools/call(compute_checksum)` include `resultType: "complete"`.
3. Equivalent representative legacy responses omit `resultType`.
4. Modern unknown resource returns JSON-RPC `-32602`, echoes request ID, and
   error data includes requested URI. Legacy unknown resource remains `-32002`.
5. Modern requests missing required `_meta`, using header/meta version
   mismatch, or selecting unsupported version return HTTP 400 typed errors.
   Assert exact stable status/code/data fields exposed by rmcp; do not pin
   incidental error prose.
6. Modern `ping`, `initialize`, `logging/setLevel`,
   `resources/subscribe`, and `resources/unsubscribe` are rejected with modern
   routing semantics (HTTP 404 where transport defines it and JSON-RPC
   `-32601`). Legacy ping succeeds; legacy resource subscribe/unsubscribe are
   `-32601`.
7. `subscriptions/listen` is `-32601` for both protocols in Phase 2.
8. Response IDs equal request IDs.

Do not assert Phase 4 cache fields yet. Do not add product fixture endpoints.

## Stdio Coverage

Extend `tests/stdio_integration.rs` with:

- explicit modern discovery client selects `2026-07-28`, lists 25 tools, and
  calls `compute_checksum`;
- explicit legacy initialize client selects `2025-11-25`, lists 25 tools, and
  calls `compute_checksum`.

Keep existing tests. Each child still uses isolated profile path and is
cancelled/reaped.

## Pure Unit Tests

In `src/server.rs`, add tests for:

- supported versions exact order and exact length 2;
- modern capability view includes only expected common capabilities;
- legacy capability view includes only expected common capabilities;
- neither view advertises logging, subscriptions, list-change, or tasks.

Assert serialized public capability shape or public capability methods, not
private builder wiring.

## Documentation

Update `AGENTS.md` fast truth and compatibility section with dual lifecycle:

- preferred modern `2026-07-28` discovery/stateless behavior;
- compatible legacy `2025-11-25` initialize/session behavior;
- Phase 2 subscription advertisement remains disabled.

Update `CHANGELOG.md` under `## [Unreleased]` with dual-lifecycle support and
new compatibility tests. Do not alter historical release entries.

## Out of Scope

- `subscriptions/listen`, event hub, resource updates, or hotplug watcher;
- any subscription capability advertisement;
- cache `ttlMs` / `cacheScope` policy (Phase 4);
- official npm conformance CI/files (Phase 4);
- tasks, elicitation, sampling, roots, MRTR/input-required behavior;
- positive cache TTLs or promoted HTTP headers;
- tool/resource/prompt list-change notifications;
- tool count/schema redesign;
- dependency bumps;
- push, merge, PR creation, or commit amendment.

## Verification

Run in this order:

```bash
cargo fmt --all -- --check
cargo test --lib server::tests --locked
cargo test --test protocol_compatibility --locked
cargo test --test http_integration --locked
cargo test --test stdio_integration --locked
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test doc_drift --locked
```

Then inspect:

```bash
git status --short
git diff --check
git diff
git log --oneline -5
```

## Commit and Recap

Include this handoff in one new commit:

`feat: add dual MCP lifecycle support`

Do not amend earlier commits. Do not push.

Return:

- files changed;
- exact lifecycle/capability behavior;
- raw HTTP cases and observed status/code/header shapes;
- every command and result;
- commit hash/message;
- blockers/deviations.

Escalate without committing if two attempts fail, rmcp behavior contradicts
this handoff, raw transport status differs from planned semantics and cannot be
explained from rmcp source, a live capability requires Phase 3 infrastructure,
or any fix would weaken tests, invent architecture, alter baselines, or expand
scope.
