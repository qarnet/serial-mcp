# Phase 4b Handoff — Protocol presets (`at_command`)

Source plan: `docs/development/FRAME_PIPELINE_PLAN.md` (Phase 4, presets half).
Phase 4a (parser relocation) is complete; `rx_parser` is now a sibling field.
Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Add an optional `protocol` field to `write`, `read`, and `subscribe`. When
set, it expands to a bundle of explicit framing/parser primitives so agents
can say "this is an AT-command device" once instead of repeating
`tx_framing`, `rx_framing`, and `rx_parser` on every call.

Phase 4b ships exactly one preset — `at_command` — as a reference for the
mechanism. Other presets (`modbus`, `slip_json`, etc.) are deferred.

## Decisions already made (do not re-litigate)

- **Ship `at_command` preset only.** Other presets deferred.
- **`protocol` field on `write`, `read`, and `subscribe`.** A single preset
  expands per-tool: for read/subscribe it sets `rx_framing` + `rx_parser`;
  for write it sets `tx_framing`. Matches the plan example ("imply TX line
  CR, RX line auto, and RX AT parser").
- **Explicit fields win over the preset.** If an agent sets both `protocol`
  and `rx_framing`/`rx_parser`/`tx_framing`, the explicit field overrides the
  preset's corresponding component. The preset fills only the gaps left by
  `None`. Predictable, flexible, no error.
- **The `at_command` preset expands to:**
  - `tx_framing`: `{ "type": "line", "ending": "cr" }` (AT commands are
    CR-terminated on TX).
  - `rx_framing`: `{ "type": "line", "ending": "auto" }` (devices may send
    LF, CRLF, or bare CR — `auto` adapts, per Phase 2).
  - `rx_parser`: `{ "type": "at_command" }`.

## In scope

### A. `ProtocolPreset` type

Add to `src/framing.rs`:

```rust
/// Built-in protocol preset. A named bundle of framing/parser primitives
/// that a single `protocol` field expands into on `write`, `read`, and
/// `subscribe`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolPreset {
    /// AT-command modem protocol. TX appends `\r`, RX splits on line
    /// endings (auto), RX frames are parsed as AT command responses/URCs.
    AtCommand,
}
```

Unit-style enum (no fields). `#[serde(rename_all = "snake_case")]` makes the
JSON value `"at_command"`. `JsonSchema` derives the standard tagged-union
shape (a string enum in JSON Schema). Add `ProtocolPreset` to the
`check_schema!` list in `src/serial.rs` `mod schema` only if it carries
unsigned fields (it does not — it is a unit enum). Verify the existing
`tool_schemas_have_no_nonstandard_uint_formats` and per-type scans do not
flag it; no new entry needed, but double check.

### B. Expansion function

Add a free function in `src/framing.rs`:

```rust
/// The TX framing implied by a protocol preset, if any.
pub fn preset_tx_framing(p: ProtocolPreset) -> TxFramingConfig { ... }

/// The RX framing implied by a protocol preset, if any.
pub fn preset_rx_framing(p: ProtocolPreset) -> RxFramingConfig { ... }

/// The RX parser implied by a protocol preset, if any.
pub fn preset_rx_parser(p: ProtocolPreset) -> ParserConfig { ... }
```

For `AtCommand`:
- `preset_tx_framing` → `TxFramingConfig { mode: TxFramingMode::Line { ending: TxLineEnding::Cr } }`.
- `preset_rx_framing` → `RxFramingConfig { mode: RxFramingMode::Line { ending: LineEnding::Auto }, max_frames: None, include_terminators: false }`.
- `preset_rx_parser` → `ParserConfig { parser_type: ParserType::AtCommand, custom_prompt: None }`.

Use match arms so adding a future preset forces updating all three
functions (exhaustive, no `_ =>` fallback that could silently miss a new
variant). Each function returns the config by value (owned).

### C. Args struct changes

`src/tools/types.rs`:

- Add to `WriteArgs`:
  ```rust
  /// Optional protocol preset. When set, fills in default `tx_framing`
  /// (and, on read/subscribe, `rx_framing` + `rx_parser`) for the named
  /// protocol. Explicit `tx_framing`/`rx_framing`/`rx_parser` fields
  /// override the preset's corresponding component.
  #[serde(default)]
  pub protocol: Option<crate::framing::ProtocolPreset>,
  ```
- Add the same field to `ReadArgs` and `SubscribeArgs`.
- Add doc comments noting "explicit wins" precedence.

### D. Expansion + override in the tools

Apply expansion early, before the framing/parser is used. Do the override
in each tool's entry point (the `io_ops`/`stream_ops` functions), NOT in
`server.rs` (keep tool entry points thin).

#### `src/tools/io_ops.rs::write`

After `parse_encoding` and before the TX framing is applied:

```rust
let tx_framing = match args.protocol {
    Some(p) => match args.tx_framing {
        Some(explicit) => Some(explicit),          // explicit wins
        None => Some(crate::framing::preset_tx_framing(p)),
    },
    None => args.tx_framing,
};
```

Then use `tx_framing` (the resolved value) instead of `args.tx_framing`
below. If `args.tx_framing` was moved, clone or take by value as needed
(args is owned).

#### `src/tools/io_ops.rs::read`

Resolve `rx_framing` and `rx_parser` similarly:

```rust
let rx_framing = match args.protocol {
    Some(p) => match args.rx_framing {
        Some(explicit) => Some(explicit),
        None => Some(crate::framing::preset_rx_framing(p)),
    },
    None => args.rx_framing,
};
let rx_parser = match args.protocol {
    Some(p) => match args.rx_parser {
        Some(explicit) => Some(explicit),
        None => Some(crate::framing::preset_rx_parser(p)),
    },
    None => args.rx_parser,
};
```

Pass the resolved `rx_framing`/`rx_parser` into `read_bytes_via_session`.
Take care with ownership: `args` is owned, so move or clone fields. Prefer
moving when possible to avoid clones.

#### `src/tools/stream_ops.rs::subscribe`

Same resolution as read, then pass resolved `rx_framing`/`rx_parser` into
`stream_rx_via_session`.

### E. Tool descriptions

`src/server.rs`: update `write`, `read`, `subscribe` descriptions to
mention the `protocol` field, that it expands to framing/parser primitives,
that explicit fields win, and name `at_command` as the available preset.

### F. Schema regression

`src/tools/mod.rs`: extend the schema tests to assert `protocol` is present
in `WriteArgs`, `ReadArgs`, and `SubscribeArgs` schemas. Keep the existing
`rx_parser_present_in_schemas` and `framing_fields_renamed_in_tool_schemas`
assertions.

`src/serial.rs` `mod schema`: verify `ProtocolPreset` (a `JsonSchema`
derive) has no unsigned fields — it does not. No new `check_schema!` entry
strictly needed, but add one if you want defense-in-depth for future
variants that might carry fields:
`check_schema!(protocol_preset_has_no_uint_formats, ProtocolPreset);`
Optional — your call.

## Out of scope

- Presets other than `at_command` (`modbus`, `slip_json`, `shell`, etc.).
- Custom/user-defined presets. Only the built-in enum.
- `protocol` on `open`/`open_profile` (those set serial params, not
  framing). Out of scope — profile defaults for framing live in Phase 5.
- Changing the `at_command` AT parser internals. The preset reuses the
  existing `ParserType::AtCommand`.
- A `protocol` field that itself carries parameters. The preset enum is
  unit-only for now; parameterized presets (if ever needed) are a future
  design.
- TX-side `rx_parser`. TX has no parser.
- Backward-compat shim. `protocol` is additive — old requests without it
  behave exactly as before.

## Relevant files and current behavior

- `src/framing.rs`:
  - `ProtocolPreset` (new), `preset_tx_framing`, `preset_rx_framing`,
    `preset_rx_parser` (new free functions).
  - `TxFramingConfig`, `TxFramingMode`, `TxLineEnding`,
    `RxFramingConfig`, `RxFramingMode`, `LineEnding`,
    `ParserConfig`, `ParserType` — all existing, reused.
- `src/tools/types.rs`: `WriteArgs`, `ReadArgs`, `SubscribeArgs` — add
  `protocol: Option<ProtocolPreset>`.
- `src/tools/io_ops.rs`: `write()` (~line 19), `read()` (~line 52). Expand
  + override here.
- `src/tools/stream_ops.rs`: `subscribe()` (~line 64). Expand + override
  here; pass resolved values into the spawned `stream_rx_via_session` task.
- `src/server.rs`: tool descriptions for write/read/subscribe.
- `src/tools/mod.rs`: schema tests.
- `src/serial.rs` `mod schema`: optional `ProtocolPreset` guard.
- `tests/proptest.rs`: add `protocol: None` to the existing
  `write_args_roundtrip`, `read_args_roundtrip`,
  `subscribe_args_roundtrip` struct literals. Optionally add a proptest
  exercising `protocol: Some(ProtocolPreset::AtCommand)` round-trip on
  each of the three args structs.
- `tests/native_sim_validation.rs`: optional end-to-end preset test (see
  test plan item 8).
- `tests/serial_pty.rs`: optional PTY preset test (see test plan item 9).

## Expected API / UX shape

```json
{
  "connection_id": "abc",
  "data": "AT+CGMI",
  "protocol": { "type": "at_command" }
}
```

Expands identically to:

```json
{
  "connection_id": "abc",
  "data": "AT+CGMI",
  "encoding": "utf8",
  "tx_framing": { "type": "line", "ending": "cr" }
}
```

Override example:

```json
{
  "connection_id": "abc",
  "data": "AT+CGMI",
  "protocol": { "type": "at_command" },
  "tx_framing": { "type": "line", "ending": "crlf" }
}
```

Here the explicit `tx_framing` wins → CRLF terminator instead of the
preset's CR. `rx_framing`/`rx_parser` (on read/subscribe) still come from
the preset.

## Test plan

Add tests:

1. **preset_tx_framing returns line CR.** Unit test in `src/framing.rs`:
   `preset_tx_framing(AtCommand)` → `Line { ending: Cr }`.

2. **preset_rx_framing returns line auto.** Unit test:
   `preset_rx_framing(AtCommand)` → `Line { ending: Auto }`.

3. **preset_rx_parser returns at_command.** Unit test:
   `preset_rx_parser(AtCommand)` → `ParserType::AtCommand`.

4. **write: protocol sets tx_framing when none explicit.** In
   `src/tools/io_ops.rs` tests (or via the http/PTY harness): a `write`
   with `protocol: AtCommand` and no `tx_framing` appends `\r` to the
   payload. Assert the framed bytes end in `0x0D`. Use the existing
   loopback/PTY harness to observe sent bytes if feasible; otherwise a unit
   test on the resolved config is acceptable.

5. **write: explicit tx_framing overrides protocol.** A `write` with
   `protocol: AtCommand` AND `tx_framing: Line { Crlf }` appends `\r\n`,
   not `\r`. Assert framed bytes end in `0x0D 0x0A`.

6. **read: protocol sets rx_framing + rx_parser when none explicit.** In
   `src/tools/helpers.rs` tests: drive `read_bytes_via_session` with
   `protocol` (resolve in `read()` first, or test the resolution logic
   directly). Assert frames are line-split and parsed as AT. Easiest: unit
   test the resolution function in `io_ops`/a small helper, then an
   integration test via the loopback harness.

7. **read: explicit rx_framing overrides protocol.** `protocol: AtCommand`
   + `rx_framing: Delimiter { "|" }` → the delimiter wins; `rx_parser`
   still comes from the preset (AtCommand). Assert frames split on `|`
   and `parsed` is populated.

8. **native_sim end-to-end preset.** In
   `tests/native_sim_validation.rs`: open a connection, `write` with
   `protocol: at_command` and an AT command the firmware echoes/responds
   to; `read` with `protocol: at_command`; assert the response frames are
   parsed as `at_command` with `response_type` `status`/`response`. This
   proves the full preset round-trip over real software serial.

9. **PTY preset (optional).** In `tests/serial_pty.rs`: similar to
   native_sim but via PTY. Optional if native_sim covers it.

10. **schema: protocol present.** In `src/tools/mod.rs`: assert `protocol`
    appears in `WriteArgs`, `ReadArgs`, `SubscribeArgs` schemas.

11. **regression: requests without protocol unchanged.** All existing
    tests pass; no behavior change when `protocol` is absent (defaults to
    `None`, no expansion).

## Constraints and invariants (from repo docs)

- **No `unwrap`/`expect`/`println!`/`todo!()`/`unimplemented!()`** in
  production code.
- **Tool failures become MCP tool results with `is_error: true`**, not
  protocol-level `McpError`. `ProtocolPreset` has no construction errors
  (unit enum), so no new error path. Expansion functions are infallible.
- **Every `uN`/`Option<uN>` field on a `JsonSchema`-deriving struct uses
  `uint_schema`/`option_uint_schema`.** `ProtocolPreset` is a unit enum —
  no unsigned fields. `protocol: Option<ProtocolPreset>` adds an
  `Option<Enum>` field, which schemars handles as a nullable enum; no
  `uint` format involved.
- **read/subscribe raw-path asymmetry preserved.** read bounded; subscribe
  full chunks. Presets do not touch the raw path.
- **Framing semantics differ by design:** read keeps later frames after
  match; subscribe stops on match. Preserve. Presets only fill in config;
  loop behavior unchanged.
- **Match metadata:** read uses `accumulated.len()`; subscribe uses
  `total_returned`. Preserve.
- **subscribe degrades bad framing configs to raw with `warn!`;** read
  propagates errors. A preset always produces a valid config, so this
  asymmetry is not exercised by the preset itself — but an explicit
  override that is invalid (e.g. empty delimiter) still follows the
  existing asymmetry. Preserve.
- **`flush_partial` contract unchanged.**
- **`frame_type_str()` unchanged per mode.**
- **Conventional commits:** `feat:` (this adds a new user-facing capability
  — the `protocol` field + preset). No attribution footers.

## Verification commands

Run after implementation, in this order:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings

# focused
cargo test --lib framing
cargo test --lib serial::schema
cargo test --lib tools::helpers
cargo test --lib tools::tests
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

## Return instructions

When done, return a concise summary covering:

- Files changed and why.
- Final `ProtocolPreset` enum shape and the three expansion functions.
- Where expansion + override happens (which functions resolve
  `tx_framing`/`rx_framing`/`rx_parser` from `protocol`).
- How ownership/moves were handled in the override (did you clone, or move
  out of `args`?).
- Schema regression results (`protocol` present, no `uint` formats).
- Tests added (unit + integration) and which existing tests were updated.
- Gate command results (`fmt`, `build`, `test`, `clippy`). Note any
  failures with root cause.
- Any scope decision you had to make that is not covered above —
  especially around the exact JSON schema shape schemars emits for
  `Option<ProtocolPreset>` and whether it needs a `null` arm.
- Any surprise in the override precedence implementation (e.g. did
  `#[serde(default)]` on `protocol` interact oddly with an absent field?).