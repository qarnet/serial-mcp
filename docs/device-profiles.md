# Device Profiles and Automatic Sessions

Every successful `open`/`open_profile` binds the connection to an observable
profile session reported in the open result, `get_status`, and
`list_connections` (`profile`: name, selection source, confidence, persistent,
generated, revision, dirty, candidates, last persistence error). This guide
covers how profiles are matched, created, learned, and recovered.

## The short normal workflow

1. `list_ports()` — inspect `profile_matches` to see what a bare `open` would
   reuse for each live port.
2. `open(port=...)` — bare open with just the port; baud defaults to
   115200/8-N-1, and the server reuses the most recently used high-confidence
   profile for a known device or creates a durable generated profile for a new
   one. The result carries the `profile` binding.
3. `transact(...)` / `read(...)` / `write(...)` — talk to the device.
4. Inspect `profile` / `profile_persistence` after durable changes to confirm
   learning.
5. Use `open_profile` only for explicit choice or weak identity, and
   `rollback_profile` to recover from a bad learned change.

## `list_ports` previews profile selection

The `list_ports` result carries `profile_matches` parallel to `ports` (same
order, always present). Each entry reports `confidence` and `outcome`:

| Outcome | Meaning |
|---|---|
| `selected` | A bare `open` reuses `selected_profile` |
| `ambiguous` | Equal-ranked profiles; pick one via `open_profile` |
| `duplicate` | Another live port shares this device's fingerprint — never auto-selected |
| `ineligible` | Weak identity with explicitly matching candidates |
| `none` | A bare open gets no existing profile: high unique identity creates a generated profile; weak/path-only identity starts a transient session |

The preview is **read-only**: nothing is marked used and no file is written.
The `serial://ports` resource carries the same map.

## Identity: high, weak, duplicate

Profile matching depends on device identity:

- **High** — USB transport + VID + PID + non-empty serial number (plus
  interface when available). Automatic reuse only happens for high identity,
  and only when the high fingerprint is unique among live ports.
- **Weak** — no USB serial number, non-USB, or path-only identity. Automatic
  (bare-open) selection treats weak identity as a non-persistent **transient**
  session and never writes a durable profile for it. Explicit `open_profile`
  can still deliberately bind a matching persistent profile to a weak-identity
  port — weak identity only limits the automatic path, not explicit choice.
- **Duplicate** — another live port shares the same high fingerprint.
  Duplicate live fingerprints degrade to transient for automatic opens:
  settings are never applied to an indistinguishable device.

## Generated and reused profiles

- **First bare `open` of a uniquely identified USB device** creates a durable
  generated profile (name `auto-{label}`) whose defaults equal the effective
  open settings.
- **Close/reopen automatically selects the most recently used profile** for
  the same device. Multiple profiles for one device resolve to the unique
  newest `last_used_at_ms`; an equal top rank is reported as ambiguity
  (`candidates`), never vector-order selection, and the session stays
  transient.
- **`profile_mode="none"`** disables automatic selection/creation for
  deliberate troubleshooting.

## Overlay precedence

Explicit `open` fields override the selected profile's defaults (baud, data
bits, stop bits, parity, flow control, log, reconnect policy,
framing/parser/protocol, ring size, read defaults). Omitted fields come from
the profile, then built-in 115200/8-N-1 defaults. An open that differs from the
selected profile's defaults is `dirty` and triggers write-through learning.

## Write-through learning

Durable changes persist back through the bound profile:

- a dirty open override is persisted right after the successful hardware open;
- durable live changes (`reconfigure`, `set_flow_control`, connection-mode
  `configure`) persist the full effective defaults through the bound profile
  after the live change succeeds;
- clean close is a safety net: a dirty or differing binding is retried on
  close.

The result carries `profile_persistence` (`persisted` / `not_needed` /
`transient` / `failed`) plus the updated `profile` binding. Reopen/restart
applies the learned settings.

**Never persisted:** DTR/RTS, BREAK, read cursor, flush, payloads/encoding/
match, per-call read/write/transact framing/parser/protocol overrides.

## Partial failure is honest

If the live change succeeds but the profile write fails, the tool result stays
successful, `state` is `failed` with the error, the binding turns `dirty`, and
the next durable mutation or clean close retries. Transient line control
(DTR/RTS, BREAK), per-call read/write/transact framing, payloads, and cursors
never touch profile defaults or revisions.

## Revision CAS and stale bindings

Persistence is guarded by the bound revision. If another client bumps or rolls
back the profile, the next learning attempt reports an explicit conflict
(`failed`, binding `stale`) instead of silently overwriting the newer profile.
A stale binding keeps reporting the conflict until reopened.

## Rollback

`rollback_profile` restores any retained prior revision (see `list_profiles`
`revisions`, newest five snapshots) as a new monotonic revision. Active
connections bound to the profile stay on their live state and become stale;
reopen applies the restored defaults. A wrong `expected_revision` or an evicted
revision is a tool error that leaves the file unchanged.

## Deletion guard

`delete_profile` is refused while a same-process open connection binds the
profile — the error lists the connection IDs.

## Explicit selection and promotion

- **`open_profile`** remains explicit selection: it requires exactly one
  matching live port (multiple matches are a tool error) and marks the profile
  most recently used. `list_profiles` exposes each profile's metadata and
  bounded revision history. Explicit bindings report the matched port's own
  identity confidence.
- **`save_profile`** on a connection bound to an auto-generated profile
  deliberately promotes it to a user-owned profile (`generated=false`) under
  the new name.

## Storage

Profiles are persisted to a single TOML store shared by every session of the
server process. The default location follows your OS user config directory
(e.g. `~/.config/serial-mcp/profiles.toml`), so device knowledge follows you
across repositories; the store's parent directory is created as needed. Use
`--profiles-path <path>` for an isolated, project-specific store. Startup fails
only when the OS config directory cannot be resolved or is unavailable, or when
the store/path is invalid or unwritable — it never silently falls back to the
current directory.
