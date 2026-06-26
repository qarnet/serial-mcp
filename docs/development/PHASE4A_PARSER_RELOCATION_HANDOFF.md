# Phase 4a Handoff — Parser relocation (`rx_parser` sibling field)

Source plan: `docs/development/FRAME_PIPELINE_PLAN.md` (Phase 4, parser
relocation half only). Presets are a separate Phase 4b handoff later.

Return a concise summary of changes, tests run, and blockers when done.

## Phase goal

Move the RX parser configuration out of `rx_framing` to a sibling `rx_parser`
field on `read` and `subscribe`. Today the parser is nested inside
`rx_framing.parser`; after this phase it is a top-level option next to
`rx_framing`. This decouples "how to split frames" from "how to interpret
frame content" and sets up Phase 4b protocol presets (which expand into
explicit `rx_framing` + `rx_parser` primitives).

Breaking JSON change. No backward-compat shim — `parser` is removed from
`RxFramingConfig` entirely.

## Decisions already made (do not re-litigate)

- **Relocate only (no presets this phase).** Presets are Phase 4b, a separate
  handoff after 4a lands and reviews.
- **Remove `parser` from `RxFramingConfig` entirely.** No deprecated alias.
  `rx_parser` is the only way to configure a parser. Clean break.
- **`rx_parser` is a sibling field on `read` and `subscribe`**, not on
  `write` (TX has no parser — parsers interpret RX frame content only).

## In scope

### A. Config shape change

Old request shape:
```json
{
  "rx_framing": {
    "type": "line",
    "ending": "auto",
    "parser": { "type": "at_command" }
  }
}
```

New request shape:
```json
{
  "rx_framing": { "type": "line", "ending": "auto" },
  "rx_parser": { "type": "at_command" }
}
```

### B. `src/framing.rs` changes

- Remove `pub parser: Option<ParserConfig>` from `RxFramingConfig`.
- Remove `parser: None` from `impl Default for RxFramingConfig`.
- Keep `ParserConfig`, `ParserType`, `ParsedFrame`, `FrameParser` trait, and
  all parser implementations (`AtCommandParser`, `JsonLinesParser`,
  `ShellPromptParser`, `RawParser`) in place. They are still used; only their
  config location moves.
- Change `FrameDecoder::new` signature to accept the parser separately:
  ```rust
  pub fn new(
      config: &RxFramingConfig,
      parser: Option<&ParserConfig>,
  ) -> Result<Self, String>
  ```
  Inside `new`, build the parser from the new `parser` arg instead of
  `config.parser`. Everything else in `new` stays.
- `FrameDecoder` struct field `parser: Option<Box<dyn FrameParser>>` stays —
  the decoder still owns the built parser at runtime. Only the CONFIG source
  moves.
- `push`, `slip_decode`, `flush_partial`, `frame_type_str` — no changes
  (they use `self.parser`, not `config.parser`).
- Update every test in `src/framing.rs` `mod tests` that constructs
  `RxFramingConfig` with a `parser` field:
  - Remove `parser: ...` from the `RxFramingConfig` literal.
  - Pass the parser as the second arg to `FrameDecoder::new(&cfg, parser_arg)`
    where `parser_arg` is `Some(&ParserConfig{...})` or `None`.
  - Grep targets: `parser: Some(ParserConfig`, `parser: None`,
    `FrameDecoder::new(&`. There are ~4-5 test sites in `src/framing.rs`
    (combined_line_at_parser, combined_line_json_parser,
    shell_prompt_custom_regex_invalid, max_frames_zero_edge uses None, etc.)
    plus `src/tools/helpers.rs` and `src/tools/rx_consume.rs` test helpers.

### C. `src/tools/types.rs` changes

- Add to `ReadArgs`:
  ```rust
  /// Optional RX parser configuration. When present, each decoded frame's
  /// content is interpreted (AT commands, JSON lines, shell prompts). Sibling
  /// to `rx_framing`; the parser operates on frames produced by `rx_framing`.
  #[serde(default)]
  pub rx_parser: Option<crate::framing::ParserConfig>,
  ```
