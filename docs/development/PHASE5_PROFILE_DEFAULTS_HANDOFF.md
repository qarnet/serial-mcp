# Phase 5 Handoff — Profile defaults for framing/parser/protocol

Source plan: `docs/development/FRAME_PIPELINE_PLAN.md` (Phase 5). Phases
1-4b are complete: `tx_framing`/`rx_framing`/`rx_parser`/`protocol` all exist
as per-call options; `protocol` expands via presets; explicit call fields win
over preset components.
Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Let saved profiles carry default `tx_framing`, `rx_framing`, `rx_parser`, and
`protocol`. When a connection is opened via `open_profile`, these defaults
are stored on the `SerialConnection`. Subsequent `write`/`read`/`subscribe`
calls consult the connection's stored defaults when the agent omits a field.
This avoids repeating framing options on every call for a device whose
protocol is known.

## Decisions already made (do not re-litigate)

- **Defaults stored on the connection.** `open_profile` reads the profile's
  framing defaults and stows them on the `SerialConnection` at open time.
  `read`/`write`/`subscribe` look up the connection (already in scope by
  connection_id) and apply its defaults for any field the call leaves
  `None`. No per-call profile registry lookup.
- **ProfileDefaults gains all four fields:** `tx_framing`, `rx_framing`,
  `rx_parser`, `protocol` (all `Option<...>`, all `#[serde(default)]`).
- **Precedence: explicit call field > connection default (from profile) >
  preset expansion (if protocol default set) > built-in `None`.** Same
  explicit-wins rule as Phase 4b, extended one layer. Concretely, per field:
  - If the call sets the field explicitly → use it.
  - Else if the connection has a stored default for that field → use it.
  - Else `None` (no default).
  `protocol` is special: when a connection default `protocol` is present
  AND the call sets no explicit `tx_framing`/`rx_framing`/`rx_parser`, the
  stored `protocol` expands via the Phase 4b preset functions to fill those
  gaps — exactly like a call-time `protocol`, just sourced from the
  connection.

## In scope

### A. `ProfileDefaults` gains four optional fields

`src/profiles.rs`:

```rust
pub struct ProfileDefaults {
    // ... existing serial-param fields ...
    /// Default TX framing applied when `write` omits `tx_framing`.
    /// Filled by the `protocol` default if present (preset expansion).
    #[serde(default)]
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    /// Default RX framing applied when `read`/`subscribe` omit `rx_framing`.
    #[serde(default)]
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    /// Default RX parser applied when `read`/`subscribe` omit `rx_parser`.
    #[serde(default)]
    pub rx_parser: Option<crate::framing::ParserConfig>,
    /// Default protocol preset. When set, expands to fill any of the three
    /// framing/parser fields that are themselves `None`.
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
}
```

Update `Default for ProfileDefaults` to include all four as `None`. Update
the `decoder: Option<String>` reserved field comment if it implies framing
(it does not — `decoder` is unrelated and stays reserved). The new fields
supersede any "future decoder" intent for framing; leave `decoder` as-is
(unused reserved slot).

### B. `ConnectionConfig` + `SerialConnection` carry the defaults

`src/serial.rs`:

- Add four fields to `ConnectionConfig`:
  ```rust
  #[serde(default)]
  pub tx_framing: Option<crate::framing::TxFramingConfig>,
  #[serde(default)]
  pub rx_framing: Option<crate::framing::RxFramingConfig>,
  #[serde(default)]
  pub rx_parser: Option<crate::framing::ParserConfig>,
  #[serde(default)]
  pub protocol: Option<crate::framing::ProtocolPreset>,
  ```
- Add the same four fields to `SerialConnection` (store as plain
  `Option<...>` — they are read-only after open; no mutex needed).
- `SerialConnection::from_io_with_config` (or wherever `Self { ... }` is
  built, ~line 657): copy the four fields from `config` into the connection.
- `build_config` (~line 822): copy the four fields from `self` back into the
  rebuilt `ConnectionConfig` (used by reconnect — preserve framing defaults
  across reconnects).
- Add accessor methods:
  ```rust
  pub fn tx_framing_default(&self) -> Option<&crate::framing::TxFramingConfig>;
  pub fn rx_framing_default(&self) -> Option<&crate::framing::RxFramingConfig>;
  pub fn rx_parser_default(&self) -> Option<&crate::framing::ParserConfig>;
  pub fn protocol_default(&self) -> Option<crate::framing::ProtocolPreset>;
  ```
  (`ProtocolPreset` is `Copy`, so return by value; the others return `&`.)

### C. `open_profile` seeds the defaults from the profile

`src/tools/port_ops.rs::open_profile`:

The `OpenArgs { ... }` literal built for `open()` (~line 246) does NOT carry
framing defaults (those live on `ConnectionConfig`, not `OpenArgs`). Trace
how `open()` converts `OpenArgs` → `ConnectionConfig` and thread the four
profile.defaults fields through to `ConnectionConfig`. Likely:
- `open()` builds `ConnectionConfig` and calls `SerialConnection::open`.
- Add the four fields to the `ConnectionConfig` literal inside `open()`,
  sourced from a new parameter or from `OpenArgs` extended with the four
  optional fields.
- Cleanest: extend `OpenArgs` with the four optional fields too (so `open`
  and `open_profile` both can pass them), OR add a separate
  `open_with_defaults` path. Prefer extending `OpenArgs` with four
  `#[serde(default)]` optional framing fields so a bare `open` call can also
  set framing defaults that stick on the connection. Document that on plain
  `open` these become connection defaults for later read/write/subscribe.

Set the four `ConnectionConfig` fields from `profile.defaults.*` inside
`open_profile` (and from `args.*` inside plain `open`).

### D. `read`/`write`/`subscribe` apply connection defaults

The override resolution in `io_ops::write`, `io_ops::read`,
`stream_ops::subscribe` (added in Phase 4b) currently resolves only
call-time `protocol` vs explicit fields. Extend each to consult the
connection's stored defaults as the new baseline layer.

New precedence per field (write uses `tx_framing`; read/subscribe use
`rx_framing` + `rx_parser`; all three use `protocol`):

```rust
// For tx_framing (write):
let tx_framing = if let Some(explicit) = args.tx_framing {
    Some(explicit)                                      // call explicit
} else if let Some(p) = args.protocol {
    // call-time protocol preset: explicit-None gap filled by preset
    Some(crate::framing::preset_tx_framing(p))
} else if let Some(default) = conn.tx_framing_default() {
    Some(default.clone())                               // connection default
} else if let Some(p) = conn.protocol_default() {
    Some(crate::framing::preset_tx_framing(p))          // connection protocol preset
} else {
    None
};
```

Mirror this shape for `rx_framing` and `rx_parser` in `read` and
`subscribe`. Note the four-layer fallback: explicit call > call protocol
preset > connection default > connection protocol preset. The two
protocol-preset layers use the same `preset_*` functions from Phase 4b.

`write` already has `conn: &Arc<SerialConnection>` available (via
`lookup_connection`). `read` and `subscribe` likewise — verify the
connection handle is in scope at the resolution point; if not, fetch it
once and reuse.

### E. `save_profile` snapshots connection defaults

`src/tools/port_ops.rs::save_profile` (~line 275): the
`ProfileDefaults { ... }` literal currently snapshots serial params only.
Add the four framing defaults from the connection:
```rust
tx_framing: conn.tx_framing_default().cloned(),
rx_framing: conn.rx_framing_default().cloned(),
rx_parser: conn.rx_parser_default().cloned(),
protocol: conn.protocol_default(),
```
This makes a saved-then-reopened profile reproduce the framing behavior.
A connection opened via plain `open` (no defaults) saves `None` for all
four — clean.

### F. `get_status` exposes defaults (optional)

`GetStatusResult` in `src/tools/types.rs` — consider adding the four
defaults so agents can inspect what a connection will apply. OPTIONAL this
phase; if added, update the `get_status` builder in `src/tools/control_ops.rs`
and add `check_schema!` coverage if any new unsigned fields appear (none
expected — all four are `Option<Enum>`/`Option<Struct>`). Your call: include
for agent visibility, or defer. If deferring, note in the return summary.

### G. Profile config-file format + examples

`ProfileDefaults` is `Serialize`/`Deserialize` — the four new fields
serialize as nested objects/strings under `[profile.defaults]` (or the JSON
equivalent). No code change needed beyond the struct fields, but verify a
sample TOML/JSON profile round-trips with the new fields. Add a test (see
test plan).

### H. Schema regression

- `ProfileDefaults` derives `JsonSchema` and has no new unsigned fields —
  verify the existing `profile_has_no_uint_formats` check in
  `src/serial.rs` `mod schema` still passes (it scans `Profile`/`ProfileSelector`,
  extend it to also scan `ProfileDefaults` if not already covered — check
  the list). Add `check_schema!(profile_defaults_has_no_uint_formats, ProfileDefaults);`
  if not present.
- `ConnectionConfig` does NOT derive `JsonSchema` (internal) — verify; if it
  does, add coverage for the four new `Option<...>` fields (no unsigned
  fields, but defense-in-depth).
