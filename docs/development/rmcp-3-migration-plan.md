# rmcp 3.0.1 Migration Plan

## Status

- Target PR: [#50](https://github.com/qarnet/serial-mcp/pull/50)
- Dependency change: `rmcp` 1.7.0 -> 3.0.1
- Planning baseline: PR head `7527c17b1a4f1e4473039ef2805ec41f7a1ee2a0`
- Target wire protocol for this migration: MCP `2025-11-25`
- Future protocol target, deliberately deferred: MCP `2026-07-28`

This document separates required compatibility work from new rmcp 3 features.
PR #50 should first restore all existing serial-mcp behavior on rmcp 3.0.1.
Modern protocol adoption and new product features belong in later, focused
changes with their own public-boundary tests.

## Sources and evidence

Authoritative sources inspected:

- rmcp 1.7.0 source:
  `/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/rust-sdk-main`
- rmcp 3.0.1 tag `rmcp-v3.0.1`:
  `/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/rust-sdk-rmcp-v3.0.1`
- rmcp 3 migration guide:
  <https://github.com/modelcontextprotocol/rust-sdk/discussions/969>
- rmcp 3 changelog:
  `crates/rmcp/CHANGELOG.md` in tagged source
- rmcp macro changelog:
  `crates/rmcp-macros/CHANGELOG.md` in tagged source
- MCP SEP-2577:
  `/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/modelcontextprotocol-main/seps/2577-deprecate-roots-sampling-and-logging.md`
- rmcp subscription implementation and constraints:
  `crates/rmcp/src/service/server.rs` and
  `examples/servers/src/subscriptions_streamhttp.rs` in tagged source

Both rmcp trees and serial-mcp are indexed in codebase-memory:

- `home-thomas-workstation-Nextcloud-Development-Resources-mcp_model_context_protocol-rust-sdk-main`
- `home-thomas-workstation-Nextcloud-Development-Resources-mcp_model_context_protocol-rust-sdk-rmcp-v3.0.1`
- `home-thomas-workstation-repos-serial-mcp`

An unmodified `cargo check --all-targets --locked --message-format short` on
PR #50 produced 41 compile errors and 20 deprecation warnings. Earlier CI
reported 75 errors because macro failures cascaded through more generated
items. Main failure groups are listed below.

## Goals

1. Build serial-mcp with rmcp 3.0.1 and Rust 1.97.1 on every supported CI OS.
2. Preserve current MCP `2025-11-25` behavior over stdio and Streamable HTTP.
3. Preserve all 27 tools, their input/output schemas, resources, prompts,
   cancellation, progress, and resource-change notifications. Only obsolete
   rmcp task metadata may disappear from tool descriptors.
4. Preserve current `subscribe` tool notification behavior during rmcp's
   logging deprecation window.
5. Make every compatibility choice explicit so later `2026-07-28` adoption
   does not happen accidentally through rmcp defaults.
6. Leave repository warning-free under `RUSTFLAGS="-D warnings"` and clippy.

## Non-goals

PR #50 will not:

- advertise or negotiate MCP `2026-07-28`;
- implement `server/discover` as a modern lifecycle entry point beyond
  explicitly reporting only currently supported protocol versions;
- implement `subscriptions/listen`;
- replace serial RX streaming with a new wire extension;
- implement SEP-2663 Tasks, MRTR input requests, cache hints, standard routing
  headers, OAuth, or distributed SSE replay;
- change tool count, names, arguments, result data, serial behavior, profile
  behavior, capture behavior, resource URIs, or prompt intent;
- bump serial-mcp package version or rewrite historical changelog entries.

## Compatibility decisions

### 1. Keep MCP 2025-11-25 for migration

`SerialHandler::get_info()` currently declares `V_2025_11_25`, and README
advertises that version. rmcp 3 adds `V_2026_07_28`, discovery, stateless modern
HTTP, modern subscriptions, MRTR, and task extensions. Enabling that protocol
without implementing its lifecycle would over-advertise server support.

rmcp 3's default `ServerHandler::supported_protocol_versions()` returns all
known versions, independently of `get_info().protocol_version`. Therefore PR
#50 must override it with exactly `[ProtocolVersion::V_2025_11_25]`. This keeps
legacy initialize behavior and prevents `server/discover` from claiming modern
support prematurely.

### 2. Preserve deprecated logging notifications temporarily

The public `subscribe` tool streams structured RX events through
`notifications/message`. Removing it would break observable behavior and many
PTY/HTTP/native_sim tests.

SEP-2577 says logging remains wire-compatible during the deprecation period,
existing implementations should continue advertising negotiated deprecated
capabilities, and implementations should not add new use. Therefore:

- retain `ServerCapabilities::enable_logging()`;
- retain `LoggingMessageNotificationParam`, `LoggingLevel`,
  `notify_logging_message`, and client collectors;
- isolate `#[allow(deprecated)]` to clearly named compatibility boundaries,
  with comments linking SEP-2577 and this plan;
- do not add new logging-based features.

`subscriptions/listen` cannot replace serial RX notifications in rmcp 3.0.1.
`SubscriptionSink::send` explicitly rejects logging, progress, task-status, and
custom notifications. Modern subscriptions only carry accepted tool-list,
prompt-list, resource-list, and resource-updated notifications.

### 3. Remove obsolete core task metadata, do not fake Tasks support

Five tools currently declare `execution(task_support = "optional")`:
`transact`, `read`, `send_break`, `subscribe`, and `capture_boot`. rmcp 3 moved
Tasks from experimental core metadata to the `io.modelcontextprotocol/tasks`
extension and removed the macro's `execution` field. serial-mcp does not
implement `TaskManager`, task handles, or `tasks/get/update/cancel`.

PR #50 must remove those five `execution(...)` attributes. It must not replace
them with capability claims that serial-mcp does not implement. Existing
request cancellation tokens and progress notifications remain unchanged; they
are not Tasks support.

This removal changes only obsolete task metadata in `tools/list`; tool names,
descriptions, annotations, input schemas, and output schemas remain.

### 4. Use rmcp constructors for non-exhaustive models

rmcp 3 makes more public structs non-exhaustive. Replace external struct
literals with constructors/builders rather than adding wildcard-like local
workarounds. This gives future rmcp releases room to add optional fields.

## Required source migration

### A. Tool macros and request metadata

Files:

- `src/server.rs`
- `src/tools/control_ops.rs`
- `src/tools/io_ops.rs`
- `src/tools/stream_ops.rs`

Changes:

1. Remove the five unsupported `execution(task_support = "optional")` tool
   attributes.
2. Replace removed `Meta` with `RequestMetaObject` for tool request extractors
   and helper parameters. `RequestMetaObject::get_progress_token()` preserves
   current progress-token behavior.
3. Keep cancellation-token and `Peer<RoleServer>` extraction unchanged.
4. Replace `ProgressNotificationParam` struct literals with:

   ```rust
   ProgressNotificationParam::new(token, progress)
       .with_total(total)
       .with_message(message)
   ```

5. Replace `LoggingMessageNotificationParam` struct literals with
   `LoggingMessageNotificationParam::new(level, data).with_logger(logger)`.
6. Keep deprecation allowances scoped to logging compatibility code, not whole
   production crate.

Observable verification:

- all 27 tools still appear;
- long-running calls still honor request cancellation;
- `send_break` still emits progress with token, progress, total, and message;
- `subscribe` still emits same JSON payloads through
  `notifications/message`.

### B. Prompt model rename

Files:

- `src/prompts/diagnose.rs`
- `src/prompts/interactive.rs`

Change `PromptMessageRole::User` to `Role::User`. Keep prompt message ordering
and text byte-for-byte.

Observable verification:

- `prompts/list` still returns `diagnose_port` and `interactive_terminal`;
- `prompts/get` returns user-role messages with unchanged text.

### C. Resource model and response changes

File: `src/server.rs`

Changes:

1. Replace removed `RawResource` with `Resource` and removed
   `RawResourceTemplate` with `ResourceTemplate`.
2. Build models with `Resource::new(uri, name)` and
   `ResourceTemplate::new(uri_template, name)`, then existing description,
   MIME, size, priority, and audience builders.
3. Construct paginated results through
   `ListResourcesResult::with_all_items(resources)` and
   `ListResourceTemplatesResult::with_all_items(resource_templates)`, then set
   `next_cursor`. Do not hand-maintain new `result_type`, `ttl_ms`, or
   `cache_scope` fields.
4. Change manual `ServerHandler::read_resource` return type from
   `ReadResourceResult` to `ReadResourceResponse`; convert each complete result
   with `.into()`.
5. Do not add TTL/cache hints in migration PR.

rmcp 3 clears `resultType` for legacy wire replies; unset `ttlMs` and
`cacheScope` remain omitted. Constructor use lets rmcp perform version-aware
normalization without serial-mcp duplicating new field defaults.

Observable verification:

- same two static resources and three templates remain;
- text, blob, profile-preview, connection-detail, and log resources retain
  their content and MIME types;
- unknown resources retain protocol-level resource-not-found errors;
- current MCP `2025-11-25` responses do not unexpectedly expose modern-only
  `resultType` or mandatory cache fields.

### D. Non-exhaustive protocol unions

File: `src/server.rs`

`Reference` is now non-exhaustive. Add a fallback arm in `get_completions()`
that returns no suggestions. Preserve existing resource and prompt completion
branches.

Review every downstream exhaustive match surfaced after production code
compiles. For rmcp protocol unions, use explicit known arms plus a conservative
wildcard; never panic on future variants.

### E. Protocol and deprecation boundary

Files:

- `src/server.rs`
- `src/tools/stream_ops.rs`
- test collector modules that directly name deprecated logging models

Changes:

1. Add `SerialHandler::supported_protocol_versions()` returning only
   `V_2025_11_25`.
2. Keep `get_info().protocol_version` at `V_2025_11_25`.
3. Keep legacy resource `subscribe`/`unsubscribe` handlers and capability for
   current clients.
4. Add narrow, documented deprecation allowances around:
   - legacy logging capability and notification sender;
   - legacy resource subscription handler implementation;
   - test collectors that prove backward compatibility.
5. Do not suppress unrelated warnings or apply crate-wide production
   `allow(deprecated)`.

Observable verification:

- initialize negotiates `2025-11-25`;
- supported/discoverable version list contains only `2025-11-25`;
- legacy resource subscribe/unsubscribe still works;
- open/close still emits resource-list and subscribed resource-update
  notifications;
- build and clippy pass with warnings denied.

### F. Test API adaptation

Likely files, based on direct rmcp usage:

- `tests/common/mod.rs`
- `tests/common/spawned.rs`
- `tests/http_integration.rs`
- `tests/resource_subscriptions.rs`
- `tests/serial_pty.rs`
- `tests/stdio_integration.rs`
- `tests/blob_resources.rs`
- `tests/protocol_emulator.rs`
- `tests/protocol_emulator_binary.rs`
- `tests/native_sim_validation/unix.rs`
- `tests/native_sim_connection_lifecycle.rs`

Rules:

1. Adapt only compiler-proven API changes after production code compiles.
2. Preserve public-behavior assertions; do not weaken tests to match rmcp.
3. Replace removed model names/variants with rmcp 3 equivalents or accessors.
4. Keep test-only deprecation allowances limited to collectors and legacy
   resource-subscription tests.
5. Add explicit protocol-version tests instead of relying only on compilation.

Expected additional change: `RawContent` may require migration to rmcp 3's
content-block representation in test helpers once macro errors no longer mask
test compilation. Follow v3 constructors/accessors, preserving extracted tool
error text behavior.

## Required regression tests

### Unit/API tests

File: `src/server.rs` test module or a focused new test module.

Add tests that prove:

1. `SerialHandler::supported_protocol_versions()` is exactly
   `[V_2025_11_25]`.
2. `get_info().protocol_version` remains `V_2025_11_25`.
3. tool catalog remains exactly 27 tools.
4. removed experimental task metadata is absent from affected tool JSON while
   cancellation/progress behavior remains covered elsewhere.

### HTTP public-boundary tests

Files: `tests/http_integration.rs`, `tests/resource_subscriptions.rs`.

Add or strengthen tests that prove:

1. real client initialization negotiates `2025-11-25`;
2. tools/list returns 27 tools and existing schemas still validate;
3. resources/list, resources/templates/list, resources/read, and prompts/get
   work over real HTTP transport;
4. legacy resources/subscribe and resources/unsubscribe still work;
5. resource list/update notifications still arrive after open/close;
6. logging-backed serial `subscribe` emits and terminates with existing payload
   semantics.

### Stdio public-boundary tests

File: `tests/stdio_integration.rs`.

Keep real child-process initialization and representative tool calls passing.
Add protocol-version assertion if current helper exposes peer info.

### Schema/evaluator tests

Files:

- `src/tools/mod.rs`
- `tests/doc_drift.rs`
- `xtask/src/agent_eval/*` only if behavior requires code adaptation
- `docs/development/agent-interface-evaluation.md` only after measurement

Run tool-schema guards. rmcp 3 and rmcp-macros 3 may change generated schema
serialization even when serial-mcp types do not change. Regenerate the agent
evaluation report and explain every byte delta. Keep
`docs/development/agent-interface-baseline.json` historical. Do not update a
baseline or prose number merely to silence a test.

## Documentation changes in PR #50

Files:

- `CHANGELOG.md`
- `AGENTS.md`
- `README.md` only if implementation changes a statement currently presented
  as executable truth
- this plan

Add an Unreleased migration note covering:

- rmcp 3.0.1 SDK migration;
- continued MCP `2025-11-25` wire target;
- removal of obsolete experimental task-support metadata;
- temporary SEP-2577 logging/resource-subscription compatibility boundary;
- no new rmcp 3 feature enabled yet.

Update `AGENTS.md` with current rmcp compatibility facts and warning policy.
Keep historical changelog sections untouched.

## Implementation order

1. **Compile surface** — tool macro attributes, metadata types, constructors,
   prompt role, resources, MRTR-aware response type, non-exhaustive matches.
2. **Compatibility boundary** — explicit protocol-version support and narrow
   deprecation allowances.
3. **Tests** — adapt rmcp client/model APIs without weakening behavior; add
   protocol and metadata regressions.
4. **Schema evaluation** — run schema guards and deterministic agent evaluator;
   document justified deltas.
5. **Docs** — update Unreleased and repository truth.
6. **Full gates** — run all project and Nix gates before push.

Each step should leave `cargo check --all-targets --locked` with fewer errors.
Do not hide unresolved categories behind broad lint allowances.

## Verification commands

Run with repository Rust/Nix environment after each focused stage:

```bash
cargo fmt --all -- --check
cargo check --all-targets --locked
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
nix flake check
```

Then run orchestrated software-only suites, including native_sim assets:

```bash
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
```

Focused diagnosis commands:

```bash
cargo test --test http_integration
cargo test --test stdio_integration
cargo test --test resource_subscriptions
cargo test --test serial_pty
cargo test --test blob_resources
cargo test --test config_schema_validation
cargo test --test doc_drift
```

Acceptance requires zero unexplained warnings, no skipped required gate, and no
hardware dependency.

## Future rmcp 3 opportunities

These are research outcomes, not PR #50 scope.

### Priority 1: MCP 2026-07-28 discovery and modern subscriptions

Potential value:

- `server/discover` and explicit version negotiation;
- stateless modern Streamable HTTP lifecycle;
- `subscriptions/listen` for tool-list, prompt-list, resource-list, and
  resource-update notifications;
- standard MCP headers and newer conformance behavior.

Required design work:

- replace legacy resource subscriber counts with a lifecycle-safe registry or
  broadcaster feeding `SubscriptionContext` sinks;
- preserve open/close resource notifications across multiple HTTP clients;
- test cancellation, disconnect, backpressure, and graceful completion;
- support legacy initialize/subscriptions during a defined transition period;
- decide whether `2025-11-25` and `2026-07-28` run concurrently.

Important limitation: this does not replace serial RX `subscribe`, because
rmcp's `SubscriptionSink` rejects logging and custom notifications.

### Priority 2: proper Tasks extension for long-running operations

Candidates: bounded `read`, `transact`, `capture_boot`, and long `send_break`
calls. `subscribe` is not a good first task because it is an open-ended stream
and task-status subscription support is not wired in rmcp 3.0.1.

Potential value:

- durable task handles;
- cooperative cancellation;
- polling through `tasks/get`;
- explicit asynchronous execution instead of obsolete `execution.taskSupport`
  hints.

Required design: task ownership per connection, result retention/TTL, profile
and serial cleanup after client loss, interaction with existing cancellation
tokens, and multi-session authorization.

### Priority 3: cache hints for stable catalogs/resources

Potential value:

- cache tool, prompt, resource, and template lists using `ttlMs` and
  `cacheScope`;
- automatic client invalidation from list/update notifications.

Risks:

- serial ports and open connections are dynamic;
- profiles can change cross-process;
- stale-on-error client defaults can hide fresh server errors.

Start only after modern notification invalidation is proven. Prefer short,
private caching for dynamic serial resources; static tool/prompt catalogs are
safer first candidates.

### Priority 4: standard HTTP routing headers

Annotate selected primitive input properties with `x-mcp-header` so rmcp emits
`Mcp-Method`, `Mcp-Name`, and `Mcp-Param-*` headers for proxies and observability.
Possible fields: `connection_id`, `profile`, or `port`.

Before adoption, assess header privacy, Base64 wrapping, proxy logs, and whether
serial identifiers should appear outside encrypted request bodies.

### Priority 5: MRTR for explicit user input

Possible uses:

- request confirmation or missing reset parameters before destructive boot
  capture;
- ask client for protocol-specific information before a transaction;
- return integrity-protected `requestState` for stateless retries.

Use rmcp's `request-state` HMAC helper if server state crosses round trips.
Do not use MRTR to replace ordinary schema validation or deterministic defaults.

### Priority 6: OAuth for remotely exposed HTTP transport

rmcp 3 adds reactive discovery, `AuthorizationRequest`, stricter issuer
validation, scope step-up, and improved token/resource handling. serial-mcp has
no MCP HTTP authentication today. This could harden non-loopback deployment,
but requires a full security design: local-only default, authorization policy,
credential storage, per-device permissions, and remote serial-control threat
model.

### Lower-priority or poor-fit opportunities

- **Distributed EventStore:** limited value while serial connections and
  profiles remain process-local; useful only with deliberate multi-instance
  architecture.
- **Non-object output schemas:** supported by rmcp 3, but all current
  serial-mcp tools intentionally return structured objects.
- **Audio prompt content:** no current serial workflow needs it.
- **Client response cache APIs:** serial-mcp is primarily a server; only test
  clients use rmcp client APIs.

## Risks and rollback points

1. **Generated schema drift:** rmcp-macros 3 can alter schema output. Measure and
   explain; never copy new byte counts blindly.
2. **Protocol over-advertisement:** default supported versions include modern
   versions. Explicit singleton override is mandatory until modern lifecycle
   implementation lands.
3. **Silent notification regression:** broad removal of deprecated logging
   would make `subscribe` appear successful while clients receive no data.
4. **Resource wire drift:** direct struct literals can leak `resultType` or omit
   new fields. Use constructors and test serialized legacy responses.
5. **False Tasks support:** retaining old tool execution metadata without
   implementing Tasks misleads clients. Remove it now; add extension later.
6. **Broad warning suppression:** crate-wide allowances can conceal future
   deprecations. Scope all exceptions and explain them.

Rollback point: until source/test migration commits are added, PR #50 can be
returned to rmcp 1.7 by restoring `Cargo.toml` and `Cargo.lock`. After migration
begins, keep commits grouped by categories above so a failed modern-feature
experiment can be removed without undoing compatibility work.

## Decisions requiring user input

No blocking decision for migration-only scope. Plan chooses behavior
preservation and MCP `2025-11-25` over simultaneous modern feature adoption.

Before a later MCP `2026-07-28` phase, user input will be needed on:

1. whether legacy `2025-11-25` clients must remain supported concurrently;
2. whether serial RX streaming may use a serial-mcp custom protocol extension
   after MCP logging removal, or must stay within standard MCP primitives;
3. whether remote HTTP authentication is a product goal;
4. which operations, if any, should become durable Tasks.
