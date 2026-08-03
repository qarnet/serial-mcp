# rmcp 3 Phase 3 Handoff: Resource Subscriptions

## Goal

Add modern `subscriptions/listen` backed by one process-wide resource event hub,
publish useful resource-update hints from serial lifecycle/RX/log paths, and add
a proactive port hotplug watcher. Preserve legacy `2025-11-25` behavior and
keep read/ring correctness authoritative.

Work only in `/home/thomas-workstation/repos/serial-mcp-pr50-analysis` at
accepted Phase 2 HEAD `97274ea6`. Do not touch primary checkout
`/home/thomas-workstation/repos/serial-mcp` (known status: only
`M src/tools/helpers.rs`). Do not use Worker subagents.

## Grounding and Fixed Decisions

Follow `docs/development/rmcp-3-migration-plan.md`, especially “Resource
notification semantics”, “Shared ResourceEventHub”, “RX pump integration”,
“Listener integration”, “Port hotplug watcher”, and Phase 3 acceptance.

Pinned rmcp API evidence:

- `SubscriptionFilter` fields and intersection behavior:
  `rust-sdk-rmcp-v3.0.1/crates/rmcp/src/model.rs:1912-2070`.
- `ServerHandler::accepted_subscription_filter` and `listen` dispatch:
  `crates/rmcp/src/handler/server.rs:147-169`.
- `SubscriptionContext::accepted()`, `cancelled()`, and `sink()`:
  `crates/rmcp/src/service/server.rs`.
- Reference server/client:
  `examples/servers/src/subscriptions_streamhttp.rs` and
  `examples/clients/src/subscriptions_streamhttp.rs`.

Use exact constants:

- hub capacity: **256** events;
- port watcher interval: **1 second**;
- event shape: `ResourceEvent::Updated(String)`.

Notifications are availability hints, not byte delivery. Never carry serial
payloads in notifications. Never move shared read cursor.

## 1. `src/resource_events.rs`

Add process-wide primitives:

- `ResourceEvent` (`Clone`, `Debug`, `PartialEq`, `Eq`);
- `ResourceEventHub` wrapping `tokio::sync::broadcast::Sender<ResourceEvent>`;
- `new(capacity)`, production default/capacity 256, `subscribe()`, and
  synchronous `publish_updated(uri)`.

Publish must never await. Ignore `send` failure when no receiver exists (trace
allowed). Each listener owns one receiver. Add unit tests for independent
receivers, no-receiver publication, and lag detection/recovery inputs.

Add URI helpers that generate detail/raw/log URIs from a connection ID and a
predicate using existing `resources::parse_resource_uri`. A subscribable URI is
only `serial://ports`, `serial://connections`, or a recognized concrete
connection detail/raw/log URI. Templates, malformed IDs, unknown schemes, and
empty IDs are rejected.

Export module from `src/lib.rs`.

## 2. Shared Ownership

Inject `Arc<ResourceEventHub>` through `SerialHandlerOptions`, builder, and
`SerialHandler`; ephemeral builder default gets a fresh hub. Production
`main.rs` creates exactly one hub per server process and clones it into every
HTTP handler factory. Stdio uses same hub for handler/watcher.

Pass hub into `RxSessionManager`/`RxSession`; update all construction/tests.
After successful `ring.append(&chunk)`, release `pump_gate`, then synchronously
publish connection detail/raw/log updates. Publication must happen after append
and outside gate. Add deterministic unit proof: subscriber cannot receive
update before bytes are in ring, and event receipt sees advanced end offset.

## 3. Modern Capability and Listener

Phase 3 capability split:

- `get_info()` must now return **modern** info because rmcp intersects listen
  filters against `get_info().capabilities`;
- modern resources capability has `subscribe: true` and no `listChanged`;
- `initialize()` continues returning legacy info with subscription disabled;
- common tools/prompts/completions unchanged; logging/tasks/list-change remain
  disabled.

Update Phase 2 pure capability tests accordingly.

Implement `accepted_subscription_filter(requested)`:

- return `Some(filter)` containing only valid, deduplicated requested
  `resource_subscriptions`, preserving first-request order;
- all three list-change flags remain absent;
- unsupported flags/URIs stripped;
- empty accepted resource list becomes `None` inside filter, not a fake URI;
- handler itself remains available (`Some(empty filter)`), letting rmcp return
  acknowledgement with accepted empty filter.

Implement `listen(context)`:

