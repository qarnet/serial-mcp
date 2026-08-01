# Phase 3B Handoff — Write-Through Learning, Conflicts, and Rollback

## Goal

Persist durable changes made to a profile-bound connection, surface partial
hardware/persistence outcomes honestly, retry dirty state on clean close, and
let agents restore a retained profile revision. This completes Phase 3.

Phase 3A (`400835e` + `17e5800`) already provides selection, generated and
transient sessions, optional open overlays, active bindings, identity safety,
and metadata/history exposure.

## Public behavior

1. `reconfigure`, `set_flow_control`, and connection-mode `configure` update the
   bound profile after the live change succeeds.
2. Explicit open overrides on an automatically/explicitly selected profile are
   learned after successful hardware open.
3. Restart/reopen applies learned settings.
4. Per-call read/write/transact/subscribe options and line-control pulses never
   change profile defaults or revision.
5. If live hardware/session mutation succeeds but profile write fails, tool
   result remains successful and reports `failed` persistence; live state stays
   changed, profile cache/file stays old, binding becomes dirty.
6. Next durable mutation or clean close retries dirty effective state.
7. Concurrent/stale profile revision changes produce explicit persistence
   conflict, never silent last-writer overwrite.
8. Rollback restores selected prior defaults as a new monotonic revision;
   active connections remain unchanged and become stale.
9. Reopen after rollback proves restored behavior on live serial traffic.

## In scope

- Per-connection async learning lock.
- Effective connection snapshot independent of handler-local RX manager.
- Revision-CAS learned updates and no-op detection.
- Common persistence result types and additive result fields.
- Dirty/error/stale binding updates.
- Post-open learning for explicit overrides.
- Close snapshot/retry after clean hardware close.
- Active-profile deletion protection.
- New `rollback_profile` tool (tool count 26).
- Behavior-first PTY/HTTP/restart/failure/conflict/rollback tests.

## Out of scope

- No profile-match map in `list_ports` (Phase 4).
- No connection recipes or facade shorthands (Phase 4).
- No generic scripting/expect engine.
- No boot capture or persistent raw capture.
- No auto-learning of transient sessions or weak identity.
- No learning from arbitrary received bytes or heuristic “known good” status.

## Durable versus transient state

Persist full effective `ProfileDefaults` after these successful live changes:

- `reconfigure`: baud, data bits, stop bits, parity, flow control
- `set_flow_control`: flow control
- connection-mode `configure`: framing/parser/protocol, reconnect policy,
  max-buffered and poll defaults currently applied by that tool
- selected-profile explicit open overlays
- clean close when binding is dirty or effective defaults differ

Never persist:

- DTR/RTS, BREAK
- read cursor, flush, subscription lifecycle
- payloads, encoding, match request
- per-call read/write/transact framing/parser/protocol overrides
- health state, reconnect attempts, counters, logs

Do not redesign `ConfigureArgs` into a patch type in this phase. Preserve its
current wire behavior; Phase 4 may simplify that interface separately.

## Profile persistence types

Add shared JsonSchema/serde enums:

```rust
#[serde(rename_all = "snake_case")]
pub enum ProfilePersistenceState {
    Persisted,
    NotNeeded,
    Transient,
    Failed,
}

#[serde(rename_all = "snake_case")]
pub enum ProfilePersistenceOperation {
    OpenOverride,
    Learned,
    CloseSnapshot,
    Rollback,
}

pub struct ProfilePersistenceResult {
    pub state: ProfilePersistenceState,
    pub operation: ProfilePersistenceOperation,
    pub profile_name: Option<String>,
    pub revision: Option<u64>,
    pub error: Option<String>,
}
```

Use schema helpers on unsigned option fields. Add all new types/results to
schema guards.

Add additive `profile` and `profile_persistence` fields where relevant:

- `OpenResult` (persistence for dirty selected-profile overlay, otherwise
  selected/created metadata remains represented by `profile`)
- `ReconfigureResult`
- `SetFlowControlResult`
- connection-mode `ConfigureResult` (`None` profile fields in profile mode is
  acceptable)
- `CloseResult`

`GetStatusResult` and `ConnectionSummary` already expose active binding.

