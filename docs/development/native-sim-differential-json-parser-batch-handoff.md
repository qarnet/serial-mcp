# Native Simulator Differential Gate — Batch 14 JSON Parser Handoff

> **Historical/superseded record:** Phase F removed active native_sim/NCS
> source and configuration. Body preserves accepted differential evidence.

## Phase goal

Promote exactly one stable characterized native row to isolated direct
native-versus-fixture comparison:

```text
native_read_json_parser_decodes_jsonout
```

This becomes **Compared Batch 14**, not baseline-and-stronger. It compares the
fixed firmware `jsonout` response and exact explicit JSON parser outcome.
Existing JSON Lines fixture proof remains independently stronger for TX and
object-only parser semantics.

## Accepted target evidence

Source row:

```text
tests/native_sim_validation/unix.rs::native_read_json_parser_decodes_jsonout
```

Stable three-endpoint public-MCP characterization:

```text
target/native-sim-differential/json-parser-characterization.json
sha256: f51b5d77bac3904d214e2ea76794cf1d10f4d5aa8849224e750af30a8e9e3a06
```

All vectors match after normalizing only endpoint/connection IDs and
`elapsed_ms`. Target sequence:

1. standard anonymous open at 115200, `profile_mode:"none"`;
2. public boot-banner literal-match read;
3. public `transact("arm_cmd 1000\r\n")` matched on exact
   `arm_cmd delay=1000\r\n`;
4. public UTF-8 `write("jsonout\r\n")`,
   `bytes_written=decoded_bytes=9`;
5. public target `read` from now, UTF-8, 3000 ms, explicit
   `rx_framing:{"type":"line"}` and `rx_parser:{"type":"json_lines"}`.

Setup is validated but excluded from normalized observations.

Target exact result:

```text
top raw payload is three firmware JSON CRLF lines (140 bytes);
bytes_read/bytes_observed/bytes_returned = 140/0/0;
stop_reason=timeout; no match, truncation, drop, or error;
position from/next/lost/remaining/start/end = 52/192/0/0/0/192;
three ordered UTF-8 line frames, indexes 0/1/2:
  {sensor:"temp", value:25.5, unit:"C"}
  {sensor:"humidity", value:60, unit:"%"}
  {sensor:"pressure", value:1013.25, unit:"hPa"}
each parser="json".
```

Firmware source `firmware/src/command.c::cmd_jsonout` is the exact static wire
oracle. Parser source `src/framing/parsers/mod.rs::JsonLinesParser` parses JSON
objects and leaves non-objects raw.

## Grounded design decisions

- Add only one narrow compatibility-peer command: `jsonout` emits the exact
  static 140-byte firmware wire response. It belongs in
  `tests/common/native_sim_differential/backend.rs::CompatibilityPeer`, not
  global fixture defaults or `protocol_peers.rs`.
- `ParsedFrameObservation` must preserve generic JSON-object keys and values,
  not hard-code the current sensor schema. Add:

  ```rust
  Json { fields: BTreeMap<String, serde_json::Value> }
  ```

  under its existing internally tagged `parser` enum. This normalizes public
  parser object `{"parser":"json", ...}` by removing only `parser` and
  retaining every remaining JSON key/value in deterministic `BTreeMap` order.
  Serialized typed output may represent keys under `fields`; raw frame payload
  remains exact independently.
- Do not change parser/product code. `optional_parsed_frame` adds only public
  parser value `"json"`; unsupported parser handling remains strict.
- Existing `json_lines_preset_writes_line_and_preserves_object_only_parser_behavior`
  remains stronger coverage and is not a baseline binding.

## In scope

### `tests/common/native_sim_differential/model.rs`

1. Import `BTreeMap`.
2. Add `ParsedFrameObservation::Json { fields: BTreeMap<String, Value> }`.
3. Extend `optional_parsed_frame` for public `parser:"json"`: copy every
   parsed object member except `parser` into `fields`; preserve values exactly.
   Update unsupported-parser error text accordingly.
4. Add `DifferentialCase::JsonParserJsonout`, serde/id
   `native_read_json_parser_decodes_jsonout`.
5. Add `DifferentialBatch::JsonParser`, `BATCH_FOURTEEN`, `ALL` length
   32 → 33, and batch mapping.

### `tests/common/native_sim_differential/backend.rs`

Add exact static `JSONOUT_RESPONSE` public bytes matching firmware source,
including all three CRLF terminators. Add only command branch
`b"jsonout" => Action::Emit(JSONOUT_RESPONSE.to_vec())`. Existing arm delay
must apply before this emit through current shared peer flow.

### `tests/common/native_sim_differential/registry.rs`

Replace only pending row with:

```rust
DifferentialRow::compared(
    "native_read_json_parser_decodes_jsonout",
    DifferentialBatch::JsonParser,
    DifferentialCase::JsonParserJsonout,
)
```

Update only current counts to:

```text
49 total / 19 compared / 14 baseline-and-stronger / 3 retired / 13 pending
```

### `tests/common/native_sim_differential/scenarios.rs`

1. Add Batch 14 schema/report constants:

   ```rust
   serial-mcp.native-sim-differential.json-parser-batch.v1
   json-parser-batch.json
   ```

2. Add `run_json_parser_batch()` and all exhaustive schema/filename/reverse
   schema/standard-open/boot matches.
3. Add `JsonParserJsonout` branch:
   - public `json_parser_setup()` reuses current neutral
     `arm_and_write_delayed("jsonout\r\n", 9)`;
   - public `read_json_parser()` uses exactly characterized explicit line and
     `json_lines` parser call, then `normalize_positioned_read`;
   - exact assertion retains anonymous logical connection, full raw
     `JSONOUT_RESPONSE`, `140/0/0`, timeout/no match/no truncation/no drops/no
     error, position `52/192/0/0/0/192`, and exact three `FrameObservation`s
     with JSON parsed maps in order.
4. Keep setup out of `ScenarioOutcome`; retain open, boot read, target read.

### `tests/native_sim_differential.rs`

1. Import Batch 14 runner/constants; add ignored
   `json_parser_batch_matches_normalized_public_outcomes` test.
2. Update counts to `19/14/3/13`, `BATCH_FOURTEEN`, expected-all chain,
   `ALL.len()==33`, direct compared row assertion, and all report
   schema/filename uniqueness/sample count tests.
3. Add focused JSON parsed-map mutation proof: changing a retained JSON key or
   value must make two typed outcomes unequal. Keep existing raw/AT mutation
   coverage.

### `tests/doc_drift.rs`

Add Batch 14 direct compared set/row lock and include it in expected compared
rows. Update current status markers to 19 compared, 13 pending,
`19/14/3/13`, and 33 covered rows. Lock Batch 14 schema/report, exact JSON
parser target facts, characterization hash, and an exact traceability mapping
claim.

### Documentation

Update only:

```text
docs/development/native-sim-test-traceability.md
docs/development/native-sim-replacement-research-progress.md
docs/development/native-sim-replacement-recommendation.md
```

State direct Compared Batch 14, static three JSON object response, explicit
line + `json_lines` parser, exact `140/0/0`, timeout, three ordered parsed
objects, position `52/192/0/0/0/192`, schema/report, characterization hash,
current `19/14/3/13` counts, 33 covered rows, existing stronger JSON Lines
fixture proof, and Phase F blocked. Rewrite Batch 13 current wording as
historical before current Batch 14 status. Traceability mapping row must say
`**Compared Batch 14.**` explicitly.

### Retire disposable evidence harness

After permanent Batch 14 passes, delete only:

```text
tests/native_sim_json_parser_characterization.rs
docs/development/native-sim-differential-json-parser-characterization-handoff.md
```

Keep ignored characterization artifact:

```text
target/native-sim-differential/json-parser-characterization.json
```

## Out of scope

- Product parser/framing implementation, firmware, NCS, CI, xtask, release,
  dependencies, Cargo/lockfiles.
- General fixture defaults, `protocol_peers.rs`, existing JSON Lines fixture
  proof, Batches 1-13, or other pending rows.
- Phase F deletion, commits, pushes, merges, PRs, or cleanup of pre-existing
  dirty work.

## Required validation

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test native_sim_differential
cargo test --locked --test doc_drift
cargo test --locked --test device_protocol_parity \
  json_lines_preset_writes_line_and_preserves_object_only_parser_behavior \
  -- --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential \
  json_parser_batch_matches_normalized_public_outcomes -- --ignored --test-threads=1
sha256sum target/native-sim-differential/json-parser-batch.json
# Repeat prior ignored Batch 14 command and sha256sum; hashes must match.
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential -- --ignored --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_read_json_parser_decodes_jsonout -- --ignored --test-threads=1
```

Use `git diff --check` before return. Do not commit.

## Escalation

Stop and report raw paired outputs if static fixture wire differs, any JSON
object key/value is lost or reordered semantically, JSON parser model needs a
product change, existing batches change, docs cannot separate historical/current
counts, or scope needs another pending row. Do not weaken object/order/raw-byte
assertions, pin unexplained output, or expand scope.

## Required return

Return changed/deleted/generated files, exact normalized Batch 14 pair result,
both report hashes, each validation result, current registry counts,
documentation lock result, deviations/blockers, and confirmation no commit was
made.
