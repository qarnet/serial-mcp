# `native_sim` Replacement Research Progress

**Checkpoint status:** Stages 0 through 5 research complete, user approved full
staged migration, and Phases A-E are complete on 2026-08-13. The full normalized
differential parity window is accepted on 2026-08-23. Phase F removed active
`native_sim`, firmware, NCS, CI native-job, Nix, and firmware-asset coupling;
retained batch material is historical evidence. Fresh clean-checkout CI
acceptance and ADR acceptance remain pending.

Current platform scope: production-path real-PTY fixture tests are Linux-only
because macOS `serialport` baud configuration invokes `IOSSIOSPEED`, which macOS
PTYs reject with `ENOTTY`. macOS still gets normal Rust fmt/build/test/clippy
and controlled-backend tests; Linux-only fixture targets compile as zero tests
there.

> Local worktree verification on 2026-08-23 passed `nix flake check --accept-flake-config` and `nix develop --ignore-env`; the shell found no `west`, `nrfutil`, or `nrfutil-sdk-manager` on `PATH` before `cargo test --locked`. This is not fresh clean-checkout CI evidence.

This file is a resumable migration checkpoint. It records evidence already
collected, conclusions, experiment failures, and next work. Public PTY
peer-disconnect behavior and reusable direct-nix fixture foundation are now
implemented. Durable protocol peers and fixture-backed public MCP preset parity
now cover all seven shipped presets. Phase E makes replacement targets required
CI evidence and adds deterministic 100-iteration public-boundary repeat gate.
Batch 6 extends executable ACK state-machine differential comparison, Batch 7
adds direct output-flush comparison, Batch 8 adds direct raw SLIP happy-path
comparison, and Batch 9 adds direct malformed raw SLIP baseline-and-stronger
comparison. Batch 10 adds direct raw SLIP malformed-then-recovery comparison,
and Batch 11 adds direct COBS-preset comparison with a static independent wire;
Batch 12 adds direct AT-parser comparison as historical evidence; Batch 13 adds
direct AT protocol-default comparison as historical evidence; Batch 14 adds
direct JSON-parser comparison as historical evidence; Batch 15 adds direct
NDJSON-preset comparison as historical evidence; Batch 16 adds direct
NMEA-0183 preset comparison as historical evidence; Batch 17 adds direct
Modbus ASCII preset comparison as historical evidence; Batch 18 adds direct
close-while-read comparison as historical evidence; Batch 19 adds a
reopen/fresh-output baseline-and-stronger comparison as historical evidence;
Batch 20 adds a flush-during-armed-delay baseline-and-stronger comparison as
historical evidence; Batch 21 adds a direct input-flush backlog comparison as
historical evidence; Batch 22 adds a direct bootloader-touch process-exit
comparison as historical evidence. The accepted full normalized parity window
covers all 22 batches and all 42 executable cases; Phase F then removed active
native/NCS source and configuration.

Batch 1 remains an isolated eight-scenario command/lifecycle batch through identical
public MCP code against native firmware and the Rust PTY fixture. Batch 2 retains
six generic matching/framing scenarios, Batch 3 retains five raw generic-framing
scenarios, Batch 4 adds two flood/buffer scenarios, Batch 5 adds three
command-diagnostic scenarios, Batch 6 adds one ACK state-machine scenario,
Batch 7 adds one output-flush scenario, Batch 8 adds one raw SLIP happy-path
scenario, Batch 9 adds one malformed raw SLIP baseline-and-stronger scenario,
and Batch 10 adds one raw SLIP recovery scenario, each with separate
schema/report output. Batch 11 adds one COBS-preset scenario with its own
schema/report output as a historical checkpoint. Batch 12 adds one AT-parser
scenario with its own schema/report output as a historical checkpoint. Batch 13
adds one protocol-default scenario with its own schema/report output as a
historical checkpoint. Batch 14 adds one JSON-parser scenario with its own
schema/report output. Batch 15 adds two NDJSON-preset scenarios with its own
schema/report output. Batch 16 adds one NMEA-0183 preset scenario with its own
schema/report output; Batch 17 adds one Modbus ASCII preset scenario with its own
schema/report output; Batch 18 adds one close-while-read scenario with its own
schema/report output; Batch 19 adds one reopen/fresh-output scenario with its own
schema/report output; Batch 20 adds one flush-during-armed-delay scenario with its
own schema/report output; Batch 21 adds one input-flush backlog scenario with its
own schema/report output; Batch 22 adds one bootloader-touch process-exit scenario
with its own schema/report output. Global registry status is **26 compared, 16
baseline-and-stronger, 7 retired, and 0 pending** rows. Pending live reads use
fixed 100 ms baseline-only delay plus independent exact-modern client for later
public write, avoiding same-transport scheduling artifact. Required fixture
proofs retain stronger readiness, exact frame-limit, framed-match, matcher, and
call-time precedence obligations. Batch 3 raw RX uses measured one-second
delayed `sendraw` output and exact target-only wire, while TX uses independent
peer-wire observations. Batch 4 uses measured live `spam 1024 hex` and
`spam 512 hex` output with source-derived xorshift bytes and a connection-level
256-byte buffer default. This is bounded historical migration evidence; the
accepted full normalized parity window covers all 22 batches and all 42
executable cases. Fresh clean-checkout CI acceptance remains pending.
Historical Batch 21 counts are 25 compared, 16 baseline-and-stronger, 3 retired, and 5
pending (`25/16/3/5`) with 41 covered rows. Current status is **26 compared, 16 baseline-and-stronger, 7 retired, and 0 pending** (`26/16/7/0`) with 46 covered rows. Historical Batch 20 counts are 24 compared, 16 baseline-and-stronger, 3 retired, and 6 pending (`24/16/3/6`). Batch 20 historical registry status is **24 compared, 16 baseline-and-stronger, 3 retired, and 6 pending** rows. Historical Batch 19 status was 24 compared, 15 baseline-and-stronger, 3 retired, and 7 pending (`24/15/3/7`) with 39 covered
rows. Historical Batch 18 status was 24 compared, 14
baseline-and-stronger, 3 retired, and 8 pending (`24/14/3/8`) with 38 covered
rows. Historical Batch 16 status was 22 compared, 14
baseline-and-stronger, 3 retired, and 10 pending (`22/14/3/10`) with 36 covered
rows. Historical Batch 15 status was 21 compared, 14
baseline-and-stronger, 3 retired, and 11 pending (`21/14/3/11`) with 35 covered
rows.

Batch 12 is direct explicit-parser evidence for
`native_read_at_parser_parses_pong`. Both native and fixture endpoints use
standard anonymous public `open` at 115200 with `profile_mode: "none"`, public
boot-banner literal-match `read`, public `transact("arm_cmd 1000\r\n")` matching
exact `arm_cmd delay=1000\r\n`, and public UTF-8 `write("ping\r\n")` with
`bytes_written=decoded_bytes=6`. Target `read` uses
`from={"type":"now"}`, `encoding="utf8"`, `timeout_ms=3000`, explicit
`rx_framing: line`, and explicit `rx_parser: at_command`; setup calls are
validated but excluded from normalized output. The one-second public arm barrier
replaces source-test sleep/flush behavior.

