# Native Simulator Differential Gate — Batch 12 AT Parser Handoff

> **Historical/superseded record:** Phase F removed active native_sim/NCS
> source and configuration. Body preserves accepted differential evidence.

## Phase goal

Promote exactly one stable characterized native row to an isolated direct
native-versus-fixture differential batch:

```text
native_read_at_parser_parses_pong
```

This becomes **Compared Batch 12**, not baseline-and-stronger. The richer AT
fixture proof remains independently required; this batch compares the exact
explicit-parser public result observed from native firmware.

## Accepted target evidence

Source row:

```text
tests/native_sim_validation/unix.rs::native_read_at_parser_parses_pong
```

Stable three-endpoint public-MCP characterization:

```text
target/native-sim-differential/at-parser-characterization.json
sha256: 52b573c8a71da8aa52fa6ce12ce81d63f5f30756839ae8db3a1e4e56a6424eb5
```

All vectors match after normalizing only endpoint/connection IDs and
`elapsed_ms`. Target call sequence:

1. standard anonymous `open` at 115200, `profile_mode:"none"`;
2. public boot-banner literal-match read;
3. public `transact("arm_cmd 1000\r\n")` with exact
   `arm_cmd delay=1000\r\n` match;
4. public UTF-8 `write("ping\r\n")`, with
   `bytes_written=decoded_bytes=6`;
5. public target `read` with:

   ```json
   {
     "from":{"type":"now"},
     "encoding":"utf8",
     "timeout_ms":3000,
     "rx_framing":{"type":"line"},
     "rx_parser":{"type":"at_command"}
   }
   ```

Setup calls are validated but excluded from the normalized result.

Exact target read:

```text
is_error=false; anonymous; utf8 top-level payload "pong\r\n";
bytes_read/bytes_observed/bytes_returned = 6/0/0;
stop_reason=timeout; no match, truncation, drop, or error;
one frame index 0: type=line, encoding=utf8, payload="pong",
parsed={parser:at_command,response_type:data,command:null,status:null,fields:["pong"]};
position from/next/lost/remaining/start/end = 52/58/0/0/0/58.
```

`next_offset=58` is raw-consumption cursor behavior, even though framed
`bytes_returned=0`.

## Grounded design decisions

- `CompatibilityPeer` already supports exact `arm_cmd 1000` followed by
  `ping` → `pong\r\n`; do **not** change `backend.rs`.
- `ParsedFrameObservation::AtCommand` already models every target parser field;
  do **not** alter `model.rs` parsing shape or add a new parsed-frame variant.
- `tests/device_protocol_parity.rs::at_command_connection_default_drives_stateful_transact_and_parser_quirk`
  remains stronger AT semantic coverage using `AtPeer`. Do not weaken, replace,
  or bind it as a baseline proof.
- The one-second public arm barrier replaces source test sleep/flush behavior.
  Do not use sleep, flush, private fixture scripting, raw fixture APIs,
  native-only branches, `max_frames`, `no_new_rx_timeout_ms`, a protocol preset,
  or the obsolete per-read `max_buffered_bytes` field.
- `from:{"type":"now"}` begins before delayed `pong` output and excludes all
  setup traffic. Both endpoints must use the same public setup.

## In scope

### `tests/common/native_sim_differential/model.rs`

1. Add `DifferentialCase::AtParserPong` with serde/id value
   `native_read_at_parser_parses_pong`.
2. Add `DifferentialBatch::AtParser`.
3. Add `BATCH_TWELVE: [Self; 1]`, append it to `ALL`, and update `ALL` length
   from 30 to 31.
4. Map `AtParserPong` to `AtParser` in `batch()`.

### `tests/common/native_sim_differential/registry.rs`

Replace only this pending row:

```rust
DifferentialRow::pending("native_read_at_parser_parses_pong")
```

with direct comparison membership:

```rust
DifferentialRow::compared(
    "native_read_at_parser_parses_pong",
    DifferentialBatch::AtParser,
    DifferentialCase::AtParserPong,
)
```

Update current global assertion to exactly:

```text
49 total / 17 compared / 14 baseline-and-stronger / 3 retired / 15 pending
```

No other registry status changes.

### `tests/common/native_sim_differential/scenarios.rs`

1. Add constants:

   ```rust
   pub const AT_PARSER_REPORT_SCHEMA_ID: &str =
       "serial-mcp.native-sim-differential.at-parser-batch.v1";
   pub const AT_PARSER_REPORT_FILENAME: &str = "at-parser-batch.json";
   ```

2. Add `run_at_parser_batch()` calling `run_batch(DifferentialBatch::AtParser)`.
3. Add `AtParser` to all exhaustive schema-ID, filename, reverse-schema,
   standard-open, boot normalization, and boot assertion matches.
