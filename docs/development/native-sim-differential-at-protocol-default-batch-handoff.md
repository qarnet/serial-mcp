# Native Simulator Differential Gate — Batch 13 AT Protocol Default Handoff

> **Historical/superseded record:** Phase F removed active native_sim/NCS
> source and configuration. Body preserves accepted differential evidence.

## Phase goal

Promote exactly one stable characterized native row to an isolated direct
native-versus-fixture differential batch:

```text
native_open_protocol_default_drives_write_and_read
```

This becomes **Compared Batch 13**, not baseline-and-stronger. It directly
proves open-time AT default propagation to bare TX and bare RX. Existing
stateful `AtPeer` fixture coverage remains independently stronger.

## Accepted target evidence

Source row:

```text
tests/native_sim_validation/unix.rs::native_open_protocol_default_drives_write_and_read
```

Stable three-endpoint public-MCP characterization:

```text
target/native-sim-differential/at-protocol-default-characterization.json
sha256: cce2c8a47d3d23eedfb857b5701428937174ab066bd6b64ce20e544776b68775
```

All vectors match after normalizing only endpoint/connection IDs and
`elapsed_ms`. Target call sequence:

1. anonymous `open` at 115200, `profile_mode:"none"`, with exactly the
   open-time default `protocol:{"type":"at_command"}`;
2. public boot-banner literal-match read, no explicit parser/framing;
3. public readiness `transact("arm_cmd 1000")` with no CR in caller payload,
   no call-time protocol/framing/parser, and literal framed match
   `"arm_cmd delay=1000"`;
4. public bare `write("ping")`, with no encoding/protocol/framing field;
5. public bare target `read` with only `from:{"type":"now"}`,
   `timeout_ms:3000`, and `encoding:"utf8"`.

Exact setup evidence:

```text
arm caller bytes=12; decoded_bytes=12; bytes_written=13; encoding=utf8.
arm match data="arm_cmd delay=1000"; match_found; matched=true;
bytes_read/bytes_observed/bytes_returned=18/0/20; match_index=0;
match_frame_index=0; frames_dropped=0.
ping caller bytes=4; decoded_bytes=4; bytes_written=5; encoding=utf8.
```

The AT TX default appended one CR on both setup command and bare ping.

Exact target read:

```text
is_error=false; anonymous; utf8 top-level payload "pong\r\n";
bytes_read/bytes_observed/bytes_returned = 6/0/0;
stop_reason=timeout; no match, truncation, drop, or error;
one frame index 0: type=line, encoding=utf8, payload="pong",
parsed={parser:at_command,response_type:data,command:null,status:null,fields:["pong"]};
position from/next/lost/remaining/start/end = 52/58/0/0/0/58.
```

## Critical matcher detail

`ProtocolPreset::AtCommand` expands to auto line RX framing and AT parser:
`src/framing/config.rs::{preset_tx_framing,preset_rx_framing,preset_rx_parser}`.
`rx_consume::consume_frames` feeds matches per decoded `frame.data`, so auto
line framing strips CRLF before matching. Therefore setup pattern is exact
framed payload `arm_cmd delay=1000`, **not** wire text with `\r\n`.

## Grounded design decisions

- `CompatibilityPeer` already accepts a CR-terminated `ping` and emits
  `pong\r\n`; do **not** change `backend.rs`.
- `ParsedFrameObservation::AtCommand` already models target output; do **not**
  change parser model/normalization.
- Add a third `OpenFraming` form for protocol-only AT defaults. It must set
  `protocol:{"type":"at_command"}` and must not set `rx_framing`.
  Preserve `AtCommandWithExplicitLine` unchanged; it is a separate precedence
  row and intentionally sets `ending:"lf", max_frames:1`.
- Setup calls are validated but excluded from normalized observations. Target
  report contains open, boot read, and target read only.
- No sleep, flush, private fixture API, native-only branch, explicit call-time
  framing/parser/protocol, `max_frames`, `no_new_rx_timeout_ms`, or obsolete
  per-read `max_buffered_bytes`.

## In scope

### `tests/common/native_sim_differential/model.rs`

1. Add `DifferentialCase::AtProtocolDefaultPong` with serde/id
   `native_open_protocol_default_drives_write_and_read`.
2. Add `DifferentialBatch::AtProtocolDefault`.
3. Add `BATCH_THIRTEEN: [Self; 1]`, append it to `ALL`, and update `ALL` length
   31 → 32.
4. Map case to batch.

### `tests/common/native_sim_differential/registry.rs`

Replace only:

```rust
DifferentialRow::pending("native_open_protocol_default_drives_write_and_read")
```

with:

```rust
DifferentialRow::compared(
    "native_open_protocol_default_drives_write_and_read",
    DifferentialBatch::AtProtocolDefault,
    DifferentialCase::AtProtocolDefaultPong,
)
```

Update exact counts:

```text
49 total / 18 compared / 14 baseline-and-stronger / 3 retired / 14 pending
```

No other registry disposition changes.

### `tests/common/native_sim_differential/scenarios.rs`

