# Native Simulator Differential Gate — Batch 16 NMEA Preset Handoff

> **Historical/superseded record:** Phase F removed active native_sim/NCS
> source and configuration. Body preserves accepted differential evidence.

## Phase goal

Promote exactly one characterized native row to isolated direct
native-versus-fixture comparison:

```text
native_read_nmea0183_preset_decodes_parsed_frame
```

This becomes **Compared Batch 16**, not baseline-and-stronger. It compares the
exact static valid `$GPGGA` firmware `sendraw` response and public
`nmea0183` preset result. Existing
`nmea0183_preset_parses_valid_independently_checksummed_sentence` remains an
independent stronger Rust-PTY proof.

## Accepted target evidence

Source row:

```text
tests/native_sim_validation/unix.rs::native_read_nmea0183_preset_decodes_parsed_frame
```

Stable three-endpoint public-MCP characterization:

```text
target/native-sim-differential/nmea-characterization.json
sha256: 513d4906b285ef35a2b82ab085968de96f51576acc636244f0eb3a44868f1578
```

All vectors match after normalizing only endpoint/connection IDs and
`elapsed_ms`. Sequence:

1. standard anonymous `open` at 115200, `profile_mode:"none"`;
2. public boot-banner literal-match read;
3. public `transact("arm_cmd 1000\r\n")`, exact
   `arm_cmd delay=1000\r\n` match;
4. public UTF-8 write, exactly 148 bytes:

   ```text
   sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A\r\n
   ```

5. public target `read` from now, UTF-8, 3000 ms, only
   `protocol:{"type":"nmea0183"}`.

Setup is validated but excluded from `ScenarioOutcome`. No explicit
framing/parser, sleep, flush, or per-call `max_buffered_bytes`.

Exact target:

```text
raw payload: $GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n
bytes_read/bytes_observed/bytes_returned = 67/0/0
stop_reason=timeout; normal, unmatched, not truncated, no drop/error
position from/next/lost/remaining/start/end = 52/119/0/0/0/119
one UTF-8 start_end frame, index 0, data without marker/terminator:
  GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47
parsed nmea:
  talker_id=GP, sentence_type=GGA, checksum_valid=true
  fields=[123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,"",""]
```

The body is 61 bytes; independent XOR is `0x47`; full sentence is 67 bytes.

## Grounded design decisions

- `src/framing/config.rs::ProtocolPreset::Nmea0183` supplies start/end framing
  with `$`/`!` starts and `\r\n` end, markers excluded, plus NMEA parser with
  `validate:true`.
- `src/framing/parsers/mod.rs::NmeaParser` returns talker ID, sentence type,
  ordered string fields, and optional checksum validity. Batch 16 needs a typed
  differential normalization variant:

  ```rust
  ParsedFrameObservation::Nmea {
      talker_id: String,
      sentence_type: String,
      fields: Vec<String>,
      checksum_valid: Option<bool>,
  }
  ```

  Parse every public field strictly. `checksum_valid` accepts boolean, null, or
  absent into `Option<bool>`; Batch 16's exact assertion requires `Some(true)`.
  Add a private `optional_bool_or_missing` helper rather than coercing values.
  Update unsupported-parser error text. Do not change product parser behavior.
- `CompatibilityPeer` must gain only one exact static `sendraw` branch for the
  source-derived NMEA command, emitting a single static response constant. Its
  existing arm delay applies before the emit. No generic hex parsing, checksum
  calculator, NMEA state machine, or global fixture-default change.
- Existing independent NMEA fixture proof remains stronger; it is not a
  baseline binding.

## In scope

### `tests/common/native_sim_differential/model.rs`

1. Add `ParsedFrameObservation::Nmea` exactly as above. Extend
   `optional_parsed_frame` for public `parser:"nmea"`, preserving talker ID,
   type, all ordered fields, and optional checksum validity. Do not change JSON,
   AT, or raw normalization.
2. Add:

   ```rust
   DifferentialCase::Nmea0183PresetGga
   DifferentialBatch::Nmea0183Preset
   ```

   with serde/id `native_read_nmea0183_preset_decodes_parsed_frame`.
3. Add `BATCH_SIXTEEN: [Self; 1]`, extend `ALL` 35 → 36 after Batch 15, and
   update all exhaustive `id()`/`batch()` mappings.

### `tests/common/native_sim_differential/backend.rs`

1. Add one public exact static response constant for the full 67-byte sentence
   including `$` and `\r\n`.
2. Add only this `CompatibilityPeer::on_command` branch, without CRLF:

   ```text
   sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A
   ```

   Emit the static constant after existing arm-delay behavior.

### `tests/common/native_sim_differential/registry.rs`

Replace only the NMEA pending row with direct comparison:

```rust
DifferentialRow::compared(
    "native_read_nmea0183_preset_decodes_parsed_frame",
    DifferentialBatch::Nmea0183Preset,
    DifferentialCase::Nmea0183PresetGga,
)
```

Resulting current counts:

```text
49 total / 22 compared / 14 baseline-and-stronger / 3 retired / 10 pending
```