- Add the same field to `SubscribeArgs`.
- Do NOT add `rx_parser` to `WriteArgs`.
- `FrameResult.parsed` (output) stays unchanged — it is populated by the
  decoder regardless of where the config came from.
- `ReadResult` doc comments mentioning "framing option" — check if any say
  "parser" lives inside framing; update wording if so. The `frames` field
  comment (~line 306) is fine; verify the `match_frame_index` comment.

### D. `src/tools/helpers.rs` changes

- `read_bytes_via_session` signature: add a `parser: Option<ParserConfig>`
  parameter (or `Option<&ParserConfig>` — prefer owned to avoid lifetime
  threading issues across the async loop). Place it near the `framing` param.
- At decoder construction (~line 323):
  ```rust
  let mut decoder: Option<FrameDecoder> = match framing.as_ref() {
      None => None,
      Some(cfg) => Some(FrameDecoder::new(cfg, parser.as_ref())?),
  };
  ```
- Update all tests in `helpers.rs` that call `read_bytes_via_session` with a
  `framing` arg carrying a parser: split the parser out into the new param.
  Grep for `parser: Some(crate::framing::ParserConfig` and
  `read_bytes_via_session(` call sites.
- The `line_framing` test helper (~line 1342) constructs `RxFramingConfig`
  with no parser — fine, but verify it still compiles after the field
  removal.

### E. `src/tools/stream_ops.rs` changes

- `stream_rx_via_session` signature: add `parser: Option<ParserConfig>` param.
- At decoder construction (~line 394):
  ```rust
  Some(cfg) => match FrameDecoder::new(cfg, parser.as_ref()) {
  ```
- `subscribe()` caller (~line 161): pass `args.rx_parser` through.

### F. `src/tools/io_ops.rs` changes

- `read()`: pass `args.rx_parser` into `read_bytes_via_session` (~line 116
  area, where `args.rx_framing` is passed).

### G. `src/tools/rx_consume.rs` changes

- `consume_frames` does NOT touch parser config — it works on the already-
  built `FrameDecoder`. No signature change.
- Update the `line_decoder()` test helper if it constructs a config with a
  parser (it does not — it uses `..Default::default()`). Verify.

### H. Tool descriptions

- `src/server.rs`: update `read` and `subscribe` descriptions to mention
  `rx_parser` as the way to interpret frame content, and clarify it is
  separate from `rx_framing`. Remove any wording that says parser lives
  inside framing.

### I. Schema regression

- `RxFramingConfig` and `RxFramingMode` are already in the `check_schema!`
  list? Verify. `ParserConfig` is not currently listed (it has no unsigned
  fields). After removing `parser` from `RxFramingConfig`, the schema shape
  changes — the existing `framing_fields_renamed_in_tool_schemas` test in
  `src/tools/mod.rs` checks for `rx_framing`/`tx_framing` presence and bare
  `framing` absence. Add an assertion that `rx_parser` is present in the
  `ReadArgs` and `SubscribeArgs` schemas, and that `parser` does NOT appear
  as a property of the `rx_framing` sub-schema. (Bare `"parser"` string may
  still appear elsewhere in the schema — scope the assertion to the
  `rx_framing` properties block if feasible, or just assert `rx_parser`
  presence and rely on the field removal to drop it from `rx_framing`.)

### J. Test migration

Update every test that sends `"parser": { "type": "..." }` inside `rx_framing`:
- `tests/native_sim_validation.rs`: ~3 sites (json_lines parser, at_command
  parser, and any others). Move `"parser"` out of the `rx_framing` object to
  a sibling `"rx_parser"` key.
- `tests/serial_pty.rs`: check for parser usage (likely none — PTY framing
  tests use line framing without parsers).