4. Add `AtParserPong` scenario branch. It must:
   - use standard anonymous open;
   - run same public arm/write setup as characterized;
   - record only open, boot read, and target read;
   - call a dedicated target helper with exact explicit line framing and
     `at_command` parser configuration above;
   - normalize through `normalize_positioned_read`;
   - assert every target field listed in **Accepted target evidence**, including
     exact `FrameObservation` and `ParsedFrameObservation::AtCommand` values.
5. Existing `arm_and_write_raw` is the shared delayed public arm/write helper
   for Batches 8-11. Rename it to neutral `arm_and_write_delayed` and update its
   three existing callers. Keep exact public behavior and all existing B8-B11
   target assertions unchanged. Reuse it for `at_parser_setup("ping\r\n", 6)`;
   make internal argument names/error labels command-neutral.
6. No extra peer behavior, no parser model change, no fixture API change.

### `tests/native_sim_differential.rs`

1. Import Batch 12 runner/constants.
2. Add ignored Linux differential test:

   ```rust
   async fn at_parser_batch_matches_normalized_public_outcomes() -> Result<()>
   ```

   It writes and checks `at-parser-batch.json` exactly like B8-B11 tests.
3. Update registry/count/isolated-batch tests:
   - expected count `17/14/3/15`;
   - `BATCH_TWELVE` equality against `executable_cases(AtParser)`;
   - append Batch 12 to expected-all chain; `ALL.len()==31`;
   - direct Compared row assertion for `AtParserPong` with no baseline proof;
   - extend schema-ID and filename uniqueness tests so every report remains
     distinct;
   - update sample report registry counts.

### `tests/doc_drift.rs`

Extend native differential locks. Add exactly one Batch 12 compared-row set and
direct-row check for `AtParser` / `AtParserPong`; add it to expected compared
set; update current pending and status markers to 15 and `17/14/3/15`; lock
Batch 12 schema/report/target markers and 31 covered rows. Preserve historical
Batch 8-11 counts as explicitly historical, not current.

### Documentation

Update only:

```text
docs/development/native-sim-test-traceability.md
docs/development/native-sim-replacement-research-progress.md
docs/development/native-sim-replacement-recommendation.md
```

Required facts in all appropriate current-status sections:

```text
Batch 12
native_read_at_parser_parses_pong
serial-mcp.native-sim-differential.at-parser-batch.v1
at-parser-batch.json
17 compared, 14 baseline-and-stronger, 3 retired, 15 pending
17/14/3/15
31 covered rows
```

Document exact public arm/write/read setup, explicit `rx_framing: line` plus
`rx_parser: at_command`, target `pong\r\n`, `6/0/0`, timeout, exact parser
frame, and `52/58/0/0/0/58`. State this is direct explicit-parser evidence;
existing `AtPeer` fixture test remains stronger stateful AT behavior. Rewrite
Batch 11 current-status language as a historical checkpoint before adding
Batch 12 current status. Keep Phase F blocked.

### Retire disposable evidence harness

After permanent Batch 12 test passes, delete only:

```text
tests/native_sim_at_parser_characterization.rs
docs/development/native-sim-differential-at-parser-characterization-handoff.md
```

Keep ignored target evidence:

```text
target/native-sim-differential/at-parser-characterization.json
```

## Out of scope

- Product, server, serial, framing/parser, firmware, NCS, CI, xtask, release,
  dependency, Cargo, and lockfile changes.
- `CompatibilityPeer`, `AtPeer`, existing native validation, and stronger AT
  fixture proof changes.
- Any other pending registry row, especially
  `native_open_protocol_default_drives_write_and_read`.
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
  at_parser_batch_matches_normalized_public_outcomes -- --ignored --test-threads=1
sha256sum target/native-sim-differential/at-parser-batch.json
# Repeat previous ignored Batch 12 command and sha256sum; hashes must match.
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_differential -- --ignored --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_read_at_parser_parses_pong -- --ignored --test-threads=1
```

Use `git diff --check` before return. Do not commit.

## Escalation

Stop and report raw native/fixture results if target differs, setup needs a
sleep/flush or special fixture API, new parser modeling is required, any
existing batch changes behavior, docs locks cannot state both historical and
current counts accurately, or another pending row is needed to make this work.
Do not weaken assertions, pin unexplained output, or expand scope.

## Required return

Return changed/deleted/generated files, exact normalized Batch 12 target and
pair result, both report hashes, each validation result, current registry
counts, documentation lock result, deviations/blockers, and confirmation that
no commit was made.
