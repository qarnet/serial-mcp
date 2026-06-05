# PLAN 1 - Unify RX Around Session Core

## Goal

Redesign RX around one shared per-connection RX session so `read` and `subscribe`
become two views over the same live byte stream.

This plan is intentionally phased.

- PLAN 1a builds the RX session core.
- Later PLAN 1 steps migrate tools onto it.

Core design decisions already locked:

- one RX pump task per connection
- only the pump reads from serial RX directly
- start semantics are **future-only**
- no retained RX history in the session
- blocking `read` keeps burst/settle behavior after first byte
- future matcher behavior becomes an option on `read` / `subscribe`, not a
  separate external RX engine

## Architecture

### Unified RX model

- `read` = blocking RX operation
- `subscribe` = non-blocking RX operation
- `timeout_ms`, future `wait_for`, future `no_new_rx_timeout_ms`, and future
  event-context are layered policies/options on top of those two modes

### Session model

Each open connection owns one `RxSession`.

`RxSession` responsibilities:
- own one background RX pump task
- receive bytes from serial port
- fan out future bytes to registered consumers
- coordinate shutdown on close/error

`RxSession` non-goals for PLAN 1a:
- no replay/history
- no ring buffer
- no event-context slicing yet

### Important semantics

- all consumers see only bytes that arrive after they register
- no tool reads old buffered session data because none is retained
- any OS-side bytes already queued before registration are implementation detail
  and should not become explicit session semantics

## Phase Plan

### PLAN 1a - Build RX session core

In scope:
- add `RxSession` per connection
- add one RX pump task per connection
- pump is the only code allowed to touch serial RX directly
- define internal consumer registration model for blocking readers and
  background subscribers
- define shutdown/replacement lifecycle
- do not change user-facing tool behavior more than necessary yet

Out of scope:
- full subscribe API redesign
- removal of external `wait_for` tool
- metadata/schema expansion beyond what is needed to support migration

### PLAN 1b - Move subscribe onto RX session

Target behavior:
- both subscribe forms are background modes
- `timeout_ms` means auto-stop later, not blocking inline collection
- one active subscription per connection still
- subscribe returns immediate ack

Agent notes:
- public `subscribe` tool stays, but implementation must stop reading directly from
  `SerialConnection`
- `StreamRegistry` may stay temporarily, but should become a thin owner of active
  subscriber handles built on `RxSession`
- timed subscribe and untimed subscribe must share the same background path
- final stop info can remain notification-based and simple in this phase

Definition of done:
- no blocking inline collect branch remains in subscribe implementation
- timed subscribe auto-stops in background
- untimed subscribe still streams until stop condition
- replacing an active subscribe for the same connection still works

### PLAN 1c - Move read onto RX session

Target behavior:
- blocking `read` uses session consumer path, not direct serial polling loop
- keep settle behavior after first byte

Agent notes:
- preserve user-visible behavior as closely as practical
- keep existing "first byte then brief settle" sampling semantics
- move accumulation logic out of direct serial polling helper and onto a blocking
  consumer fed by `RxSession`

Definition of done:
- read tool no longer competes with subscribe by reading port directly
- read still returns burst sample behavior
- close/cancel/timeout continue to behave correctly

### PLAN 1d - Replace external wait_for engine

Target behavior:
- wait-for matching becomes internal option/policy on top of unified RX core
- external tool can remain temporarily for compatibility, but no separate RX loop

Agent notes:
- long-term public direction is "wait_for becomes an option, not a separate tool"
- short-term compatibility wrapper is acceptable if needed
- matching remains literal byte-substring only in this phase
- subscribe-with-match in this phase should be designed as stop-on-first-match,
  while future plans may extend it to match-many notifications

Definition of done:
- no standalone `wait_for` polling path remains
- wait-for logic consumes bytes from `RxSession` like read/subscribe do
- API compatibility can be preserved temporarily if cheaper for migration

## PLAN 1a Scope

In scope:
- internal RX session abstraction
- internal blocking-consumer abstraction for future `read`
- internal background-subscriber abstraction for future `subscribe`
- connection lifecycle integration: start on demand, stop on close

Out of scope:
- buffer budget manager
- stop/truncation metadata expansion
- silence timeout
- event-context behavior
- history/replay

## Internal Model For Agent Implementation

Recommended internal pieces:

- `RxSessionManager`
  - lookup/create session for connection
  - clean shutdown on connection close

- `RxSession`
  - holds connection reference
  - owns pump task
  - owns registries for consumers

- `RxConsumer`
  - blocking reader consumer type
  - background subscriber consumer type

- `RxEvent`
  - data chunk
  - closed
  - read error
  - replaced/cancelled if useful later

### Suggested concrete shape

Keep PLAN 1a concrete and boring. Favor simple ownership and explicit channels.

- `SerialHandler`
  - gains shared `rx_sessions: Arc<RxSessionManager>`

- `ConnectionManager`
  - remains source of truth for open/close of `SerialConnection`
  - does **not** own RX fanout logic directly

- `RxSessionManager`
  - keyed by `connection_id`
  - `get_or_create(connection: Arc<SerialConnection>) -> Arc<RxSession>`
  - `remove(connection_id)` on close

- `RxSession`
  - `connection_id: String`
  - `connection: Arc<SerialConnection>`
  - `pump_task: Mutex<Option<JoinHandle<()>>>`
  - registry for blocking consumers
  - registry for streaming consumers
  - shutdown token / close token

- blocking consumer registry
  - should support one-shot delivery of future bytes
  - consumer can accumulate its own per-call state later in PLAN 1c
  - PLAN 1a only needs a future-proof primitive, not the full read semantics

- streaming consumer registry
  - should support pushing byte chunks to background subscribers
  - one subscription per connection remains public API rule later, but internal
    registry can still be generic

### Suggested internal transport primitive

For PLAN 1a, prefer explicit `mpsc` fanout over clever broadcast/history abstractions.

- pump receives bytes from `SerialConnection::read`
- pump clones/pushes chunks to registered consumer channels
- each consumer owns its own accumulation/matching logic later

Why this is preferred for PLAN 1a:
- matches future-only semantics
- avoids replay/history complexity
- avoids false coupling to later ring-buffer/event-context work
- easier to unit test with fake consumers

### Anti-goals

Do **not** add these in PLAN 1a:

- retained byte history
- cursor/sequence replay model
- regex/glob matching
- event slicing
- buffer budgeting
- silent auto-stop timeout logic
- new public tool arguments

Implementation preference:
- keep PLAN 1a minimal
- avoid exposing new user-facing schema yet
- build core so later plans can migrate tool logic cleanly

## Agent Execution Rules

When implementing PLAN 1a, agent should follow these rules:

1. Do not redesign public MCP tool schemas yet unless a tiny compatibility shim is
   required.
2. Keep existing user-visible behavior working during transition.
3. Introduce RX session core first, then move one tool at a time in later plans.
4. Preserve current close semantics work from previous fix: closing a connection
   must stop pump and all attached consumers.
5. Keep direct serial RX reads in exactly one place once PLAN 1a lands.

## Core Invariants

These invariants define success for PLAN 1a:

1. For one open connection, only one task reads from serial RX.
2. All future RX features must consume bytes from `RxSession`, not directly from
   `SerialConnection::read`.
3. Closing a connection shuts down the pump and attached consumers deterministically.
4. Creating an RX session must be idempotent for the same connection.
5. Removing an RX session must not leak background tasks.

## Data Flow

Target data flow after PLAN 1a:

1. tool or internal helper asks `RxSessionManager` for session
2. manager creates session on first use
3. session starts pump on first use
4. pump reads chunk from serial port
5. pump forwards chunk to active consumer channels
6. consumer-specific logic handles bytes outside pump
7. close/error cancels session and drops consumers

Pseudo-flow:

```text
tool/helper -> RxSessionManager -> RxSession -> pump task -> SerialConnection::read
                                                -> consumer A channel
                                                -> consumer B channel
```

## Migration Boundaries

PLAN 1a should prepare, not finish, migration.

Must be ready for:
- PLAN 1b moving `subscribe` to session consumer path
- PLAN 1c moving `read` to blocking session consumer path
- PLAN 1d replacing separate wait-for polling loop with matcher consumer

Should not yet require:
- removing `stream_ops::StreamRegistry`
- removing existing helper functions
- removing current direct-read code paths in same patch if that makes review too large

Recommended strategy:

1. add `RxSessionManager` and `RxSession`
2. add internal tests for pump lifecycle
3. wire handler to own session manager
4. optionally add one hidden/internal call site to exercise creation/teardown
5. stop before public behavior churn