1. Add:

   ```rust
   pub const AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID: &str =
       "serial-mcp.native-sim-differential.at-protocol-default-batch.v1";
   pub const AT_PROTOCOL_DEFAULT_REPORT_FILENAME: &str =
       "at-protocol-default-batch.json";
   ```

2. Add `run_at_protocol_default_batch()`.
3. Add `AtProtocolDefault` to exhaustive schema-ID, filename,
   reverse-schema, boot-normalization, and boot-assertion matches.
4. Extend `OpenFraming` with `AtCommandDefaults`; in `PublicSession::start`,
   insert only open `protocol:{"type":"at_command"}` for it. Keep standard
   and explicit-line cases unchanged.
5. Assign `AtProtocolDefaultPong` that open option.
6. Add scenario branch:
   - `at_protocol_default_setup()` executes public default-framed
     `transact("arm_cmd 1000")` with stripped frame match string, validates
     every setup field in **Accepted target evidence**, then public bare
     `write("ping")` and validates exact 4 decoded / 5 written UTF-8 result;
   - `read_at_protocol_default()` issues bare target read request exactly as
     characterized and normalizes with `normalize_positioned_read`;
   - exact assertion validates every target field above, including logical
     connection, no error/drops/match/truncation, one exact line AT frame, and
     six-field position.
7. Do not include setup write/transact in `ScenarioOutcome`; retain only open,
   boot read, target read.

### `tests/native_sim_differential.rs`

1. Import Batch 13 runner/constants.
2. Add ignored test:

   ```rust
   async fn at_protocol_default_batch_matches_normalized_public_outcomes() -> Result<()>
   ```

3. Update all exhaustive registry/count/isolated batch checks:
   - expected `18/14/3/14`;
   - `BATCH_THIRTEEN` and `executable_cases(AtProtocolDefault)` equality;
   - append to expected-all chain and require `ALL.len()==32`;
   - direct Compared row assertion for `AtProtocolDefaultPong`, with no
     baseline proof;
   - extend report schema-ID/filename uniqueness checks;
   - update sample report registry counts.

### `tests/doc_drift.rs`

Add Batch 13 compared set/direct row lock; include it in expected compared
rows; update count/current status markers to 18 compared, 14 pending, and
`18/14/3/14`; lock Batch 13 schema, report, bare default semantics,
characterization hash, and 32 covered rows. Retain earlier Batch 8-12 counts
only where explicitly historical.

### Documentation

Update only:

```text
docs/development/native-sim-test-traceability.md
docs/development/native-sim-replacement-research-progress.md
docs/development/native-sim-replacement-recommendation.md
```

State exact Batch 13 source row, direct Compared classification, protocol-only
open default, bare `ping` CR addition (`4→5`), stripped framed arm match
reason, bare target read, `pong\r\n`, `6/0/0`, AT parsed line frame,
`52/58/0/0/0/58`, schema/report, characterization hash, current
`18/14/3/14` counts, 32 covered rows, stronger `AtPeer` proof, and Phase F
blocked. Rewrite Batch 12 current wording as historical before Batch 13 current
status.

### Retire disposable evidence harness

After permanent Batch 13 test passes, delete only:

```text
tests/native_sim_at_protocol_default_characterization.rs
docs/development/native-sim-differential-at-protocol-default-characterization-handoff.md
```

Retain ignored evidence artifact:

```text
target/native-sim-differential/at-protocol-default-characterization.json
```

## Out of scope

- Product, serial/framing/parser implementation, firmware, NCS, CI, xtask,
  release, dependencies, Cargo, lockfiles.
- `CompatibilityPeer`, `AtPeer`, existing native validation, Batch 12,
  existing stronger fixture proof.
- Any other pending row, especially JSON/NDJSON or lifecycle rows.
- Phase F deletion, commits, pushes, merges, PRs, or cleanup of pre-existing
  dirty files.

## Required validation

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked --test native_sim_differential
cargo test --locked --test doc_drift
cargo test --locked --test device_protocol_parity \
  at_command_connection_default_drives_stateful_transact_and_parser_quirk \
  -- --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential \
  at_protocol_default_batch_matches_normalized_public_outcomes \
  -- --ignored --test-threads=1
sha256sum target/native-sim-differential/at-protocol-default-batch.json
# Repeat prior ignored Batch 13 command and sha256sum; hashes must match.
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential -- --ignored --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_open_protocol_default_drives_write_and_read -- --ignored --test-threads=1
```

Use `git diff --check` before return. Do not commit.

## Escalation

Stop and report raw native/fixture results if bare default TX/RX differs,
framed setup matcher behavior differs, a protocol-default open requires an
explicit framing override, any existing batch behavior changes, docs cannot
separate historical/current counts, or another row is needed. Do not weaken
assertions, pin unexplained output, or expand scope.

## Required return

Return changed/deleted/generated files, exact normalized Batch 13 target and
pair result, both report hashes, each validation result, current registry
counts, documentation lock result, deviations/blockers, and confirmation that
no commit was made.
