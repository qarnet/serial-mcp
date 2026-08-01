# Phase 3A Handoff — Automatic Profile Session Selection

## Goal

Bind every successful `open`/`open_profile` connection to an observable profile
session. Bare `open` automatically reuses the most recently used
high-confidence profile, creates a durable generated profile for a new
high-confidence device, and uses a non-persistent transient session when the
device cannot be identified safely.

Phase 3B will add durable write-through learning, CAS conflicts, close retries,
and rollback. Do not implement those in this subphase.

## Public behavior

1. First bare open of a uniquely identified USB device creates a generated
   persistent profile and reports it in `OpenResult.profile`.
2. Close/reopen of that device automatically selects the same profile.
3. Multiple profiles for the same device select the unique most-recently-used
   profile; an equal top rank is reported as ambiguity, never vector-order
   selection.
4. Weak identity (no USB serial number) opens with a transient profile session
   and does not write a durable profile.
5. `open_profile` remains explicit selection, applies weak selectors only when
   exactly one live port matches, and marks the profile most recently used.
6. Explicit open fields override selected profile defaults. Omitted fields come
   from selected profile, then built-in defaults.
7. `get_status` and `list_connections` expose the same active binding across
   separate HTTP sessions.
8. `profile_mode="none"` disables automatic selection/creation for deliberate
   troubleshooting and returns an observable disabled/transient binding.

## In scope

- Optional open-overlay fields and 115200/8-N-1 built-in fallback.
- Process-wide injectable port enumeration used consistently by tools/resources.
- Identity confidence and automatic eligibility.
- Fresh-store resolution, generated name allocation, and usage metadata updates.
- Profile binding stored on `SerialConnection` so HTTP sessions share it.
- Profile metadata/revisions exposed by `list_profiles`.
- Additive profile session fields in open/status/connection summaries.
- Behavior tests through public MCP plus real PTY traffic with synthetic USB
  enumeration.

## Out of scope

- No profile write-through from `reconfigure`, `set_flow_control`, connection
  `configure`, or close yet.
- No rollback tool yet.
- No profile-match map in `list_ports` yet (Phase 4).
- No connection recipes or shorthand facade.
- No boot capture or persistent raw capture.
- No profile application for weak identity unless caller uses `open_profile`.
- No test-only fields in public tool schemas.

## Port provider

Add a production abstraction rather than a test-only `OpenArgs` escape hatch:

```rust
pub trait PortProvider: Send + Sync {
    fn list_available(&self) -> crate::error::Result<Vec<PortInfo>>;
}

pub struct SystemPortProvider;
```

`SystemPortProvider` delegates to current OS enumeration. Add
`Arc<dyn PortProvider>` to `SerialHandlerOptions`/builder and share it through
each handler. Use it for:

- `list_ports`
- bare `open` identity capture
- `open_profile` port matching
- `serial://ports`
- resource port counts

Tests inject a static provider whose `PortInfo.name` points at a real PTY slave
while identity fields describe a synthetic USB device. This tests the full
public `open` path and actual serial I/O without hardware.

## Identity rules

Public enum:

```rust
#[serde(rename_all = "snake_case")]
pub enum IdentityConfidence { High, Medium, Low, None }
```

Automatic persistent reuse is allowed only for `High`:

- transport is USB
- VID exists
- PID exists
- non-empty serial number exists
- interface participates when available

Canonical generated selector includes only transport, VID, PID, serial number,
and optional interface. Do not include path, description, manufacturer,
product, or current formatted hardware ID in generated high-confidence
selectors.

USB VID/PID without serial is `Medium` and auto-ineligible. Other useful
identity is `Low`; path-only/unknown is `None`. Medium/low/none sessions are
transient and never persisted automatically.

If multiple currently enumerated ports share the same high fingerprint,
downgrade automatic resolution to ambiguity/transient. Never apply settings to
an indistinguishable physical device.

Manual `open_profile` keeps existing selector semantics, including weak
selectors, because the caller made an explicit choice. It must now fail when
its selector matches more than one live port instead of choosing the first.

## Automatic resolution and ranking

Add a ProfileStore fresh-read transaction that acquires the file lock, reloads
from disk, updates cache, and resolves candidates. Do not rely only on cache
because another process may have changed profiles.

For bare `open` in auto mode:

1. Enumerate target `PortInfo` and all live ports.
2. Compute identity confidence/fingerprint.
3. If not high or fingerprint duplicated live: transient session.
4. Find profiles whose selectors match target and contain the same high
   identity fields.
5. No candidates: open with explicit/built-in settings, then atomically create
   a generated profile after hardware open succeeds.
6. One candidate: select it.
7. Multiple: choose unique maximum `last_used_at_ms`; `None` sorts oldest.
8. Equal top timestamps: transient ambiguous session with candidate names.

On successful persistent selection, atomically `mark_used`:

- increment `use_count`
- update `last_used_at_ms`
- do not bump configuration revision or add history
- ensure timestamp is monotonically greater than any profile's existing
  `last_used_at_ms` (`max(now, max_existing + 1)`) to avoid same-millisecond
  ranking ties created by this server
- return effective profile from same transaction

Failed hardware open does not mark usage or create profile.

## Generated names

Allocate atomically under store lock:

1. label from product, else manufacturer, else `usb-{vid:04x}-{pid:04x}`
2. lowercase ASCII
3. each non-alphanumeric run becomes `-`
4. trim `-`, cap label at 32 chars, fallback `serial-device`
5. base `auto-{label}`
6. never overwrite any existing profile
7. choose first free suffix: base, `-2`, `-3`, ...

Generated profile metadata: `generated=true`, revision 1, use count 1,
created/updated/last-used timestamp set. Its defaults equal effective live open
settings.

