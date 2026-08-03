# rmcp 3 Phase 1 Review Fix Handoff

## Goal

Close Phase 1 review defects after commit `8339477c`: remove remaining public
and internal residue from deleted serial-mcp streaming/logging surfaces, repair
current documentation, regenerate measured interface facts, run gates, and
commit one focused follow-up. Do not amend `8339477c`.

Work only in:

`/home/thomas-workstation/repos/serial-mcp-pr50-analysis`

## Grounding

- `src/tools/stream_ops.rs` and serial-mcp `subscribe`/`unsubscribe` tools are
  gone in `8339477c`.
- Current tool count is 25.
- Repository searches show no production caller for:
  - `SerialConnection::record_notification_drop`;
  - `LogBuffer::notification_dropped`;
  - `RxStopController::{record_data,check_max_buffered_bytes,read_error,
    channel_closed,peer_disconnected}`;
  - `RxStopMetadata::{read_error,channel_closed,peer_disconnected,
    budget_exhausted}`.
- `NotificationDropped`, `SubscribeStarted`, and `SubscribeStopped` remain only
  as dead `LogEvent` variants.
- `notification_drop_count` remains in connection/status/tool schemas but has
  no producer and therefore can only report zero.
- `ReadError`, `ChannelClosed`, `PeerDisconnected`, and `BudgetExhausted` remain
  in `RxStopReason`, but no retained production path constructs them. Current
  paths use the pump/ring state and tool errors instead.
- `docs/protocols.md` still presents deleted subscribe behavior as current.
- `AGENTS.md` still names `SubscribeArgs` and reports stale current evaluator
  metrics (`27 tools / 288177 bytes`). Current pre-fix generated report is
  `25 tools / 269958 bytes`; removals in this fix will change byte totals, so
  rerun evaluator and record measured post-fix values rather than copying that
  number.
- Deleted fuzz corpus entries inspected during review all begin with obsolete
  `poll_interval_ms`; their deletion is intentional. Do not restore them.

## In Scope

### 1. Remove dead streaming/logging residue

Remove these fields, variants, helpers, mappings, and tests completely:

- `LogEvent::{NotificationDropped,SubscribeStarted,SubscribeStopped}` and
  `LogBuffer::notification_dropped` in `src/log_buffer.rs`;
- `notification_drop_count` from `SerialConnection`, `ConnectionStatus`,
  `GetStatusResult`, status construction/mapping, and HTTP assertions;
- `RxStopReason::{ReadError,ChannelClosed,PeerDisconnected,BudgetExhausted}`;
- matching `RxStopMetadata` constructors;
- matching `RxStopController` methods and unit tests;
- production-unused `RxStopController::record_data` and
  `check_max_buffered_bytes`, plus tests existing only for them;
- obsolete `rx_consume` match arms and `tests/proptest.rs` expected values.

Keep retained outcomes and behavior unchanged: `data_complete`, `timeout`,
`match_found`, `max_buffered_bytes`, `no_new_rx_timeout`,
`connection_closed`, `cancelled`, `max_frames`, `framing_error`, and `drained`.
Update `ReadResult.stop_reason` schema prose to list these actual outcomes,
including `drained`.

### 2. Remove stale current subscribe wording

Update current source comments/schema descriptions in at least:

- `src/buffer_budget.rs`
- `src/framing/config.rs`
- `src/framing/mod.rs`
- `src/log_buffer.rs`
- `src/match_config.rs`
- `src/profiles.rs`
- `src/rx_metadata.rs`
- `src/serial/config.rs`
- `src/serial/connection.rs`
- `src/server.rs`
- `src/stop_controller.rs`
- `src/tools/port_ops.rs`
- `src/tools/types.rs`
- stale test comments, if any

Use present behavior, not vague replacement language:

- RX configuration/matching applies to `read`, `transact` read half, and
  `capture_boot` read pipeline;
- TX configuration applies to `write` and `transact` write half;
- `read_ops` counts successful read-pipeline operations;
- matcher bounded-window behavior now has one retained raw read path;
- tool catalog guard is `tool_catalog_has_exactly_twenty_five_tools`.