- `tests/proptest.rs`: `rx_framing_config_roundtrip_all_modes` constructs
  `RxFramingConfig` with `parser: Some(...)`. Remove `parser` from those
  literals. Optionally add a `parser_config_roundtrip` test for
  `ParserConfig` serde if not already covered. Update
  `read_args_roundtrip` / `subscribe_args_roundtrip` to include `rx_parser:
  None` in the struct literal (new field).

## Out of scope

- Protocol presets (`{ "protocol": { "type": "at_command" } }`). Phase 4b.
- Any new parser type. The four existing parsers stay.
- TX parser. TX has no parser.
- Profile defaults for `rx_parser` (Phase 5).
- Any change to `ParsedFrame` output shape or `FrameResult`.
- Backward-compat alias for `rx_framing.parser`. Clean break.

## Relevant files and current behavior

- `src/framing.rs`:
  - `RxFramingConfig` (line ~24): has `mode`, `parser`, `max_frames`,
    `include_terminators`. Remove `parser`.
  - `FrameDecoder::new` (line ~547): reads `config.parser` to build the
    parser. Change to read a separate arg.
  - `FrameDecoder` struct (line ~398): `parser: Option<Box<dyn FrameParser>>`
    field stays.
  - `mod tests`: ~4-5 sites construct `RxFramingConfig { parser: ..., ... }`
    and call `FrameDecoder::new(&cfg)`.
- `src/tools/types.rs`:
  - `ReadArgs` (line ~63): `rx_framing` field. Add `rx_parser`.
  - `SubscribeArgs` (line ~121): same.
  - `FrameResult.parsed` (line ~255): output, unchanged.
- `src/tools/helpers.rs`:
  - `read_bytes_via_session` (line ~294): `framing` param. Add `parser`.
  - Decoder build at line ~323.
  - `mod tests`: `char_framing_*` tests, `line_framing` helper.
- `src/tools/stream_ops.rs`:
  - `stream_rx_via_session` (line ~347): `framing` param. Add `parser`.
  - `subscribe` caller at line ~161.
- `src/tools/io_ops.rs`: `read()` passes `args.rx_framing` at ~line 116.
- `src/tools/rx_consume.rs`: `consume_frames` — no config contact, no change.
- `src/server.rs`: `read`/`subscribe` tool descriptions.
- `src/tools/mod.rs`: `framing_fields_renamed_in_tool_schemas` test.
- `src/serial.rs` `mod schema`: `check_schema!` list — verify
  `RxFramingConfig`/`RxFramingMode` coverage (likely already there from
  Phase 1).
- `tests/native_sim_validation.rs`: 3 parser-in-framing JSON sites.
- `tests/proptest.rs`: `rx_framing_config_roundtrip_all_modes` +
  `read_args_roundtrip` / `subscribe_args_roundtrip`.

## Expected API / UX shape

After Phase 4a:

```json
{
  "rx_framing": { "type": "line", "ending": "auto" },
  "rx_parser": { "type": "at_command" }
}
```

`rx_parser` is optional, sibling to `rx_framing`, on `read` and `subscribe`
only. `rx_framing` no longer has a `parser` property. `write` unchanged (no
parser). `FrameResult.parsed` output unchanged.

`rx_parser` without `rx_framing` is a no-op (parser operates on frames; no
framing = no frames = nothing to parse). Document this. Do not error —
silently ignore is fine, matching current behavior where a parser without
framing produces no parsed output.

## Test plan

1. **rx_parser present in schemas.** Extend
    `framing_fields_renamed_in_tool_schemas` in `src/tools/mod.rs` to assert
    `ReadArgs` and `SubscribeArgs` schemas contain `rx_parser`.
2. **rx_framing no longer has parser property.** Add an assertion (same test
    or a new one) that the `rx_framing` sub-schema does NOT expose a
    `parser` property.
3. **FrameDecoder::new takes parser separately.** Existing framing unit tests
    pass after migrating call sites to the new 2-arg `new(&cfg, parser)`.
