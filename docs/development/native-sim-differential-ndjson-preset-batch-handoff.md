# Native Simulator Differential Gate — Batch 15 NDJSON Preset Handoff

## Phase goal

Promote exactly these two characterized native rows to one isolated direct
native-versus-fixture Batch 15:

```text
native_read_ndjson_preset_decodes_json_frames
native_read_ndjson_preset_skips_empty_lines
```

Both rows become **Compared Batch 15**, not baseline-and-stronger. They prove
the public `ndjson` protocol preset's auto-line framing, `skip_empty:true`, and
JSON parser behavior against exact static firmware `sendraw` bytes. Existing
`ndjson_preset_parses_records_and_skips_blank_whitespace_lines` remains an
independent stronger Rust-PTY proof.

## Accepted target evidence

Source rows:

```text
tests/native_sim_validation/unix.rs::native_read_ndjson_preset_decodes_json_frames
tests/native_sim_validation/unix.rs::native_read_ndjson_preset_skips_empty_lines
```

Stable three-endpoint public-MCP characterization:

```text
target/native-sim-differential/ndjson-characterization.json
sha256: 10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3
```

Only endpoint/connection IDs and `elapsed_ms` were normalized. Each case uses
standard anonymous `open` at 115200 with `profile_mode:"none"`, boot-banner
literal-match `read`, exact public
`transact("arm_cmd 1000\r\n")`, then one public UTF-8 `sendraw` write. Setup
is validated but excluded from the normalized scenario outcome. The target
always starts at `from:{"type":"now"}`, uses UTF-8 and 3000 ms, and supplies
only `protocol:{"type":"ndjson"}`; no explicit framing/parser, sleep, flush,
or obsolete per-call buffer argument is allowed.

### Case A — JSON records with one blank line

```text
payload: {"a":1}\n\n{"b":2}\n
command: sendraw hex 7B2261223A317D0A0A7B2262223A327D0A\r\n
write bytes: 48
```

Exact target:

```text
raw payload exact, UTF-8
bytes_read/bytes_observed/bytes_returned = 17/0/0
stop_reason=timeout; normal, unmatched, not truncated, no drop/error
position from/next/lost/remaining/start/end = 52/69/0/0/0/69
ordered line frames:
  0: {"a":1}, parser json, fields {a:1}
  1: {"b":2}, parser json, fields {b:2}
```

### Case B — blank plus whitespace-only lines

```text
payload: {"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n
command: sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A\r\n
write bytes: 74
```

Exact target:

```text
raw payload exact, UTF-8
bytes_read/bytes_observed/bytes_returned = 30/0/0
stop_reason=timeout; normal, unmatched, not truncated, no drop/error
position from/next/lost/remaining/start/end = 52/82/0/0/0/82
ordered line frames:
  0: {"a":1}, parser json, fields {a:1}
  1: {"b":2}, parser json, fields {b:2}
  2: {"c":3}, parser json, fields {c:3}
```