- `OpenArgs` derives `JsonSchema` — if you extend it with the four optional
  fields, add `check_schema!(open_args_has_no_uint_formats, OpenArgs);` (or
  verify it's already covered). No unsigned fields expected.

### I. Tool descriptions

`src/server.rs`: update `open`, `open_profile`, `write`, `read`,
`subscribe`, `save_profile` descriptions to mention that profile/connection
defaults apply when call fields are omitted, with explicit-wins precedence.

## Out of scope

- Per-call profile registry lookup (defaults live on the connection).
- Auto-discovery of a profile by port identity at `open` time (only
  `open_profile` applies defaults; plain `open` with the new optional
  framing fields is the only way to seed defaults without a named profile).
- Applying defaults from a profile to an already-open connection
  (reconfigure-framing tool). Out of scope.
- Changing the `protocol` preset set. Only `at_command`.
- Live reconfiguration of framing defaults after open (no `set_framing`
  tool). Defaults are set at open and read-only thereafter.
- `get_status` framing-default exposure (optional this phase — see F).
- Profile defaults for `match`, `encoding`, `timeout_ms`, etc. Only framing/
  parser/protocol.

## Relevant files and current behavior

- `src/profiles.rs`:
  - `ProfileDefaults` (line ~52): add four fields. Update `Default` impl
    (line ~95). No new unsigned fields.
  - Existing `decoder: Option<String>` reserved field stays — do not
    repurpose.
- `src/serial.rs`:
  - `ConnectionConfig` (line ~196): add four fields.
  - `SerialConnection` (line ~573): add four fields.
  - `from_io_with_config` / `Self { ... }` build (line ~657): copy fields.
  - `build_config` (line ~822): copy fields back for reconnect.
  - Add four accessor methods.
  - `mod schema` `check_schema!` list: ensure `ProfileDefaults` covered.
- `src/tools/port_ops.rs`:
  - `open` (~line 1-220 area): if `OpenArgs` extended, thread fields into
    `ConnectionConfig`.
  - `open_profile` (line ~225): seed `ConnectionConfig` framing fields from
    `profile.defaults`.
  - `save_profile` (line ~275): snapshot the four connection defaults.
- `src/tools/io_ops.rs`:
  - `write` (line ~19): extend the `tx_framing` resolution to consult
    `conn.tx_framing_default()` and `conn.protocol_default()`.
  - `read` (line ~52): extend `rx_framing` + `rx_parser` resolution to
    consult `conn.*_default()` and `conn.protocol_default()`.
- `src/tools/stream_ops.rs`:
  - `subscribe` (line ~64): same extension as `read`.
- `src/tools/types.rs`:
  - `OpenArgs` (line ~13): optionally extend with four `#[serde(default)]`
    optional framing fields.
  - `GetStatusResult` (line ~357): optionally add four defaults fields.
- `src/server.rs`: tool descriptions.
- `src/serial.rs` `mod schema`: extend `check_schema!` list if needed.
- `tests/proptest.rs`: existing profile/args roundtrip tests — add the new
  fields as `None` to struct literals; add a profile-with-framing roundtrip.
- `tests/native_sim_validation.rs`: optional e2e test (see test plan).
- `tests/serial_pty.rs`: optional PTY test (see test plan).

## Expected API / UX shape

Profile (TOML, illustrative):
```toml
[[profile]]
name = "my-modem"
[profile.selector]
vid = 0x1915
pid = 0xc6db
[profile.defaults]
baud_rate = 115200
protocol = { type = "at_command" }
```

`open_profile` with this profile stores `protocol = AtCommand` on the
connection. Subsequent `write` with `data: "AT+CGMI"` and NO `tx_framing` →
preset expands → CR appended. `read` with NO `rx_framing`/`rx_parser` →
preset expands → line auto + AT parser.

Override at call time:
```json
{ "connection_id": "abc", "data": "AT+CGMI", "tx_framing": { "type": "line", "ending": "crlf" } }
```
Explicit `tx_framing` wins over the connection's `protocol` default → CRLF
instead of CR.

A connection opened via plain `open` (no profile) has all four defaults
`None` → call fields behave exactly as before (no regression).

## Test plan

1. **ProfileDefaults roundtrip with framing fields.** Unit test in
   `src/profiles.rs`: build a `ProfileDefaults` with `protocol: Some(AtCommand)`
   and the three primitive fields `None`; serialize/deserialize; assert
   fields preserved.
2. **ProfileDefaults roundtrip with explicit primitives.** Same, but set
   `rx_framing: Some(line lf)` and `protocol: None`; assert preserved.
3. **ConnectionConfig roundtrip with defaults.** Unit test in
   `src/serial.rs`: build a `ConnectionConfig` with the four framing
   defaults, round-trip via serde, assert preserved.
4. **open_profile stores defaults on connection.** In
   `tests/native_sim_validation.rs` (or PTY): open via a profile with
   `protocol: at_command`; assert `conn.protocol_default()` returns
   `Some(AtCommand)` via `get_status` (if F implemented) or via a
   connection-status call.
5. **write applies connection default.** e2e: open via profile with
   `protocol: at_command`; `write` with `data: "ping"` and NO `tx_framing`;
   assert firmware responds (preset CR was applied from the connection
   default, not the call). Mirror `native_write_protocol_preset_appends_cr`
   but the `protocol` field is NOT set on the `write` call — only on the
   profile.
6. **read applies connection default.** e2e: same connection; `read` with
   NO `rx_framing`/`rx_parser`/`protocol`; assert `pong` frame parsed as
   `at_command` (preset rx_framing + rx_parser came from the connection
   default).
7. **explicit call field beats connection default.** e2e: open via profile
   with `protocol: at_command`; `read` with explicit
   `rx_framing: { line, lf }` and NO `protocol`; assert frame `data` ends
   `\r` (explicit `lf` beat the connection's `auto` default). Proves the
   four-layer precedence.
8. **save_profile snapshots defaults.** Open via a profile with
   `protocol: at_command`; `save_profile` to a new name; reload profiles;
   assert the new profile's `defaults.protocol == Some(AtCommand)`.
9. **plain open has no framing defaults.** Open via plain `open` (no
   profile); `read`/`write` with no framing fields behave as before (no
   preset expansion, no error). Regression guard.
10. **schema regression.** `check_schema!(profile_defaults_has_no_uint_formats,
    ProfileDefaults)` (if added) passes. `OpenArgs` scan passes if extended.
11. **proptest roundtrip.** Add `protocol: None` / `tx_framing: None` /
    `rx_framing: None` / `rx_parser: None` to existing struct literals.
    Optionally add a `profile_defaults_roundtrip_with_framing` test.

## Constraints and invariants (from repo docs)

- **No `unwrap`/`expect`/`println!`/`todo!()`/`unimplemented!()`** in
  production code.
- **Tool failures become MCP tool results with `is_error: true`**, not
  protocol-level `McpError`. Framing defaults are config, not errors; a bad
  profile default (e.g. empty delimiter in `rx_framing`) surfaces via the
  existing read-propagates / subscribe-degrades asymmetry at call time —
  profile loading itself does not validate framing defaults (keep it that
  way; profiles are loose config).
- **Every `uN`/`Option<uN>` field on a `JsonSchema`-deriving struct uses
  `uint_schema`/`option_uint_schema`.** The four new fields are
  `Option<Enum>`/`Option<Struct>` — no `uint` format. Verify `ProfileDefaults`
  and `OpenArgs` (if extended) scans pass.
- **read/subscribe raw-path asymmetry preserved.** read bounded; subscribe
  full chunks. Defaults do not touch the raw path.
- **Framing semantics differ by design:** read keeps later frames after
  match; subscribe stops on match. Preserve. Defaults only fill config.
- **Match metadata:** read uses `accumulated.len()`; subscribe uses
  `total_returned`. Preserve.
- **subscribe degrades bad framing configs to raw with `warn!`;** read
  propagates errors. A bad connection default follows the same asymmetry at
  call time.
- **`flush_partial` contract unchanged.**
- **`frame_type_str()` unchanged per mode.**
- **Reconnect preserves connection identity/config** (`build_config`
  rebuilds `ConnectionConfig` for reconnect). The four new fields MUST be
  copied in `build_config` so a reconnect keeps the framing defaults.
- **Conventional commits:** `feat:` (new user-facing capability — profile
  defaults). No attribution footers.

## Verification commands

Run after implementation, in this order:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# focused
cargo test --lib profiles
cargo test --lib serial::schema
cargo test --lib tools::helpers
cargo test --test proptest
cargo test --test serial_pty
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
cargo test --test config_schema_validation
cargo test --test http_integration
cargo test --test stdio_integration
```

If native_sim firmware is not built, run `fw-build-native` first. The
orchestrator xtask can also be used:

```bash
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
```

CRITICAL: run `cargo test --test native_sim_validation -- --ignored` (not
just fmt/build/clippy) to actually execute the `#[ignore]`d e2e tests. The
Phase 4b follow-up shipped a defect because this step was skipped.

## Return instructions

When done, return a concise summary covering:

- Files changed and why.
- Final precedence chain (four layers) and where each resolution block
  lives (which functions).
- How `open_profile` seeds `ConnectionConfig` framing fields from
  `profile.defaults` (did you extend `OpenArgs` or add a separate path?).
- How `save_profile` snapshots the four connection defaults.
- Whether `get_status` exposes the defaults (F implemented or deferred).
- Schema regression coverage (ProfileDefaults, OpenArgs if extended).
- Tests added (unit + e2e) and which existing tests were updated.
- Gate command results — INCLUDING `native_sim_validation -- --ignored`
  (do not skip this). Note any failures with root cause.
- Any scope decision you had to make that is not covered above — especially
  around reconnect preservation (`build_config`) and whether plain `open`
  accepts the four optional framing fields.
- Any surprise in the four-layer precedence implementation (e.g. borrow
  conflicts when consulting `conn` inside the async resolution block).