Target result is anonymous, normal UTF-8 `pong\r\n`, with
`bytes_read/bytes_observed/bytes_returned=6/0/0`, `stop_reason=timeout`, no
match, truncation, drop, or error, one index-0 `line` frame with payload `pong`,
and parsed
`{"parser":"at_command","response_type":"data","command":null,"status":null,"fields":["pong"]}`.
Positions are `52/58/0/0/0/58` in
`from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
order. Target characterization remains at
`target/native-sim-differential/at-parser-characterization.json` with SHA-256
`52b573c8a71da8aa52fa6ce12ce81d63f5f30756839ae8db3a1e4e56a6424eb5`.
Existing `at_command_connection_default_drives_stateful_transact_and_parser_quirk`
`AtPeer` coverage remains stronger stateful AT behavior. Batch 12 is historical;
Batch 13 is historical at 18 compared, 14 baseline-and-stronger, 3 retired, and
14 pending (`18/14/3/14`) with 32 covered rows; Batch 14 is historical at
`19/14/3/13` with 33 covered rows; Batch 15 and Batch 16 are historical and
Batch 17, Batch 18, Batch 19, Batch 20, and Batch 21 are historical; Batch 22 is
historical direct Compared evidence. Phase F remains blocked.

Batch 13 is historical direct Compared evidence for
`native_open_protocol_default_drives_write_and_read`. Open carries the
protocol-only default `protocol: {"type":"at_command"}` and no `rx_framing`.
Setup uses default-framed `transact("arm_cmd 1000")` with stripped framed arm match
`arm_cmd delay=1000`, then bare `ping` adds CR (`4→5`). Target bare read
uses only `from={"type":"now"}`, `timeout_ms=3000`, and UTF-8 encoding. Target
is `pong\r\n`, with `bytes_read/bytes_observed/bytes_returned=6/0/0`, no match,
drop, truncation, or error, one AT-parsed UTF-8 line frame with fields
`["pong"]`, and positions `52/58/0/0/0/58`.
Schema is `serial-mcp.native-sim-differential.at-protocol-default-batch.v1`,
report is `at-protocol-default-batch.json`, and characterization is
`target/native-sim-differential/at-protocol-default-characterization.json` with
SHA-256
`cce2c8a47d3d23eedfb857b5701428937174ab066bd6b64ce20e544776b68775`.
the stronger `AtPeer` proof remains required. Batch 14 is historical direct
Compared evidence for `native_read_json_parser_decodes_jsonout`. Both endpoints use
standard anonymous public `open` at 115200 with `profile_mode: "none"`, public
boot-banner literal-match `read`, public `transact("arm_cmd 1000\r\n")` matching
exact `arm_cmd delay=1000\r\n`, and public UTF-8 `write("jsonout\r\n")` with
`bytes_written=decoded_bytes=9`. Target `read` uses `from={"type":"now"}`,
UTF-8, 3000 ms, explicit `rx_framing: {"type":"line"}`, and explicit
`rx_parser: {"type":"json_lines"}`. The static three JSON object response is
140 bytes. The normalized target has `bytes_read/bytes_observed/bytes_returned`
of `140/0/0`, stop reason `timeout`, no match, truncation, drops, or error, and
three ordered parsed objects in UTF-8 line frames for `temp`, `humidity`, and
`pressure`. Positions are `52/192/0/0/0/192`. Schema is
`serial-mcp.native-sim-differential.json-parser-batch.v1`, report is
`json-parser-batch.json`, and characterization is
`target/native-sim-differential/json-parser-characterization.json` with SHA-256
`f51b5d77bac3904d214e2ea76794cf1d10f4d5aa8849224e750af30a8e9e3a06`. Existing
`json_lines_preset_writes_line_and_preserves_object_only_parser_behavior` fixture
proof remains stronger. The existing stronger JSON Lines fixture proof remains
stronger. Batch 14 is historical; Batch 15 is historical direct Compared
evidence for two NDJSON-preset rows. Batch 16 is historical direct Compared
evidence for one NMEA-0183 preset row. Batch 17 is historical direct Compared
evidence for one Modbus ASCII preset row. Batch 18 is historical direct Compared
evidence for one close-while-read row; Batch 19 is historical baseline-and-stronger
evidence for one reopen/fresh-output row; Batch 20 is historical
baseline-and-stronger evidence for one flush-during-armed-delay row; Batch 21 is
historical direct Compared evidence for one input-flush backlog row; Batch 22 is
historical direct Compared evidence for one bootloader-touch process-exit row.

Batch 15 compares `native_read_ndjson_preset_decodes_json_frames` and
`native_read_ndjson_preset_skips_empty_lines` through standard anonymous public
`open` at 115200 with `profile_mode: "none"`, boot-banner literal-match `read`,
public `transact("arm_cmd 1000\r\n")` readiness, and exact static UTF-8
`sendraw` writes. Target `read` starts at `from={"type":"now"}`, uses UTF-8
and 3000 ms, and supplies only `protocol: {"type":"ndjson"}`. Preset semantics
are auto line framing, `skip_empty:true`, and the JSON parser; no explicit
framing/parser, sleep, or flush is used.

The two raw payloads are `{"a":1}\n\n{"b":2}\n` and
`{"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n`, sent by
`sendraw hex 7B2261223A317D0A0A7B2262223A327D0A` and
`sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A`.
Exact normalized outcomes are
`17/0/0` with positions `52/69/0/0/0/69` and `30/0/0` with positions
`52/82/0/0/0/82`, respectively. Both stop by `timeout`, retain exact UTF-8 raw
payload, emit ordered JSON frames only for records, and have no match,
truncation, drops, or error; blank and whitespace-only lines emit no frames.
Schema is `serial-mcp.native-sim-differential.ndjson-preset-batch.v1`, report is
`ndjson-preset-batch.json`, and characterization is
`target/native-sim-differential/ndjson-characterization.json` with SHA-256
`10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3`. The
`ndjson_preset_parses_records_and_skips_blank_whitespace_lines` fixture proof
remains stronger. Batch 15 historical checkpoint is `21/14/3/11` with 35
covered rows; Phase F remains blocked.

Batch 16 NMEA-0183 preset differential batch is complete:

- exact row is `native_read_nmea0183_preset_decodes_parsed_frame`, a direct
  Compared row in isolated `Nmea0183Preset` batch membership with no
  baseline-proof binding;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`, public boot-banner literal-match `read`, public
  `transact("arm_cmd 1000\r\n")` matching exact `arm_cmd delay=1000\r\n`, and
  public UTF-8 `write` of the exact 148-byte command
  `sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A\r\n`;
- target `read` uses `from={"type":"now"}`, UTF-8, 3000 ms, and only
  `protocol: {"type":"nmea0183"}`. The preset supplies start/end framing with
  markers excluded and NMEA parsing; no explicit framing/parser, sleep, flush,
  or generic `sendraw` parser is used;
- static raw wire is the exact 67-byte valid sentence
  `$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n`.
  The normalized target has `67/0/0` bytes read/observed/returned, stop reason
  `timeout`, no match, truncation, drops, or error, one UTF-8 `start_end` frame,
  parsed `talker_id="GP"`, `sentence_type="GGA"`, ordered fields
  `["123519","4807.038","N","01131.000","E","1","08","0.9","545.4","M","46.9","M","",""]`,
  and `checksum_valid:true`; positions are `52/119/0/0/0/119`;
- schema is `serial-mcp.native-sim-differential.nmea0183-preset-batch.v1`, report
  is `nmea0183-preset-batch.json`, and retained characterization is
  `target/native-sim-differential/nmea-characterization.json` with SHA-256
  `513d4906b285ef35a2b82ab085968de96f51576acc636244f0eb3a44868f1578`;
- existing `nmea0183_preset_parses_valid_independently_checksummed_sentence`
  fixture proof remains stronger. Historical Batch 16 registry was `22/14/3/10`
  with 36 covered rows; Phase F blocked.

Batch 17 Modbus ASCII preset differential batch is complete:

- exact row is `native_read_modbus_ascii_preset_decodes_parsed_frame`, a direct
  Compared row in isolated `ModbusAsciiPreset` batch membership with no
  baseline-proof binding;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`, public boot-banner literal-match `read`, public
  `transact("arm_cmd 1000\r\n")` matching exact `arm_cmd delay=1000\r\n`, and
  public UTF-8 `write` of the exact 48-byte command
  `sendraw hex 3A30313033303030303030303146420D0A\r\n`;
- target `read` uses `from={"type":"now"}`, UTF-8, 3000 ms, and only
  `protocol: {"type":"modbus_ascii"}`. The preset supplies start/end framing
  with markers excluded and Modbus ASCII parsing; no explicit framing/parser,
  sleep, flush, or generic `sendraw` parser is used;
- static raw wire is the exact 17-byte valid response `:010300000001FB\r\n`.
  The normalized target has `17/0/0` bytes read/observed/returned, stop reason
  `timeout`, no match, truncation, drops, or error, one UTF-8 `start_end` frame,
  parsed `address=1`, `function_code=3`, `data=[0,0,0,1]`, and
  `checksum_valid:true`; positions are `52/69/0/0/0/69`;
- schema is `serial-mcp.native-sim-differential.modbus-ascii-preset-batch.v1`,
  report is `modbus-ascii-preset-batch.json` with SHA-256
  `97f9c00dc98e5cd83c440b2e90b7d9f72e58428f9e873f7f418475ef3a79ef9b`, and characterization is
  `target/native-sim-differential/modbus-ascii-characterization.json` with
  SHA-256 `b390bdc693778be29a06e40033694e1032986a1700995d065f06d8092fc7973c`;
- existing `modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation`
  fixture proof remains stronger. Historical Batch 17 registry is `23/14/3/9`
  with 37 covered rows; Phase F blocked.

Historical Batch 18 close-while-read differential batch is complete:

- exact row is `native_close_while_read_active_returns_normal_result`, a direct
  Compared row in isolated `CloseWhileRead` batch membership with no
  baseline-proof binding;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`, public boot-banner literal-match `read`, public
  `transact("arm_cmd 1000\r\n")` matching exact `arm_cmd delay=1000\r\n`, and
  public UTF-8 marker write of the exact command
  `sendraw hex 524541442D52454144592D4D41524B45520D0A\r\n`;
- the secondary modern client starts an unmatched `from={"type":"now"}` read
  with `encoding: "utf8"`, `timeout_ms: 3000`, and
  `NEVER-CLOSE-MATCH`; the primary client polls normalized `get_status.rx_bytes`
  until marker progress increases by 19 and proves the pending read remains unfinished before close;
- primary `close` returns normal anonymous `source="disabled"` profile with
  `profile_persistence.operation="close_snapshot"` and
  `profile_persistence.state="transient"`; pending read returns exact
  `READ-READY-MARKER\r\n` with `bytes_read/bytes_observed/bytes_returned` of
  `19/19/19`, `stop_reason="connection_closed"`, unmatched, no truncation,
  frames, drops, or error, and positions `52/71/0/0/0/71`;
- schema is `serial-mcp.native-sim-differential.close-while-read-batch.v1`,
  report is `close-while-read-batch.json` with SHA-256
  `fef0499d6504635c104fe3d125fe572c49e2ef4bec69a5b39c32bdeb50361a09`, and
  retained characterization is
  `target/native-sim-differential/close-while-read-characterization.json` with
  SHA-256 `06a7adb2b4f1c1c6f8b3c8fd507ba9e5004df6dd7a9f4ef6b64c4fff87c3f69b`;
- existing `close_interrupts_readiness_proven_live_read_with_connection_closed`
  fixture proof remains stronger. Historical Batch 18 registry was `24/14/3/8`
  with 38 covered rows; Phase F blocked.

**Historical Baseline-and-stronger Batch 19** reopen/fresh-output differential batch is
complete:

- exact row is `native_reopen_then_match_finds_fresh_output`, a
  baseline-and-stronger row in isolated `ReopenFreshOutput` batch membership;
  its required stronger proof is
  `reopen_same_path_returns_distinct_id_and_only_fresh_generation`;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`; firmware boot output is synchronized only on first
  open because it is emitted once at process start, not on reopen;
- first public `write("ping\r\n")` is normal UTF-8 with
  `bytes_written=decoded_bytes=6`; first positioned literal `pong` read returns
  exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/
  truncation, and positions `32/38/0/0/0/38`; first close retains the exact
  disabled/transient anonymous close-profile checks;
- second open uses the same endpoint path and requires a distinct raw
  connection ID. A public status-only probe requires `rx_bytes=0` before the
  second command and is excluded from normalized observations;
- second pending `from={"type":"now"}` UTF-8 literal-match read uses the
  bounded 100 ms baseline admission delay before an independent modern client
  writes `ping\r\n`; second read returns exact `pong\r\n`, `6/6/6`,
  `match_found`, index 0, no frames/drops/error/truncation, and positions
  `0/6/0/0/0/6`;
- schema is `serial-mcp.native-sim-differential.reopen-fresh-output-batch.v1`,
  report is `reopen-fresh-output-batch.json` with SHA-256
  `7c9d9071156739a0a2bc81a9ef2adba48a40132ba6bd1208b99e8e04a847d02e`;
- existing `reopen_same_path_returns_distinct_id_and_only_fresh_generation`
  fixture proof remains stronger. Historical registry is `24/15/3/7` with 39 covered rows;
  Phase F blocked.

**Historical Baseline-and-stronger Batch 20** flush-during-armed-delay differential batch is
complete:

- exact row is `native_flush_during_arm_cmd_delay`, a baseline-and-stronger row
  in isolated `FlushDuringArmDelay` batch membership; its required stronger
  proof is `flush_after_command_acceptance_does_not_cancel_delayed_response`;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`; the existing boot synchronization remains the first
  two normalized observations. Public `transact("arm_cmd 1000\r\n")` validates
  exact `arm_cmd delay=1000\r\n` acknowledgement, but setup result is excluded
  from normalized observations;
- public primary `write("ping\r\n")` is normal anonymous UTF-8 with
  `bytes_written=decoded_bytes=6`; the existing `INHERITED_BASELINE_DELAY` is
  exactly 100 ms and is baseline admission only, not a peer-acceptance proof;
- public primary `flush(target="both")` is retained as a normal anonymous
  `FlushObservation` with exact target `both`. A positioned public read starts
  at explicit `from={"type":"now"}`, uses UTF-8, 3000 ms, and literal
  `pong\r\n`; it returns exact `pong\r\n`, `6/6/6`, `match_found`, index 0,
  no truncation/frames/drops/error, and positions
  `52/58/0/0/52/58` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order. Flush clears retained data by setting `start_offset` to the monotonic
  live edge 52, not zero;
- schema is `serial-mcp.native-sim-differential.flush-during-arm-delay-batch.v1`,
  report is `flush-during-arm-delay-batch.json` with observed SHA-256
  `4808262720b252c53f95e88a56ea1d8565b238251884aed364c0e43d0b07f500` after two
   matching runs. The Batch 20 historical registry is `24/16/3/6`
   with 40 covered rows; Phase F blocked.

**Historical Compared Batch 21** input-flush backlog differential batch is complete:

- exact row is `native_flush_input_clears_host_rx`, a direct Compared row in
  isolated `FlushInputBacklog` batch membership with no baseline-proof binding;
  the existing `flush_input_discards_known_old_marker_and_keeps_new_marker`
  fixture proof remains stronger;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`; boot synchronization remains the first two normalized
  observations;
- public UTF-8 write of
  `sendraw hex 4F4C442D4D41524B45520D0A\r\n` returns normal anonymous
  `bytes_written=decoded_bytes=38` and emits `OLD-MARKER\r\n`; status-only
  `get_status` polling proves old marker RX progress at `rx_bytes >= 44` and is
  excluded from normalized output;
- public `flush(target="input")` returns normal anonymous with exact target
  `input`; public UTF-8 write of
  `sendraw hex 4E45572D4D41524B45520D0A\r\n` returns normal anonymous
  `bytes_written=decoded_bytes=38` and emits `NEW-MARKER\r\n`; status-only
  polling proves new marker RX progress at `rx_bytes >= 56` and is excluded from
  normalized output;
- positioned public `read` starts at explicit
  `from={"type":"buffer_start"}`, uses UTF-8, 3000 ms, and literal
  `NEW-MARKER\r\n`; it returns only exact `NEW-MARKER\r\n`, `12/12/12`,
  `match_found`, index 0, no truncation/frames/drops/error, and positions
  `44/56/0/0/44/56` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order. The returned payload proves old marker absence after input flush;
- schema is `serial-mcp.native-sim-differential.input-flush-batch.v1`, report is
  `input-flush-batch.json` with SHA-256
  `ff95074207f7de216780ede42a6d21583e4e88e5c5e5c6f81af4b588fa5dcfd8` after two
  matching runs. Historical registry is `25/16/3/5` with 41 covered rows; Phase F
  blocked.

**Historical Compared Batch 22** bootloader-touch process-exit differential batch is complete:

- exact row is `native_bootloader_touch_exits_42`, a direct Compared row in
  isolated `BootloaderTouchExit` batch membership with no baseline-proof
  binding; the existing `touch_write_causes_small_rust_child_peer_to_exit_42`
  fixture proof remains independent and stronger for process-exit coverage;
- native endpoint is the existing firmware PTY. Fixture endpoint is a real
  dedicated small Rust child process with its own raw PTY, not
  `FixtureExit::Crashed`; child emits exact `serial-mcp test firmware ready\r\n`
  before publishing `PTY_PATH`;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`; matching boot-banner read remains the second
  normalized observation. Public UTF-8 `write("touch\r\n")` is normal anonymous
  with exact `bytes_written=decoded_bytes=7`;
- both endpoints then exit exactly 42. Typed normalized outcomes retain
  `{"kind":"process_exit","exit_code":42}`. The terminal
  `touch exit(42)\r\n` response is deliberately not read, so this batch makes
  no response-delivery claim; no public `close` follows peer exit;
- schema is `serial-mcp.native-sim-differential.bootloader-touch-exit-batch.v1`,
  report is `bootloader-touch-exit-batch.json` with SHA-256
  `91befb4e3af3edd65c70c58208be03c09c8c29aed04f9b432e18c1d5becd4d9c` after two
  matching runs. Historical registry is `26/16/3/4` with 42 covered rows; Phase F
  blocked.

Post-Batch 22 retirement: `native_list_ports_includes_identity_fields` is **Retired**, not a Batch 23 or differential case. Deterministic injected-provider identity/profile/schema proofs supersede ambient OS field/null checks. The three public real-PTY/injected-provider proofs are `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`, `list_ports_preview_selected_winner_matches_later_bare_open`, and `list_ports_preview_output_validates_against_generated_schema`. No new differential batch, report, or hash was created. The pre-txbuf-retirement checkpoint is `26/16/4/3` with 43 covered rows (26 compared, 16 baseline-and-stronger, 4 retired, and 3 pending); full parity and Phase F remain blocked.

Post-Batch-22 txbuf retirement: `native_txbuf_status_reports_pending` is **Retired**, not a Batch 23 or differential case. Source did not observe a nonzero pending TX queue; `txbuf` and `hold` are firmware-only commands. `device_command_parity::held_output_reports_nonzero_queue_then_drains_and_recovers` proves nonzero held output, blocked read, drain, and recovery. No Batch 23, report, or hash was created. The historical pre-auto-reconnect-retirement checkpoint is `26/16/5/2` with 44 covered rows; full parity and Phase F remain blocked.

Post-Batch-22 auto-reconnect retirement: `native_auto_reconnect_preserves_connection` is **Retired**, not a Batch 23 or differential case. Source only invokes `reconnect` while already open; strengthened `device_fixture::public_mcp_ping_hold_disconnect_replace_and_reconnect` preserves same ID/open state and post-no-op traffic, then proves peer loss, replacement, reconnect, and fresh exchange. No Batch 23, report, or hash was created. This is the historical pre-capture-boot-retirement checkpoint at `26/16/6/1` with 45 covered rows; full parity and Phase F remain blocked.

Post-Batch-22 capture-boot retirement: `native_capture_boot_arm_only_captures_post_arm_command_output` is **Retired**, not a Batch 23 or differential case. Source only checks arm-only post-mark output after a consumed stale banner; strengthened `device_command_parity::capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor` excludes stale bytes, preserves private-mark replay and shared cursor/history, and captures post-arm output. No Batch 23, report, or hash was created.

All 49 registry rows now have compared, baseline-and-stronger, or explicit retired disposition; full differential parity and Phase F remain blocked.

## Scope Guard

Current work includes narrow product disconnect fix plus test-only durable PTY
 fixture plus Batch 1, Batch 2, Batch 3, Batch 4, Batch 5, Batch 6, Batch 7,
  Batch 8, Batch 9, Batch 10, Batch 11, Batch 12, Batch 13, Batch 14, Batch 15,
  Batch 16, Batch 17, Batch 18, Batch 19, Batch 20, Batch 21, and Batch 22 differential execution. Existing 43 `native_sim_validation` tests and 6
`native_sim_connection_lifecycle` tests remain required temporary differential
oracle until replacement parity is proven. Phase E does not remove firmware,
NCS setup, native CI gate, release dependency, Nix inputs, or firmware asset
behavior.

## Repository and Host Baseline

| Item | Recorded value |
|---|---|
| Git HEAD used for baseline | `0cb72f81d905933531609321c7cf9282a1ffbab9` |
| Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)` |
| Cargo | `cargo 1.97.1 (c980f4866 2026-06-30)` |
| Host | `x86_64-unknown-linux-gnu` |
| Kernel | `Linux 7.1.1 x86_64 GNU/Linux` on NixOS |
| Nix | `2.34.8` |
| NCS | `v3.3.0`, installed through sdk-manager |
| Local NCS footprint | 9.0 GiB at `/home/thomas-workstation/ncs/v3.3.0` |
| `west` | 1.5.0 |
| `nrfutil` | 8.2.0, commit `c910332`, 2026-04-21 |
| Python | 3.13.14 locally; Python candidate survey used current 3.14 docs |
| `socat` | not installed in current shell; no local prototype result yet |
| Firmware build tree | 16 MiB locally |
| Firmware executable | 610,988 bytes |

Exact current native commands:

```bash
cargo test --locked --test native_sim_validation -- --ignored --test-threads=1
cargo test --locked --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

Local normalized raw output and timing JSON live below ignored
`target/native-sim-research/stage0/`.

| Suite | Result | Test runtime | End-to-end command wall time |
|---|---:|---:|---:|
| `native_sim_validation` | 43/43 passed | 80.82 s | 82.512 s |
| `native_sim_connection_lifecycle` | 6/6 passed | 6.99 s | 8.449 s |

## CI Cost and Failure Evidence

Recent successful GitHub run
[`31631588932`](https://github.com/qarnet/serial-mcp/actions/runs/31631588932)
shows:

- whole `native_sim firmware + test` job: about 11 minutes;
- NCS sdk-manager install/restore step: 4 minutes 22 seconds;
- pristine firmware build: about 35 seconds;
- native suites: 81.79 seconds + 7.09 seconds test runtime, about 1 minute
  54 seconds including Rust compile between steps;
- NCS cache upload: 8,118,945,638 bytes, about 8.1 GB archive;
- firmware build leaves 92 GB free on current 145 GB runner after explicit
  preinstalled-tool cleanup.

Failure history already proves toolchain cost creates failure surface unrelated
to serial-mcp behavior:

- run
  [`30913675111`](https://github.com/qarnet/serial-mcp/actions/runs/30913675111)
  failed in old unpinned `nrfutil` bootstrap before firmware or native tests;
- run
  [`31276982026`](https://github.com/qarnet/serial-mcp/actions/runs/31276982026)
  reached native tests after successful NCS cache restore and firmware build,
  then all 43 native-validation cases failed before connecting because reqwest
  0.13.4 had no rustls crypto provider installed. This was dependency/config
  breakage, not native firmware behavior, but duplicated job setup made failure
  hit native gate separately;
- `firmware/AGENTS.md` records prior ENOSPC failures and current disk-reclamation
  workaround. Treat that as repository evidence, but final report must separate
  historical runner size claims from current 145 GB runner measurements.

## Stage 0: 49-Test Traceability Findings

All expected tests exist: 43 in `tests/native_sim_validation/unix.rs`, 6 in
`tests/native_sim_connection_lifecycle.rs`. No count discrepancy.

Provisional disposition totals after reading every test and its assertions:

| Disposition | Count | Meaning |
|---|---:|---|
| unchanged | 20 | scenario remains a distinct public-boundary proof |
| parameterized | 6 | preserve assertions while consolidating repeated setup/data cases |
| strengthened | 17 | current assertion is weak, race-prone, or does not prove its name |
| split | 3 | current test mixes claims or omits one claimed decoder/precedence path |
| retired | 3 | only after cited stronger public-boundary proof remains |

Highest-risk current claims:

1. `native_auto_reconnect_preserves_connection` calls `reconnect` while state is
   already open. `SerialConnection::reconnect` returns immediately in that state
   (`src/serial/connection.rs:492-496`). It proves neither disappearance nor
   recovery. Replacement must add true peer loss plus endpoint restoration.
2. `native_read_length_prefixed_framing_decodes` injects length-prefixed bytes
   but final read does not configure length-prefixed RX framing. Keep raw-byte
   observation if useful and add a real public decoder test.
3. `native_read_buffer_budget_stops_under_flood` accepts both
   `max_buffered_bytes` and `drained` and needs stronger result-field checks.
4. `native_txbuf_status_reports_pending` never proves a nonzero pending queue;
   split idle status, held queue, and recovery.
5. `native_close_while_read_active_returns_normal_result` is covered by
   historical readiness-proven Batch 18; its retained characterization remains
   native-only evidence, while the direct fixture pair proves the same public
   close/read result.
6. `native_explicit_rx_framing_beats_connection_default` only supplies both
   values at open. Its name implies call-time precedence that it does not test.
7. Historical `native_read_cobs_preset_decodes_frame` generated its fixture
   with production `TxFramingMode::Cobs`, creating shared-oracle risk; Batch 11
   replaces that path with static independent wire `00 05 70 6f 6e 67 00`.
8. `native_list_ports_after_open` depends on ambient host enumeration and does
   not prove opened PTY appears. Stronger deterministic provider/public-resource
   tests already exist.

Three proposed retirements require preserving stronger proofs:

- `native_list_ports_after_open`: deterministic public list/resource behavior
  exists in `tests/http_integration.rs` and `tests/serial_pty.rs`;
- `native_flush_after_write`: its race allows output flush to discard a queued
  write; `native_flush_output_after_full_delivery_is_safe` proves the valid
  delivered-before-flush contract;
- `native_reopen_same_port_after_close_works`: duplicated by the stronger
  reopen-then-match case, which itself needs unique fresh-output strengthening.

A full row-by-row table is a required next committed artifact; source research
for all rows is complete and must not be redone from scratch.

## Existing Firmware Behavior Derived from Source

Replacement should model observed behavior, not Zephyr internals.

Current firmware is one command latch, not a command queue:

- non-terminator bytes append to one 255-byte command buffer;
- CR or LF sets one `cmd_ready` bit;
- main loop copies and clears the buffer, then dispatches synchronously;
- bytes arriving before main loop consumes ready command can merge commands;
- overflow truncates silently and marks command ready;
- embedded NUL truncates later C-string parsing.

Observable state overlays:

- one-shot `arm_cmd` delay;
- persistent slow dispatch delay;
- ACK sequence mode;
- byte-trace mode with 8-bit sequence;
- firmware-side line diagnostics;
- deterministic xorshift spam producer;
- 4096-byte TX queue with 64-byte drain chunks and silent excess drop;
- TX hold/resume;
- raw byte injection;
- `touch` process exit code 42.

Zephyr scheduler, IRQ, `k_timer`, ring-buffer internals, native UART device
pointer, and exact tick timing are not observable acceptance requirements.

Important source/document contradictions found:

- `firmware/AGENTS.md` says `touch` writes GPREGRET `0x57`; current
  `firmware/src/command.c:506-508` only queues `touch exit(42)` then calls
  `exit(42)`;
- firmware docs place touch test in lifecycle suite; actual exit-42 test is in
  `tests/native_sim_validation/unix.rs`;
- `uart_drv_printf` can pass a would-have-written length larger than its
  128-byte stack buffer to `uart_drv_send`. Long formatted output is undefined
  firmware behavior and must not become replacement contract.

## Historical NCS and `native_sim` Coupling Inventory Summary

Active chain:

```text
CI native-sim job
  -> pinned nrfutil + sdk-manager
  -> NCS v3.3.0 toolchain/cache
  -> west build firmware
  -> zephyr.exe PTY process
  -> NativeSimFirmware harness
  -> 43 + 6 ignored public-MCP tests
```

Phase F disposition after accepted parity:

- `.github/workflows/ci.yml`: native job setup/cache/build removed; replacement
  suite remains in normal Rust gate and release `needs` was updated;
- `scripts/install-nrfutil-ci.sh` and its offline tests deleted;
- `firmware/` including C source, Kconfig, board config, helper scripts, and
  `.clangd` deleted;
- `tests/common/firmware.rs`, native wrappers, and native-named suites deleted
  after all traceable assertions had replacement coverage;
- `flake.nix` `nix-nrf-dev` input/mkNrfShell/multilib firmware setup and orphaned
  `flake.lock` nodes removed;
- xtask firmware build/path logic removed;
- evaluator completion references, active docs, README commands, and doc-drift
  guards updated.

Keep:

- Unix `nix` dev dependency if direct PTY foundation wins;
- ordinary `libudev-dev`, `pkg-config`, tokio-serial, and serialport build deps;
- controlled backend tests for DTR/RTS, BREAK, and deterministic I/O failure;
- historical `CHANGELOG.md` references, classified as history;
- optional Nordic/Zephyr documentation MCP config only if repository policy
  still wants it; it is not build/CI coupling.

No unrelated product path currently requires NCS.

## Stage 1: Virtual-Serial Survey Conclusion

Detailed source-linked evidence and provisional weighted scores live in
[`native-sim-virtual-serial-candidate-survey.md`](native-sim-virtual-serial-candidate-survey.md).

Prototype shortlist:

1. direct `nix::pty::openpty` — incumbent, no new crate family, direct owned FDs;
2. `rustix-openpty 0.2.0` + `rustix 1.1.4` — strongest low-level Rust
   challenger with owned descriptors;
3. Python standard-library `os.openpty` — strongest independent scripting
   runtime comparator;
4. `socat 1.8.1.3+` — strongest external-native relay comparator, currently
   unavailable in local shell.

Hard primary-fixture rejections:

- `virtual-serialport`: in-memory mock pipe, no real OS path;
- abandoned `pty` crate: unmaintained advisory;
- tty0tty kernel module: privileged Linux driver install;
- QEMU and Renode: real PTY possible but irrelevant emulator cost;
- Windows ConPTY: terminal handle, not real COM port accepted by serialport.

Deprioritized, not falsely hard-rejected:

- `portable-pty`, `rust-pty`, and `pty-process`: expose usable Unix PTYs but add
  terminal/process policy and dependency weight without required fidelity gain;
- direct libc/minimal C: no fidelity gain over safe Rust wrappers; adds unsafe
  ownership burden;
- `virtualport` and tty0tty userspace relay: weaker deterministic ownership and
  portability than direct APIs.

Desk-research prior favors direct `nix` (4.80/5), then rustix challenger
(4.60/5), Python (4.55/5), and socat (4.15/5). These are not final scores.

## Stage 2: Disposable Prototype State

Worktree now contains ignored test target `tests/native_sim_research.rs`, shared
test-only boundary helpers in `tests/common/native_sim_research.rs`, and a Python
stdlib helper under `tests/fixtures/`. Linux-only dev dependencies currently add
`rustix-openpty 0.2` and `rustix 1.1` for comparison. These changes are
disposable research code, not accepted architecture.

Intended identical experiment per candidate:

- public MCP opens only candidate slave path;
- host-to-peer and peer-to-host `0x00..=0xFF` round trips;
- split command assembly;
- fragmented delayed response;
- 32 KiB flood under 1024-byte result bound;
- clean close and same-path reopen;
- destroy pair and open distinct replacement endpoint;
- peer-master close while read is pending;
- explicit bounded fixture cleanup;
- 100 repetitions;
- FD/process baseline and final counts;
- normalized JSON under `target/native-sim-research/stage2/`.

Prototype chronology and failures worth preserving:

1. Initial direct-nix run failed endpoint replacement because Linux immediately
   reused old `/dev/pts/N` after pair destruction. Experiment now reserves one
   spare PTY before allocating replacement so replacement path is provably
   distinct. This confirms raw PTY number reuse cannot be used as disappearance
   evidence.
2. Next direct-nix run timed out waiting for pending read to stop after dropping
   fixture object. Root cause: test retained no explicit `close_peer` operation;
   endpoint object stayed owned by variable while only replacement wrapper was
   dropped incorrectly. Fixture trait gained explicit `close_peer` and
   `shutdown` operations; later one- and 100-run experiments exercised them.
3. Peer-close result exposes a product classification gap: Linux PTY master
   closure can surface `EIO` (`ErrorKind::Other`), while
   `is_fatal_disconnect` currently recognizes only NotFound, PermissionDenied,
   ConnectionReset, ConnectionAborted, BrokenPipe, and Interrupted. If public
   read did not reach `connection_closed`; all candidates returned timeout in
   every iteration. Do not change product behavior during research without
   separate approval.
4. Python helper compiled cleanly and completed 100 runs. `socat` remains
   untested because executable is absent; do not
   install it without explicit approval. If Nix already exposes a pinned package
   through an ephemeral non-installing shell, that may be used after command is
   reviewed.

Latest one-iteration smoke after those fixes:

| Candidate | Result | FD baseline/final | Peer-close public stop | Note |
|---|---|---:|---|---|
| direct nix | passed experiment harness | 10/10 | `timeout` | peer-master close did not produce `connection_closed` within product read pipeline |
| rustix challenger | experiment behavior completed, metrics gate falsely failed | unchanged before rerun | expected same kernel result | global process-count metric observed unrelated concurrent host processes; metric is being changed to direct-child count |
| Python stdlib | passed experiment harness | 10/10 | `timeout` | helper child exited and no direct child remained |

These are smoke results, not 100-run acceptance. Direct nix and Python both
disprove current assumption that dropping PTY master necessarily yields public
`connection_closed`: read stopped by its 1-second wall timeout. Final report must
distinguish OS HUP from product disconnect classification and propose explicit
characterization/fix rather than falsely marking disconnect proof complete.

Final 100-run result for all three candidates: arbitrary bytes, split command,
fragmented response, bounded flood/hold, same-path reopen, distinct replacement,
FD cleanup, and direct-child cleanup passed; public peer close failed 100/100 as
`timeout`. Nix and rustix each took about 122.45 s; Python took 129.58 s. Strict
FD baseline/final was 10/10 and direct children 0/0 for every candidate.

Smallest eventual regression test remains:

> MCP opens candidate slave path through production serial code, exchanges
> arbitrary bytes, peer closes, public read observes a bounded disconnect
> outcome, and explicit fixture shutdown returns FD/process counts to baseline.

## Emulator Architecture Recommendation So Far

Use hybrid test architecture:

1. reusable in-process Rust `PtyDevice` owns PTY master/slave, cancellation,
   peer task, state machine, bounded output queue, and explicit async shutdown;
2. most scenarios use in-process HTTP `TestServer` plus real public MCP client,
   preserving dependency injection and fast diagnostics;
3. small spawned-`serial-mcp` smoke covers CLI/startup composition;
4. controlled backend remains source of truth for modem lines, BREAK, exact
   release-on-cancel, and injected OS errors;
5. no physical UART claims from PTY tests.

Fixture ownership contract:

- retain slave FD while same-pair reopen must remain possible;
- cancellation token stops device task;
- `shutdown()` closes peer, awaits task with a bound, aborts only as fallback,
  closes retained descriptors, and returns observable exit reason;
- `Drop` is best effort only; acceptance tests call async shutdown;
- output queue has explicit byte capacity, chunk size, hold/drop/block policy,
  and drain signal;
- delays use injected fixture clock/actions, while public MCP timeout assertions
  retain bounded real time;
- scripted actions: emit, fragmented emit, delay, silence, malformed bytes,
  saturate, close, crash, and endpoint replacement;
- protocol peers share transport/scheduling core but keep independent state and
  oracle code.

Separate process is needed only for explicit crash/exit-code or shipped-binary
smoke behavior. Whole suite should not pay child-process cost.

## Protocol Peer Survey Conclusion

Provisional exact dependency recommendation for independent valid encoding:

```toml
[dev-dependencies]
slip-codec = "=0.4.0"
cobs = "=0.5.1"
rmodbus = "=0.12.2"
```

All three are pure Rust, need no native library or install script, and were
verified under Rust 1.97.1 in isolated research: 9, 41, and 30 unit tests plus 7
`rmodbus` doctests passed respectively. Full lockfile RustSec audit remains an
implementation gate.

Per-surface recommendation:

| Surface | Recommended peer/oracle |
|---|---|
| AT | small local DCE state machine; surveyed crates are host/DTE-oriented |
| SLIP | `slip-codec 0.4.0` for valid codec; local malformed/noise injection |
| JSON lines | existing `serde_json` + local line state + official JSON/JSON Lines vectors |
| COBS | `cobs 0.5.1` for valid and strict malformed/recovery cross-checks |
| NDJSON | local JSON-line peer with explicit blank/whitespace policy |
| NMEA-0183 | local generator + committed GPSD/AIS/proprietary golden vectors; avoid young large dependency |
| Modbus ASCII | `rmodbus 0.12.2` protocol/server core + local serial stream wrapper |
| generic framing | small local builders + static independent vectors |
| shell/raw | local state/prompt peer + arbitrary binary vectors |

Rejected protocol assumptions:

- `tokio-modbus` supports RTU/TCP, not ASCII;
- `atat` is a DTE/client stack, not modem/DCE simulator;
- NMEA parser crates generally do not supply stateful generator peers;
- static `sendraw` playback alone cannot satisfy AT URC interleaving or mutable
  Modbus server state.

Product-contract questions discovered during survey:

- JSON parser currently emits structured output only for JSON objects; arrays
  and scalars become raw despite JSON Lines allowing any JSON value;
- AT parser branch ordering appears to classify `+CME ERROR`/`+CMS ERROR` as a
  generic `+COMMAND:` response before error handling;
- NMEA sentence without checksum remains accepted with
  `checksum_valid: null`, even when validation is enabled;
- production COBS invalid-code path may be unreachable for plain delimited COBS;
  strict helper behavior must not silently redefine product recovery policy.

These findings need characterization tests or separate product decisions. They
must not be hidden by fixture implementation.

## Independent References Selected

- SLIP: [RFC 1055](https://www.rfc-editor.org/rfc/rfc1055)
- COBS: [Cheshire/Baker paper](https://stuartcheshire.org/papers/COBSforToN.pdf)
- JSON: [RFC 8259](https://www.rfc-editor.org/rfc/rfc8259)
- JSON Lines: <https://jsonlines.org/>
- NDJSON 1.0: <https://github.com/ndjson/ndjson-spec>
- AT: ITU-T V.250 plus modem-vendor CME/CMS/URC examples
- NMEA: NMEA official standard page plus
  [GPSD NMEA](https://gpsd.gitlab.io/gpsd/NMEA.html) and
  [AIVDM](https://gpsd.gitlab.io/gpsd/AIVDM.html) vectors
- Modbus: official Modbus over Serial Line specification plus independent
  request/response examples

## Decisions Not Yet Made

- normalized macOS disconnect assertion and compact boundary gate;
- exact protocol dependency set after prototypes and lockfile audit;
- whether discovered parser behaviors are bugs, supported quirks, or migration
  changes;
- ADR acceptance. ADR is `Implemented`; it remains pending `Accepted` status
  until fresh clean-checkout CI evidence exists.

## Next Work in Order

1. Preserve accepted parity evidence and characterize parser questions before
   changing product semantics.
2. Obtain fresh clean-checkout CI evidence for final active-source acceptance.
3. Update ADR to Accepted only after that final migration evidence.

## Research Artifact Verification

Completed on 2026-08-13:

- `cargo fmt --all -- --check`;
- `cargo build --all-targets --locked`;
- `cargo test --locked` — 676 library tests plus all non-ignored integration
  suites passed; 49 existing native tests and 3 disposable boundary prototypes
  remained intentionally ignored in default run;
- `cargo clippy --all-targets --locked -- -D warnings`;
- `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`;
- `cargo test --locked --test doc_drift` — 74 passed;
- disposable emulator-core tests — 5 passed;
- disposable protocol-peer tests — 7 passed;
- boundary prototype compile/clippy gate passed;
- `git diff --check`;
- `nix flake check "path:$PWD"` — passed after cache warm-up. First two attempts
  reached dependency/project builds but exceeded 20-minute command bounds; third
  40-minute-bound retry completed. Existing cross-compile/app metadata warnings
  remain non-fatal.

Native 43+6 baseline also passed before research edits; normalized outputs remain
under ignored `target/native-sim-research/stage0/`.

## Approved Implementation Progress

User approved full staged migration on 2026-08-13.

Phase A peer-disconnect truth is complete:

- `SerialConnection::read` now maps an underlying zero-byte read to
  `UnexpectedEof("serial peer closed")` rather than polling until wall timeout;
- fatal-disconnect classification includes `UnexpectedEof` but deliberately
  still excludes generic `ErrorKind::Other`;
- focused public-MCP PTY test starts a known-pending `from=now` read, closes
  master, and requires `stop_reason="connection_closed"`;
- direct-nix black-box gate passes 100/100 after fix: all required byte,
  split/fragment, flood/hold, same-path reopen, replacement, disconnect, and
  cleanup checks pass; peer-close reason is only `connection_closed`; wall time
  48.352 s; FD 10→10; direct children 0→0.

Phase B fixture foundation is complete:

- durable Unix-only test harness lives under `tests/common/device_fixture/`;
- direct `nix::pty::openpty` boundary uses raw, nonblocking PTY master through
  Tokio `AsyncFd`; one event-loop task owns master FD, command input, output
  queue, scripts, peer state, and exact hangup lifetime;
- fixture owns temporary directory, no-clobber initial stable symlink, retained
  slave descriptor, atomic replacement symlink, cancellation token, readiness
  watch channel, and bounded explicit shutdown;
- typed actions cover emit, fragmented chunks, delay/silence, malformed bytes,
  saturation, close, and crash state;
- output queue exposes byte capacity, chunk bound, hold/release, explicit
  `DropNew`/`BlockProducer` policy, and cumulative accepted/dropped/drained
  counters;
- input and scripts reject configured bounds before partial acceptance;
- public MCP test proves fragmented `ping`, held output, release/match,
  readiness-proven peer disconnect as `connection_closed`, a distinct physical
  replacement behind same stable path, and successful public `reconnect` plus
  fresh data;
- shipped real `serial-mcp` child opens fixture stable path and completes ping;
- 100 fixture spawn/shutdown repetitions restore Linux `/proc/self/fd` count;
- superseded rustix/Python boundary and pure-core prototype source/dependencies
  are removed; historical measurements remain in research docs. Protocol peer
  prototypes and their three exact pins remain for Phase D promotion.

Phase B verification:

```text
cargo test --locked --test device_fixture -- --test-threads=1  # 7 passed
cargo test --locked --test serial_pty pty_peer_close_stops_pending_read_as_connection_closed  # 1 passed
cargo test --locked --test native_sim_protocol_peer_research  # 7 passed; historical prototype target later promoted and removed
cargo build --all-targets --locked  # passed
cargo clippy --locked --test device_fixture -- -D warnings  # passed
cargo fmt --all -- --check  # passed
git diff --check  # passed
```

Next phase: Phase C command parity. Native suite remains untouched and required
until all 49 scenarios reach differential parity.

Phase C first command/lifecycle batch is complete:

- new required target `tests/device_command_parity.rs` ports five traceability
  rows: ping, pending-read/later-write, split writes/exact peer bytes, named
  connection summary, and fresh same-path reopen;
- fixture now exposes bounded peer observations for complete commands and exact
  raw PTY input chunks; exact-byte tests concatenate OS chunks rather than
  treating read boundaries as protocol semantics;
- replacement strengthens pending-read proof with explicit held queue/readiness,
  and fresh reopen proof with distinct connection IDs, `from=now`, sequence 2,
  and absence of sequence 1;
- replacement suite passes 5/5 and focused native oracle cases pass 5/5;
- `cargo clippy --locked --test device_command_parity -- -D warnings` passes.

Phase C second command batch is complete:

- replacement target now has 12 passing cases;
- added exact status counter deltas, complete reconfigure transitions with live
  data, stateful ACK ordering/disable, nonzero held queue and drain recovery,
  unique-marker input flush, command-acceptance-barrier delayed flush, and safe
  output flush after full delivery;
- seven focused native oracle cases pass, bringing mapped differential rows to
  12/49;
- `cargo clippy --locked --test device_command_parity -- -D warnings` passes.

Phase C generic framing/matcher batch is complete:

- new required target `tests/device_framing_parity.rs` passes 8/8;
- covers regex/glob, exact ordered line frames, `max_frames`, live framed match
  with frame/index proof, delimiter, real length-prefixed decoder, start/end,
  all explicit line endings, call-time framing precedence, and exact TX framing
  bytes observed directly by peer;
- 11 focused native framing/matcher cases pass, including TX framing; mapped
  differential coverage is now 23/49 rows;
- replacement framing clippy with `-D warnings` passes;
- fixture timing revealed two non-contract details handled explicitly: buffered
  history can satisfy matcher before framed live path, and one decoder chunk can
  contain more than `max_frames`; tests force live framing and separated peer
  emissions rather than changing product semantics.

Batch 3 raw generic-framing differential batch is complete:

- exact rows are `native_read_delimiter_framing_decodes`,
  `native_read_length_prefixed_framing_decodes`,
  `native_read_start_end_framing_decodes`,
  `native_write_tx_framing_modes_observed_via_trace`, and
  `native_read_explicit_line_endings_split_correctly`;
- `tests/native_sim_differential.rs` adds five executable cases with isolated
  schema ID `serial-mcp.native-sim-differential.raw-generic-framing-batch.v1`
  and report `target/native-sim-differential/raw-generic-framing-batch.json`;
- delimiter, start/end, and explicit LF/CR/CRLF reads are compared exactly;
  length-prefixed RX and four-mode TX wire observation are
  baseline-and-stronger rows bound to their required fixture proofs;
- measured native characterization is durable evidence: delayed `sendraw`
  after `arm_cmd 1000` and `from=now` captured target-only bytes, while fresh
  trace started at sequence zero and emitted exact one-line-per-byte
  `RX[n]=0xhh\r\n` output with no extra lines, drops, or errors;
- target reads retain complete public payload, effective encoding, counters,
  frames, match fields, drops, and errors; TX report observations retain exact
  independent host-to-peer bytes, direction, and framing mode;
- Batch 1 remains eight cases and Batch 2 remains six cases. At this Batch 3
  checkpoint, all three report files stayed isolated; full differential outcome
  parity and Phase F remained blocked. No physical UART or clean-checkout-
  without-NCS claim was made.

Batch 4 flood/buffer differential batch is complete:

- exact rows are `native_read_match_on_spam_complete` and
  `native_read_buffer_budget_stops_under_flood`, both baseline-and-stronger;
- `tests/native_sim_differential.rs` adds two executable cases with isolated
  schema ID `serial-mcp.native-sim-differential.flood-buffer-batch.v1` and
  report `target/native-sim-differential/flood-buffer-batch.json`;
- the fixture compatibility peer recognizes only exact `spam 1024 hex` and
  `spam 512 hex` commands and schedules start text, 10 ms-delayed 256-byte
  chunks, and completion text in native order;
- native `spam 1024 hex` produces a deterministic 1088-byte UTF-8 stream with
  `Spam complete` at index 1056; live native `spam 512 hex` with connection
  default 256 produces the exact first 256 bytes and all public byte counters
  at 256 with `max_buffered_bytes` stop metadata. Both executable target reads
  retain all six stable public offset/backlog fields: `from_offset`,
  `next_offset`, `bytes_lost`, `buffered_remaining`, `start_offset`, and
  `end_offset`. Measured values in that order are
  `32/1120/0/0/0/1120` for `spam 1024` and `32/288/0/31/0/319` for live
  `spam 512`;
- required stronger proofs remain
  `finite_flood_matcher_reaches_unique_completion_marker` and
  `live_buffer_budget_caps_finite_flood_with_exact_stop_metadata`;
- raw characterization retained variable diagnostic `elapsed_ms`, which is
  omitted from typed Batch 4 outcomes, and variable prefilled `spam 65536 hex`
  backlog fields. The prefilled 65536 sample is excluded from executable parity
  because those values varied across fresh native runs; it is not an executable
  comparator.

Batch 5 command-diagnostic differential batch is complete:

- exact rows are `native_framing_reports_single_split_command`,
  `native_trace_reports_exact_split_byte_sequence`, and
  `native_partial_line_buffered_then_completed`, all baseline-and-stronger;
- `tests/native_sim_differential.rs` adds three executable cases with isolated
  schema ID `serial-mcp.native-sim-differential.command-diagnostics-batch.v1`
  and report `target/native-sim-differential/command-diagnostics-batch.json`;
- target reads use neutral `normalize_positioned_read`, retaining all six
  cursor/backlog fields. Measured framed output is exactly 54 bytes with
  `match_index=48` and table `44/98/0/0/0/98`; trace output is exactly 78
  bytes with `match_index=72` and table `42/120/0/0/0/120`; partial completion
  is exact `pong\r\n`, 6 bytes, `match_index=0`, and table
  `32/38/0/0/0/38`;
- native CRLF framing emits duplicate `LINE len=4 data="ping"` diagnostics;
  the fixture compatibility peer reproduces this measured public output and
  does not change firmware or fixture core APIs. Trace output is the exact
  lower-case six-record `RX[n]=0xhh\r\n` sequence, beginning with
  `RX[0]=0x70\r\n` and ending with `RX[5]=0x0a\r\n`, followed by `pong\r\n`;
- all three rows retain the stronger
  `split_writes_preserve_one_command_and_exact_wire_order` proof. Partial
  `pi` remains unfinished after the second bounded 100 ms observation before
  `ng\r\n` is sent. No flush, status probe, raw fixture API, or direct serial
  I/O is used.

Batch 6 ACK state-machine differential batch is complete:

- exact row is `native_ack_command_provides_pre_execution_ack`, a direct
  Compared row in isolated `AckState` batch membership with no baseline-proof
  binding; the existing
  `ack_peer_orders_ack_before_response_and_stops_after_disable` remains the
  stronger Rust-PTY semantic proof;
- `tests/native_sim_differential.rs` adds one executable case with schema ID
  `serial-mcp.native-sim-differential.ack-state-batch.v1` and report
  `target/native-sim-differential/ack-state-batch.json`;
- after the 32-byte boot banner, ordinary shared-cursor public writes and
  positioned reads compare this exact five-step sequence: `ack on\r\n` writes 8
  and reads `ack on\r\n` (8 bytes, match index 0, position
  `32/40/0/0/0/40`); `ping\r\n` writes 6 and reads
  `ack 0\r\npong\r\n` (13 bytes, match index 7, position
  `40/53/0/0/0/53`); the second `ping\r\n` reads
  `ack 1\r\npong\r\n` (13 bytes, match index 7, position
  `53/66/0/0/0/66`); `ack off\r\n` writes 9 and reads
  `ack 2\r\nack off\r\n` (16 bytes, match index 7, position
  `66/82/0/0/0/82`); final `ping\r\n` reads `pong\r\n` (6 bytes, match
  index 0, position `82/88/0/0/0/88`);
- every target read is normal UTF-8 `match_found`, `matched: true`, with no
  frames, no drops, no error, and no truncation; `bytes_read`,
  `bytes_observed`, and `bytes_returned` equal exact payload length. ACK mode
  remains enabled while dispatching `ack off`, so `ack 2\r\n` stays before
  `ack off\r\n`. No `transact`, `from: now`, secondary client, sleep, flush,
  status, raw fixture API, or direct serial I/O is used. Result properties:
  no frames, no drops, no error, no truncation.

Batch 7 output-flush differential batch is complete:

- exact row is `native_flush_output_after_full_delivery_is_safe`, a direct
  Compared row in isolated `OutputFlush` batch membership with no baseline-proof
  binding; the existing
  `output_flush_after_full_delivery_preserves_later_traffic` remains the
  stronger Rust-PTY semantic proof;
- `tests/native_sim_differential.rs` adds one executable case with schema ID
  `serial-mcp.native-sim-differential.output-flush-batch.v1` and report
  `target/native-sim-differential/output-flush-batch.json`;
- standard anonymous open uses `profile_mode: "none"`, `name: null`, and baud
  115200. After the 32-byte boot banner, the exact public order is first
  `write("ping\r\n")`, positioned `read(match="pong")`,
  `flush(target="output")`, second `write("ping\r\n")`, and second positioned
  `read(match="pong")`;
- both writes are normal anonymous UTF-8 results with
  `bytes_written=decoded_bytes=6`. Both reads return exact `pong\r\n`, all
  three byte counters are 6, `match_index=0`, and stop reason is
  `match_found`; no frames, drops, error, or truncation are present. Positions
  are `32/38/0/0/0/38` then `38/44/0/0/0/44` in
  `from/next/lost/remaining/start/end` order;
- First matched `pong` is first-command fully-delivered/consumed boundary
  before output-only flush. Flush is normal anonymous output-only result and
  retains RX cursor, so second read starts at 38. No sleep, readiness shortcut,
  or weaker timing substitution is used. `elapsed_ms` is the only
  nondeterministic field removed from raw characterization. Typed differential
  reports retain modeled outcome fields; caller-supplied request echoes
  (`timeout_ms`, `no_new_rx_timeout_ms`) are not modeled. This intentional omission
  is separate from the request-echo model boundary. Batch 7 has no baseline
  proof binding.
  At the Batch 7 checkpoint, this was 26 covered rows before Batch 8 promotion.

Batch 8 SLIP happy-path differential batch is complete:

- exact row is `native_read_slip_decodes_frame`, a direct Compared row in
  isolated `SlipHappy` batch membership with no baseline-proof binding; the
  existing `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
  remains the stronger Rust-PTY semantic proof;
- `tests/native_sim_differential.rs` adds one ignored executable case with
  schema ID `serial-mcp.native-sim-differential.slip-happy-batch.v1` and report
  `target/native-sim-differential/slip-happy-batch.json`;
- both endpoints use the same all-public scaffold: `transact` sends exact
  `arm_cmd 1000\r\n` and matches exact `arm_cmd delay=1000\r\n`; public
  `write` then sends exact `sendraw hex C0706F6E67C0\r\n`; public `read` uses
  `from: {"type":"now"}`, `encoding: "hex"`, and raw
  `rx_framing: {"type":"slip","max_frames":1}`. No sleep, flush, direct
  fixture script, raw fixture API, native-only branch, fixture-core API, or
  product change is used;
- compatibility peer arming acknowledgement is immediate. The one-shot
  armed delay applies before the next exact command's output, then the peer
  emits exact raw `c0 70 6f 6e 67 c0` with no extra response. Setup write
  result is normal UTF-8 with `bytes_written=decoded_bytes=26`;
- normalized target read compares modeled outcome fields: anonymous
  `is_error=false`, effective hex wire payload `c0 70 6f 6e 67 c0`, counters
  `6/0/6` for `bytes_read/bytes_observed/bytes_returned`, `max_frames`, no
  match/truncation/drop/error, one raw `slip` frame at index zero with hex
  payload `70 6f 6e 67` and no parser, and positions
  `52/58/0/0/0/58` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order. `elapsed_ms` is the only nondeterministic field removed from raw
  characterization. Typed differential reports retain modeled outcome fields;
  caller-supplied request echoes (`timeout_ms`, `no_new_rx_timeout_ms`) are not
  modeled;
- this is raw `rx_framing: slip` happy-path evidence, not preset, malformed,
  recovery, or full protocol parity. The Batch 8 checkpoint recorded 14
  compared, 13 baseline-and-stronger, 3 retired, and 19 pending, with 27 covered rows.

Batch 9 malformed SLIP differential batch is complete:

The historical Batch 8 checkpoint recorded 13 baseline-and-stronger rows; the
current Batch 9 registry records 14.

- exact row is `native_read_slip_malformed_escape_returns_partial_result`, a
  baseline-and-stronger row in isolated `SlipMalformed` batch membership. Its
  required stronger proof is
  `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`;
- `tests/native_sim_differential.rs` adds one ignored executable case with
  schema ID `serial-mcp.native-sim-differential.slip-malformed-batch.v1` and
  report `target/native-sim-differential/slip-malformed-batch.json`;
- both endpoints use identical public setup: `transact("arm_cmd 1000\r\n",
  match exact "arm_cmd delay=1000\r\n")`, public
  `write("sendraw hex C0DB41C0\r\n")`, and public
  `read(from={"type":"now"}, encoding="utf8",
  rx_framing={"type":"slip"})`. The arm acknowledgement is exact and the
  anonymous UTF-8 setup write is normal with
  `bytes_written=decoded_bytes=22`. Setup is omitted from normalized
  observations. No sleep, flush, private fixture script, raw fixture API,
  native-only branch, or `max_frames` is used;
- the exact normalized target is anonymous and normal, with effective fallback
  hex payload `c0 db 41 c0`, counters `4/0/0` for
  `bytes_read/bytes_observed/bytes_returned`, `framing_error`, no match,
  truncation, frames, or drops, and error
  `SLIP framing error: invalid escape byte 0x41`. Positions are
  `52/56/0/0/0/56` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order;
- `elapsed_ms` is removed only from raw characterization. unmodeled request echoes
  remain outside typed outcomes. This original raw `rx_framing: slip`
  malformed result intentionally has no frame before the error. The stronger
  `protocol: slip` fixture proof owns valid-frame-before-error plus recovery;
  Batch 9 does not claim full SLIP protocol parity. Batch 9 counts are an
  explicit historical checkpoint: 14 compared, 14 baseline-and-stronger,
  3 retired, and 18 pending, with 28 covered rows. They are not current.

Batch 10 malformed-then-recovery SLIP differential batch is complete:

- exact row is `native_read_slip_recovers_after_error_on_next_call`, a direct
  Compared row in isolated `SlipRecovery` batch membership with no baseline
  proof binding;
- `tests/native_sim_differential.rs` adds one ignored executable case with
  schema ID `serial-mcp.native-sim-differential.slip-recovery-batch.v1` and
  report `target/native-sim-differential/slip-recovery-batch.json`;
- both endpoints use the shared public double-arm/sendraw scaffold. Public
  `transact("arm_cmd 1000\r\n")` with exact `arm_cmd delay=1000\r\n` and public
  `write` send the malformed `sendraw hex C0DB41C0\r\n`; the exact malformed
  public raw `rx_framing: slip` read is retained. The scaffold repeats public
  arm and write for `sendraw hex C0706F6E67C0\r\n`, then uses recovery read
  arguments exactly as `connection_id`, `from={"type":"now"}`,
  `encoding="hex"`, `timeout_ms=3000`, and `rx_framing={"type":"slip"}`;
  both setup calls are validated but excluded from normalized observations;
- observation order is existing open, existing boot read, exact malformed read,
  exact recovery read. No setup output, sleep, flush, raw fixture API,
  native-only path, `max_frames`, `no_new_rx_timeout_ms`, or protocol preset is
  normalized;
- malformed target remains effective hex `c0 db 41 c0`,
  `bytes_read/bytes_observed/bytes_returned=4/0/0`,
  `framing_error`, no frame/drop/match/truncation, exact error
  `SLIP framing error: invalid escape byte 0x41`, and positions
  `52/56/0/0/0/56`;
- recovery target is effective hex `c0 70 6f 6e 67 c0`,
  `bytes_read/bytes_observed/bytes_returned=6/0/0`, timeout, no
  drop/match/truncation/error, one raw SLIP frame at index zero with hex payload
  `70 6f 6e 67`, and positions `76/82/0/0/0/82`. Cursor advancement
  is raw-consumption based even with zero `bytes_returned`;
- the stronger `protocol: slip`
  `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
  proof retains valid-frame-before-error plus recovery. Batch 10 is raw
  `rx_framing: slip` evidence, not full SLIP protocol parity. Batch 10's
  historical checkpoint was 15 compared, 14 baseline-and-stronger, 3 retired,
  and 17 pending with 29 covered rows; it is historical only.

Batch 11 COBS-preset differential batch is complete:

- exact row is `native_read_cobs_preset_decodes_frame`, a direct Compared row in
  isolated `CobsPreset` batch membership with no baseline-proof binding;
- `tests/native_sim_differential.rs` adds one ignored executable case with
  schema ID `serial-mcp.native-sim-differential.cobs-preset-batch.v1` and
  report `target/native-sim-differential/cobs-preset-batch.json`;
- both endpoints use standard anonymous open and the same public
  `transact("arm_cmd 1000\r\n")`, then `write("sendraw hex 0005706F6E6700\r\n")`;
  the arm delay applies before output, setup write is normal UTF-8 with
  `bytes_written=decoded_bytes=28`, and setup calls are excluded from
  normalized observations;
- the target read uses `protocol: {"type":"cobs"}`, not raw COBS framing, and
  receives static independent plain-COBS wire `00 05 70 6f 6e 67 00`. The
  normalized result is normal anonymous hex with
  `bytes_read/bytes_observed/bytes_returned=7/0/0`, `timeout`, no
  match/truncation/drop/error, and one `cobs` frame at index 0 with hex payload
  `70 6f 6e 67` and parsed raw frame `{"parser":"raw"}`;
- positions are `52/59/0/0/0/59`. broader zero-containing COBS TX/RX fixture proof
  remains stronger coverage; Batch 11 does not claim full COBS protocol parity.
  Batch 11's historical checkpoint was 16 compared, 14 baseline-and-stronger, 3
  retired, and 16 pending (`16/14/3/16`) with 30 covered rows.

Batch 12 AT-parser differential batch is complete:

- exact row is `native_read_at_parser_parses_pong`, a direct Compared row in
  isolated `AtParser` batch membership with no baseline-proof binding;
- `tests/native_sim_differential.rs` adds one ignored executable case with schema
  ID `serial-mcp.native-sim-differential.at-parser-batch.v1` and report
  `target/native-sim-differential/at-parser-batch.json`;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`, public boot-banner literal-match `read`, public
  `transact("arm_cmd 1000\r\n")` matching exact `arm_cmd delay=1000\r\n`, and
  public UTF-8 `write("ping\r\n")` with `bytes_written=decoded_bytes=6`;
- the target public `read` uses `from={"type":"now"}`, `encoding="utf8"`,
  `timeout_ms=3000`, explicit `rx_framing: line`, and explicit
  `rx_parser: at_command`. Setup calls are validated but excluded from the
  normalized observations. The one-second public arm barrier replaces source
  test sleep/flush behavior; no private fixture API, native-only branch,
  `max_frames`, `no_new_rx_timeout_ms`, or protocol preset is used;
- the normalized target is anonymous, normal UTF-8 `pong\r\n`, with
  `bytes_read/bytes_observed/bytes_returned=6/0/0`, `stop_reason=timeout`, no
  match, truncation, drop, or error, and one index-0 `line` frame with UTF-8
  payload `pong` and parsed
  `{"parser":"at_command","response_type":"data","command":null,"status":null,"fields":["pong"]}`;
- positions are `52/58/0/0/0/58` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order. Target characterization remains at
  `target/native-sim-differential/at-parser-characterization.json` with SHA-256
  `52b573c8a71da8aa52fa6ce12ce81d63f5f30756839ae8db3a1e4e56a6424eb5`;
- this is direct explicit-parser evidence. Existing
  `at_command_connection_default_drives_stateful_transact_and_parser_quirk`
  `AtPeer` coverage remains stronger stateful AT behavior. Batch 12 is historical
  evidence at 17 compared, 14 baseline-and-stronger, 3 retired, and 15 pending
  (`17/14/3/15`) with 31 covered rows.

Batch 13 AT protocol-default differential batch is complete:

- exact row is `native_open_protocol_default_drives_write_and_read`, a direct
  Compared row in isolated `AtProtocolDefault` batch membership with no
  baseline-proof binding;
- both endpoints use anonymous public `open` at 115200 with
  `profile_mode: "none"` and the protocol-only default
  `protocol: {"type":"at_command"}`; `rx_framing` is omitted so the open
  default drives both bare TX and bare RX;
- public setup uses default-framed `transact("arm_cmd 1000")` and stripped
  framed match `arm_cmd delay=1000`, then bare `ping` adds one CR (`4→5`).
  Setup calls are validated but excluded from normalized observations;
- the target bare `read` uses only `from={"type":"now"}`, `timeout_ms=3000`,
  and UTF-8 encoding. It returns `pong\r\n`,
  `bytes_read/bytes_observed/bytes_returned=6/0/0`, `timeout`, no match,
  truncation, drop, or error, one UTF-8 `line` frame parsed by AT as data with
  fields `["pong"]`, and positions `52/58/0/0/0/58`;
- schema is `serial-mcp.native-sim-differential.at-protocol-default-batch.v1`,
  report is `at-protocol-default-batch.json`, and retained characterization is
  `target/native-sim-differential/at-protocol-default-characterization.json`
  with SHA-256
  `cce2c8a47d3d23eedfb857b5701428937174ab066bd6b64ce20e544776b68775`;
- this is 32 covered rows at historical status `18/14/3/14`. Stronger `AtPeer`
  proof remains required.

Batch 14 JSON-parser differential batch is complete:

- exact row is `native_read_json_parser_decodes_jsonout`, a direct Compared row
  in isolated `JsonParser` batch membership with no baseline-proof binding;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`, public boot-banner literal-match `read`, public
  `transact("arm_cmd 1000\r\n")` matching exact `arm_cmd delay=1000\r\n`, and
  public UTF-8 `write("jsonout\r\n")` with `bytes_written=decoded_bytes=9`;
- the target uses `from={"type":"now"}`, UTF-8, 3000 ms, explicit line framing,
  and explicit `json_lines` parsing. Setup calls are validated but excluded from
  normalized output;
- the static three JSON object response is 140 bytes. The normalized target has
  `bytes_read/bytes_observed/bytes_returned=140/0/0`, stop reason `timeout`, no
  match, truncation, drops, or error, three ordered UTF-8 line frames parsed as
  JSON objects for `temp`, `humidity`, and `pressure`, and positions
  `52/192/0/0/0/192`;
- schema is `serial-mcp.native-sim-differential.json-parser-batch.v1`, report is
  `json-parser-batch.json`, and retained characterization is
  `target/native-sim-differential/json-parser-characterization.json` with
  SHA-256
  `f51b5d77bac3904d214e2ea76794cf1d10f4d5aa8849224e750af30a8e9e3a06`;
- existing `json_lines_preset_writes_line_and_preserves_object_only_parser_behavior`
  fixture proof remains stronger. Batch 14 historical status is 19 compared,
  14 baseline-and-stronger, 3 retired, and 13 pending (`19/14/3/13`) with 33
  covered rows.

Batch 15 NDJSON-preset differential batch is complete:

- exact rows are `native_read_ndjson_preset_decodes_json_frames` and
  `native_read_ndjson_preset_skips_empty_lines`, direct Compared rows in isolated
  `NdjsonPreset` batch membership with no baseline-proof binding;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`, public boot-banner literal-match `read`, public
  `transact("arm_cmd 1000\r\n")` matching exact `arm_cmd delay=1000\r\n`, and
  exact static UTF-8 `sendraw` writes of 48 and 74 bytes after the shared
  one-second arm barrier;
- target reads use `from={"type":"now"}`, UTF-8, 3000 ms, and only
  `protocol: {"type":"ndjson"}`. The preset supplies auto line framing,
  `skip_empty:true`, and JSON parsing; setup calls are validated but excluded;
- static payloads are `{"a":1}\n\n{"b":2}\n` and
  `{"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n`. Exact normalized targets are
  `17/0/0` with positions `52/69/0/0/0/69` and `30/0/0` with positions
  `52/82/0/0/0/82`; both stop by `timeout`, preserve raw UTF-8 bytes, emit only
  ordered JSON record frames, and have no match, truncation, drops, or error;
  blank and whitespace-only lines emit no frames;
- schema is `serial-mcp.native-sim-differential.ndjson-preset-batch.v1`, report is
  `ndjson-preset-batch.json`, and retained characterization is
  `target/native-sim-differential/ndjson-characterization.json` with SHA-256
  `10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3`;
- existing `ndjson_preset_parses_records_and_skips_blank_whitespace_lines`
  fixture proof remains stronger. Current status is 21 compared,
  14 baseline-and-stronger, 3 retired, and 11 pending (`21/14/3/11`) with 35
  covered rows; Phase F blocked.

Batch 1 remains eight cases, Batch 2 remains six cases, Batch 3 remains five
cases, Batch 4 remains two cases, Batch 5 remains three cases, and Batch 6
remains one case; Batch 7, Batch 8, Batch 9, Batch 10, Batch 11, Batch 12, and
Batch 13 and Batch 14 remain one case each; Batch 15 has two cases. All fifteen
report files stay isolated; 35 covered rows, full differential outcome parity,
and Phase F remain blocked.
Native suites remain untouched and required.

Phase C protocol-preset batch is complete:

- durable Linux-only peer/oracle helpers now live in
  `tests/common/device_fixture/protocol_peers.rs`; they retain independent
  pinned `slip-codec`, `cobs`, and `rmodbus` use; seven helper-unit cases live
  only in `tests/device_protocol_parity.rs`;
- new required target `tests/device_protocol_parity.rs` has seven public real-PTY
  MCP cases plus an exact independent seven-preset coverage registry;
- public cases map AT default/state/URC/parser quirk, SLIP TX/happy/partial
  framing error/recovery, JSON Lines TX plus object-only parsing, COBS
  zero-containing TX/RX, NDJSON whitespace skipping, NMEA parsed checksum, and
  stateful Modbus ASCII LRC mutation;
- superseded `tests/common/protocol_peer_research.rs` and
  `tests/native_sim_protocol_peer_research.rs` are removed after equivalent
  helper and public coverage exists;
- `cargo test --locked --test device_protocol_parity -- --test-threads=1`
  passed 15/15;
- full native oracle rerun passed 43/43 validation and 6/6 lifecycle with
  `--ignored --test-threads=1`; native suites remain required;
- regression checks after promotion: `device_fixture` passed 7/7 and
  `device_framing_parity` passed 8/8; peer-oracle unit tests run only in
  `device_protocol_parity`.

Phase C remaining non-protocol parity batch is complete:

- `device_command_parity` now passes 19/19. Added finite deterministic flood
  matcher completion, live 256-byte buffer-budget metadata, close-owned pending
  read, open-time plus live `none` flow-control summary, arm-only capture, and
  real child-process `touch` exit 42 coverage;
- flood budget config uses only live `configure.defaults.max_buffered_bytes`.
  Public result requires exact `stop_reason="max_buffered_bytes"`, byte bounds,
  observed-byte accounting, coherent `truncated`, and status truncation count;
- arm-only capture uses stale and post-mark output with `reset=null`, asserts
  stale exclusion, post-mark match, atomic-mark relationship, private-read
  accounting, explicit offset replay, and retained-history replay. PTYs still
  make no DTR/RTS physical claim; controlled backend remains reset-line proof;
- touch exit uses a small Rust child peer launched from the integration target.
  Public MCP `write("touch\\r\\n")` succeeds, then test observes actual child
  status 42. No Python and no `FixtureExit::Crashed` surrogate;
- lifecycle flow control remains intentionally limited to `none` on real PTY.
  It proves public open/result/summary consistency only. Existing
  `serial_pty::learning_set_flow_control_persists_and_applies_on_reopen` and
  controlled backend retain stronger configuration/physical-effect coverage;
- true reconnect was already stronger in
  `device_fixture::public_mcp_ping_hold_disconnect_replace_and_reconnect`.
   Partial-line/framing/trace claims now have Batch 5 normalized public outcome
   rows and remain bound to the existing exact split raw-PTY command proof.
  Ambient identity rows retire only with existing exact public `http_integration`
  and `serial_pty` citations in traceability;
- current mapping is 49/49 rows: every row has a required replacement or a
  cited stronger retired proof. This historical mapping is retained after Phase
  F active source/configuration removal; fresh clean-checkout CI acceptance
  remains pending.

Focused verification for this batch:

```text
cargo test --locked --test device_command_parity -- --test-threads=1  # 19 passed
```

The 43+6 native checkpoint was deliberately not rerun because it had already
freshly passed before this work, per task scope. The former migration checkpoint
required full differential outcome comparison before native/NCS deletion work;
the accepted Phase E window resolved that comparison. Phase F active source and
configuration removal is recorded above; fresh clean-checkout no-NCS acceptance
remains pending.

## Phase E Required Replacement and Repeat Gate

Phase E is complete:

> Full normalized differential parity window accepted on 2026-08-23: all 42 executable cases passed in three serial 22-batch runs; two consecutive canonical 22-report manifests matched SHA-256 `b31b3f3da1412d210096a618cc8d6b6acc5bbed167de525c8122704342c3d3fe`.

The accepted window includes all 22 batches and all 42 executable cases. The
historical 49-row registry classification is `26/16/7/0`; Phase F removed
active native/NCS source and configuration, and fresh clean-checkout no-NCS
acceptance remains pending.

- `xtask test` and `test-all` run `device_fixture`,
  `device_command_parity`, `device_framing_parity`, and
  `device_protocol_parity` normally;
- build/test CI explicitly reruns all replacement targets after ordinary
  `cargo test` on Linux x86_64. Production-path real-PTY fixture execution is
  Linux-only because macOS `serialport` baud configuration invokes
  `IOSSIOSPEED`, which macOS PTYs reject with `ENOTTY`; macOS still runs normal
  Rust fmt/build/test/clippy, its Linux-only fixture targets compile as zero
  tests, and controlled-backend coverage remains active. Windows adds no
  real-COM execution;
- Linux x86_64 explicitly invokes ignored
  `phase_e_public_boundary_repeat_gate` with `--test-threads=1`. It runs 100
  fixed-order iterations under seed `0x50484153455f4545`, each using real
  `DeviceFixture`, `TestServer`, MCP client, and public tools for ping, bounded
  flood stop metadata, hold/release, output flush and later exchange, peer
  disconnect, stable-path endpoint replacement/reconnect, and bounded explicit
  client/server/fixture teardown;
- `tests/doc_drift.rs` owns mechanical 49-row lock: every exact native name has
  one mapping row, unknown/duplicate rows fail, replacement identifiers and
  retirement proofs must exist in test source, and CI/xtask command wiring is
  guarded. Batch 7 and Batch 8 schema/report, direct-Compared/no-baseline
  status, exact positions, and deliberate `elapsed_ms` omission are also
  drift-locked.

Native 43+6 is retained only as historical parity evidence. Phase E parity
evidence is accepted; Phase F removed active native/NCS coupling. Fresh
clean-checkout no-NCS acceptance remains pending, and Phase E assertions were
not relaxed.

## Resume Checklist

After context loss, read in this order:

1. this checkpoint;
2. `native-sim-replacement-research-plan.md`;
3. `native-sim-virtual-serial-candidate-survey.md`;
4. `native-sim-boundary-prototype-results.md` and
   `native-sim-replacement-recommendation.md`;
5. ignored results under `target/native-sim-research/`;
6. `tests/common/device_fixture/{mod.rs,core.rs}` and
   `tests/device_fixture.rs`;
7. `native-sim-test-traceability.md` before porting next scenario batch.

Do not rewrite historical parity evidence or generated report paths.