Blank lines and the three-space line stay in raw payload but emit no frames.
`bytes_returned=0` is deliberate framed-read behavior. No model change is
needed: Batch 14's `ParsedFrameObservation::Json { fields: BTreeMap<String,
Value> }` already retains all public parsed object keys and values.

## Grounded implementation shape

- `src/framing/config.rs::ProtocolPreset::Ndjson` expands to auto line framing
  with `skip_empty:true` and the `JsonLines` parser. Do not alter product
  framing/parser code.
- `tests/common/native_sim_differential/backend.rs::CompatibilityPeer` already
  implements narrow static `sendraw` branches for the SLIP/COBS batches and
  preserves the one-second `armed_delay` before every branch. Add exactly two
  new static branches, keyed by the two command strings without CRLF, each
  emitting the corresponding exact payload bytes. Keep them in this narrow
  differential peer, not global fixture defaults or `protocol_peers.rs`.
- Reuse `PublicSession::arm_and_write_delayed`; it gives the public readiness
  barrier without adding a sleep. Use static scenario command strings with CRLF
  and exact 48/74-byte write assertions.
- Reuse the Batch 14 typed JSON observation shape. Add a small shared NDJSON
  frame constructor/assertion only if it makes both exact cases clearer; do not
  weaken any raw payload, frame order/index, parsed-map, counter, or position
  assertion.

## In scope

### `tests/common/native_sim_differential/model.rs`

1. Add exactly:

   ```rust
   DifferentialCase::NdjsonPresetJsonFrames
   DifferentialCase::NdjsonPresetSkipsEmptyLines
   DifferentialBatch::NdjsonPreset
   ```

2. Add `BATCH_FIFTEEN: [Self; 2]` in source-row order.
3. Extend `ALL` from 33 to 35, append Batch 15 after Batch 14, and extend the
   serde IDs, `id()`, and `batch()` exhaustively.
4. Do not change normalization or `ParsedFrameObservation`.

### `tests/common/native_sim_differential/backend.rs`

1. Add public static payload constants for both exact NDJSON byte sequences so
   scenario assertions do not duplicate response bytes.
2. Add only these `CompatibilityPeer::on_command` branches:

   ```text
   sendraw hex 7B2261223A317D0A0A7B2262223A327D0A
   sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A
   ```

   Each emits its matching constant after the existing armed delay. No generic
   hex parser, state machine, or global fixture behavior.

### `tests/common/native_sim_differential/registry.rs`

Replace only the two NDJSON pending rows with direct `DifferentialRow::compared`
rows bound to `DifferentialBatch::NdjsonPreset` and their matching cases.
Resulting current counts:

```text
49 total / 21 compared / 14 baseline-and-stronger / 3 retired / 11 pending
```

### `tests/common/native_sim_differential/scenarios.rs`

1. Add constants:

   ```text
   serial-mcp.native-sim-differential.ndjson-preset-batch.v1
   ndjson-preset-batch.json
   ```

2. Add `run_ndjson_preset_batch()` and complete every exhaustive batch/schema/
   filename/reverse-schema/standard-open/boot matching branch. Update
   `validate_report` current counts to `21/14/3/11`.
3. Add one shared public `read_ndjson_preset()` using exactly:

   ```json
   {
     "connection_id":"<current>",
     "from":{"type":"now"},
     "encoding":"utf8",
     "timeout_ms":3000,
     "protocol":{"type":"ndjson"}
   }
   ```

4. Add one setup method per static `sendraw` command. Both must reuse
   `arm_and_write_delayed` and assert exact write sizes 48 and 74.
5. Add both `execute_case` branches. Each retains only open, boot read, and
   target positioned read in `ScenarioOutcome`; setup remains validated but
   excluded.
6. Add exact target assertions described above. Parsed JSON fields must use the
   existing `ParsedFrameObservation::Json` form; all frame indexes, payloads,
   counters, no-error fields, and positions must remain explicit.

### `tests/native_sim_differential.rs`

1. Import Batch 15 runner/report constants and add ignored
   `ndjson_preset_batch_matches_normalized_public_outcomes`.
2. Update registry count lock to `21/14/3/11`, add isolated `BATCH_FIFTEEN`
   membership, append it to expected-all, and lock `ALL.len()==35`.
3. Lock both Batch 15 registry rows as direct Compared rows, with their exact
   batch/case pairs and no baseline proof binding.
4. Extend schema and filename uniqueness/deterministic serialization assertions
   so Batch 15 cannot alias an earlier report type or filename. Update sample
   report registry counts.

### `tests/doc_drift.rs`

1. Add a two-row `DIFFERENTIAL_BATCH_FIFTEEN_COMPARED_ROWS` lock and include it
   in the expected compared set. Update expected current counts to `21/14/3/11`.
2. Add direct-row guards requiring `DifferentialBatch::NdjsonPreset` and the
   correct case per source row, no baseline binding.
3. Add exact mapping and lower-validation-disposition locks for both NDJSON
   rows. Each mapping claim must be identical to its corresponding lower table
   disposition claim. Lock schema ID, filename, both exact payload behaviors,
   `17/0/0` and `30/0/0`, positions `52/69/0/0/0/69` and
   `52/82/0/0/0/82`, characterization path/hash, stronger fixture proof,
   `21/14/3/11`, 35 covered rows, and Phase F blocked.
4. Preserve historical markers through Batch 14. Do not label historical
   `19/14/3/13` as current after Batch 15.

### Documentation

Update only:

```text
docs/development/native-sim-test-traceability.md
docs/development/native-sim-replacement-research-progress.md
docs/development/native-sim-replacement-recommendation.md
```

State Batch 15 is current direct Compared evidence; Batch 14 becomes
historical. Document both payloads, `protocol:{"type":"ndjson"}`, auto line +
`skip_empty` + JSON semantics, exact counters/positions, schema/report,
characterization hash, current `21/14/3/11` counts, 35 covered rows, stronger
NDJSON fixture proof, and Phase F still blocked.

### Retire disposable characterization harness

After permanent Batch 15 passes, delete only:

```text
tests/native_sim_ndjson_characterization.rs
docs/development/native-sim-differential-ndjson-characterization-handoff.md
```

Keep the ignored evidence artifact:

```text
target/native-sim-differential/ndjson-characterization.json
```

Keep this Batch 15 handoff and all unrelated existing dirty files.

## Out of scope

- Product code, firmware, NCS, CI, xtask, release workflows, dependencies,
  Cargo manifests/lockfiles, schema changes, and profile behavior.
- General fixture defaults, `protocol_peers.rs`, existing NDJSON fixture proof,
  Batches 1-14, NMEA/Modbus/lifecycle pending rows, Phase F work.
- Committing, staging, pushing, merging, PR work, or cleanup of pre-existing
  dirty worktree content.

## Required validation

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test native_sim_differential
cargo test --locked --test doc_drift
cargo test --locked --test device_protocol_parity \
  ndjson_preset_parses_records_and_skips_blank_whitespace_lines \
  -- --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential \
  ndjson_preset_batch_matches_normalized_public_outcomes \
  -- --ignored --test-threads=1
sha256sum target/native-sim-differential/ndjson-preset-batch.json
# Repeat previous ignored Batch 15 command and sha256sum; hashes must match.
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential -- --ignored --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_read_ndjson_preset_decodes_json_frames -- --ignored --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_read_ndjson_preset_skips_empty_lines -- --ignored --test-threads=1
git diff --check
```

## Escalation

Stop and report raw paired outcomes if fixture/static native payloads differ,
either blank/whitespace line emits a frame, any parsed JSON field/order/raw byte
is lost, report hashes differ across identical reruns, docs cannot keep current
and historical status distinct, or scope needs a product/model/general-peer
change. Do not add sleeps, weaken exact target assertions, pin unexplained
output, or promote another pending row.

## Required return

Return changed/deleted/generated files, exact normalized outcomes for both
cases, both Batch 15 report hashes, validation results, current registry
counts, documentation-lock result, deviations/blockers, and confirmation that
no files were staged or committed.