Hardware/session success plus persistence failure returns `is_error != true`;
the result carries changed live values and `state="failed"`. Hardware mutation
failure remains the existing tool error and performs no profile update.

## Binding and serialization

Add to `SerialConnection`:

- one `tokio::sync::Mutex<()>` learning lock
- methods to lock learning and atomically read/update active binding

Extend binding/session result with `stale: bool`. Semantics:

- `dirty`: live effective defaults are not durably represented by binding
  revision
- `stale`: durable profile revision changed externally (CAS conflict or
  rollback) and connection must not overwrite it
- `last_persistence_error`: exact most recent persistence/conflict error

For each durable operation, hold the connection learning lock across:

```text
live mutation
> effective snapshot
> CAS persistence attempt
> binding result update
```

This prevents concurrent requests on one connection from snapshotting each
other's half-applied state.

## Effective snapshot

Create one shared helper that builds full `ProfileDefaults` from
`SerialConnection`:

- current serial parameters
- current framing/parser/protocol defaults
- connection-stored immutable RX buffer size
- current max-buffered and poll defaults
- reconnect policy
- log capacity/enabled
- current connection name

Use it for learning, close retry, and explicit `save_profile`. Do not consult a
handler-local `RxSessionManager`.

Derive/implement `PartialEq` for `ProfileDefaults` and required nested types so
store can detect no-op snapshots without serialization comparison.

## Store CAS update

Add named operation:

```rust
pub async fn update_learned_defaults(
    &self,
    profile_name: String,
    expected_revision: u64,
    defaults: ProfileDefaults,
) -> Result<LearnedUpdate, String>;

pub struct LearnedUpdate {
    pub profile: Profile,
    pub changed: bool,
}
```

Inside existing locked reload-under-lock transaction:

1. Require profile exists.
2. Require current metadata revision equals expected revision.
3. If defaults equal current defaults: return unchanged profile,
   `changed=false`; no revision/history/timestamp update.
4. Otherwise push current selector/defaults to bounded history, preserve
   selector and usage/creation/generated metadata, bump revision, stamp update,
   persist atomically, return resulting profile.

Conflict error must include profile name, expected revision, and actual
revision. Never merge stale full snapshots.

The ProfileStore mutation primitive currently always writes even for no-op.
Allow a no-write/no-cache-change outcome or equivalent so `NotNeeded` truly
does not rewrite the file. Keep cancellation-safe cache publication for writes.

## Learning composition

Add one helper returning `(ProfileSessionResult, ProfilePersistenceResult)`:

- no binding or non-persistent binding: `Transient`, no store call
- persistent clean/no-op: `NotNeeded`
- CAS changed: `Persisted`, update binding revision, clear dirty/stale/error
- store failure/conflict after live success: `Failed`, keep expected revision,
  set dirty, set stale when conflict/missing/newer revision, record error

On later durable operation, retry full effective snapshot when dirty and not
stale. A stale binding must continue reporting conflict rather than overwrite a
newer/rolled-back profile.

## Open override learning

Phase 3A marks selected/explicit bindings dirty when overlay differs. After
binding attach and before returning `OpenResult`, call learning composition when
dirty. Generated bindings are clean because profile was created from effective
settings. Metadata-only `mark_used` failure remains in binding error and does
not close hardware.

Open result must remain success when override persistence fails.

## Reconfigure and flow control

For `reconfigure` and `set_flow_control`:

1. Look up connection and acquire learning lock.
2. Apply existing hardware mutation.
3. If hardware mutation fails, return existing tool error; no store call.
4. Snapshot effective defaults.
5. Attempt bound-profile learning.
6. Return live state plus profile/persistence result.

## Connection configure

Acquire learning lock before applying current connection-mode setters. Preserve
existing set of live-mutated fields and existing ConfigureArgs behavior. After
setters, snapshot full effective defaults and learn. Profile-mode configure is a
direct profile operation: return existing response with active profile fields
`None`.

An external profile-mode configure may bump a profile bound to an active
connection. Next connection learning CAS must fail and set binding stale.

## Close snapshot

