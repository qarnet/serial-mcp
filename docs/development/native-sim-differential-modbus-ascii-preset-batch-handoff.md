# Native Simulator Differential Gate — Batch 17 Modbus ASCII Preset Handoff

> **Historical/superseded record:** Phase F removed active native_sim/NCS
> source and configuration. Body preserves accepted differential evidence. This
> Modbus batch handoff is retained by design.

## Phase goal

Promote exactly one characterized native row to isolated direct
native-versus-fixture comparison:

```text
native_read_modbus_ascii_preset_decodes_parsed_frame
```

This becomes **Compared Batch 17**, not baseline-and-stronger. It compares the
exact static valid Modbus ASCII firmware `sendraw` response and public
`modbus_ascii` preset result. Existing
`modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation`
remains independent stronger Rust-PTY proof.

## Accepted target evidence

Source row:

```text
tests/native_sim_validation/unix.rs::native_read_modbus_ascii_preset_decodes_parsed_frame
```

Stable three-endpoint public-MCP characterization:

```text
target/native-sim-differential/modbus-ascii-characterization.json
sha256: b390bdc693778be29a06e40033694e1032986a1700995d065f06d8092fc7973c
```

All vectors match after normalizing only endpoint/connection IDs and
`elapsed_ms`. Sequence:

1. standard anonymous `open` at 115200, `profile_mode:"none"`;
2. public boot-banner literal-match read;
3. public `transact("arm_cmd 1000\r\n")`, exact
   `arm_cmd delay=1000\r\n` match;
4. public UTF-8 write, exactly 48 bytes:

   ```text
   sendraw hex 3A30313033303030303030303146420D0A\r\n
   ```

5. public target `read` from now, UTF-8, 3000 ms, only
   `protocol:{"type":"modbus_ascii"}`.

Setup is validated but excluded from `ScenarioOutcome`. No explicit
framing/parser, sleep, flush, or per-call `max_buffered_bytes`.

Exact target:

```text
raw payload: :010300000001FB\r\n
bytes_read/bytes_observed/bytes_returned = 17/0/0
stop_reason=timeout; normal, unmatched, not truncated, no drop/error
position from/next/lost/remaining/start/end = 52/69/0/0/0/69
one UTF-8 start_end frame, index 0, data without marker/terminator:
  010300000001FB
parsed modbus_ascii:
  address=1, function_code=3, data=[0,0,0,1], checksum_valid=true
```

PDU is `01 03 00 00 00 01`; independently calculated Modbus ASCII LRC is
`0xFB`; full `:` + uppercase-hex + `\r\n` frame is 17 bytes.

## Grounded decisions

- `src/framing/config.rs::ProtocolPreset::ModbusAscii` supplies start/end RX
  framing (`:` / `\r\n`), markers excluded, and `ModbusAscii` parser with
  `validate:true`.
- `src/framing/parsers/mod.rs::ModbusAsciiParser` exposes public address,
  function code, decoded byte-array data, and optional checksum validity.
  Batch 17 must normalize all four strictly; static assertion requires
  `Some(true)`.
- Compatibility fixture needs only one static source-derived `sendraw` branch
  after existing arm delay. Do not use generic hex parsing, Modbus server state,
  `rmodbus`, or a global fixture-default change in differential backend. The
  existing separate protocol-parity `rmodbus` proof stays stronger.
- Storage Phase 3 put temporary Modbus characterization inside existing
  `tests/native_sim_differential.rs`. Permanent Batch 17 replaces that nested
  temporary module with typed direct comparison in same target. Do not add a
  top-level target. Keep characterization artifact as protected evidence.

## In scope

### `tests/common/native_sim_differential/model.rs`

1. Add exactly:

   ```rust
   ParsedFrameObservation::ModbusAscii {
       address: u8,
       function_code: u8,
       data: Vec<u8>,
       checksum_valid: Option<bool>,
   }
   ```

2. Extend `optional_parsed_frame` for public `parser:"modbus_ascii"`. Parse
   address/function as range-checked `u8`, `data` as range-checked byte array,
   and checksum as bool/null/absent `Option<bool>`. Add narrow helpers as needed;
   do not coerce invalid values or alter existing parser branches/error behavior
   except unsupported-parser text.
3. Add exactly:

   ```rust
   DifferentialCase::ModbusAsciiPresetRead
   DifferentialBatch::ModbusAsciiPreset
   ```

   serde/id must be `native_read_modbus_ascii_preset_decodes_parsed_frame`.
   Add `BATCH_SEVENTEEN: [Self; 1]`, grow `ALL` 36 → 37 after Batch 16, and
   update all exhaustive `id()` / `batch()` mappings.

### `tests/common/native_sim_differential/backend.rs`

1. Add one public static full response constant:

   ```rust
   pub const MODBUS_ASCII_PRESET_RESPONSE: &[u8] = b":010300000001FB\r\n";
   ```

2. Add only this `CompatibilityPeer::on_command` branch (no CRLF because core
   strips command terminator):

   ```text
   sendraw hex 3A30313033303030303030303146420D0A
   ```

   Emit static response after existing one-shot arm delay. No generic Modbus
   state, decoder, checksum calculator, or default-peer change.

### `tests/common/native_sim_differential/registry.rs`

Replace only pending Modbus row with:

```rust
DifferentialRow::compared(
    "native_read_modbus_ascii_preset_decodes_parsed_frame",
    DifferentialBatch::ModbusAsciiPreset,
    DifferentialCase::ModbusAsciiPresetRead,
)
```

Resulting current counts:

```text
49 total / 23 compared / 14 baseline-and-stronger / 3 retired / 9 pending
```

### `tests/common/native_sim_differential/scenarios.rs`

1. Add Batch 17 public constants:

   ```text
   serial-mcp.native-sim-differential.modbus-ascii-preset-batch.v1
   modbus-ascii-preset-batch.json
   ```

   Add `run_modbus_ascii_preset_batch()`.
2. Complete every exhaustive batch/schema/filename/reverse-schema/open/boot
   match. Update `validate_report` counts to `23/14/3/9`.
3. Add exact command constant including CRLF and setup through:

   ```rust
   arm_and_write_delayed(MODBUS_ASCII_PRESET_SENDRAW_COMMAND, 48)
   ```

4. Add target positioned read using only:

   ```json
   {
     "connection_id":"<current>",
     "from":{"type":"now"},
     "encoding":"utf8",
     "timeout_ms":3000,
     "protocol":{"type":"modbus_ascii"}
   }
   ```

5. Assert every exact fact in accepted target evidence: logical anonymous
   connection, full static raw frame, `17/0/0`, timeout, no
   match/truncation/drop/error, positions `52/69/0/0/0/69`, and exact one
   `start_end` `FrameObservation` with typed `ModbusAscii` parsed data.
6. Retain only open, boot read, and target read. Setup stays validated but
   unmodeled.

### `tests/native_sim_differential.rs`

1. Import Batch 17 runner/constants and add ignored:

   ```rust
   modbus_ascii_preset_batch_matches_normalized_public_outcomes
   ```

2. Update exact counts to `23/14/3/9`; add `BATCH_SEVENTEEN`, append expected
   all, lock `ALL.len()==37`, and lock direct Modbus row with no baseline
   binding.
3. Add focused Modbus mutation proof from typed parsed frame. Changing address,
   function code, byte data, or checksum validity must make normalized outcomes
   unequal.
4. Extend report schema/filename uniqueness and sample-report-count checks for
   Batch 17.
5. Delete only temporary nested `mod modbus_ascii_characterization { ... }`
   after permanent comparison is in place. Do not delete source native row or
   generated characterization artifact.

### `tests/doc_drift.rs`

1. Add `DIFFERENTIAL_BATCH_SEVENTEEN_COMPARED_ROWS`, include it in expected
   direct compared set, and update current counts to `23/14/3/9`.
2. Add direct registry-row guard requiring `ModbusAsciiPreset` and
   `ModbusAsciiPresetRead`, no baseline binding.
3. Extend cross-document marker locks with current Batch 17 schema/report,
   exact frame/48-byte source command, protocol-only Modbus read, `17/0/0`,
   `52/69/0/0/0/69`, parsed fields, characterization path/hash, stronger
   fixture proof, `23/14/3/9`, 37 covered rows, and Phase F blocked.
4. Preserve historical Batch 16 `22/14/3/10` / 36-covered NMEA evidence, but
   do not leave it described as current.

### Documentation

Update only:

```text
docs/development/native-sim-test-traceability.md
docs/development/native-sim-replacement-research-progress.md
docs/development/native-sim-replacement-recommendation.md
```

State Batch 17 is current direct Compared evidence and Batch 16 is historical.
Document protocol-only Modbus read, static valid frame, parsed fields, exact
counters/positions, Batch 17 schema/report, characterization hash, current
`23/14/3/9`, 37 covered rows, stronger Modbus fixture proof, and Phase F
blocked. Preserve previous historical batch evidence.

## Artifact hash workflow

Do not invent report hash. After implementation, run focused ignored Batch 17
once, calculate SHA-256 for `modbus-ascii-preset-batch.json`, insert that exact
value into docs/doc-drift locks, then repeat same command and require equal hash.
The accepted characterization hash is already fixed above.

## Retire temporary evidence harness

After permanent Batch 17 passes, delete only nested temporary characterization
module from `tests/native_sim_differential.rs`. Keep:

```text
target/native-sim-differential/modbus-ascii-characterization.json
```

Keep this Batch 17 handoff and every unrelated existing dirty file.

## Out of scope

- Product framing/parser code, firmware/NCS, CI, xtask, release, dependencies,
  Cargo manifests/lockfiles, generic fixture defaults, or `protocol_peers.rs`.
- Batch 16 and earlier behavior, lifecycle/pending rows, Phase F deletion,
  staging, commits, pushes, merges, PRs, cache cleanup, or storage policy work.

## Required validation

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test native_sim_differential
cargo test --locked --test doc_drift
cargo test --locked --test device_protocol_parity \
  modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation \
  -- --test-threads=1
./firmware/bin/fw-build-native
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential \
  modbus_ascii_preset_batch_matches_normalized_public_outcomes \
  -- --ignored --test-threads=1
sha256sum target/native-sim-differential/modbus-ascii-preset-batch.json
# Repeat previous ignored Batch 17 command and sha256sum; hashes must match.
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential -- --ignored --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_read_modbus_ascii_preset_decodes_parsed_frame -- --ignored --test-threads=1
git diff --check
git status --short
```

## Escalation

Stop and report raw paired outcomes if fixture/static Modbus wire differs,
marker handling or parsed address/function/data/checksum shape differs, model
needs product changes, hash is unstable, docs cannot separate historical/current
counts, or scope needs another pending row. Do not weaken exact raw bytes,
parsed fields, checksum status, counter/position checks, or add sleeps.

## Required return

Return changed/deleted/generated files, exact normalized Batch 17 pair result,
lock result, deviations/blockers, and confirmation nothing was staged or
committed.
