# rmcp 3 Migration Phase 1 Handoff

## Goal

Make the repository compile and pass its existing software gates on rmcp 3.0.1
while landing the already-approved removal of MCP logging, the serial-mcp
`subscribe`/`unsubscribe` tools, legacy resource subscribe handlers, and their
subscription-only configuration. Do not implement MCP `2026-07-28` discovery
or `subscriptions/listen` in this phase.

Work from the current PR #50 head. Read `AGENTS.md` and
`docs/development/rmcp-3-migration-plan.md` first.

## Grounding evidence

Current command:

```bash
cargo check --all-targets --locked
```

reports 41 errors and 20 warnings. Primary non-cascade failures:

- `#[tool(execution(...))]` removed from rmcp macros;
- `rmcp::model::Meta` removed; replacement is `RequestMetaObject`;
- `ProgressNotificationParam` is non-exhaustive and requires constructors;
- `RawResource`/`RawResourceTemplate` replaced by
  `Resource`/`ResourceTemplate`;
- list results gained `result_type`, `ttl_ms`, and `cache_scope`;
- `read_resource` now returns `ReadResourceResponse`;
- `Reference` became non-exhaustive;
- `PromptMessageRole` became `Role`;
- logging models and `notify_logging_message` are deprecated;
- the first macro failures cause missing `*_tool_attr` cascades.

Authoritative rmcp 3.0.1 source:

- request metadata and progress constructors:
  `crates/rmcp/src/model/meta.rs` and `crates/rmcp/src/model.rs`;
- resource constructors:
  `crates/rmcp/src/model/resource.rs`;
- annotations:
  `crates/rmcp/src/model/annotated.rs`;
- MRTR-aware resource response:
  `crates/rmcp/src/model/mrtr.rs`;
- handler signatures and version stripping:
  `crates/rmcp/src/handler/server.rs`.

Local source root:

```text
/home/thomas-workstation/Nextcloud/Development-Resources/mcp_model_context_protocol/rust-sdk-rmcp-v3.0.1
```

## In scope

### 1. Mechanical rmcp 3 API migration

Files:

- `src/server.rs`
- `src/tools/control_ops.rs`
- `src/tools/io_ops.rs`
- `src/prompts/diagnose.rs`
- `src/prompts/interactive.rs`
- compiler-proven direct-rmcp test sites

Required choices:

1. Delete all five `execution(task_support = "optional")` attributes. Do not
   replace them with Tasks metadata.
2. Replace function parameters/imports of removed `Meta` with
   `RequestMetaObject`. Preserve `get_progress_token()` behavior.
3. Build progress values only through:

   ```rust
   ProgressNotificationParam::new(token, progress)
       .with_total(total)
       .with_message(message)
   ```

4. Replace `RawResource` and `RawResourceTemplate` with `Resource` and
   `ResourceTemplate`. Preserve audience/priority by attaching:

   ```rust
   Annotations::default()
       .with_priority(...)
       .with_audience(...)
   ```

   through `.with_annotations(...)`.
5. Build list results with `with_all_items`, then set `next_cursor`. Leave
   `ttl_ms` and `cache_scope` absent in this phase; Phase 4 adds the
   version-correct modern cache policy. Constructors must retain
   `resultType: complete` so rmcp can strip it for legacy peers.
6. Change `read_resource` to return `Result<ReadResourceResponse, McpError>`.
   Convert each complete `ReadResourceResult` with `.into()`. Do not implement
   MRTR/input-required behavior.
7. Add a conservative wildcard arm to completion reference matching that
   returns no suggestions. No `todo!()` or `unimplemented!()`.
8. Use `Role::User` in prompt messages.

### 2. Remove old streaming and logging surfaces

Production files:

- delete `src/tools/stream_ops.rs`;
- remove `pub mod stream_ops` from `src/tools/mod.rs`;
- remove `StreamRegistry`, its builder option, and all construction/injection
  from `src/server.rs`, `src/main.rs`, and tests;
- remove handler fields used only by old subscriptions (`streams`,
  `subscribers`);
- remove serial-mcp tool handlers/catalog entries for `subscribe` and
  `unsubscribe`;
- remove `SubscribeArgs`, `SubscribeResult`, `UnsubscribeArgs`,
  `UnsubscribeResult`, and every `Subscribe*Notification` type from
  `src/tools/types.rs`;
- remove legacy MCP `resources/subscribe` and `resources/unsubscribe` handler
  implementations;
- remove old resource notification helper/calls that emit
  `notifications/resources/list_changed` or use the old subscriber count;
- remove `enable_logging`, `enable_resources_subscribe`, and all three
  `listChanged` capability flags. Phase 1 capabilities are tools, resources,
  prompts, and completions only;
- remove all rmcp logging imports, collectors, and calls. Keep Rust `tracing`
  and connection `get_log`/`clear_log`/`export_log` behavior.

No compatibility alias, hidden tool, custom notification, or deprecation
allowance.

### 3. Remove subscription-only configuration

`poll_interval_ms` has no consumer after old streaming removal and must not stay
as a public no-op. Remove it from:

- `OpenArgs`, `ProfileDefaults`, `ConnectionConfig`, resolved open settings,
  connection atomics/accessors/setters/effective-default snapshots;
- profile/open/configure schemas and tool descriptions;
- profile learning/comparison/write-through code;
- limit constants and schema helpers used only by streaming;
- test fixtures and assertions.

Also remove stream-only min/max chunk constants and poll validators when no
remaining production use exists. Keep `max_buffered_bytes`: `read` and
`capture_boot` still use it.

Persistence behavior:

- no profile schema-version bump;
- an existing TOML profile containing `poll_interval_ms` must still load;
- after a real durable mutation/reload, obsolete key may be absent;
- do not reject or corrupt legacy profile files solely because they contain
  old key.

Add a real temp-file persistence regression in `src/profile_store.rs` or the
smallest existing profile persistence test boundary. Do not assert private
fields only.

### 4. Adapt tests without weakening retained behavior

Files include:

- `tests/common/mod.rs`, `tests/common/spawned.rs`,
  `tests/common/controlled.rs`;
- `tests/http_integration.rs`, `tests/stdio_integration.rs`;
- `tests/serial_pty.rs`, `tests/protocol_emulator.rs`;
- `tests/native_sim_validation/unix.rs`,
  `tests/native_sim_connection_lifecycle.rs`;
- `tests/proptest.rs`, `fuzz/fuzz_targets/tool_call_json.rs`;
- `src/tools/mod.rs`, `src/tools/rx_validate.rs`, `src/serial/mod.rs`;
- delete `tests/resource_subscriptions.rs` for now; Phase 3 recreates it for
  modern `subscriptions/listen`.

Test helper decision:

- replace logging `NotificationCollector` with a no-op `TestClientHandler`;
- `connect_client`/spawned helpers may continue returning `(client, ())` to
  minimize unrelated caller churn;
- retain a progress-only collector and progress receiver;
- delete `next_notification` and all old logging-message parsing.

Remove tests/stages whose only public behavior is deleted subscribe/unsubscribe.
When a schema/framing/matcher test also proves `read` behavior, keep or rewrite
the read assertion. Do not delete read, framing, parser, matcher, ring-wrap,
cursor, loss, cancellation, capture, or encoding-fallback coverage merely
because an adjacent subscribe assertion disappeared.

### 5. Update current product surface

Update executable truth and current guidance to 25 tools:

- `README.md`
- `AGENTS.md`
- `docs/agent-config.md`
- `docs/development/FEATURES.md`
- `docs/development/agent-interface-evaluation.md`
- `tests/doc_drift.rs`
- tool-count/catalog tests and evaluator expectations
- `CHANGELOG.md` under `## [Unreleased]`

Preserve historical release table/body text describing older versions and
their 27-tool/subscribe behavior. Keep committed historical evaluator baseline
`docs/development/agent-interface-baseline.json` unchanged. Regenerate current
evaluation output through the evaluator; do not hand-edit unexplained byte
counts.

Current instructions must teach `read` for buffered/bounded/unsolicited data.
Do not teach `subscriptions/listen` yet; it ships in Phase 3.

## Out of scope

- `supported_protocol_versions`, discovery, stateless lifecycle, or raw modern
  request metadata enforcement;
- version-specific initialize capability view;
- `ResourceEventHub`, hotplug watcher, or `subscriptions/listen`;
- modern `ttlMs`/`cacheScope` policy;
- official conformance workflow;
- Tasks, MRTR, OAuth, promoted HTTP parameter headers;
- package version bump;
- unrelated serial/profile behavior.

## Invariants

- `read`, `transact`, and `capture_boot` retain existing ring/cursor/matcher/
  framing/parser/timeout/silence/encoding behavior.
- Pump gate and reset-line release guarantees remain unchanged.
- Tool operational failures remain MCP tool results where currently expected.
- Every remaining tool keeps title and output schema.
- No production `unwrap`, `expect`, `println!`, `todo!`, or `unimplemented!`.
- No broad lint/deprecation allowance.
- Historical changelog and evaluator baseline remain historical.

## Verification

Run in this order:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test doc_drift --locked
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```

If native_sim firmware already exists, also run:

```bash
cargo run --manifest-path xtask/Cargo.toml -- test
```

Do not download or run MCP conformance package in this phase.

## Delegator instructions

Coordinate mechanical slices through Workers as useful, then inspect every
change and integrate centrally. Run all required verification. Inspect git
status, diff, and recent log before committing. Stage only intended files.

Commit completed phase with:

```text
feat: migrate rmcp 3 server surface
```

Do not push, merge, amend, open PRs, or add attribution footers.

Return one recap containing:

1. files changed/deleted;
2. behavior and wire-surface changes;
3. tests/commands and exact results;
4. commit hash/message;
5. deviations or blockers;
6. suggested Phase 2 follow-up.

Escalate instead of guessing after two failed attempts at one blocker, any
conflict with rmcp/repository evidence, missing design decision, unexplained
warning/flaky failure, need to weaken a test, or scope expansion. Preserve
partial worktree state and report exact commands/errors, status, evidence, one
precise question, and smallest suspected next step. Do not commit incomplete or
knowingly failing work.
