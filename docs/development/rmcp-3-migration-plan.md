# rmcp 3.0.1 and MCP 2026-07-28 Migration Plan

## Status

- Target PR: [#50](https://github.com/qarnet/serial-mcp/pull/50)
- Dependency change: `rmcp` 1.7.0 -> 3.0.1
- Planning baseline: PR commit `6c5ad8952c87a692c53c63e12ed0959ebddbcf8a`
- Primary protocol: MCP `2026-07-28`
- Compatibility protocol: MCP `2025-11-25`, reduced feature set
- Public tool target: 25 tools (`subscribe` and `unsubscribe` removed)

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

## Phased implementation

### Phase 1 — rmcp 3 compile surface

Scope:

- metadata/model/constructor/response migrations;
- remove obsolete task descriptor fields;
- keep existing protocol behavior temporarily while code compiles;
- adapt tests only as required by renamed rmcp APIs.

Files:

- `Cargo.toml`, `Cargo.lock`;
- `src/server.rs`;
- `src/tools/control_ops.rs`, `src/tools/io_ops.rs`;
- prompt files;
- direct-rmcp test files.

Acceptance:

```bash
cargo check --all-targets --locked
cargo fmt --all -- --check
```

No broad lint allowances. No new protocol feature yet.

### Phase 2 — dual protocol and discovery

Scope:

- modern-first supported version list;
- modern discovery capabilities;
- legacy initialize capability view;
- modern and legacy client helpers;
- explicit negotiation tests over stdio and HTTP.

Files:

- `src/server.rs`, `src/main.rs` as needed;
- `tests/common/mod.rs`, `tests/common/spawned.rs`;
- `tests/http_integration.rs`, `tests/stdio_integration.rs`.

Acceptance behavior:

- modern client discovers and negotiates `2026-07-28`;
- legacy client initializes and negotiates `2025-11-25`;
- both can list/call core tools and read resources;
- legacy capabilities omit subscription/logging;
- modern capabilities advertise resource subscriptions but no list-change or
  logging capabilities.

### Phase 3 — resource event hub and modern subscriptions

Scope:

- process-wide event hub;
- `accepted_subscription_filter` and `listen`;
- open/close/state/RX/log resource updates;
- port hotplug watcher;
- modern subscription client tests;
- no tool or prompt list-change support.

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

### Phase 4 — remove legacy streaming tools and logging

Scope:

- delete `subscribe`/`unsubscribe` tools and implementation;
- remove MCP logging capability/models/collectors;
- remove legacy resources subscribe/unsubscribe handlers;
- reduce exact tool catalog count 27 -> 25;
- update agent evaluator and docs from measured output.

Files include:

- `src/server.rs`, `src/tools/mod.rs`, `src/tools/types.rs`;
- delete `src/tools/stream_ops.rs` if no remaining consumer;
- `src/main.rs`, test helpers, subscription tests;
- `README.md`, `AGENTS.md`, `CHANGELOG.md`, `server.json` only if executable
  truth requires it;
- `tests/doc_drift.rs`;
- `docs/development/agent-interface-evaluation.md`;
- `docs/development/FEATURES.md`.

Acceptance:

- no `rmcp` logging API usage remains;
- no `#[allow(deprecated)]` needed for logging or legacy subscriptions;
- no public tool named `subscribe` or `unsubscribe`;
- agent guidance teaches modern resource listen + independent `read`;
- exact tool count and every prose reference say 25;
- historical evaluator baseline remains unchanged;
- current evaluator report explains measured catalog delta.

### Phase 5 — modern cache-shape compliance and complete gates

Scope:

- zero/private cache fields required by modern responses;
- no positive caching policy;
- schema snapshots/evaluator review;
- full software-only gates and documentation consistency.

Verification:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test config_schema_validation
cargo test --test doc_drift
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
nix flake check
```

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
