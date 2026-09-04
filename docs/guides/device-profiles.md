# Device profiles and automatic sessions

Every successful `open` or `open_profile` binds its connection to a profile
session. The session appears under `profile` in the open result, `get_status`,
and `list_connections`.

The profile includes the name, selection source, confidence, persistence,
generated flag, revision, dirty state, candidates, and last persistence error.
This guide explains profile matching, creation, learning, and recovery.

## Normal workflow

1. Call `list_ports()`. Inspect `profile_matches` to see which profile a bare `open` would reuse for each live port.
2. Call `open(port=...)` with only the port. Baud defaults to 115200/8-N-1.
   The server reuses the most recently used high-confidence profile for a known
   device. It creates a durable generated profile for a new device. The result
   includes the `profile` binding.
3. Use `transact(...)`, `read(...)`, or `write(...)` to communicate with the device.
4. After durable changes, inspect `profile` and `profile_persistence`. This confirms whether learning succeeded.
5. Use `open_profile` for an explicit choice or weak identity. Use `rollback_profile` to recover from a bad learned change.

## `list_ports` previews profile selection

The `list_ports` result contains `profile_matches` parallel to `ports`, in the
same order and always present. Each entry reports `confidence` and `outcome`:

| Outcome | Meaning |
|---|---|
| `selected` | A bare `open` reuses `selected_profile` |
| `ambiguous` | Equal-ranked profiles; choose one with `open_profile` |
| `duplicate` | Another live port shares this device's fingerprint, so it is never auto-selected |
| `ineligible` | Weak identity with explicitly matching candidates |
| `none` | A bare open has no existing profile. High unique identity creates a generated profile; weak or path-only identity starts a transient session |

The preview is read-only. It does not mark profiles used or write the profile
store. The `serial://ports` resource carries the same map.

## Identity with high, weak, and duplicate outcomes

Profile matching uses device identity:

- High identity combines USB transport, VID, PID, and a non-empty serial number.
  The interface is included when available. Automatic reuse requires high
  identity and a unique high fingerprint among live ports.
- Weak identity means no USB serial number, non-USB transport, or path-only
  identity. Bare-open selection treats weak identity as a non-persistent
  transient session. It never writes a durable profile. Explicit `open_profile`
  can still deliberately bind a matching persistent profile to a weak-identity
  port. Weak identity limits automatic selection, not explicit choice.
- Duplicate identity means another live port has the same high fingerprint.
  Duplicate live fingerprints become transient for automatic opens. Settings are
  not applied to an indistinguishable device.

## Generated and reused profiles

- The first bare `open` of a uniquely identified USB device creates a durable
  generated profile named `auto-{label}`. Its defaults equal the effective open
  settings.
- Close and reopen automatically selects the most recently used profile for the
  same device. When several profiles match, the store requires one unique newest
  `last_used_at_ms`. An equal top rank is reported as ambiguity with
  `candidates`. Selection never depends on vector order. The session stays
  transient when the result is ambiguous.
- `profile_mode="none"` disables automatic selection and creation for troubleshooting.

## Overlay precedence

Explicit `open` fields override the selected profile's defaults. These fields
include baud, data bits, stop bits, parity, flow control, log, reconnect policy,
framing/parser/protocol, ring size, and read defaults. Omitted fields come from
the profile. Remaining fields use the built-in 115200/8-N-1 defaults. If the
resulting settings differ from the selected profile's defaults, the open is
`dirty` and triggers write-through learning.

## Write-through learning

Durable changes are written back through the bound profile:

- A dirty open override is persisted immediately after the hardware opens
  successfully.
- Durable live changes persist the full effective defaults after the live change
  succeeds. These changes include `reconfigure`, `set_flow_control`, and
  connection-mode `configure`.
- A clean close retries a dirty or differing binding.

Results include `profile_persistence` and the updated `profile` binding.
`profile_persistence` is `persisted`, `not_needed`, `transient`, or `failed`.
Reopening or restarting the server applies learned settings.

The following values are never persisted:

- DTR/RTS and BREAK
- the read cursor and flush operations
- payloads, encoding, and match settings
- per-call read, write, and transact framing, parser, and protocol overrides

## Partial failures

If a live change succeeds but the profile write fails, the tool result remains
successful. Its `state` is `failed` and carries the error. The binding becomes
`dirty`. The next durable mutation or clean close retries the write.

Transient line control includes DTR/RTS and BREAK. Per-call read/write/transact
framing, payloads, and cursors also never change profile defaults or revisions.

## Revision CAS and stale bindings

Persistence uses the bound revision. If another client updates or rolls back the
profile, the next learning attempt reports an explicit conflict. It returns
`failed` with a stale binding instead of overwriting the newer profile. A stale
binding keeps reporting the conflict until the connection is reopened.

## Rollback

`rollback_profile` restores a retained prior revision from `list_profiles`
`revisions`, which keeps the newest five snapshots, as a new monotonic revision.
Active connections bound to the profile keep their live state and become stale.
Reopening applies the restored defaults. A wrong `expected_revision` or an
evicted revision is a tool error, and the file remains unchanged.

## Deletion guard

`delete_profile` is refused while a same-process open connection is bound to the
profile. The error lists the connection IDs.

## Explicit selection and promotion

- `open_profile` is the explicit selection path. It requires exactly one
  matching live port. Multiple matches are a tool error.
  It marks the profile most recently used. `list_profiles` exposes profile
  metadata and bounded revision history. Explicit bindings report the matched
  port's own identity confidence.
- `save_profile` on a connection bound to an auto-generated profile promotes
  that connection to a user-owned profile with `generated=false`. The new name
  becomes the profile name.

## Storage

Profiles are stored in one TOML file shared by every session in the server
process. The default path follows the OS user config directory. For example, it
can be `~/.config/serial-mcp/profiles.toml`. Device knowledge is then available
across repositories. The store creates its parent directory as needed. Use
`--profiles-path <path>` for an isolated, project-specific store.

Startup fails when the OS config directory cannot be resolved or is unavailable.
It also fails when the store or path is invalid or unwritable. It never silently
falls back to the current directory.