After PLAN 1a, recommended migration order is:

1. migrate `subscribe`
2. migrate `read`
3. migrate `wait_for`

Reason:
- subscribe currently has the largest semantic split
- read next removes direct competition with background stream consumption
- wait_for last can reuse the settled session model

## File Touch Guidance

Expected primary files:

- `src/server.rs`
- `src/serial.rs`
- maybe new module like `src/rx.rs` or `src/session_rx.rs`
- tests in `tests/` and/or unit tests near new module

Recommended separation:

- keep `src/serial.rs` focused on low-level connection operations
- put RX session orchestration in a new module
- avoid overloading `stream_ops.rs` with core session concerns

## Suggested New Module API

Example only. Agent may adjust names if cleaner.

```rust
pub struct RxSessionManager { ... }

impl RxSessionManager {
    pub async fn get_or_create(&self, connection: Arc<SerialConnection>) -> Arc<RxSession>;
    pub async fn remove(&self, connection_id: &str);
}

pub struct RxSession { ... }

impl RxSession {
    pub async fn subscribe_stream(&self, ...);
    pub async fn subscribe_blocking(&self, ...);
    pub async fn shutdown(&self);
}
```

Important: `subscribe_blocking` here means internal blocking consumer path, not the
public MCP `subscribe` tool.

## Behavior Decisions Already Made

- future-only semantics
- no retained history, likely never
- keep read settle behavior
- subscribe future matcher behavior later starts as stop-on-first-match
- future later stage may evolve subscribe matcher into match-many until timeout or
  buffer full with per-match notifications

## Why Future-Only Won

- simpler for agents
- avoids stale-vs-fresh ambiguity
- avoids duplicate delivery headaches
- makes `read` and `subscribe` deterministic
- replay/history concerns stay out of core RX semantics

## Documentation Phase

Update docs after PLAN 1a implementation to explain internal intent briefly, but
reserve full user-facing RX behavior docs for PLAN 1b/1c when tools switch over.

## Testing Phase

PLAN 1a tests should focus on core internals, not final API semantics.

Cover:
- one pump task per connection
- session shutdown on connection close
- consumer registration/unregistration
- multiple consumers can coexist without direct serial double-read
- pump propagates close/error events correctly

Recommended concrete tests:

- `get_or_create` returns same session for same connection id
- pump starts lazily on first consumer registration
- two consumers both receive future chunk from one port read
- removing session cancels pump
- connection close causes session shutdown even if consumers still registered
- pump exits cleanly on serial close/error without hanging test process

## Review Checklist

Reviewer/agent should verify these before considering PLAN 1a done:

- no new public API churn beyond minimal plumbing
- one obvious place in code now owns RX pumping
- new module boundaries are understandable
- session manager lifecycle tied cleanly to connection lifecycle
- later PLAN 1b/1c migration looks easier, not harder

## Risks

- lifecycle leaks: pump task not stopping cleanly
- duplicated byte delivery due to bad fanout logic
- races around close/replacement
- over-designing PLAN 1a before later plans land

## PLAN 1a - Blockers

These issues should be treated as blockers before declaring PLAN 1a solid enough
for wider migration work:

1. `RxSessionManager::remove()` currently shuts down session state but does not
   await pump-task exit.
   - Session removal should become truly deterministic.
   - Before deeper migration work, removal path should cancel pump and wait for
     `join_pump()` completion.

2. Consumer-drop policy is still implicit.
   - `fanout()` currently removes consumers when `try_send()` fails.
   - This is acceptable for PLAN 1a bootstrap, but PLAN 1b/1c must define
     explicit behavior for slow/full blocking and streaming consumers.

3. PLAN 1a tests prove creation and shutdown mechanics, but later migration
   should not proceed without verifying that no detached pump tasks survive
   session removal under repeated create/remove cycles.

## Locked Decisions

- no retained session history
- read keeps settle behavior
- subscribe later becomes background-only regardless of timeout presence
- future matcher becomes option, not separate RX engine

## Future Notes

- PLAN 1b should discuss final stop notifications for background subscribe
- PLAN 1d should remove the separate `wait_for` tool from public surface once
  replacement API is ready
- later matcher-on-subscribe should eventually support match-many with
  notifications per match, not just stop-on-first-match
- future event-context work should operate in consumer logic, not in the pump
