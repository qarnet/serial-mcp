# rmcp 3.0.1 and MCP 2026-07-28 Migration Plan

## Status

- Target PR: [#50](https://github.com/qarnet/serial-mcp/pull/50)
- Dependency change: `rmcp` 1.7.0 -> 3.0.1
- Planning baseline: PR commit `6c5ad8952c87a692c53c63e12ed0959ebddbcf8a`
- Primary protocol: MCP `2026-07-28`
- Compatibility protocol: MCP `2025-11-25`, reduced feature set
- Public tool target: 25 tools (`subscribe` and `unsubscribe` removed)
- Conformance runner: `@modelcontextprotocol/conformance@0.2.0-alpha.10`

This is a major protocol migration, not a dependency-only update. Modern
clients get discovery, stateless HTTP, and standard resource subscriptions.
Legacy clients keep the core serial tools, resources, prompts, cancellation,
and progress, but do not get subscriptions or features that exist only in the
modern protocol.

## Agreed product decisions

1. MCP `2026-07-28` is the preferred protocol.
2. MCP `2025-11-25` remains accepted for core backward compatibility.
3. Compatibility is intentionally asymmetric: modern-only features are not
   emulated for legacy clients.
4. Remove MCP logging support and all `notifications/message` use.
5. Remove the serial-mcp `subscribe` and `unsubscribe` tools.
6. Replace them with standard `subscriptions/listen` resource-update
   notifications. Notifications signal availability; `read` remains the
   independent, lossless data path.
7. Add physical serial-port hotplug notifications using updates to the existing
   `serial://ports` resource.
8. Do not advertise tool-list or prompt-list change notifications; those
   catalogs are static for one server build.
9. Defer Tasks, positive cache lifetimes, promoted HTTP parameter headers, and
   MRTR product behavior. Record them in `docs/development/FEATURES.md`.
10. OAuth remains out of scope because serial-mcp is local-first.
11. Remove `poll_interval_ms` from open/profile/configure/connection defaults.
    It only controls the deleted RX-stream tool and has no role in event-driven
    `subscriptions/listen`. Existing profile files may contain the old key;
    serde ignores it on read and the next durable rewrite drops it. No profile
    schema-version bump is needed.

Interpretation note: “subscription is only available to modern clients” means
the standard MCP `subscriptions/listen` method, not a retained serial-mcp
`subscribe` tool. Both old tools are removed from every protocol version.

## Sources and evidence

Authoritative sources inspected:

- rmcp 1.7.0 source:
  `/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/rust-sdk-main`
- rmcp 3.0.1 tag `rmcp-v3.0.1`:
  `/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/rust-sdk-rmcp-v3.0.1`
- rmcp migration guide:
  <https://github.com/modelcontextprotocol/rust-sdk/discussions/969>
- rmcp changelogs:
  `crates/rmcp/CHANGELOG.md` and `crates/rmcp-macros/CHANGELOG.md`
- modern subscription model:
  `crates/rmcp/src/model.rs`, `crates/rmcp/src/service/server.rs`, and
  `examples/servers/src/subscriptions_streamhttp.rs`
- discovery/version handling:
  `crates/rmcp/src/handler/server.rs`
- Tasks example:
  `examples/servers/src/common/task_demo.rs`
- MRTR example:
  `examples/servers/src/mrtr.rs`
- MCP logging deprecation:
  `/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/modelcontextprotocol-main/seps/2577-deprecate-roots-sampling-and-logging.md`
- official MCP conformance framework:
  <https://github.com/modelcontextprotocol/conformance>, package version
  `0.2.0-alpha.10`;
- rmcp's pinned dual-version conformance workflow:
  `.github/workflows/conformance.yml` at tag `rmcp-v3.0.1`.

Indexed codebase-memory projects:

- `home-thomas-workstation-Nextcloud-Development-Resources-mcp_model_context_protocol-rust-sdk-main`
- `home-thomas-workstation-Nextcloud-Development-Resources-mcp_model_context_protocol-rust-sdk-rmcp-v3.0.1`
- `home-thomas-workstation-repos-serial-mcp`

An unmodified rmcp 3 PR build reports 41 primary compile errors and 20
deprecation warnings. Earlier CI shows 75 errors after macro cascades. Major
categories are obsolete task metadata, removed `Meta`/resource/prompt models,
non-exhaustive structs and enums, MRTR-aware response signatures, logging
deprecation, and legacy resource subscription APIs.

## Protocol compatibility matrix

| Capability | MCP 2026-07-28 | MCP 2025-11-25 |
|---|---:|---:|
| `server/discover` lifecycle | Yes | No; legacy `initialize` |
| 25 serial tools | Yes | Yes |
| Resources list/read/templates | Yes | Yes |
| Prompts and completions | Yes | Yes |
| Request cancellation/progress | Yes | Yes |
| `subscriptions/listen` | Yes | No |
| Port hotplug update notifications | Yes | No; poll/read normally |
| RX-availability update notifications | Yes | No; `read` normally |
| Legacy `resources/subscribe` | No | No |
| serial-mcp `subscribe`/`unsubscribe` tools | Removed | Removed |
| MCP logging capability/messages | No | No |
| Tasks extension | Deferred | Not applicable |
| MRTR input requests | Deferred | Not supported by protocol |

Legacy `read` remains complete. It can read buffered bytes immediately, wait
for new data, match patterns, decode frames, and report offsets/loss without
any subscription. Modern resource notifications are an optional wake-up layer
and must not participate in read cursor movement, framing, matching, or byte
retention.

## Compatibility verification strategy

Version-dependent behavior is release-critical. Typed rmcp tests alone are not
enough because client deserialization can hide missing, extra, or version-wrong
wire fields. Every protocol branch needs a public-boundary proof, and wire-shape
branches need raw JSON assertions.

### Test layers and files

1. **Pure behavior tests**
   - `src/server.rs`: supported-version ordering, modern and legacy capability
     views, and subscription-filter reduction;
   - `src/resource_events.rs`: event filtering, lag recovery, deduplication, and
     hotplug snapshot comparison;
   - `src/rx_session.rs`: publication occurs after ring append and outside the
     pump gate.
2. **Typed rmcp integration tests**
   - add `TestProtocol::{Modern, Legacy}` and explicit lifecycle helpers in
     `tests/common/mod.rs` and `tests/common/spawned.rs`;
   - add `tests/protocol_compatibility.rs` for the same core calls through
     distinct modern and legacy HTTP clients;
   - extend `tests/stdio_integration.rs` with explicit discover and initialize
     clients rather than relying on rmcp's default lifecycle;
   - rewrite `tests/resource_subscriptions.rs` around modern
     `subscriptions/listen`; do not retain legacy resource-subscribe tests.
3. **Raw HTTP wire tests**
   - keep these in `tests/protocol_compatibility.rs` beside the typed tests;
   - send exact headers, `_meta`, initialize/discover payloads, and parse both
     JSON and SSE responses;
   - assert HTTP status, JSON-RPC code, response ID, session headers,
     capabilities, `resultType`, and cache fields without round-tripping through
     rmcp response models.
4. **Official conformance**
   - add an Ubuntu `mcp-conformance` CI job in `.github/workflows/ci.yml`;
   - run the built HTTP server with an isolated profile path;
   - pin `@modelcontextprotocol/conformance@0.2.0-alpha.10` and run only generic
     scenarios that do not require the framework's named fixture
     tools/resources;
   - upload conformance output on success and failure;
   - store narrowly justified per-check exceptions in
     `conformance/expected-failures.yaml`.
5. **Official Inspector interoperability smoke**
   - pin `@modelcontextprotocol/inspector@2.0.0` (the official v2 release for
     MCP `2026-07-28`; Node `>=22.19.0`);
   - use CLI mode only against the built HTTP server — no browser, TUI,
     Playwright, or floating `latest` tag;
   - exercise `initialize`, `tools/list`, `resources/list`, `prompts/list`, and
     `tools/call` for hardware-free `compute_checksum`, with JSON output where
     supported;
   - treat this as real-client interoperability coverage, not a replacement
     for `@modelcontextprotocol/conformance` or the Rust integration tests.

### Required protocol matrix

| Behavior | Modern `2026-07-28` proof | Legacy `2025-11-25` proof |
|---|---|---|
| Lifecycle | discovery succeeds without initialize or session ID | initialize + initialized succeeds; session ID lifecycle remains valid |
| Version list/selection | discovery lists `2026-07-28` first and selects it | initialize selects exactly `2025-11-25` for explicit legacy client |
| Common capability/catalog | 25 tools plus resources/prompts/completions | same 25 tools plus resources/prompts/completions |
| Capability exclusions | no logging/list-change; resources subscription enabled | no logging/list-change/subscription capability |
| Core call | `compute_checksum` succeeds without hardware | same call and result succeed |
| Modern listen | accepted filter acknowledged; resource updates tagged with subscription ID | `subscriptions/listen` rejected as method not found |
| Removed legacy methods | initialize, ping, logging, resource subscribe/unsubscribe return modern HTTP 404 + JSON-RPC `-32601` | legacy resource subscribe/unsubscribe return JSON-RPC `-32601`; ping remains available |
| Result discriminator | ordinary list/read/prompt/tool/completion results contain `resultType: "complete"` | same responses omit `resultType` |
| Cache shape | cacheable list/read responses contain `ttlMs: 0`, `cacheScope: "private"` | cache fields omitted |
| Missing resource | `-32602` and requested URI in error data | legacy `-32002` behavior preserved |
| Request envelope | required `_meta`; header/meta mismatch and unsupported version are HTTP 400 typed errors | session header and negotiated protocol header work after initialize |
| Cancellation/progress | `send_break(duration_ms=2000)` on an injected loopback emits matching progress, accepts request-scoped cancellation, and releases BREAK | same public behavior through legacy lifecycle |
| Logging removal | no `notifications/message` during representative tool execution | no logging capability and no logging messages |

Use representative responses from every response family for discriminator/cache
checks: `tools/list`, `compute_checksum`, `resources/list`,
`resources/read(serial://ports)`, `prompts/list`, one existing prompt, and
`completion/complete`. This catches handler-specific constructors that a single
tool call would miss.

For cancellation/progress, parameterize one test body over both lifecycle
modes. Attach a known progress token, wait for its first progress notification,
cancel that request rather than the client service, and prove a later tool call
still succeeds. This verifies version-independent request lifecycle behavior
without asserting private token maps or handler internals.

### Official conformance scenario set

Run these against MCP `2025-11-25`:

```text
server-initialize
server-session-lifecycle
ping
completion-complete
tools-list
resources-list
prompts-list
```

Run these against MCP `2026-07-28`:

```text
server-stateless
completion-complete
tools-list
resources-list
prompts-list
caching
sep-2164-resource-not-found
```

`server-stateless` supplies broad raw-wire coverage: discovery, mandatory
request metadata, version/header errors, removed method routing, response ID
echo, listen acknowledgment, subscription IDs, and filter isolation. Its
diagnostic probes assume named conformance-only tools that serial-mcp must not
add to its public catalog. Baseline only these four check IDs:

```yaml
server:
  - server-stateless:sep-2575-server-rejects-undeclared-capability
  - server-stateless:sep-2575-missing-capability-http-400
  - server-stateless:sep-2575-http-server-no-independent-requests-on-stream
  - server-stateless:sep-2575-server-no-log-without-loglevel
```

Each is untestable without `test_missing_capability`,
`test_streaming_elicitation`, or `test_logging_tool`; local public-boundary
tests cover applicable serial-mcp behavior instead. Do not baseline the whole
scenario. Never baseline `wire-schema-valid`, discovery, version/header,
method-routing, subscription, cache, or serial-mcp-owned behavior. The
conformance runner fails stale per-check baselines, so an upstream scenario
improvement forces review.

Do not run `--suite all` against the product server. Most full-suite scenarios
require fixture names such as `test://static-text` and
`test_simple_text_tool`; adding hidden product endpoints would weaken catalog
and tool-count guarantees. Targeted generic scenarios provide a strict gate
without test-only production behavior.

### Official Inspector smoke

The official Inspector v2 package is a second independent MCP client and catches
client-integration failures that schema-focused conformance checks may not. Pin:

```text
@modelcontextprotocol/inspector@2.0.0
```

Run CLI one-shots against the same isolated HTTP server used by conformance:

```text
initialize
tools/list
resources/list
prompts/list
tools/call compute_checksum
```

Use `--transport http`, `--format json` where available, bounded connection
timeouts, and non-interactive auth behavior. Assert command success plus compact
semantic output (server identity/version, 25 tools, expected resources/prompts,
and checksum result); do not snapshot Inspector prose. Inspector is not the
normative conformance runner and must remain a separate named CI step.

## Resource notification semantics

### Resource updates, not resource-list changes

Physical port insertion/removal changes the contents of `serial://ports`; it
does not change the MCP resource catalog. Correct notification:

```text
notifications/resources/updated  uri=serial://ports
```

Likewise, opening or closing a connection changes
`serial://connections`, while RX changes per-connection resources. Use:

| Event | Updated resource URI(s) |
|---|---|
| OS port appeared/disappeared/identity changed | `serial://ports` |
| connection opened/closed | `serial://connections`, affected detail URI |
| connection state/reconfigure/reconnect changed | affected detail URI |
| RX bytes appended | affected `/raw`, `/log`, and detail URI |
| log cleared or changed through a tool | affected `/log` URI |

`notifications/resources/list_changed` is reserved for actual catalog changes.
Current catalog remains two static resources plus three templates, so
serial-mcp does not advertise or emit resource-list changes in this phase.

Tool-list and prompt-list `listChanged` capabilities also remain disabled.

### Client flow

Modern client:

1. Discover MCP `2026-07-28`.
2. Call `subscriptions/listen` with selected resource URIs.
3. Receive resource-update notification.
4. Call `read` with desired cursor/match/framing options, or read a resource.
5. Cancel and reopen the listen request as needed.

No byte payload travels in the resource-update notification. This avoids
using logging/custom notifications as an application data channel and leaves
the existing bounded ring/read pipeline authoritative.

## Architecture

### Shared `ResourceEventHub`

Add a process-wide resource event hub shared by every stdio/HTTP handler:

```rust
enum ResourceEvent {
    Updated(String),
}
```

Recommended implementation:

- `tokio::sync::broadcast` sender;
- fixed capacity: 256 events;
- synchronous/non-blocking publish from RX and lifecycle paths;
- each `subscriptions/listen` request owns one receiver;
- lag is recoverable: listener emits one update for every accepted URI whose
  current state may have changed, rather than terminating or blocking RX;
- unknown/no-receiver publish errors are ignored after optional trace logging.

Files:

- add `src/resource_events.rs`;
- inject `Arc<ResourceEventHub>` through `SerialHandlerOptions` and
  `SerialHandlerBuilder`;
- create one hub in production `main.rs` and each test-server process;
- clone the same hub into every handler factory invocation.

Modern HTTP is stateless. A handler-local event channel would split publishers
and listeners across different handler instances and silently lose updates.
Process-wide ownership is mandatory.

### RX pump integration

Files: `src/rx_session.rs`, construction paths in `src/server.rs`/`main.rs`.

After `ring.append(&chunk)` succeeds, publish update events for the connection's
raw/log/detail resource URIs. Publishing must happen after append so a client
woken by the event can immediately observe the new ring end offset.

The pump must never await notification delivery. `pump_gate` still covers only
serial read plus ring append for `capture_boot` atomicity. Event publication
occurs after append and must not alter cursor, ring, or gate invariants.

### Listener integration

Implement on `SerialHandler`:

- `accepted_subscription_filter(&SubscriptionFilter)`;
- `listen(SubscriptionContext)`.

Acceptance rules:

- accept only `resource_subscriptions`;
- reject/strip tool-list, prompt-list, and resource-list change flags;
- accept `serial://ports` and `serial://connections`;
- accept recognized concrete connection detail/raw/log URIs;
- strip unknown URI schemes/templates and malformed connection IDs;
- preserve requested URI order where practical, deduplicating repeats;
- let rmcp intersect accepted values with advertised capabilities.

Listen loop:

1. wait on context cancellation or next hub event;
2. ignore events outside accepted URI set;
3. send through `context.sink().notify_resource_updated(uri)`;
4. on broadcast lag, conservatively notify all accepted resources once;
5. on closed hub, complete gracefully;
6. on sink closure/cancellation, terminate without retry loop.

Per-listener network backpressure may delay that listener but must not block the
RX pump, another listener, or serial tools. Duplicate updates may be coalesced;
notifications are availability hints, not a byte ledger.

### Port hotplug watcher

Add one process-wide polling watcher around existing `PortProvider`:

- poll interval: 1 second;
- sort snapshots by stable `PortInfo` fields before comparison so OS enumeration
  order does not generate false updates;
- first successful snapshot establishes baseline without notification;
- changed successful snapshot publishes `Updated(URI_PORTS)`;
- enumeration failure logs warning and retains prior baseline;
- recovery compares next successful snapshot to prior successful baseline;
- watcher has shutdown cancellation token and deterministic join in tests.

Use a mutable test `PortProvider` to prove add/remove/identity-change behavior.
Do not put watcher logic inside `list_ports`; notification must be proactive.

## Protocol negotiation and discovery

### Supported versions

`supported_protocol_versions()` returns, in preference order:

```rust
[
    ProtocolVersion::V_2026_07_28,
    ProtocolVersion::V_2025_11_25,
]
```

Modern clients use `ClientLifecycleMode::Discover` or `Auto` and select
`2026-07-28`. Existing clients may continue sending legacy `initialize` with
`2025-11-25`.

### Version-specific capabilities

rmcp gates method dispatch by negotiated version:

- `subscriptions/listen` is rejected for legacy requests;
- legacy `resources/subscribe`/`unsubscribe` are rejected for modern requests.

serial-mcp removes legacy subscribe handlers entirely. Capability advertisement
must also differ:

- common capabilities: tools, resources, prompts, completions;
- modern discovery: resources subscription enabled;
- legacy initialize: resources subscription disabled;
- logging disabled everywhere;
- all three list-change flags disabled.

`subscriptions/listen` checks `self.get_info().capabilities`, so `get_info()`
must describe modern capabilities. Override `initialize()` to return a legacy
capability view when the client requests `2025-11-25`, while setting peer info
as rmcp's default implementation does. Override `discover()` only if needed to
make supported-version ordering, identity metadata, instructions, and modern
capabilities explicit.

Discovery cache fields use rmcp's safe default: `ttlMs=0`,
`cacheScope=private`. Positive caching remains deferred.

### Compatibility risk

MCP `2026-07-28` released shortly before this migration. Real OpenCode, Claude
Code, Codex, and other clients may not support discovery yet. Dual-version
support prevents a hard cutoff, but each target client needs an integration
smoke test:

- modern-capable client negotiates `2026-07-28` and can listen;
- older client initializes as `2025-11-25` and can use core tools/resources;
- no client receives a capability for a method serial-mcp will reject.

## rmcp 3 compile migration

### Tool descriptors and removed Tasks metadata

File: `src/server.rs`.

Remove `execution(task_support = "optional")` from `transact`, `read`,
`send_break`, `subscribe`, and `capture_boot`. `subscribe` is then deleted;
the other four remain normal cancellable tools. Do not claim Tasks support.

### Request metadata

Files:

- `src/server.rs`
- `src/tools/control_ops.rs`
- `src/tools/io_ops.rs`

Replace removed `Meta` with `RequestMetaObject`. Preserve progress-token
extraction, request cancellation token, and `Peer<RoleServer>` behavior.

### Non-exhaustive constructors

Files: `src/tools/control_ops.rs` and other compiler-proven sites.

Use:

```rust
ProgressNotificationParam::new(token, progress)
    .with_total(total)
    .with_message(message)
```

Do not use external struct literals for rmcp non-exhaustive models.

### Prompts

Files: `src/prompts/diagnose.rs`, `src/prompts/interactive.rs`.

Replace `PromptMessageRole::User` with `Role::User`. Preserve prompt text and
ordering.

### Resources and MRTR-aware responses

File: `src/server.rs`.

- `RawResource` -> `Resource`;
- `RawResourceTemplate` -> `ResourceTemplate`;
- use `with_all_items` constructors for list results;
- return `ReadResourceResponse` and convert complete
  `ReadResourceResult` values with `.into()`;
- add conservative wildcard arms for non-exhaustive protocol enums;
- use constructors so rmcp strips modern `resultType` for legacy replies.

### Remove logging-backed stream implementation

Delete or retire:

- `src/tools/stream_ops.rs`;
- `StreamRegistry` and related builder/main/test wiring;
- `SubscribeArgs`, `SubscribeResult`, `UnsubscribeArgs`, `UnsubscribeResult`;
- all `Subscribe*Notification` wire types used only by logging messages;
- `LoggingLevel`, `LoggingMessageNotificationParam`,
  `notify_logging_message`, and `enable_logging()`;
- logging notification collectors in test helpers;
- `subscribe`/`unsubscribe` entries from tool catalog and documentation.

Do not remove internal tracing or per-connection event logs. MCP logging
deprecation affects protocol messages, not `tracing` or `get_log`/`export_log`.

## Cache fields required by modern responses

Product caching remains deferred, but modern response constructors must produce
valid `2026-07-28` shapes. Use non-cacheable defaults:

- `ttlMs=0`;
- `cacheScope=private`.

Apply to discovery and any list/read result for which the modern schema requires
these fields. Legacy replies must keep legacy-compatible omission behavior where
rmcp provides it.

Because `#[tool_handler]` and `#[prompt_handler]` generate list methods with
unset cache fields, add explicit `list_tools` and `list_prompts` handlers if
needed. Use existing routers and preserve deterministic ordering. This is wire
compliance, not positive response caching.

Document positive TTL policy as deferred in `FEATURES.md`.

## Deferred feature documentation

Update `docs/development/FEATURES.md`:

### Tasks extension — Later

Record SEP-2663 opportunity:

- async server-directed task handles;
- `tasks/get`, `tasks/update`, `tasks/cancel`;
- cooperative cancellation and task TTL;
- candidates: long `read`, `transact`, `capture_boot`, `send_break`, and future
  firmware/file-transfer operations;
- do not use for open-ended RX notification stream while rmcp requires polling
  and task-status subscriptions remain unavailable.

### Positive cache hints — Later

Record candidate policy:

- long/public for tool, prompt, and template catalogs;
- short/private for port list;
- zero/private for connections, logs, and RX;
- only enable positive TTL after notification invalidation and stale-on-error
  behavior are tested.

### Standard HTTP parameter headers — Later

rmcp automatically provides `Mcp-Method` and `Mcp-Name` for modern HTTP.
Defer `x-mcp-header` promotion of `connection_id`, `port`, or `profile` until
privacy/proxy-log behavior is reviewed. Never promote commands, serial payloads,
selectors, or capture filenames.

### MRTR — Not a priority

Record possible future elicitation for destructive reset confirmation or
physical power-cycle prompts, plus requirement to authenticate echoed
`requestState`. Existing schemas, destructive hints, and cancellation cover
current workflows; no MRTR behavior now.

### Hotplug and multiple subscriptions

After this migration lands, remove or rewrite old wishlist entries for hotplug
watch and multiple public subscriptions because standard resource subscriptions
and hotplug updates will be shipped. FEATURES keeps only unshipped work.

### MCP Bundle distribution — Future

Record MCPB as a separate release/distribution opportunity, not migration
scope: package platform release binaries plus a manifest for one-click local
stdio installation. Resolve cross-platform bundle layout, manifest validation,
signing, updates, and desktop-client testing in a dedicated future phase.

## Phased implementation

### Phase 1 — rmcp 3 compile surface and obsolete-stream removal

Scope:

- metadata/model/constructor/response migrations;
- remove obsolete task descriptor fields;
- remove MCP logging and the `subscribe`/`unsubscribe` tools now, because their
  deprecated rmcp models prevent a warning-clean all-target build;
- remove legacy `resources/subscribe`/`resources/unsubscribe` handlers and
  subscription/list-change/logging capability flags; modern resource
  subscription stays disabled until Phase 3;
- remove subscription-only `poll_interval_ms`, stream chunk limits/schema
  helpers, and their profile/open/configure/connection plumbing rather than
  leaving a public no-op setting;
- remove `StreamRegistry`, `stream_ops`, subscription-only wire types, schema
  tests, fuzz inputs, integration stages, and notification collectors;
- keep `read`, RX ring, matcher, framing, parser, and capture behavior intact;
- update current-surface tool counts and docs to 25 while preserving historical
  release-note text about older versions.

Files:

- `Cargo.toml`, `Cargo.lock`;
- `src/server.rs`;
- `src/tools/control_ops.rs`, `src/tools/io_ops.rs`;
- `src/main.rs`, `src/tools/mod.rs`, `src/tools/types.rs`,
  `src/tools/rx_validate.rs`, `src/tools/helpers.rs`, `src/limits.rs`,
  `src/schema_helpers.rs`;
- `src/profiles.rs`, `src/serial/config.rs`, `src/serial/connection.rs`,
  `src/serial/manager.rs`, and `src/tools/port_ops.rs` for obsolete
  `poll_interval_ms` removal;
- delete `src/tools/stream_ops.rs`;
- prompt files;
- directly affected unit, property, fuzz, PTY, protocol-emulator, HTTP, stdio,
  and resource-subscription tests;
- current surface docs/evaluator files: `README.md`, `AGENTS.md`,
  `docs/agent-config.md`, `docs/development/FEATURES.md`,
  `docs/development/agent-interface-evaluation.md`, and `CHANGELOG.md`'s
  Unreleased section. Do not rewrite historical changelog entries.

Acceptance:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test doc_drift --locked
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```

No `allow(deprecated)` bridge, broad lint allowance, hidden replacement tool,
or modern subscription implementation yet. Tool operational behavior outside
the two removed tools remains unchanged.

### Phase 2 — dual protocol and discovery

Scope:

- modern-first supported version list;
- modern discovery capabilities;
- legacy initialize capability view;
- modern and legacy client helpers;
- explicit negotiation and raw-wire tests over stdio and HTTP;
- keep resource subscription advertisement disabled until Phase 3 installs
  `accepted_subscription_filter` and `listen` in the same change.

Files:

- `src/server.rs`, `src/main.rs` as needed;
- `tests/common/mod.rs`, `tests/common/spawned.rs`;
- add `tests/protocol_compatibility.rs`;
- `tests/http_integration.rs`, `tests/stdio_integration.rs`.

Acceptance behavior:

- modern client discovers and negotiates `2026-07-28`;
- legacy client initializes and negotiates `2025-11-25`;
- both can list/call core tools and read resources;
- legacy capabilities omit subscription/logging;
- modern capabilities omit logging and all list-change flags;
- raw HTTP tests prove session/header, `_meta`, method-gate, `resultType`, and
  version-specific resource-error behavior.

Verification:

```bash
cargo test --test protocol_compatibility --locked
cargo test --test http_integration --locked
cargo test --test stdio_integration --locked
```

### Phase 3 — resource event hub and modern subscriptions

Scope:

- process-wide event hub;
- `accepted_subscription_filter` and `listen`;
- enable modern resource-subscription capability atomically with those
  handlers;
- open/close/state/RX/log resource updates;
- port hotplug watcher;
- modern subscription client tests;
- no tool or prompt list-change support.

Phase 3 notes (adopted during implementation):

- **Pinned rmcp 3.0.1 ack artifact:** the wire `subscriptions/listen`
  acknowledgement may echo a repeated requested VALID URI. rmcp computes the
  final accepted filter as `requested.intersection(&candidate)
  .intersection(&advertised)` (handler/server.rs), and `SubscriptionFilter::intersection`
  is left-biased over the REQUESTED list (model.rs), so client-side
  duplicates survive into the ack regardless of the handler's candidate.
  serial-mcp therefore enforces deduplication in handler/listener semantics:
  `accepted_subscription_filter` returns the valid, deduplicated,
  first-occurrence-ordered candidate, and `listen` re-deduplicates
  `context.accepted()` before matching and lag recovery, so duplicate
  requested URIs never produce duplicate notifications. Do not patch or fork
  rmcp; tests assert the acknowledged URI set + first-occurrence order and
  explicitly permit the raw Vec echo.
- **Stateless RX session ownership:** modern `2026-07-28` HTTP serves every
  request with a fresh handler instance, so the RX session registry (ring +
  pump + shared cursor) must be process-wide like the hub — a handler-local
  `RxSessionManager` would split the ring across requests and break the
  documented modern client flow (listen → `read`). One `Arc<RxSessionManager>`
  is created per server process from the shared budget+hub, injected through
  `SerialHandlerOptions`/builder, and cloned into every handler factory;
  `build()` constructs one only when none was injected (standalone use).

Files:

- add `src/resource_events.rs`;
- `src/lib.rs`, `src/main.rs`, `src/server.rs`;
- `src/rx_session.rs`;
- relevant port/control/io operations;
- `tests/common/mod.rs` and focused subscription/hotplug tests.

Public-boundary tests must cover:

- acknowledgment contains accepted resource filter;
- unsupported flags/URIs are stripped;
- RX append triggers update only after bytes are readable;
- notification leaves shared read cursor untouched;
- `read` works without any listener;
- two listeners receive same update independently;
- cancelling one listener leaves another active;
- slow/lagged listener does not block pump and recovers conservatively;
- disconnect/close produces final useful resource updates;
- physical port add/remove/identity change emits `serial://ports` update;
- unchanged/reordered/erroring enumeration emits no false update;
- HTTP stateless handler instances share same hub;
- stdio listener cancellation completes cleanly.

Verification:

```bash
cargo test --test resource_subscriptions --locked
cargo test --test protocol_compatibility --locked
cargo test --test http_integration --locked
cargo test --test stdio_integration --locked
```

### Phase 4 — modern cache compliance, conformance, and complete gates

Scope:

- zero/private cache fields required by modern responses;
- no positive caching policy;
- pinned official conformance job and narrow expected-failure file;
- pinned official Inspector `2.0.0` CLI interoperability smoke;
- schema snapshots/evaluator review;
- full software-only gates and documentation consistency.

Files:

- `src/server.rs` for explicit cacheable list responses;
- resource read handlers in `src/server.rs`;
- `tests/protocol_compatibility.rs` for modern-present/legacy-absent wire
  assertions;
- `.github/workflows/ci.yml` for the Ubuntu `mcp-conformance` gate;
- add `conformance/expected-failures.yaml` with only the four documented
  fixture-gap checks;
- add a small checked-in script or xtask entry for deterministic Inspector CLI
  assertions rather than embedding fragile shell parsing in workflow YAML.

Verification:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test config_schema_validation
cargo test --test doc_drift
cargo test --test protocol_compatibility --locked
cargo test --test resource_subscriptions --locked
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
nix flake check
```

CI additionally runs pinned Node tooling against the built HTTP server:

```text
@modelcontextprotocol/conformance@0.2.0-alpha.10
@modelcontextprotocol/inspector@2.0.0
Node 22.19.0
```

Also run the pinned official conformance scenario sets from “Official
conformance scenario set” against the built HTTP binary. This command downloads
and executes the pinned npm package, so obtain explicit approval before running
it on a developer machine; CI may run it as a declared networked gate. Every
scenario must pass apart from the four exact per-check fixture gaps. Archive
the generated JSON/Markdown reports for failure diagnosis.

No hardware test is required. Native_sim and PTY suites provide serial-path
coverage.

## Invariants

1. `read` remains fully usable without subscription or discovery-specific
   helper calls.
2. Resource notifications never consume bytes or move any cursor.
3. RX pump never awaits event delivery.
4. Ring offsets, wrap loss, framing, parser, matching, cancellation, and
   `capture_boot` pump-gate behavior remain unchanged.
5. One client cannot block another or serial capture.
6. Legacy clients never receive capabilities for unavailable methods.
7. Modern clients never receive MCP logging messages.
8. Tool operational errors remain MCP tool results where current behavior does;
   malformed/protocol errors stay protocol errors.
9. Tool output schemas and titles remain present after macro migration.
10. Profile, capture-store, and port-provider process-wide ownership remains.

## Risks

### Client support

Dual protocol avoids a hard cutoff, but modern subscriptions only benefit
clients implementing discovery/listen. Test known clients before changing user
guidance from polling to subscription-first behavior.

### Notification storms

High-rate UART can produce frequent updates. Notifications are coalescible
hints. Bounded broadcast capacity and lag recovery prevent memory growth and
pump backpressure.

### Stateless handler isolation

Modern HTTP uses fresh request handling without legacy session assumptions.
Any handler-local event state will fail under real HTTP even if unit tests pass.
Use distinct clients and real HTTP transport tests.

### Resource/update race

Publish only after state commit or ring append. Tests must prove client can
observe updated state immediately after notification.

### Schema drift

rmcp-macros 3 changes generated models and removes experimental task metadata.
Measure catalog changes; never update evaluator numbers or baselines without an
explained field-level delta.

## Out of scope

- Tasks implementation;
- positive response caching;
- `x-mcp-header` parameter promotion;
- MRTR elicitation behavior;
- OAuth/authentication;
- distributed SSE `EventStore`;
- custom serial notification extension;
- package version bump;
- unrelated serial protocol or profile changes.

## Open questions

No blocking product questions remain for planning. Implementation should use
the agreed interpretation above: dual protocol, modern-only standard
subscriptions, no subscription tools, resource-update wakeups, independent
`read`, and deferred Tasks/cache/header/MRTR work.