4. **Parser still applies to frames.** `combined_line_at_parser`,
    `combined_line_json_parser`, `shell_prompt_custom_regex_invalid` (in
    `src/framing.rs`) still pass after migration — parser built from the new
    arg, frames still parsed.
5. **read integration: parser via rx_parser.** In `src/tools/helpers.rs`
    tests, drive `read_bytes_via_session` with `framing` (line) and `parser`
    (at_command) as separate args; assert `ReadOutcome.frames[i].parsed` is
    populated.
6. **native_sim parser tests migrated.** `native_read_json_parser_decodes_jsonout`
    and `native_read_at_parser_parses_pong` pass with `"rx_parser"` sibling
    instead of `"parser"` inside `rx_framing`.
7. **proptest roundtrip migrated.** `rx_framing_config_roundtrip_all_modes`
    passes without `parser` in the `RxFramingConfig` literals.
    `read_args_roundtrip` / `subscribe_args_roundtrip` pass with `rx_parser:
    None` added.
8. **regression: all non-parser framing tests green.** No behavior change
    for framing without a parser.

## Constraints and invariants (from repo docs)

- **No `unwrap`/`expect`/`println!`/`todo!()`/`unimplemented!()`** in
  production code.
- **Tool failures become MCP tool results with `is_error: true`**, not
  protocol-level `McpError`. Invalid `rx_parser` (e.g. bad regex) surfaces
  as a tool error via the existing `FrameDecoder::new` error path — keep
  the read-propagates / subscribe-degrades asymmetry for construction errors.
- **Every `uN`/`Option<uN>` field on a `JsonSchema`-deriving struct uses
  `uint_schema`/`option_uint_schema`.** `ParserConfig` has no unsigned
  fields; `rx_parser` is `Option<ParserConfig>`. No new schema regression
  entries needed, but verify.
- **read/subscribe raw-path asymmetry preserved.** read bounded; subscribe
  full chunks. Do not merge.
- **Framing semantics differ by design:** read keeps later frames after
  match; subscribe stops on match. Preserve. Parser relocation does not
  touch this.
- **Match metadata:** read uses `accumulated.len()`; subscribe uses
  `total_returned`. Preserve.
- **subscribe degrades bad framing configs to raw with `warn!`;** read
  propagates errors. A bad `rx_parser` (invalid regex) is a construction
  error — same asymmetry applies: read returns `Err`, subscribe degrades
  (parser becomes None, framing still works). Preserve this.
- **`flush_partial` increments `frame_count` and emits a `Frame` with
  `parsed: None`.** Keep. Parser relocation does not change flush behavior.
- **`frame_type_str()` unchanged per mode.** No change.
- **Conventional commits:** `refactor:` (this is a config-shape refactor +
  breaking change, not a new feature) or `feat:` if you prefer to signal
  the new `rx_parser` field. Pick one; `refactor:` is more accurate since
  no new capability is added — only relocated. No attribution footers.

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

If native_sim firmware is not built, run `fw-build-native` first (see
`AGENTS.md` Firmware section). The orchestrator xtask can also be used:

```bash
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets
cargo run --manifest-path xtask/Cargo.toml -- test-all
```

## Return instructions

When done, return a concise summary covering:

- Files changed and why.
- Final `FrameDecoder::new` signature and how the parser threads from
  `ReadArgs.rx_parser` / `SubscribeArgs.rx_parser` through
  `read_bytes_via_session` / `stream_rx_via_session` into `new`.
- Confirmation that `RxFramingConfig` no longer has a `parser` field and
  no test constructs it with one.
- New/updated schema assertions for `rx_parser` presence and `parser`
  absence from `rx_framing`.
- Tests migrated (framing unit tests, helpers tests, native_sim JSON
  payloads, proptest).
- Gate command results (`fmt`, `build`, `test`, `clippy`). Note any
  failures with root cause.
- Any scope decision you had to make that is not covered above — especially
  around the read-propagates / subscribe-degrades asymmetry for a bad
  `rx_parser` regex.
- Any surprise in the `FrameDecoder::new` signature change blast radius.