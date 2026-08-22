# Native Simulator Differential Gate — NMEA Characterization Handoff

## Goal

Measure one pending native row before permanent fixture/model/registry work:

```text
native_read_nmea0183_preset_decodes_parsed_frame
```

Create a disposable native-only public-MCP characterization test and stable
target artifact. Evidence only. Do not implement Batch 16 yet.

## Grounding

- Native source row:
  `tests/native_sim_validation/unix.rs::native_read_nmea0183_preset_decodes_parsed_frame`.
- `ProtocolPreset::Nmea0183` uses start/end RX framing with `$` and `!` starts,
  `\r\n` end, markers excluded, plus NMEA parser with checksum validation:
  `src/framing/config.rs`.
- `NmeaParser` strips framing markers defensively, parses talker/type/fields,
  and requires a valid XOR checksum under the preset:
  `src/framing/parsers/mod.rs`.
- Existing independent stronger fixture proof:
  `tests/device_protocol_parity.rs::nmea0183_preset_parses_valid_independently_checksummed_sentence`.
  Do not change it.
- The existing typed differential model deliberately lacks NMEA parser support.
  That is expected for characterization; do not change it in this phase.

## Scope

Add only:

```text
tests/native_sim_nmea_characterization.rs
```

At test runtime, write only:

```text
target/native-sim-differential/nmea-characterization.json
```

Add this handoff only. Do not change permanent differential model, registry,
fixture, product code, firmware, NCS, docs, CI, dependencies, or versions.

## Exact wire stimulus

Use the native source's exact body:

```text
GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,
```

Compute its XOR checksum in the test with an independent small byte-fold helper.
It must produce `47`, so the emitted sentence is exactly:

```text
$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n
```

The body is 61 bytes, sentence is 67 bytes, and uppercase `sendraw` command is
exactly:

```text
sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A\r\n
```

Assert caller UTF-8 `write` has `bytes_written=decoded_bytes=148`.

## Public procedure

Run three fresh native_sim endpoints, each with fresh firmware, server, and
modern client. For every endpoint:

1. Standard anonymous `open` at 115200 with `profile_mode:"none"`, `name:null`.
2. Public literal boot-banner read. Retain it.
3. Public `transact("arm_cmd 1000\r\n")` with exact literal
   `arm_cmd delay=1000\r\n` match. Validate setup, exclude it from retained
   observations.
4. Public UTF-8 `write` of the exact `sendraw` command above. Validate setup,
   exclude it from retained observations.
5. Public target `read`:

   ```json
   {
     "connection_id":"<current>",
     "from":{"type":"now"},
     "encoding":"utf8",
     "timeout_ms":3000,
     "protocol":{"type":"nmea0183"}
   }
   ```

No explicit framing/parser, `max_frames`, silence timeout, sleep, flush, or
obsolete per-call `max_buffered_bytes`. The public arm barrier replaces source
test sleep/flush timing.

## Required structural assertions

Before serializing each normalized endpoint vector, assert:

- normal target result and UTF-8 complete raw sentence data;
- one ordered frame with `frame_type:"start_end"`, UTF-8 encoding, and data
  equal to sentence body plus `*47` but without `$` or `\r\n` markers;
- parsed object has `parser:"nmea"`, `talker_id:"GP"`,
  `sentence_type:"GGA"`, `checksum_valid:true`, and exact ordered NMEA fields:

  ```text
  123519, 4807.038, N, 01131.000, E, 1, 08, 0.9, 545.4, M, 46.9, M, "", ""
  ```

- no unexpected error or dropped frame.

Do not predict or hard-code target counters, stop reason, offsets, or any other
native result field before measurement. Preserve every returned structured field
in the artifact after normalization.

## Normalization and artifact

Normalize only runtime connection ID/endpoint and recursively remove only
`elapsed_ms`. The three endpoint vectors must match exactly. Use canonical
pretty JSON with trailing newline:

```json
{
  "schema_id":"serial-mcp.native-sim-nmea-characterization.v1",
  "case":"native_read_nmea0183_preset_decodes_parsed_frame",
  "outcomes":[...],
  "omitted_dynamic_fields":["elapsed_ms"]
}
```

## Required validation

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked --test device_protocol_parity \
  nmea0183_preset_parses_valid_independently_checksummed_sentence \
  -- --test-threads=1
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_nmea_characterization \
  records_native_nmea_public_outcomes -- --ignored --test-threads=1
sha256sum target/native-sim-differential/nmea-characterization.json
# Repeat exact ignored test and sha256sum. Hashes must match.
SERIAL_MCP_NATIVE_SIM_BIN="$PWD/build/native_sim/firmware/zephyr/zephyr.exe" \
  cargo test --locked --test native_sim_validation \
  native_read_nmea0183_preset_decodes_parsed_frame -- --ignored --test-threads=1
git diff --check
```

## Out of scope

- Permanent Batch 16, `ParsedFrameObservation::Nmea`, compatibility-peer work,
  registry/doc-drift/documentation changes, Modbus and every other pending row.
- Product parser/framing code, firmware/NCS, CI/xtask/release/dependencies,
  Cargo/lockfiles, commits, pushes, merges, PRs, and cleanup of existing dirty
  worktree content.

## Escalation

Stop with raw evidence if any endpoint differs, checksum/marker/parser shape
differs, target needs sleep/flush, or permanent model/fixture/product changes
appear necessary. Do not weaken field/order/marker assertions or invent target
counters/offsets.

## Required return

No commit. Return changed files, complete normalized target result, both hashes,
validation results, deviations/blockers, and current status.