1. snapshot accepted resource URI set/order from `context.accepted()`;
2. subscribe to hub;
3. `tokio::select!` cancellation vs receiver;
4. matching `Updated(uri)` -> `context.sink().notify_resource_updated(uri)`;
5. unrelated event ignored;
6. `Lagged(_)` -> notify every accepted URI once, in accepted order;
7. closed hub/cancellation/sink failure -> terminate cleanly; no retry loop.

Use rmcp error mapping only for genuine sink failure if API requires a result;
cancellation and closed hub complete successfully.

## 4. Publish Resource Updates

Use small hub helper methods, not duplicated string formatting. Publish after
successful public behavior:

- port opened: `serial://connections` + new detail URI;
- port closed: `serial://connections` + closed detail URI;
- reconfigure, set-flow-control, reconnect/state change, connection-mode
  configure: detail URI;
- RX append: detail + raw + log, after ring append;
- write/transact/send-break/control operations that add connection log/state:
  relevant detail/log URI;
- `clear_log`: log URI;
- input flush/ring state change: detail/raw URI.

Do not publish on validation/tool failure. Duplicate hints are acceptable.
Profile-only changes with no live connection URI need no event. Existing tool
results/errors/profile learning remain unchanged.

## 5. Port Hotplug Watcher

Add process-owned watcher in `src/resource_events.rs` (or one focused sibling):

- consumes shared `Arc<dyn PortProvider>`, hub, cancellation token, interval;
- canonicalize each successful snapshot by sorting full stable `PortInfo`
  identity fields, not enumeration order;
- first success establishes baseline without event;
- changed success publishes `URI_PORTS` once;
- unchanged/reordered success publishes nothing;
- failure warns and retains prior successful baseline;
- recovery compares against retained baseline;
- deterministic shutdown/join API for tests.

Production `main.rs` must use one shared `SystemPortProvider` for handler and
watcher. Start one watcher in HTTP and stdio modes, cancel and join on server
shutdown. Test harness supports injected mutable provider and deterministic
short interval/manual tick without sleeping one production second.

## 6. Public-Boundary Tests

Recreate `tests/resource_subscriptions.rs` around modern
`subscriptions/listen`. Use typed modern clients and real in-process HTTP MCP;
add raw checks in `tests/protocol_compatibility.rs` only where version wire
shape matters.

Required proofs:

- modern discovery advertises resource subscriptions; legacy initialize does
  not;
- acknowledgement contains only accepted, valid, deduplicated resource URIs;
- list-change flags, templates, malformed and unknown URIs stripped;
- legacy listen stays `-32601`;
- RX append notification arrives only after bytes are readable;
- notification does not move shared read cursor;
- `read` works with no listener;
- two listeners get same update independently;
- cancelling one leaves other active;
- forced hub lag yields conservative one-per-accepted-URI recovery and never
  blocks publisher/pump;
- open/close/state/RX/log operations emit expected URI hints;
- two stateless HTTP handler instances share hub;
- stdio listener cancellation completes cleanly;
- mutable provider add/remove/identity change emits ports update;
- reorder/unchanged/error emits no false update; recovery works.

Test observable notifications/resources and subsequent reads, not private Arc
identity or helper call counts.

## Documentation

Update `AGENTS.md`, README common flow, both prompts/server instructions only as
needed to teach modern `subscriptions/listen` as optional wakeup mechanism;
`read` remains primary lossless data path. Update `CHANGELOG.md` Unreleased.
Update tool descriptions only if current text says no ongoing monitoring.
Tool count remains 25.

## Out of Scope

- legacy resource subscribe/unsubscribe handlers;
- tool/prompt/resource list-change notifications;
- cache field policy/conformance CI (Phase 4);
- payload streaming over notifications;
- persistent capture changes, tasks, MRTR, positive cache TTLs;
- dependency/version changes, baseline rewrite, push/merge/PR/amend.

## Verification

```bash
cargo fmt --all -- --check
cargo test --lib resource_events --locked
cargo test --lib rx_session --locked
cargo test --test resource_subscriptions --locked
cargo test --test protocol_compatibility --locked
cargo test --test http_integration --locked
cargo test --test stdio_integration --locked
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --test doc_drift --locked
```

Inspect status/diff/log and `git diff --check`. Commit all scoped work plus this
handoff as new commit:

`feat: add modern resource subscriptions`

Do not push. Recap files, ownership/lifecycle behavior, every required public
proof, watcher evidence, commands/results, commit hash, deviations/blockers.
Escalate before commit on rmcp API contradiction, pump-gate ordering ambiguity,
shutdown leak, flaky timing, unexplained notification loss, required architecture
beyond this handoff, test weakening, or two failed approaches.