Do not rename Rust crate dependency `tracing_subscriber`; substring matches in
that identifier are unrelated.

### 3. Repair current docs

Update `docs/protocols.md` so it describes only retained tools:

- explain RX fields on `read`, `transact`, and `capture_boot`, and TX fields on
  `write` and `transact`;
- describe malformed frame behavior as a call/read-pipeline stop, not a
  subscription stop;
- describe precedence as shared by `write`, `read`, `transact`, and
  `capture_boot` through existing shared helpers;
- describe checksum drops and framing errors through `ReadResult`, noting that
  `TransactResult.read` and `CaptureBootResult.read` carry that shape;
- remove `SubscribeStopNotification` and
  `SubscribeEncodingErrorNotification` field references.

Update `AGENTS.md`:

- remove `SubscribeArgs` and other current-behavior residue;
- remove `subscription lifecycle` from never-persisted state;
- retain explicit historical/removal statements where useful;
- after evaluator runs, set current report metrics to measured 25-tool values;
- keep committed historical baseline at `26 tools / 258964 bytes`.

Do not rewrite historical `CHANGELOG.md`, migration plan, Phase 1 handoff, or
historical evaluator baseline references merely because they mention removed
features. `docs/development/FEATURES.md` future `subscribe-style` hotplug item
belongs to later modern `subscriptions/listen` work and is out of scope.

### 4. Add drift guards

In `src/tools/mod.rs`, add
`tool_catalog_omits_removed_streaming_surface`. Serialize each tool catalog
entry and fail with tool name if generated descriptions/input/output schemas
contain any removed current surface:

- backticked/literal `subscribe` wording or `Subscribe` type names;
- `poll_interval_ms`;
- `notification_drop_count`;
- `peer_disconnected`;
- `budget_exhausted`;
- `channel_closed`;
- `read_error`.

Avoid rejecting generic future MCP resource-subscription terminology outside
tool schemas.

In `tests/doc_drift.rs`, add
`current_protocol_guide_omits_removed_streaming_tool` covering
`docs/protocols.md` and its deleted notification type names. Keep test narrow;
historical docs are allowed to mention old behavior.

### 5. Refresh measured docs

Run evaluator after schema cleanup:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval --baseline docs/development/agent-interface-baseline.json
```

Update `docs/development/agent-interface-evaluation.md` and `AGENTS.md` with
actual generated 25-tool aggregate bytes and any changed largest-tool values.
Do not alter historical baseline JSON.

## Out of Scope

- Phase 2 lifecycle/discovery implementation.
- Modern `subscriptions/listen`, event hub, or hotplug implementation.
- Cache TTL/header policy or conformance implementation.
- Changelog history rewrites.
- Restoring obsolete `poll_interval_ms` fuzz corpus.
- Dependency/version changes.
- Push, merge, PR creation, or commit amendment.

## Verification

Run, in this order:

```bash
cargo fmt --all -- --check
cargo test --lib tool_catalog_omits_removed_streaming_surface --locked
cargo test --test doc_drift current_protocol_guide_omits_removed_streaming_tool --locked
cargo run --manifest-path xtask/Cargo.toml -- agent-eval --baseline docs/development/agent-interface-baseline.json
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Then inspect:

```bash
git status --short
git diff --check
git diff
```

Search current source and current protocol guide for removed residue. Expected
remaining mentions should be intentional historical/removal text only, not
generated tool schemas or current behavior.

## Commit and Recap

Stage only intended files and this handoff. Commit as a new commit with message:

`fix: remove obsolete streaming residue`

Do not push. Return:

- files changed;
- public/schema behavior removed;
- actual evaluator metrics;
- every command run and result;
- commit hash/message;
- remaining intentional search matches;
- blockers or deviations.

Escalate without committing if two attempts fail, repository evidence
contradicts this handoff, removal affects a live production path, evaluator
changes cannot be explained, or any gate requires weakening tests or expanding
architecture.