`port_ops::close` must obtain and retain `Arc<SerialConnection>` before
`ConnectionManager::close` removes it. Hold learning lock across clean hardware
close and snapshot persistence.

Sequence:

1. retain connection Arc and pre-close binding
2. call existing `ConnectionManager::close`
3. only after successful hardware close, snapshot effective defaults
4. learn only if persistent binding differs/dirty; no-op is `NotNeeded`
5. return captured profile and persistence fields

Persistence failure does not reopen hardware or turn close into tool error.
Server RX/TX/subscription cleanup and resource notification must still run.
Hardware close failure keeps existing operational error semantics.

## Active-profile deletion

Reject `delete_profile` when any same-process open connection binds that
profile. Error must list connection IDs. Cross-process active ownership cannot
be known; later missing-profile CAS protects those processes.

## Rollback tool

Add tool `rollback_profile`; tool count becomes 26.

Input:

```rust
pub struct RollbackProfileArgs {
    pub profile_name: String,
    pub revision: u64,
    pub expected_revision: u64,
}
```

Output:

```rust
pub struct RollbackProfileResult {
    pub profile_name: String,
    pub restored_from_revision: u64,
    pub previous_revision: u64,
    pub revision: u64,
    pub selector: ProfileSelector,
    pub defaults: ProfileDefaults,
    pub metadata: ProfileMetadata,
    pub active_connections_unchanged: usize,
    pub persistence: ProfilePersistenceResult,
}
```

Store transaction:

1. require current revision equals expected revision
2. find requested prior snapshot; missing/evicted revision is tool error
3. push current state into history
4. restore target selector/defaults
5. set new revision `current + 1` (never move revision backward)
6. preserve generated/created/last-used/use-count metadata
7. stamp updated time, cap history at five, write atomically
8. return resulting profile

Rollback never changes live hardware. Iterate same-process connections bound to
profile, mark bindings stale+dirty with explanatory error, count them for
output. Next reopen applies restored defaults.

Update exact tool count and lists in README, Cargo description, server.json,
stdio/HTTP expectations, tool-router schema tests, and AGENTS.md. All tools
retain title/output schema.

## Behavior-first tests

Use real PTY plus injected high-confidence `StaticPortProvider`.

### Learning and restart

- generated profile revision 1 → reconfigure baud → revision 2; close/reopen
  applies baud on live status
- set_flow_control persists and applies on reopen
- connection configure framing persists; reopen and actual framed read proves it
- explicit open override persists immediately and next reopen uses it
- multiple changes create bounded revision history

### Non-learning operations

Capture profile revision/defaults, perform read/write/transact per-call
protocol/framing/match/encoding, DTR/RTS, BREAK, flush, subscribe/unsubscribe;
prove revision/defaults unchanged. Split tests if one giant test obscures
failure source.

### Partial failure and retry

Unix read-only profile directory:

- live reconfigure succeeds
- result `is_error != true`, status shows new baud
- persistence state failed, binding dirty, cache/file old
- restore permissions, clean close retries and persists
- reopen uses new baud

Equivalent focused coverage for connection configure or flow control where
practical; at least one full public path is mandatory.

### CAS/stale behavior

- connection bound at revision N
- another client profile-mode config bumps to N+1
- live reconfigure succeeds but persistence reports conflict, binding stale,
  newer profile remains untouched
- close does not overwrite stale profile

### Rollback

- create revisions through live mutations
- rollback prior baud as new monotonic revision
- active connection status remains unchanged and binding stale
- close cannot overwrite rollback
- close/reopen applies rolled-back baud
- rollback framing revision and prove actual framed traffic after reopen
- wrong expected revision and evicted revision return tool errors without file
  change

### Deletion

- deleting profile bound to open connection returns error with connection ID
- after close, delete succeeds

### Schema/docs

- 26 tools exact
- all new result schemas/title/uint guards
- enum wire names
- README flow explains automatic learning, failed persistence, revision rollback

## Verification

Run focused 3B tests, then:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

## Commit requirements

Inspect status/diff/log, stage only 3B files, commit conventional message, and
return files/behavior/tests/hash/deviations. Do not amend, push, merge, open PR,
add attribution, or begin Phase 4.