### `tests/common/native_sim_differential/scenarios.rs`

1. Add Batch 16 constants:

   ```text
   serial-mcp.native-sim-differential.nmea0183-preset-batch.v1
   nmea0183-preset-batch.json
   ```

2. Add `run_nmea0183_preset_batch()` and complete all exhaustive batch/schema/
   filename/reverse-schema/standard-open/boot matches. Update `validate_report`
   counts to `22/14/3/10`.
3. Add exact command constant including CRLF, setup through
   `arm_and_write_delayed(command, 148)`, and a target read using only:

   ```json
   {
     "connection_id":"<current>",
     "from":{"type":"now"},
     "encoding":"utf8",
     "timeout_ms":3000,
     "protocol":{"type":"nmea0183"}
   }
   ```

4. Target is a positioned normalized read. Assert every exact fact above:
   logical anonymous connection, full raw static sentence, `67/0/0`, timeout,
   no match/truncation/drop/error, positions `52/119/0/0/0/119`, and exact one
   `start_end` `FrameObservation` with `ParsedFrameObservation::Nmea`.
5. Retain only open, boot read, and target read in outcome. Setup stays validated
   but unmodeled.

### `tests/native_sim_differential.rs`

1. Import runner/constants and add ignored
   `nmea0183_preset_batch_matches_normalized_public_outcomes` test.
2. Update exact counts to `22/14/3/10`, add `BATCH_SIXTEEN`, append expected-all,
   lock `ALL.len()==36`, and lock direct NMEA row with no baseline binding.
3. Add focused NMEA mutation proof: starting from a typed NMEA parsed frame,
   changing a talker ID, ordered field, or checksum validity must make outcomes
   unequal. This proves user-visible parser data survives normalization.
4. Extend all Batch report schema/filename uniqueness and sample-report-count
   checks for Batch 16.

### `tests/doc_drift.rs`

1. Add `DIFFERENTIAL_BATCH_SIXTEEN_COMPARED_ROWS`, include it in expected direct
   compared set, and update current counts to `22/14/3/10`.
2. Add direct registry-row guard requiring `Nmea0183Preset` and
   `Nmea0183PresetGga`, no baseline binding.
3. Add exact mapping and lower-validation-disposition lock. Both must use the
   same claim. Lock schema/report, static GGA payload, `67/0/0`,
   `52/119/0/0/0/119`, parsed GP/GGA/checksum-valid fields, characterization
   path/hash, existing stronger NMEA proof, `22/14/3/10`, 36 covered rows, and
   Phase F blocked.
4. Preserve historical Batch 15 `21/14/3/11` with 35 covered rows; do not leave
   it described as current.

### Documentation

Update only:

```text
docs/development/native-sim-test-traceability.md
docs/development/native-sim-replacement-research-progress.md
docs/development/native-sim-replacement-recommendation.md
```

State Batch 16 is current direct Compared evidence and Batch 15 is historical.
Document protocol-only NMEA read, static valid GGA sentence, parser fields,
exact counters/positions, schema/report, characterization hash, current
`22/14/3/10`, 36 covered rows, stronger NMEA fixture proof, and Phase F blocked.

### Retire disposable evidence harness

After permanent Batch 16 passes, delete only:

```text
tests/native_sim_nmea_characterization.rs
docs/development/native-sim-differential-nmea-characterization-handoff.md
```

Keep artifact:

```text
target/native-sim-differential/nmea-characterization.json
```

Keep this Batch 16 handoff and every unrelated existing dirty file.

## Out of scope

- Product framing/parser code, firmware/NCS, CI, xtask, release, dependencies,
  Cargo manifests/lockfiles, schema changes, general fixture defaults, or
  `protocol_peers.rs`.
- Batch 15 and earlier, Modbus/lifecycle/pending rows, Phase F deletion,
  commits, staging, pushes, merges, PRs, and cleanup of pre-existing work.

## Required validation

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test native_sim_differential
cargo test --locked --test doc_drift
cargo test --locked --test device_protocol_parity \
  nmea0183_preset_parses_valid_independently_checksummed_sentence \
  -- --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential \
  nmea0183_preset_batch_matches_normalized_public_outcomes \
  -- --ignored --test-threads=1
sha256sum target/native-sim-differential/nmea0183-preset-batch.json
# Repeat previous ignored Batch 16 command and sha256sum; hashes must match.
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential -- --ignored --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_read_nmea0183_preset_decodes_parsed_frame -- --ignored --test-threads=1
git diff --check
```

## Escalation

Stop and report raw paired outcomes if fixture/static NMEA wire differs, marker
handling or parsed talker/type/field/checksum shape differs, model needs product
changes, artifact hash is unstable, docs cannot separate historic/current counts,
or scope needs another pending row. Do not weaken exact raw bytes, parser fields,
checksum status, counter/position checks, or add sleeps.

## Required return

Return changed/deleted/generated files, exact normalized Batch 16 pair result,
both report hashes, each validation result, final registry counts, documentation
lock result, deviations/blockers, and confirmation that nothing was staged or
committed.