If generated persistence fails after hardware open, keep connection open and
bind a transient session carrying the error. Do not report open failure or
pretend profile persisted.

## Open overlay and precedence

Current scalar `OpenArgs` fields lose presence information. Change
default-bearing fields to `Option<T>` with `#[serde(default)]`:

- baud rate
- data bits, stop bits, parity, flow control
- log capacity/enabled
- reconnect policy
- RX buffer size
- max buffered bytes
- poll interval

Framing/parser/protocol are already optional. Existing explicit JSON remains
valid; omitted baud now resolves to 115200.

Add:

```rust
#[serde(rename_all = "snake_case")]
pub enum ProfileMode { Auto, None }
```

`OpenArgs.profile_mode: Option<ProfileMode>` defaults to Auto. Do not add a
profile name to `open`; `open_profile` remains explicit named selection.

Per-field precedence:

```text
explicit open field > selected profile default > built-in default
```

Create a private resolved/effective open-settings struct with concrete values;
`ConnectionConfig` remains concrete. Do not scatter `unwrap_or` precedence
across tool logic.

`OpenProfileArgs` log capacity/enabled/RX buffer overrides must become optional
so omission can use profile defaults. Existing explicit values remain valid.

Store effective `rx_buffer_size` on `SerialConnection`; profile snapshots must
not depend on whichever handler-local `RxSessionManager` receives a later
request. While touching `from_io_with_config`, remove the duplicate LogBuffer
construction that currently calls `opened()` on one buffer and stores another.

## Active binding and result shape

Store binding directly on `SerialConnection` behind the repository's standard
mutex convention:

```rust
pub struct ActiveProfileBinding {
    pub profile_name: String,
    pub source: ProfileSelectionSource,
    pub confidence: IdentityConfidence,
    pub persistent: bool,
    pub generated: bool,
    pub revision: Option<u64>,
    pub dirty: bool,
    pub candidates: Vec<String>,
    pub last_persistence_error: Option<String>,
}
```

Sources: `automatic`, `explicit`, `generated`, `transient`, `disabled`.

Use the same serializable/JsonSchema shape (or a lossless result conversion) as
`ProfileSessionResult`. Every connection has a binding after successful public
open. Connections inserted directly by low-level tests may have `None`.

Add optional/additive profile fields to:

- `OpenResult`
- `GetStatusResult`
- `ConnectionSummary` (therefore `list_connections`)

Expose `ProfileMetadata` and `Vec<ProfileRevision>` in `ProfileSummary` so
agents can understand selection and future rollback revisions. Do not add a
rollback tool in 3A.

An automatically selected profile with explicit overrides may be marked
`dirty=true`; 3B will persist effective changes. Generated profile defaults
already equal effective settings and start clean.

## Tool flow

Refactor shared open plumbing rather than recursively calling auto-open from
`open_profile`:

- bare `open`: resolve auto/transient/disabled, merge settings, open hardware,
  then create/mark profile and attach binding
- `open_profile`: resolve named profile and exactly one port, merge optional
  overrides, open hardware, mark used, attach explicit binding
- start RX session only after hardware open as today
- preserve allowlist check before `ConnectionManager::open`
- preserve resource-list notifications

Persistent metadata failure after hardware success returns successful open with
`last_persistence_error`; never close a working port merely because profile
metadata failed.

## ProfileStore additions

Named methods, no tool-owned cache mutation:

- fresh automatic resolution for one port/all live ports
- atomic generated create/name allocation
- `mark_used(name) -> Profile`

Keep pure helpers for confidence, canonical selector, candidate ranking, and
name normalization directly unit/property testable.

## Behavior-first tests

### Public MCP + real PTY with injected PortProvider

1. First high-confidence bare open creates generated persistent profile;
   `list_profiles`, open result, status, and list-connections agree.
2. Close/reopen automatically selects same profile and increments usage.
3. A different serial number with same VID/PID gets a different profile.
4. Two live ports with duplicate high fingerprint produce transient ambiguity,
   not profile application.
5. Weak PTY identity produces transient session and leaves profile store empty.
6. `profile_mode="none"` suppresses profile creation/selection.
7. Explicit open field overrides selected profile for live connection and marks
   binding dirty.
8. Separate HTTP client observes active binding and generated profile.
9. `open_profile` with two matching ports returns tool error; exactly one works
   and becomes last-used winner for later bare open.
10. Equal top-ranked profile timestamps produce observable ambiguity.
11. Per-call read/write/transact options do not alter usage/revision/defaults.

Exercise actual serial traffic in at least one generated and one automatically
selected session so success is not merely result-field wiring.

### Pure behavior tests

- confidence tiers
- canonical high selector survives path change
- ranking unique winner/equal tie
- generated name normalization, truncation, collisions
- concurrent generated allocation never overwrites
- usage updates do not bump revision/history
- optional open field schema no longer requires baud/default-bearing fields

### Existing compatibility

- Existing explicit open JSON calls continue to work.
- Existing `open_profile` tests pass after optional override update.
- Tool schema title/output guards and uint-format guards include new types.

## Expected files

- `src/serial.rs`
- `src/profiles.rs`
- `src/profile_store.rs`
- `src/server.rs`
- `src/tools/types.rs`
- `src/tools/helpers.rs`
- `src/tools/port_ops.rs`
- test common server/PTY provider helpers
- `tests/http_integration.rs`
- `tests/serial_pty.rs` as useful
- `tests/proptest.rs`
- `README.md`, `AGENTS.md`, schema/doc drift tests
- this handoff

## Verification

Run focused new profile-session tests, then:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

## Commit requirements

Inspect status/diff/log, stage only 3A files, commit with a concise conventional
message, and return files/behavior/tests/hash/deviations. Do not amend, push,
merge, open a PR, add attribution, or implement 3B.
