# `native_sim` Test Traceability and NCS Coupling

**Status:** Phase E required Rust PTY gate and full normalized differential parity
window accepted on 2026-08-23. All 49 historical native cases retain one
explicit mapping row. Phase F removed active firmware, NCS, native-job,
release-dependency, and Nix coupling from source/config; retained rows and
batch evidence remain historical. Fresh clean-checkout CI acceptance remains
pending, so ADR status stays Proposed.

## Full normalized parity window — 2026-08-23

> Full normalized differential parity window accepted on 2026-08-23: all 42 executable cases passed in three serial 22-batch runs; two consecutive canonical 22-report manifests matched SHA-256 `b31b3f3da1412d210096a618cc8d6b6acc5bbed167de525c8122704342c3d3fe`.

> Local worktree verification on 2026-08-23 passed `nix flake check --accept-flake-config` and `nix develop --ignore-env`; the shell found no `west`, `nrfutil`, or `nrfutil-sdk-manager` on `PATH` before `cargo test --locked`. This is not fresh clean-checkout CI evidence.

The 49-row registry classification is `26/16/7/0`: 26 Compared, 16
BaselineAndStronger, 7 Retired, and 0 pending. Seven rows are retired with
cited stronger public proofs:

- `native_list_ports_after_open` — `call_tool_list_ports_returns_structured_result`, `ports_resource_includes_profile_match_map`, and `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`.
- `native_flush_after_write` — `output_flush_after_full_delivery_preserves_later_traffic`.
- `native_reopen_same_port_after_close_works` — `reopen_same_path_returns_distinct_id_and_only_fresh_generation`.
- `native_list_ports_includes_identity_fields` — `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`, `list_ports_preview_selected_winner_matches_later_bare_open`, and `list_ports_preview_output_validates_against_generated_schema`.
- `native_txbuf_status_reports_pending` — `held_output_reports_nonzero_queue_then_drains_and_recovers`.
- `native_auto_reconnect_preserves_connection` — `public_mcp_ping_hold_disconnect_replace_and_reconnect`.
- `native_capture_boot_arm_only_captures_post_arm_command_output` — `capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor`.

Batch 22 extends executable native-versus-fixture comparison through shared
public MCP scenarios. Global registry status is **26 compared, 16 baseline-and-stronger, 7 retired, and 0 pending** rows. Batch 1 remains an
isolated eight-case command/lifecycle report; Batch 2 remains six generic
matching/framing cases; Batch 3 remains five raw generic-framing cases; Batch 4
adds two flood/buffer cases; Batch 5 adds three command-diagnostic cases; Batch
6 adds one ACK state-machine case; Batch 7 adds one output-flush case; Batch 8
adds one raw SLIP happy-path case; Batch 9 adds one malformed raw SLIP
baseline-and-stronger case; Batch 10 adds one direct raw SLIP recovery case;
Batch 11 adds one direct COBS-preset case as a historical checkpoint; Batch 12
adds one direct AT-parser case as a historical checkpoint; Batch 13 adds one
protocol-default case as a historical checkpoint; Batch 14 adds one JSON-parser
case as a historical checkpoint; Batch 15 is historical and adds two
NDJSON-preset cases; Batch 16 is historical and adds one NMEA-0183 preset case;
Batch 17 is historical and adds one Modbus ASCII preset case; Batch 18 is
historical and adds one close-while-read case; Batch 19 is historical and adds
one reopen/fresh-output baseline-and-stronger case; Batch 20 is historical and adds
one flush-during-armed-delay baseline-and-stronger case; Batch 21 is historical and
adds one input-flush backlog Compared case; Batch 22 is historical and adds one
bootloader-touch process-exit Compared case. Each batch has a separate report.
The accepted full normalized parity window covers all 22 batches and all 42
executable rows. Phase F subsequently removed active native/NCS source and
configuration; fresh clean-checkout CI acceptance remains pending. Historical Batch 21 counts are 25 compared, 16 baseline-and-stronger,
3 retired, and 5 pending (`25/16/3/5`) with 41 covered rows. Current counts are 26 compared, 16 baseline-and-stronger,
7 retired, and 0 pending (`26/16/7/0`) with 46 covered rows. Global registry status is **26 compared, 16 baseline-and-stronger, 7 retired, and 0 pending** rows. Current status is **26 compared, 16 baseline-and-stronger, 7 retired, and 0 pending** (`26/16/7/0`) with 46 covered rows. All 49 registry rows now have compared, baseline-and-stronger, or explicit retired disposition; full normalized differential parity is accepted, active native/NCS source and configuration coupling is removed, and fresh clean-checkout CI acceptance remains pending. Historical Batch 20 counts are 24 compared, 16 baseline-and-stronger,
3 retired, and 6 pending (`24/16/3/6`). Batch 20 historical registry status is **24 compared, 16 baseline-and-stronger, 3 retired, and 6 pending** rows. Historical Batch 19 status was 24
compared, 15 baseline-and-stronger, 3 retired, and 7 pending (`24/15/3/7`)
with 39 covered rows. Historical Batch 18 status was 24
compared, 14 baseline-and-stronger, 3 retired, and 8 pending (`24/14/3/8`)
with 38 covered rows. Historical Batch 16 status was 22 compared,
14 baseline-and-stronger, 3 retired, and 10 pending (`22/14/3/10`) with 36 covered
rows. Historical Batch 15 status was 21 compared,
14 baseline-and-stronger, 3 retired, and 11 pending (`21/14/3/11`)
with 35 covered rows.

Batch 12 is direct explicit-parser evidence for
`native_read_at_parser_parses_pong`. Both endpoints use standard anonymous public
`open` at 115200 with `profile_mode: "none"`, public boot-banner literal-match
`read`, public `transact("arm_cmd 1000\r\n")` matching exact
`arm_cmd delay=1000\r\n`, and public UTF-8 `write("ping\r\n")` with
`bytes_written=decoded_bytes=6`. Target `read` starts with
`from={"type":"now"}`, `encoding="utf8"`, `timeout_ms=3000`, explicit
`rx_framing: line`, and explicit `rx_parser: at_command`. Setup calls are
validated but excluded from normalized observations; the one-second public arm
barrier replaces source-test sleep/flush behavior.

The normalized target is anonymous, normal UTF-8 `pong\r\n`, with
`bytes_read/bytes_observed/bytes_returned=6/0/0`, `stop_reason=timeout`, no
match, truncation, drop, or error, and one frame at index 0 with type `line`,
encoding `utf8`, payload `pong`, and parsed
`{"parser":"at_command","response_type":"data","command":null,"status":null,"fields":["pong"]}`.
Positions are `52/58/0/0/0/58` in
`from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
order. The retained target characterization is
`target/native-sim-differential/at-parser-characterization.json` with SHA-256
`52b573c8a71da8aa52fa6ce12ce81d63f5f30756839ae8db3a1e4e56a6424eb5`.
Batch 12 report schema is
`serial-mcp.native-sim-differential.at-parser-batch.v1` and report filename is
`at-parser-batch.json`.
The existing `at_command_connection_default_drives_stateful_transact_and_parser_quirk`
`AtPeer` fixture test remains stronger stateful AT behavior. Batch 12 remains
historical partial evidence; Batch 13 remains historical partial evidence; Batch
14 remains historical partial evidence; Batch 15 remains historical partial
evidence; Batch 16 is historical partial evidence; Batch 17 is historical partial
evidence; Batch 18 is historical direct Compared evidence; Batch 19 is historical
baseline-and-stronger evidence; Batch 20 is historical baseline-and-stronger
evidence; Batch 21 is historical direct Compared evidence; Batch 22 is historical
direct Compared evidence; Phase F blocked.

Batch 13 is historical direct Compared evidence for
`native_open_protocol_default_drives_write_and_read`. Open uses a protocol-only
default `protocol: {"type":"at_command"}` and does not set `rx_framing`.
Public setup uses default-framed `transact("arm_cmd 1000")` with the stripped
framed arm match `arm_cmd delay=1000`, then bare `ping` receives the AT TX CR
addition (`4→5`). Target bare `read` uses only `from={"type":"now"}`,
`timeout_ms=3000`, and UTF-8 encoding. It returns `pong\r\n` with
`bytes_read/bytes_observed/bytes_returned=6/0/0`, one UTF-8 line frame parsed by
AT as data with fields `["pong"]`, and positions `52/58/0/0/0/58`.
The report schema is
`serial-mcp.native-sim-differential.at-protocol-default-batch.v1`, report is
`at-protocol-default-batch.json`, and retained characterization is
`target/native-sim-differential/at-protocol-default-characterization.json` with
SHA-256
`cce2c8a47d3d23eedfb857b5701428937174ab066bd6b64ce20e544776b68775`.
This covers 32 covered rows; stronger `AtPeer` proof remains required. Batch 14
was current direct Compared evidence for
`native_read_json_parser_decodes_jsonout`. Both endpoints use standard anonymous
public `open` at 115200 with `profile_mode: "none"`, public boot-banner literal
match `read`, public `transact("arm_cmd 1000\r\n")` matching exact
`arm_cmd delay=1000\r\n`, and public UTF-8 `write("jsonout\r\n")` with
`bytes_written=decoded_bytes=9`. Target `read` uses `from={"type":"now"}`,
UTF-8, 3000 ms, explicit `rx_framing: {"type":"line"}`, and explicit
`rx_parser: {"type":"json_lines"}`. The static three JSON object response is
140 bytes. The normalized target has `bytes_read/bytes_observed/bytes_returned`
of `140/0/0`, stop reason `timeout`, no match, truncation, drops, or error, and
three ordered UTF-8 line frames parsed as JSON objects for `temp`, `humidity`,
and `pressure`. Positions are `52/192/0/0/0/192`. The report schema is
`serial-mcp.native-sim-differential.json-parser-batch.v1`, report is
`json-parser-batch.json`, and retained characterization is
`target/native-sim-differential/json-parser-characterization.json` with SHA-256
`f51b5d77bac3904d214e2ea76794cf1d10f4d5aa8849224e750af30a8e9e3a06`. Existing
`json_lines_preset_writes_line_and_preserves_object_only_parser_behavior` fixture
proof remains stronger. The existing stronger JSON Lines fixture proof remains
stronger. Batch 14 is historical partial evidence: 19 compared,
14 baseline-and-stronger, 3 retired, and 13 pending (`19/14/3/13`) with 33
covered rows. Batch 15 is historical direct Compared evidence
for the two NDJSON preset rows
`native_read_ndjson_preset_decodes_json_frames` and
`native_read_ndjson_preset_skips_empty_lines`.

**Compared Batch 15.** Static NDJSON payload `{"a":1}\n\n{"b":2}\n`;
`protocol: {"type":"ndjson"}` uses auto line framing, `skip_empty:true`, and
JSON parser; exact `17/0/0` timeout result with ordered parsed `a`/`b` frames and
positions `52/69/0/0/0/69`; command is
`sendraw hex 7B2261223A317D0A0A7B2262223A327D0A` with a 48-byte write; stronger
NDJSON fixture proof remains independent.

**Compared Batch 15.** Static NDJSON payload `{"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n`;
`protocol: {"type":"ndjson"}` uses auto line framing, `skip_empty:true`, and
JSON parser; exact `30/0/0` timeout result with ordered parsed `a`/`b`/`c` frames
and positions `52/82/0/0/0/82`; blank and whitespace-only lines emit no frames;
command is
`sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A` with a
74-byte write; stronger NDJSON fixture proof remains independent. Batch 15 schema is
`serial-mcp.native-sim-differential.ndjson-preset-batch.v1`, report is
`ndjson-preset-batch.json`, and retained characterization is
`target/native-sim-differential/ndjson-characterization.json` with SHA-256
`10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3`. Batch 15
historical checkpoint is `21/14/3/11` with 35 covered rows.

**Compared Batch 16.** Static 67-byte valid GGA sentence
`$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n`;
fixture command is `sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A`;
`protocol: {"type":"nmea0183"}` uses start/end framing with markers excluded and
NMEA parser; exact `67/0/0` timeout result with one UTF-8 `start_end` frame,
parsed `talker_id="GP"`, `sentence_type="GGA"`, exact ordered fields
`["123519","4807.038","N","01131.000","E","1","08","0.9","545.4","M","46.9","M","",""]`,
`checksum_valid:true`, and positions `52/119/0/0/0/119`; stronger NMEA fixture
proof remains independent. Batch 16 schema is
`serial-mcp.native-sim-differential.nmea0183-preset-batch.v1`, report is
`nmea0183-preset-batch.json`, and retained characterization is
`target/native-sim-differential/nmea-characterization.json` with SHA-256
`513d4906b285ef35a2b82ab085968de96f51576acc636244f0eb3a44868f1578`.
Historical Batch 16 registry was `22/14/3/10` with 36 covered rows; Phase F
blocked.

**Compared Batch 17.** Static valid Modbus ASCII response
`:010300000001FB\r\n`; the fixture command is
`sendraw hex 3A30313033303030303030303146420D0A` with a 48-byte UTF-8 write.
The target uses `protocol: {"type":"modbus_ascii"}` only; the preset supplies
start/end framing with markers excluded and Modbus ASCII parsing. Exact target
result is `17/0/0` bytes read/observed/returned, `timeout`, no match,
truncation, drop, or error, with positions `52/69/0/0/0/69`. One UTF-8
`start_end` frame has payload `010300000001FB` and parsed fields
`address=1`, `function_code=3`, `data=[0,0,0,1]`, and `checksum_valid:true`.
Batch 17 schema is
`serial-mcp.native-sim-differential.modbus-ascii-preset-batch.v1`, report is
`modbus-ascii-preset-batch.json` with SHA-256
`97f9c00dc98e5cd83c440b2e90b7d9f72e58428f9e873f7f418475ef3a79ef9b`, and retained characterization is
`target/native-sim-differential/modbus-ascii-characterization.json` with SHA-256
`b390bdc693778be29a06e40033694e1032986a1700995d065f06d8092fc7973c`.
The existing `modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation`
fixture proof remains stronger. Historical Batch 17 registry was `23/14/3/9` with
37 covered rows; Phase F blocked.

**Historical Compared Batch 18.** Readiness-proven public close sequence uses `arm_cmd 1000\r\n`, baseline `get_status.rx_bytes`, secondary unmatched `from={"type":"now"}` read, and primary marker command `sendraw hex 524541442D52454144592D4D41524B45520D0A\r\n` writing `READ-READY-MARKER\r\n` after `rx_bytes` increases by 19 while pending read remains unfinished; primary close returns normal anonymous profile with `source="disabled"`, `profile_persistence.operation="close_snapshot"`, and `profile_persistence.state="transient"`; pending read returns exact marker with `19/19/19`, `connection_closed`, no match/truncation/frames/drops/error, and positions `52/71/0/0/0/71`; stronger fixture proof remains independent. Batch 18 schema is
`serial-mcp.native-sim-differential.close-while-read-batch.v1`, report is
`close-while-read-batch.json` with SHA-256
`fef0499d6504635c104fe3d125fe572c49e2ef4bec69a5b39c32bdeb50361a09`, and retained characterization is
`target/native-sim-differential/close-while-read-characterization.json` with SHA-256
`06a7adb2b4f1c1c6f8b3c8fd507ba9e5004df6dd7a9f4ef6b64c4fff87c3f69b`. Historical
Batch 18 registry was `24/14/3/8` with 38 covered rows; Phase F blocked.

**Historical Baseline-and-stronger Batch 19.** Standard anonymous first `open` at 115200 with `profile_mode: "none"`; boot banner is synchronized once because firmware emits it only at process start; first `write("ping\r\n")` is normal UTF-8 `6/6`, and positioned first literal `pong` read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/truncation, positions `32/38/0/0/0/38`; normal first close retains disabled/transient close-profile checks; same endpoint second `open` requires a distinct raw connection ID, public status verifies `rx_bytes=0` before second command, and pending second `from={"type":"now"}` UTF-8 `pong` read uses bounded 100 ms baseline admission before independent-client `write("ping\r\n")`; second read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/truncation, positions `0/6/0/0/0/6`; the status probe is not normalized; stronger fixture proof remains independent. Batch 19 schema is
`serial-mcp.native-sim-differential.reopen-fresh-output-batch.v1`, report is
`reopen-fresh-output-batch.json` with SHA-256
`7c9d9071156739a0a2bc81a9ef2adba48a40132ba6bd1208b99e8e04a847d02e`. Current
registry is historical `24/15/3/7` with 39 covered rows; Phase F blocked.

**Historical Baseline-and-stronger Batch 20.** Standard anonymous `open` at 115200 with
`profile_mode: "none"`; boot banner and arm acknowledgement are validated before
normalized observations. Public `transact("arm_cmd 1000\r\n")` validates exact
`arm_cmd delay=1000\r\n` acknowledgement but setup result is not normalized.
Public `write("ping\r\n")` is normal anonymous UTF-8 `6/6`. Existing
`INHERITED_BASELINE_DELAY` supplies one bounded 100 ms baseline admission window;
it is not a peer-acceptance proof. Public `flush(target="both")` is retained as
a normal anonymous `FlushObservation` with exact target `both`. Positioned
`from={"type":"now"}` UTF-8 literal `pong\r\n` read returns exact `pong\r\n`,
`6/6/6`, `match_found`, index 0, no truncation/frames/drops/error, and positions
`52/58/0/0/52/58` in
`from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
order. Stronger fixture proof remains
`flush_after_command_acceptance_does_not_cancel_delayed_response`. Batch 20
schema is `serial-mcp.native-sim-differential.flush-during-arm-delay-batch.v1`,
report is `flush-during-arm-delay-batch.json` with SHA-256
`4808262720b252c53f95e88a56ea1d8565b238251884aed364c0e43d0b07f500`, and the
Batch 20 historical registry is `24/16/3/6` with 40 covered rows; Phase F blocked.

**Historical Compared Batch 21.** Standard anonymous `open` at 115200 with `profile_mode: "none"` and boot-banner read remain the first two normalized observations; public UTF-8 old-marker write `sendraw hex 4F4C442D4D41524B45520D0A\r\n` is normal anonymous with exact `bytes_written=decoded_bytes=38`; status-only `get_status` polling proves old marker reached retained RX at `rx_bytes >= 44` and is not normalized; public `flush(target="input")` is normal anonymous with exact target `input`; public UTF-8 new-marker write `sendraw hex 4E45572D4D41524B45520D0A\r\n` is normal anonymous with exact `bytes_written=decoded_bytes=38`; status-only polling proves `rx_bytes >= 56` and is not normalized; positioned `from={"type":"buffer_start"}` UTF-8 literal `NEW-MARKER\r\n` read returns exact `NEW-MARKER\r\n`, `12/12/12`, `match_found`, index 0, no truncation/frames/drops/error, positions `44/56/0/0/44/56`; stronger fixture proof `flush_input_discards_known_old_marker_and_keeps_new_marker` remains independent. Batch 21 schema is `serial-mcp.native-sim-differential.input-flush-batch.v1`, report is `input-flush-batch.json` with SHA-256 `ff95074207f7de216780ede42a6d21583e4e88e5c5e5c6f81af4b588fa5dcfd8` after two matching runs, and historical registry is `25/16/3/5` with 41 covered rows; Phase F blocked.

**Historical Compared Batch 22.** Standard anonymous `open` at 115200 with `profile_mode: "none"` and matching boot-banner read remain the first two normalized observations; fixture side is a real dedicated small Rust child PTY, not `FixtureExit::Crashed`; child emits exact `serial-mcp test firmware ready\r\n` before `PTY_PATH`; public UTF-8 `write("touch\r\n")` is normal anonymous with exact `bytes_written=decoded_bytes=7`; both native firmware and child endpoint exit exactly 42, retained as typed `process_exit` observation; terminal `touch exit(42)\r\n` response delivery is not claimed; no public `close` follows peer exit; stronger fixture proof `touch_write_causes_small_rust_child_peer_to_exit_42` remains independent. Batch 22 schema is `serial-mcp.native-sim-differential.bootloader-touch-exit-batch.v1`, report is `bootloader-touch-exit-batch.json` with SHA-256 `91befb4e3af3edd65c70c58208be03c09c8c29aed04f9b432e18c1d5becd4d9c` after two matching runs, and historical registry is `26/16/3/4` with 42 covered rows; Phase F blocked.

Post-Batch 22 retirement: `native_list_ports_includes_identity_fields` is **Retired**, not a Batch 23 or differential case. Deterministic public real-PTY/injected-provider proofs `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`, `list_ports_preview_selected_winner_matches_later_bare_open`, and `list_ports_preview_output_validates_against_generated_schema` supersede ambient OS field/null checks. No new differential batch, report, or hash was created. The pre-txbuf-retirement checkpoint is `26/16/4/3` with 43 covered rows (26 compared, 16 baseline-and-stronger, 4 retired, and 3 pending); full parity and Phase F remain blocked.

Post-Batch-22 txbuf retirement: `native_txbuf_status_reports_pending` is **Retired**, not a Batch 23 or differential case. Source did not observe a nonzero pending TX queue; `txbuf` and `hold` are firmware-only commands. `device_command_parity::held_output_reports_nonzero_queue_then_drains_and_recovers` proves nonzero held output, blocked read, drain, and recovery. No Batch 23, report, or hash was created. The historical pre-auto-reconnect-retirement checkpoint is `26/16/5/2` with 44 covered rows; full parity and Phase F remain blocked.

Post-Batch-22 auto-reconnect retirement: `native_auto_reconnect_preserves_connection` is **Retired**, not a Batch 23 or differential case. Source only invokes `reconnect` while already open; strengthened `device_fixture::public_mcp_ping_hold_disconnect_replace_and_reconnect` preserves same ID/open state and post-no-op traffic, then proves peer loss, replacement, reconnect, and fresh exchange. No Batch 23, report, or hash was created. This is the historical pre-capture-boot-retirement checkpoint at `26/16/6/1` with 45 covered rows; full parity and Phase F remain blocked.

Post-Batch-22 capture-boot retirement: `native_capture_boot_arm_only_captures_post_arm_command_output` is **Retired**, not a Batch 23 or differential case. Source only checks arm-only post-mark output after a consumed stale banner; strengthened `device_command_parity::capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor` excludes stale bytes, preserves private-mark replay and shared cursor/history, and captures post-arm output. No Batch 23, report, or hash was created.

Batch 21 retains exact marker bytes: old response `OLD-MARKER\r\n`; new response
`NEW-MARKER\r\n`.

## Implemented Replacement Mapping

Current required replacement targets: `tests/device_fixture.rs`,
`tests/device_command_parity.rs`, `tests/device_framing_parity.rs`, and
`tests/device_protocol_parity.rs`.

| Native case | Replacement case | Differential evidence |
|---|---|---|
| `native_ping_roundtrip` | `ping_roundtrip_uses_real_path_and_literal_match` | real-path literal `match_found` proof |
| `native_pending_read_then_write_ping_roundtrip` | `pending_read_receives_later_output_after_readiness_proven_hold` | hold/readiness proof |
| `native_split_writes_preserve_command_order` | `split_writes_preserve_one_command_and_exact_wire_order` | exact concatenated PTY input |
| `native_framing_reports_single_split_command` | `split_writes_preserve_one_command_and_exact_wire_order` | duplicate CRLF `LINE len=4 data="ping"` diagnostic plus one `pong` |
| `native_trace_reports_exact_split_byte_sequence` | `split_writes_preserve_one_command_and_exact_wire_order` | exact lower-case `RX[n]=0xhh\r\n` trace plus one `pong` |
| `native_read_match_on_spam_complete` | `finite_flood_matcher_reaches_unique_completion_marker` | exact live `spam 1024 hex` completion marker |
| `native_read_buffer_budget_stops_under_flood` | `live_buffer_budget_caps_finite_flood_with_exact_stop_metadata` | live `spam 512 hex` exact 256-byte budget metadata |
| `native_bootloader_touch_exits_42` | `touch_write_causes_small_rust_child_peer_to_exit_42` | **Compared Batch 22.** Standard anonymous `open` at 115200 with `profile_mode: "none"` and matching boot-banner read remain the first two normalized observations; fixture side is a real dedicated small Rust child PTY, not `FixtureExit::Crashed`; child emits exact `serial-mcp test firmware ready\r\n` before `PTY_PATH`; public UTF-8 `write("touch\r\n")` is normal anonymous with exact `bytes_written=decoded_bytes=7`; both native firmware and child endpoint exit exactly 42, retained as typed `process_exit` observation; terminal `touch exit(42)\r\n` response delivery is not claimed; no public `close` follows peer exit; stronger fixture proof `touch_write_causes_small_rust_child_peer_to_exit_42` remains independent. |
| `native_list_ports_after_open` | `call_tool_list_ports_returns_structured_result`; `ports_resource_includes_profile_match_map`; `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports` | **Retired.** Deterministic public tool/resource preview proof is stronger than ambient enumeration. |
| `native_list_ports_includes_identity_fields` | `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`; `list_ports_preview_selected_winner_matches_later_bare_open`; `list_ports_preview_output_validates_against_generated_schema` | **Retired.** Deterministic injected-provider identity/profile/schema proofs supersede ambient OS field/null checks. |
| `native_flush_after_write` | `output_flush_after_full_delivery_preserves_later_traffic` | **Retired.** Contract permits queued-output discard; stronger fully delivered output proof retains valid behavior. |
| `native_get_status_after_write_increments_tx_counter` | `status_reports_exact_io_deltas_and_activity` | exact TX/RX/write deltas |
| `native_reconfigure_baud_rate_persists` | `reconfigure_updates_status_and_connection_remains_functional` | transitions, status, post-change traffic |
| `native_ack_command_provides_pre_execution_ack` | `ack_peer_orders_ack_before_response_and_stops_after_disable` | **Compared.** Direct ACK state-machine comparison; existing Rust-PTY proof remains stronger semantic coverage. |
| `native_txbuf_status_reports_pending` | `held_output_reports_nonzero_queue_then_drains_and_recovers` | **Retired.** Source checks only idle `txbuf` diagnostics and hold recovery; deterministic public real-PTY proof covers nonzero held output, blocked read, drain, and recovery. |
| `native_flush_input_clears_host_rx` | `flush_input_discards_known_old_marker_and_keeps_new_marker` | **Historical Compared Batch 21.** Standard anonymous `open` at 115200 with `profile_mode: "none"` and boot-banner read remain the first two normalized observations; public UTF-8 old-marker write `sendraw hex 4F4C442D4D41524B45520D0A\r\n` is normal anonymous with exact `bytes_written=decoded_bytes=38`; status-only `get_status` polling proves old marker reached retained RX at `rx_bytes >= 44` and is not normalized; public `flush(target="input")` is normal anonymous with exact target `input`; public UTF-8 new-marker write `sendraw hex 4E45572D4D41524B45520D0A\r\n` is normal anonymous with exact `bytes_written=decoded_bytes=38`; status-only polling proves `rx_bytes >= 56` and is not normalized; positioned `from={"type":"buffer_start"}` UTF-8 literal `NEW-MARKER\r\n` read returns exact `NEW-MARKER\r\n`, `12/12/12`, `match_found`, index 0, no truncation/frames/drops/error, positions `44/56/0/0/44/56`; stronger fixture proof `flush_input_discards_known_old_marker_and_keeps_new_marker` remains independent. |
| `native_flush_during_arm_cmd_delay` | `flush_after_command_acceptance_does_not_cancel_delayed_response` | **Historical Baseline-and-stronger Batch 20.** Standard anonymous `open` at 115200 with `profile_mode: "none"`; boot banner and arm acknowledgement are validated before normalized observations; public `transact("arm_cmd 1000\r\n")` validates exact `arm_cmd delay=1000\r\n` acknowledgement but setup result is not normalized; public `write("ping\r\n")` is normal UTF-8 `6/6`; bounded 100 ms baseline admission window is not a peer-acceptance proof; public `flush(target="both")` is normal anonymous with exact target `both`; positioned `from={"type":"now"}` UTF-8 literal `pong\r\n` read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no truncation/frames/drops/error, positions `52/58/0/0/52/58`; stronger fixture proof `flush_after_command_acceptance_does_not_cancel_delayed_response` remains independent. |
| `native_flush_output_after_full_delivery_is_safe` | `output_flush_after_full_delivery_preserves_later_traffic` | **Compared.** First matched `pong` is delivery boundary; output-only flush retains cursor and later traffic. Existing Rust-PTY proof remains stronger semantic coverage. |
| `native_partial_line_buffered_then_completed` | `split_writes_preserve_one_command_and_exact_wire_order` | `pi` stays pending for bounded delay; `ng\r\n` completes command |
| `native_read_regex_matches_pong` | `regex_and_glob_matchers_find_complete_peer_line` | regex mode |
| `native_read_glob_matches_pong_line` | `regex_and_glob_matchers_find_complete_peer_line` | glob mode |
| `native_auto_reconnect_preserves_connection` | `public_mcp_ping_hold_disconnect_replace_and_reconnect` | **Retired.** Source only invokes `reconnect` while already open; strengthened public real-PTY proof preserves same ID/open state and post-no-op traffic, then proves peer loss, replacement, reconnect, and fresh exchange. |
| `native_read_line_framing_splits_lines` | `line_framing_returns_exact_ordered_peer_frames` | original native test remains required; exact ordered frame proof is retained |
| `native_read_json_parser_decodes_jsonout` | `json_lines_preset_writes_line_and_preserves_object_only_parser_behavior` | **Compared Batch 14.** Static three JSON object response; explicit line framing plus `json_lines` parser; exact `140/0/0` timeout result with three ordered parsed objects and positions `52/192/0/0/0/192`; existing JSON Lines fixture proof remains stronger. |
| `native_read_at_parser_parses_pong` | `at_command_connection_default_drives_stateful_transact_and_parser_quirk` | **Compared Batch 12.** Direct explicit-parser `line` + `at_command` result; existing `AtPeer` test remains stronger stateful AT behavior |
| `native_read_framing_max_frames_stops` | `max_frames_stops_after_exact_limit` | exact frame limit |
| `native_read_framing_plus_match_combined` | `framing_plus_match_returns_matching_frame_and_index` | framed match/index |
| `native_open_protocol_default_drives_write_and_read` | `at_command_connection_default_drives_stateful_transact_and_parser_quirk` | **Compared Batch 13.** Historical protocol-only open default controls bare write/read; stripped framed arm match and bare `ping` CR addition; existing `AtPeer` proof remains stronger. |
| `native_explicit_rx_framing_beats_connection_default` | `call_time_line_framing_beats_connection_delimiter_default` | call-time precedence |
| `native_read_slip_decodes_frame` | `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery` | **Compared Batch 8.** Direct raw `rx_framing: slip` happy-path result; existing richer `protocol: slip` fixture coverage remains stronger semantic proof. |
| `native_read_slip_malformed_escape_returns_partial_result` | `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery` | **Baseline-and-stronger Batch 9.** Original raw `rx_framing: slip` one-frame-free malformed result; stronger `protocol: slip` proof retains valid-frame-before-error and recovery. |
| `native_read_delimiter_framing_decodes` | `delimiter_length_prefixed_and_start_end_decode_exact_payloads` | delimiter decoder |
| `native_read_length_prefixed_framing_decodes` | `delimiter_length_prefixed_and_start_end_decode_exact_payloads` | real length-prefixed decoder |
| `native_read_start_end_framing_decodes` | `delimiter_length_prefixed_and_start_end_decode_exact_payloads` | start/end decoder |
| `native_write_tx_framing_modes_observed_via_trace` | `tx_framing_modes_produce_exact_independent_wire_vectors` | direct raw peer vectors |
| `native_read_explicit_line_endings_split_correctly` | `explicit_line_endings_split_with_documented_terminator_semantics` | LF/CR/CRLF table |
| `native_read_slip_recovers_after_error_on_next_call` | `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery` | **Compared Batch 10.** Shared public double-arm/sendraw scaffold compares raw `rx_framing: slip` malformed output followed by exact next-call recovery; stronger `protocol: slip` proof remains separate. |
| `native_read_cobs_preset_decodes_frame` | `cobs_preset_uses_independent_zero_byte_vector_for_write_and_read` | **Compared Batch 11.** Direct `protocol: {"type":"cobs"}` preset comparison uses static independent wire `00 05 70 6f 6e 67 00`; one raw-parser COBS frame remains distinct from raw COBS framing, while broader zero-containing COBS TX/RX fixture proof remains stronger semantic coverage. |
| `native_read_ndjson_preset_decodes_json_frames` | `ndjson_preset_parses_records_and_skips_blank_whitespace_lines` | **Compared Batch 15.** Static NDJSON payload `{"a":1}\n\n{"b":2}\n`; `protocol: {"type":"ndjson"}` uses auto line framing, `skip_empty:true`, and JSON parser; exact `17/0/0` timeout result with ordered parsed `a`/`b` frames and positions `52/69/0/0/0/69`; stronger NDJSON fixture proof remains independent. |
| `native_read_ndjson_preset_skips_empty_lines` | `ndjson_preset_parses_records_and_skips_blank_whitespace_lines` | **Compared Batch 15.** Static NDJSON payload `{"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n`; `protocol: {"type":"ndjson"}` uses auto line framing, `skip_empty:true`, and JSON parser; exact `30/0/0` timeout result with ordered parsed `a`/`b`/`c` frames and positions `52/82/0/0/0/82`; blank and whitespace-only lines emit no frames; stronger NDJSON fixture proof remains independent. |
| `native_read_nmea0183_preset_decodes_parsed_frame` | `nmea0183_preset_parses_valid_independently_checksummed_sentence` | **Compared Batch 16.** Static 67-byte valid GGA sentence `$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n`; `protocol: {"type":"nmea0183"}` uses start/end framing with markers excluded and NMEA parser; exact `67/0/0` timeout result with one UTF-8 `start_end` frame, parsed `talker_id="GP"`, `sentence_type="GGA"`, exact ordered fields `["123519","4807.038","N","01131.000","E","1","08","0.9","545.4","M","46.9","M","",""]`, `checksum_valid:true`, and positions `52/119/0/0/0/119`; stronger NMEA fixture proof remains independent. |
| `native_read_modbus_ascii_preset_decodes_parsed_frame` | `modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation` | **Compared Batch 17.** Static valid Modbus ASCII response `:010300000001FB\r\n`; `protocol: {"type":"modbus_ascii"}` uses start/end framing with markers excluded and Modbus ASCII parser; exact `17/0/0` timeout result with one UTF-8 `start_end` frame, parsed `address=1`, `function_code=3`, `data=[0,0,0,1]`, `checksum_valid:true`, and positions `52/69/0/0/0/69`; stronger Modbus fixture proof remains independent. |
| `native_capture_boot_arm_only_captures_post_arm_command_output` | `capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor` | **Retired.** Source only checks arm-only post-mark output after a consumed stale banner; deterministic public real-PTY proof excludes stale bytes, preserves private-mark replay and shared cursor/history, and captures post-arm output. |
| `native_named_connection_appears_in_list_connections` | `named_connection_summary_uses_fixture_stable_path` | named public summary |
| `native_set_flow_control_updates_summary_and_result` | `flow_control_none_at_open_and_live_set_are_reflected_in_summary` | live result/summary |
| `native_close_while_read_active_returns_normal_result` | `close_interrupts_readiness_proven_live_read_with_connection_closed` | **Historical Compared Batch 18.** Readiness-proven public close sequence uses `arm_cmd 1000\r\n`, baseline `get_status.rx_bytes`, secondary unmatched `from={"type":"now"}` read, and primary marker command `sendraw hex 524541442D52454144592D4D41524B45520D0A\r\n` writing `READ-READY-MARKER\r\n` after `rx_bytes` increases by 19 while pending read remains unfinished; primary close returns normal anonymous profile with `source="disabled"`, `profile_persistence.operation="close_snapshot"`, and `profile_persistence.state="transient"`; pending read returns exact marker with `19/19/19`, `connection_closed`, no match/truncation/frames/drops/error, and positions `52/71/0/0/0/71`; stronger fixture proof remains independent. |
| `native_reopen_same_port_after_close_works` | `reopen_same_path_returns_distinct_id_and_only_fresh_generation` | **Retired.** Stronger distinct-ID and fresh-generation proof retains reopen behavior. |
| `native_reopen_then_match_finds_fresh_output` | `reopen_same_path_returns_distinct_id_and_only_fresh_generation` | **Historical Baseline-and-stronger Batch 19.** Standard anonymous first `open` at 115200 with `profile_mode: "none"`; boot banner is synchronized once because firmware emits it only at process start; first `write("ping\r\n")` is normal UTF-8 `6/6`, and positioned first literal `pong` read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/truncation, positions `32/38/0/0/0/38`; normal first close retains disabled/transient close-profile checks; same endpoint second `open` requires a distinct raw connection ID, public status verifies `rx_bytes=0` before second command, and pending second `from={"type":"now"}` UTF-8 `pong` read uses bounded 100 ms baseline admission before independent-client `write("ping\r\n")`; second read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/truncation, positions `0/6/0/0/0/6`; the status probe is not normalized; stronger fixture proof remains independent. |
| `native_open_with_flow_control_persists_in_summary` | `flow_control_none_at_open_and_live_set_are_reflected_in_summary` | open-time result/summary |

Historical Phase E gate: all 49 native rows now have one required replacement, cited
stronger proof, or explicit retired disposition. Current focused target totals are
`device_fixture` 7/7, `device_command_parity` 19/19,
`device_framing_parity` 8/8, and `device_protocol_parity` 15/15 (7 public
cases + 7 durable peer-oracle units + 1 exact seven-preset coverage registry).
Linux x86_64 additionally invokes
`phase_e_public_boundary_repeat_gate` with `--ignored --test-threads=1`: 100
fixed-order iterations, seed `0x50484153455f4545`. Each iteration uses a real
`DeviceFixture`, `TestServer`, MCP client, and public tools for ping, bounded
flood, held/released output, post-delivery flush, peer disconnect,
stable-path replacement/reconnect, and explicit teardown. Historical native
oracle totals were 43/43 validation and 6/6 lifecycle. The accepted full
normalized differential parity window covers all 22 batches and all 42
executable cases; Phase F subsequently removed active native/NCS source and
configuration. Fresh clean-checkout CI acceptance remains pending.

## Historical Executable Differential Batches

The following batch details retain historical native-versus-fixture evidence;
they are not active test targets.

`tests/native_sim_differential.rs` is Linux-only and ignored because it needs
the prebuilt `native_sim` firmware binary. Each listed case invokes the same
typed scenario function once against `NativeSimFirmware` and once against a
dedicated `DeviceFixture` compatibility peer. Both paths use only public MCP
`open` and tool calls over real PTY slave paths. Successful output writes
deterministic paired normalized JSON under `target/native-sim-differential/`.

Batch 1 remains exactly its original eight cases and writes
`command-lifecycle-batch.json`. Its accepted historical split was 7 compared,
1 baseline-and-stronger, 3 retired, and 38 pending before Batch 2 moved six
rows in the global registry.

Compared rows:

1. `native_ping_roundtrip`
2. `native_split_writes_preserve_command_order`
3. `native_get_status_after_write_increments_tx_counter`
4. `native_reconfigure_baud_rate_persists`
5. `native_named_connection_appears_in_list_connections`
6. `native_set_flow_control_updates_summary_and_result`
7. `native_open_with_flow_control_persists_in_summary`

Baseline-and-stronger row:

- `native_pending_read_then_write_ping_roundtrip` uses historical bounded 100 ms
  baseline-only scheduling delay and independent exact-modern client for later
  write. This avoids same-transport scheduling artifact while preserving public
  process-wide connection state. It is not a readiness proof; required fixture test
  `pending_read_receives_later_output_after_readiness_proven_hold` remains the
  stronger proof and owns readiness semantics.

Batch 2 writes `generic-matching-framing-batch.json` with these six pairs:

1. `native_read_regex_matches_pong` — baseline-and-stronger; required proof
   `regex_and_glob_matchers_find_complete_peer_line`.
2. `native_read_glob_matches_pong_line` — baseline-and-stronger; required proof
   `regex_and_glob_matchers_find_complete_peer_line`.
3. `native_read_line_framing_splits_lines` — compared with a deterministic
   nested-command equivalent for line framing, not the original `pong`/`info`
   payloads; the original native test remains required and
   `line_framing_returns_exact_ordered_peer_frames` retains its fixture proof.
4. `native_read_framing_max_frames_stops` — baseline-and-stronger; required
   proof `max_frames_stops_after_exact_limit`.
5. `native_read_framing_plus_match_combined` — baseline-and-stronger; required
   proof `framing_plus_match_returns_matching_frame_and_index`.
6. `native_explicit_rx_framing_beats_connection_default` —
   baseline-and-stronger; required proof
   `call_time_line_framing_beats_connection_delimiter_default`.

Batch 2 normalizes complete framed data: top-level effective encoding and
bytes, independent counters, `match_frame_index`, every frame's index/type/
encoding/payload, AT parsed fields, `frames_dropped`, and framing error. Its
live reads retain Batch 1's bounded secondary exact-modern client pattern;
fixture proofs remain owner of stronger readiness or exclusion obligations.

The line-framing pair uses one semantic adaptation. The original
`native_read_line_framing_splits_lines` test remains required and still sends
`ping`, then dynamic `info`; the `info` response contains a firmware compile timestamp.
Batch 2 therefore does not compare those original `pong`/`info`
payloads or normalize that timestamp. Solely for deterministic line-framing
comparison, Batch 2 uses the nested-command equivalent `write cmd 1 ping`,
which produces the exact two-line stream `ack 1 exec>ping\r\npong\r\n` on both
peers. The required `line_framing_returns_exact_ordered_peer_frames` fixture
replacement proof remains in force.

Batch 3 writes `raw-generic-framing-batch.json` with five pairs and schema ID
`serial-mcp.native-sim-differential.raw-generic-framing-batch.v1`:

1. `native_read_delimiter_framing_decodes` — compared; delayed raw
   `|pong|` produces the exact empty and `pong` delimiter frames.
2. `native_read_length_prefixed_framing_decodes` — baseline-and-stronger;
   delayed raw `04 70 6f 6e 67` exercises the public one-byte length decoder;
   `delimiter_length_prefixed_and_start_end_decode_exact_payloads` is the
   required stronger fixture proof.
3. `native_read_start_end_framing_decodes` — compared; delayed raw
   `<<pong>>` produces the exact `pong` frame.
4. `native_write_tx_framing_modes_observed_via_trace` — baseline-and-stronger;
   exact independent host-to-peer vectors cover delimiter, length-prefixed,
   start/end, and SLIP; `tx_framing_modes_produce_exact_independent_wire_vectors`
   is the required stronger fixture proof.
5. `native_read_explicit_line_endings_split_correctly` — compared; one
   scenario retains LF, CR, and CRLF observations in deterministic table order.

Measured native characterization established that `arm_cmd 1000` followed by
`sendraw` and a public `from=now` read captures target-only delayed bytes: no
unexpected prefix appeared. Fresh native trace starts at sequence zero and
emits exactly one `RX[n]=0xhh\r\n` line per target byte, with no extra lines,
drops, or errors. Batch 3 uses that evidence rather than obsolete command-echo
theory: native setup calls are not normalized outcomes, while target reads and
independent peer-wire bytes remain exact.

Batch 4 writes `flood-buffer-batch.json` with two baseline-and-stronger pairs
and schema ID `serial-mcp.native-sim-differential.flood-buffer-batch.v1`:

1. `native_read_match_on_spam_complete` uses a pending public `from=now` read
   followed by exact `spam 1024 hex\r\n`. Native source initializes xorshift32
   state `0x12345678`, emits lower-case hex bytes in 256-byte chunks every
   10 ms, and emits exact start/completion text. The normalized UTF-8 stream is
   1088 bytes, with `Spam complete` at match index 1056.
2. `native_read_buffer_budget_stops_under_flood` first configures the live
   connection default to `max_buffered_bytes: 256`, then uses a pending public
   `from=now` read followed by exact `spam 512 hex\r\n`. The normalized result
   retains the exact first 256 stream bytes, `max_buffered_bytes` stop reason,
   unmatched state, all three byte counters at 256, and no truncation, frames,
   drops, or error. Both executable target reads retain all six stable public
   offset/backlog fields: `from_offset`, `next_offset`, `bytes_lost`,
   `buffered_remaining`, `start_offset`, and `end_offset`. The measured values
   are `32/1120/0/0/0/1120` for `spam 1024` and
   `32/288/0/31/0/319` for live `spam 512`, in that field order. The required stronger proofs are
   `finite_flood_matcher_reaches_unique_completion_marker` and
   `live_buffer_budget_caps_finite_flood_with_exact_stop_metadata`.

Raw characterization retained variable diagnostic `elapsed_ms` for live spam;
that wall-clock diagnostic is omitted from typed Batch 4 outcomes. The prefilled
`spam 65536 hex` sample retained variable backlog fields and is excluded from
executable parity because those values varied across fresh native runs. Live
512 supplies deterministic bounded semantics instead.

Batch 5 writes `command-diagnostics-batch.json` with three
baseline-and-stronger pairs and schema ID
`serial-mcp.native-sim-differential.command-diagnostics-batch.v1`:

1. `native_framing_reports_single_split_command` enables `framing on`, then
   sends `pi`, `ng`, and `\r\n` through a pending public `from=now` read. The
   exact 54-byte UTF-8 payload is
   `LINE len=4 data="ping"\r\nLINE len=4 data="ping"\r\npong\r\n`, with
   `match_index=48` and positioned cursor table `44/98/0/0/0/98` in
   `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
   order. The duplicate CRLF framing diagnostic is required native behavior;
   it is preserved, not collapsed.
2. `native_trace_reports_exact_split_byte_sequence` enables `trace on` and
   uses the same split public writes. The exact 78-byte payload contains the
   six lower-case records `RX[0]=0x70\r\n` through `RX[5]=0x0a\r\n`, followed
   by `pong\r\n`; `match_index=72` and the positioned table is
   `42/120/0/0/0/120`.
3. `native_partial_line_buffered_then_completed` starts the same pending
   public read, writes `pi`, proves the reader remains unfinished after the
   first bounded 100 ms delay, writes `ng\r\n`, and then observes exact
   `pong\r\n` (6 bytes, `match_index=0`) with positioned table
   `32/38/0/0/0/38`. No flush, status probe, raw fixture API, or direct serial
   I/O is used.

All eight batch reports remain isolated; Batch 5 and Batch 8 do not claim physical UART
behavior, clean-checkout parity without NCS, CI wiring, or full differential
parity. Native suites and Phase F remain blocked on complete differential
outcome comparison.

Batch 6 writes `ack-state-batch.json` with one direct Compared pair and schema ID
`serial-mcp.native-sim-differential.ack-state-batch.v1`. It uses five ordinary
shared-cursor public `write`/positioned `read` pairs after the 32-byte boot
banner, in exact order:

| Step | Write bytes | Exact read payload | Read bytes | Match index | `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset` |
|---|---:|---|---:|---:|---|
| `ack on\r\n` | 8 | `ack on\r\n` | 8 | 0 | `32/40/0/0/0/40` |
| `ping\r\n` | 6 | `ack 0\r\npong\r\n` | 13 | 7 | `40/53/0/0/0/53` |
| `ping\r\n` | 6 | `ack 1\r\npong\r\n` | 13 | 7 | `53/66/0/0/0/66` |
| `ack off\r\n` | 9 | `ack 2\r\nack off\r\n` | 16 | 7 | `66/82/0/0/0/82` |
| `ping\r\n` | 6 | `pong\r\n` | 6 | 0 | `82/88/0/0/0/88` |

Every target read is normal UTF-8 `match_found`, `matched: true`, with no
frames, no drops, no error, and no truncation; `bytes_read`, `bytes_observed`,
and `bytes_returned` each equal exact payload length. `ack off` remains
pre-acknowledged while ACK mode is enabled: exact payload preserves `ack 2\r\n`
before `ack off\r\n`. Direct Compared classification has no baseline-proof
binding. Existing `ack_peer_orders_ack_before_response_and_stops_after_disable`
remains required as an additional stronger public Rust-PTY proof. Result
properties include no frames, no drops, no error, and no truncation.

Batch 7 writes `output-flush-batch.json` with schema ID
`serial-mcp.native-sim-differential.output-flush-batch.v1` and one direct
Compared pair. It uses standard anonymous open (`profile_mode: "none"`,
`name: null`, baud 115200) and this exact public observation order after the
32-byte boot banner:

| Step | Exact result | Position (`from/next/lost/remaining/start/end`) |
|---|---|---|
| first `write("ping\r\n")` | normal anonymous UTF-8 write, `bytes_written=decoded_bytes=6` | — |
| first positioned `read(match="pong")` | normal `pong\r\n`, 6/6/6 counters, `match_index=0`, `match_found` | `32/38/0/0/0/38` |
| `flush(target="output")` | normal anonymous `{connection_id, name:null, target:"output"}` | — |
| second `write("ping\r\n")` | normal anonymous UTF-8 write, `bytes_written=decoded_bytes=6` | — |
| second positioned `read(match="pong")` | same exact normal `pong\r\n`, 6/6/6 counters, `match_index=0`, `match_found` | `38/44/0/0/0/44` |

Both reads retain `is_error:false`, `matched:true`, no frames, no drops, no
error, no truncation, and all six position fields. First matched `pong` proves
first command is fully delivered/consumed before output-only flush. No sleep,
readiness shortcut, or weaker timing substitution is used. Output flush
retains RX cursor: second read starts at 38. Existing
`output_flush_after_full_delivery_preserves_later_traffic` remains the stronger
Rust-PTY proof. `elapsed_ms` is the only nondeterministic field removed from raw
characterization. Typed differential reports retain modeled outcome fields;
caller-supplied request echoes (`timeout_ms`, `no_new_rx_timeout_ms`) are not
modeled. This intentional omission is separate from the request-echo model
boundary. Batch 7 has no baseline
proof binding. At the Batch 7 checkpoint, the registry had 26 covered rows and
status 13/13/3/20; Phase F remained blocked.

Batch 8 writes `slip-happy-batch.json` with schema ID
`serial-mcp.native-sim-differential.slip-happy-batch.v1` and one direct
Compared pair. It promotes `native_read_slip_decodes_frame` from Pending to
Compared without a baseline-proof binding. Both native and compatibility
fixture endpoints use the same all-public setup: `transact` sends
`arm_cmd 1000\r\n` and matches exact `arm_cmd delay=1000\r\n`, then public
`write` sends `sendraw hex C0706F6E67C0\r\n`, and public `read` starts at
`from: {"type":"now"}` with `encoding: "hex"` and raw
`rx_framing: {"type":"slip","max_frames":1}`. The compatibility peer
arms a one-second delay for the next exact command, acknowledges arming
immediately, then delays the exact raw `c0 70 6f 6e 67 c0` output before
emitting it; no fixture-core or product API is added.

The normalized target read is an anonymous normal result with effective hex
wire payload `c0 70 6f 6e 67 c0`, counters `bytes_read=6`,
`bytes_observed=0`, `bytes_returned=6`, stop reason `max_frames`, no match,
truncation, drop, or error, and one raw `slip` frame at index zero with hex
payload `70 6f 6e 67` and no parser. Positions are
`52/58/0/0/0/58` in `from_offset/next_offset/bytes_lost/buffered_remaining/
start_offset/end_offset` order. `elapsed_ms` is the only nondeterministic field
removed from raw characterization. Typed differential reports retain modeled
outcome fields; caller-supplied request echoes (`timeout_ms`,
`no_new_rx_timeout_ms`) are not modeled.
The existing `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
proof remains the stronger Rust-PTY SLIP semantic proof; Batch 8 does not
claim preset, malformed, recovery, or full protocol parity from this one raw
`rx_framing: slip` row. The Batch 8 checkpoint recorded 14 compared, 13 baseline-and-stronger,
3 retired, and 19 pending with 27 covered rows; Phase F remained blocked.

Batch 9 writes `slip-malformed-batch.json` with schema ID
`serial-mcp.native-sim-differential.slip-malformed-batch.v1` and one
baseline-and-stronger pair. Both native and compatibility fixture endpoints use
the same all-public scaffold: `transact("arm_cmd 1000\r\n", match exact
"arm_cmd delay=1000\r\n")`, then `write("sendraw hex C0DB41C0\r\n")`, then
`read(from={"type":"now"}, encoding="utf8",
rx_framing={"type":"slip"})`. The setup arm acknowledgement is exact and the
anonymous UTF-8 setup write is normal with `bytes_written=decoded_bytes=22`.
No setup call is retained in normalized observations; no sleep, flush,
private fixture script, raw fixture API, native-only branch, or `max_frames` is
used.

The normalized target is an anonymous normal fallback result with effective hex
payload `c0 db 41 c0`, `bytes_read/bytes_observed/bytes_returned` equal to
`4/0/0`, `stop_reason="framing_error"`, no match, truncation, frames, or drops,
and exact error `SLIP framing error: invalid escape byte 0x41`. Positions are
`52/56/0/0/0/56` in
`from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
order. `elapsed_ms` is removed only from raw characterization; unmodeled request echoes
remain outside typed outcomes. The stronger
`slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
proof uses `protocol: slip` and retains valid-frame-before-error plus recovery;
Batch 9 preserves the original raw `rx_framing: slip` one-frame-free behavior
and does not claim full SLIP protocol parity. Batch 9 counts are an explicit
historical checkpoint: 14 compared, 14 baseline-and-stronger, 3 retired, and
18 pending (14/14/3/18) with 28 covered rows. They are not current.

Batch 10 writes `slip-recovery-batch.json` with schema ID
`serial-mcp.native-sim-differential.slip-recovery-batch.v1` and one direct
Compared pair. Both endpoints use the shared public double-arm/sendraw scaffold:
public `transact("arm_cmd 1000\r\n")` with exact
`arm_cmd delay=1000\r\n`, public `write("sendraw hex C0DB41C0\r\n")`, and raw
public `read(from={"type":"now"}, encoding="utf8", rx_framing={"type":"slip"})`
for the malformed target; then the same public arm/write scaffold with
`sendraw hex C0706F6E67C0` for recovery. Both setup calls are validated but
excluded from normalized observations. Observation order is existing open,
existing boot read, exact malformed read, and exact recovery read. No setup
output, sleep, flush, private fixture script, raw fixture API, native-only path,
`max_frames`, or `no_new_rx_timeout_ms` enters the normalized outcome.

The first target remains exact Batch 9 behavior: effective hex payload
`c0 db 41 c0`, `bytes_read/bytes_observed/bytes_returned` equal `4/0/0`,
`stop_reason="framing_error"`, no match, truncation, frames, or drops, exact
error `SLIP framing error: invalid escape byte 0x41`, and positions
`52/56/0/0/0/56`. The recovery target is exact effective hex payload
`c0 70 6f 6e 67 c0`, `bytes_read/bytes_observed/bytes_returned` equal `6/0/0`,
`stop_reason="timeout"`, no match, truncation, drops, or error, one raw SLIP frame
at index zero with hex payload `70 6f 6e 67`, and positions
`76/82/0/0/0/82`. Cursor advancement uses raw consumption even when
`bytes_returned=0`. The stronger `protocol: slip`
`slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
proof retains valid-frame-before-error plus recovery; Batch 10 compares raw
`rx_framing: slip` malformed-plus-recovery behavior and does not claim full
SLIP protocol parity. Batch 10's historical checkpoint was 15 compared, 14
baseline-and-stronger, 3 retired, and 17 pending with 29 covered rows; it is
historical only.

Batch 11 writes `cobs-preset-batch.json` with schema ID
`serial-mcp.native-sim-differential.cobs-preset-batch.v1` and one direct
Compared pair. Both endpoints use standard anonymous open, the shared public
`arm_cmd 1000` then `sendraw hex 0005706F6E6700` setup, and exclude setup
results from normalized observations. The one-shot arm delay applies before
the exact output; setup write is normal anonymous UTF-8 with
`bytes_written=decoded_bytes=28`.

The COBS target calls `read` with `protocol: {"type":"cobs"}`, not raw
`rx_framing: cobs`, and uses static independent plain-COBS wire
`00 05 70 6f 6e 67 00`. The normalized target is an anonymous normal hex
result with `bytes_read/bytes_observed/bytes_returned=7/0/0`, stop reason
`timeout`, no match/truncation/drop/error, one `cobs` frame at index 0 with
hex payload `70 6f 6e 67` and parsed raw frame `{"parser":"raw"}`. Positions
are `52/59/0/0/0/59`. The broader zero-containing COBS TX/RX fixture proof
remains stronger coverage; Batch 11 does not claim full COBS protocol parity.
Batch 11 checkpoint was historical: 16 compared, 14 baseline-and-stronger, 3
retired, and 16 pending (`16/14/3/16`) with 30 covered rows. Batch 12 is a
historical checkpoint at 17 compared, 14 baseline-and-stronger, 3 retired, and
15 pending (`17/14/3/15`) with 31 covered rows. Batch 13 is historical at
18 compared, 14 baseline-and-stronger, 3 retired, and 14 pending
(`18/14/3/14`) with 32 covered rows. Batch 14 was historical at
`19/14/3/13` with 33 covered rows. Batch 15 is historical at `21/14/3/11`
with 35 covered rows; Batch 16 is historical at `22/14/3/10` with 36 covered
rows; Batch 17 is historical at `23/14/3/9` with 37 covered rows; Batch 18 is
historical at `24/14/3/8` with 38 covered rows; Batch 19 is historical at
`24/15/3/7` with 39 covered rows; Batch 20 is historical at `24/16/3/6` with 40
covered rows; Batch 21 is historical at `25/16/3/5` with 41 covered rows; Batch
22 is historical at `26/16/3/4` with 42 covered rows; post-Batch-22 pre-txbuf-retirement checkpoint is `26/16/4/3` with 43 covered rows (26 compared, 16 baseline-and-stronger, 4 retired, and 3 pending); Phase F blocked.

## Disposition Rules

- **Unchanged:** preserve case and assertions through replacement fixture.
- **Parameterized:** preserve named scenario and assertions in a shared case
  table.
- **Strengthened:** current test survives only with stronger public-boundary
  assertions described below.
- **Split:** preserve current claim and add omitted or disentangled claim as a
  separate case.
- **Retired:** remove only after cited stronger public-boundary test remains in
  required CI. No coverage disappears silently.

Summary: 19 unchanged, 6 parameterized, 15 strengthened, 2 split, 7 retired.

## 43 Validation Tests

| Current test | Firmware command/state | Public MCP behavior proved | Disposition and migration requirement |
|---|---|---|---|
| `native_ping_roundtrip` | `ping` → `pong` | real-path open/write/read, literal match, `match_found` | **Unchanged.** Use default Rust device peer. |
| `native_pending_read_then_write_ping_roundtrip` | delayed later `ping` | pending read receives later output | **Strengthened.** Replace sleep with fixture readiness barrier; prove read was pending. |
| `native_split_writes_preserve_command_order` | `pi` + `ng` + CRLF | write ordering and peer command assembly | **Unchanged.** |
| `native_framing_reports_single_split_command` | firmware `framing on` line diagnostic | duplicate CRLF framing diagnostic, exact 54-byte target read, and one `pong` | **Baseline-and-stronger Batch 5.** Exact public result is compared; `device_command_parity::split_writes_preserve_one_command_and_exact_wire_order` remains stronger proof. |
| `native_trace_reports_exact_split_byte_sequence` | `trace on` | exact six-record lower-case trace for `ping\r\n` plus one `pong` | **Baseline-and-stronger Batch 5.** Exact public result is compared; the same split-write proof remains stronger. |
| `native_read_match_on_spam_complete` | deterministic spam stream | matcher survives flood and stops at completion marker | **Unchanged; mapped.** `device_command_parity::finite_flood_matcher_reaches_unique_completion_marker`. |
| `native_read_buffer_budget_stops_under_flood` | native original 65,536-byte spam; Batch 4 uses live 512-byte spam with default 256 | bounded read under flood | **Strengthened; mapped.** `device_command_parity::live_buffer_budget_caps_finite_flood_with_exact_stop_metadata` configures 256 live, asserts exact `max_buffered_bytes`, counters, coherent truncation, and no unsupported per-read field; variable prefilled 65,536 backlog fields stay characterization-only. |
| `native_bootloader_touch_exits_42` | `touch` → process exit 42 | public write triggers peer process exit | **Compared Batch 22.** Standard anonymous `open` at 115200 with `profile_mode: "none"` and matching boot-banner read remain the first two normalized observations; fixture side is a real dedicated small Rust child PTY, not `FixtureExit::Crashed`; child emits exact `serial-mcp test firmware ready\r\n` before `PTY_PATH`; public UTF-8 `write("touch\r\n")` is normal anonymous with exact `bytes_written=decoded_bytes=7`; both native firmware and child endpoint exit exactly 42, retained as typed `process_exit` observation; terminal `touch exit(42)\r\n` response delivery is not claimed; no public `close` follows peer exit; stronger fixture proof `touch_write_causes_small_rust_child_peer_to_exit_42` remains independent. |
| `native_list_ports_after_open` | none; ambient OS enumeration | only asserts some host port exists | **Retired.** Stronger deterministic public proofs: `http_integration::call_tool_list_ports_returns_structured_result`, `http_integration::ports_resource_includes_profile_match_map`, and `serial_pty::list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`. |
| `native_list_ports_includes_identity_fields` | none; ambient OS enumeration | weak type/null checks | **Retired.** Deterministic injected-provider identity/profile/schema proofs supersede ambient OS field/null checks: `serial_pty::list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`, `serial_pty::list_ports_preview_selected_winner_matches_later_bare_open`, and `serial_pty::list_ports_preview_output_validates_against_generated_schema`. |
| `native_flush_after_write` | `ping`, immediate output flush | racy delivery survives output flush | **Retired.** Contract permits queued TX discard. Keep stronger fully-delivered case below. |
| `native_get_status_after_write_increments_tx_counter` | `ping` | status counters/activity exposed | **Strengthened.** Assert monotonic exact deltas from before/after snapshots. |
| `native_reconfigure_baud_rate_persists` | PTY ignores physical baud; ping remains functional | MCP config/state mutation | **Strengthened.** Assert every result and status transition; state explicitly that PTY proves configuration, not wire baud. |
| `native_ack_command_provides_pre_execution_ack` | `ack on/off`, sequence | ordered stateful peer responses | **Compared.** Exact five-step public payload/position comparison, including `ack 2\r\n` before `ack off\r\n`; `device_command_parity::ack_peer_orders_ack_before_response_and_stops_after_disable` remains stronger semantic proof. |
| `native_txbuf_status_reports_pending` | TX hold/ring status | idle queue and hold recovery | **Retired.** Source checks only idle `txbuf` diagnostics and hold recovery; `device_command_parity::held_output_reports_nonzero_queue_then_drains_and_recovers` proves nonzero held output, blocked read, drain, and recovery. |
| `native_flush_input_clears_host_rx` | spam then input flush | stale RX discard | **Historical Compared Batch 21.** Standard anonymous `open` at 115200 with `profile_mode: "none"` and boot-banner read remain the first two normalized observations; public UTF-8 old-marker write `sendraw hex 4F4C442D4D41524B45520D0A\r\n` is normal anonymous with exact `bytes_written=decoded_bytes=38`; status-only `get_status` polling proves old marker reached retained RX at `rx_bytes >= 44` and is not normalized; public `flush(target="input")` is normal anonymous with exact target `input`; public UTF-8 new-marker write `sendraw hex 4E45572D4D41524B45520D0A\r\n` is normal anonymous with exact `bytes_written=decoded_bytes=38`; status-only polling proves `rx_bytes >= 56` and is not normalized; positioned `from={"type":"buffer_start"}` UTF-8 literal `NEW-MARKER\r\n` read returns exact `NEW-MARKER\r\n`, `12/12/12`, `match_found`, index 0, no truncation/frames/drops/error, positions `44/56/0/0/44/56`; stronger fixture proof `flush_input_discards_known_old_marker_and_keeps_new_marker` remains independent. |
| `native_flush_during_arm_cmd_delay` | one-shot delayed next command | flush race does not cancel peer command already received | **Historical Baseline-and-stronger Batch 20.** Standard anonymous `open` at 115200 with `profile_mode: "none"`; boot banner and arm acknowledgement are validated before normalized observations; public `transact("arm_cmd 1000\r\n")` validates exact `arm_cmd delay=1000\r\n` acknowledgement but setup result is not normalized; public `write("ping\r\n")` is normal UTF-8 `6/6`; bounded 100 ms baseline admission window is not a peer-acceptance proof; public `flush(target="both")` is normal anonymous with exact target `both`; positioned `from={"type":"now"}` UTF-8 literal `pong\r\n` read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no truncation/frames/drops/error, positions `52/58/0/0/52/58`; stronger fixture proof `flush_after_command_acceptance_does_not_cancel_delayed_response` remains independent. |
| `native_flush_output_after_full_delivery_is_safe` | ping fully delivered before output flush | safe output flush after delivery, later traffic works | **Compared Batch 7.** First matched `pong` is delivery boundary; existing output-only Rust-PTY proof remains stronger. |
| `native_partial_line_buffered_then_completed` | partial `pi`, later `ng\r\n` | partial line remains pending, then exact `pong\r\n` completes | **Baseline-and-stronger Batch 5.** The differential case proves two bounded unfinished observations; the split-write public proof remains stronger. |
| `native_read_regex_matches_pong` | ping | regex matcher | **Parameterized.** Preserve regex case with glob case. |
| `native_read_glob_matches_pong_line` | ping | per-line glob matcher | **Parameterized.** Preserve glob case with regex case. |
| `native_auto_reconnect_preserves_connection` | no peer loss; `reconnect` called while open | only no-op reconnect success | **Retired.** Source only invokes `reconnect` while already open; `device_fixture::public_mcp_ping_hold_disconnect_replace_and_reconnect` preserves same ID/open state and post-no-op traffic, then proves peer loss, replacement, reconnect, and fresh exchange. |
| `native_read_line_framing_splits_lines` | `ping`, `info` | line decoder emits multiple frames | **Required original native test remains.** It retains ordered pong/info assertions; Batch 2 uses the deterministic nested-command equivalent described above, while `line_framing_returns_exact_ordered_peer_frames` retains its fixture replacement proof. |
| `native_read_json_parser_decodes_jsonout` | three changing sensor objects | JSON-lines parser output | **Compared Batch 14.** Static three JSON object response; explicit line framing plus `json_lines` parser; exact `140/0/0` timeout result with three ordered parsed objects and positions `52/192/0/0/0/192`; existing JSON Lines fixture proof remains stronger. |
| `native_read_at_parser_parses_pong` | ping treated as AT data line | explicit AT parser classification | **Compared Batch 12.** Direct explicit `rx_framing: line` plus `rx_parser: at_command` evidence; existing `AtPeer` worksheet remains stronger stateful AT behavior. |
| `native_read_framing_max_frames_stops` | three command responses | exact two-frame stop | **Unchanged.** |
| `native_read_framing_plus_match_combined` | ping | framing plus matcher interaction | **Strengthened.** Require matching frame and frame index; do not accept null frames. |
| `native_open_protocol_default_drives_write_and_read` | AT-like ping response | protocol-only open default controls bare write/read; stripped framed arm match, bare `ping` CR addition, and AT-parsed `pong\r\n` | **Compared Batch 13.** Existing `AtPeer` proof remains stronger. |
| `native_explicit_rx_framing_beats_connection_default` | open carries protocol and explicit RX framing | open-field precedence only | **Split.** Rename open-time explicit-field proof; add call-time explicit RX framing over connection default. |
| `native_read_slip_decodes_frame` | raw RFC 1055 frame | SLIP happy-path decode | **Compared Batch 8.** Direct raw `rx_framing: slip` public result; source vector is independent, while richer `protocol: slip` fixture coverage remains stronger semantic proof. |
| `native_read_slip_malformed_escape_returns_partial_result` | malformed escape | structured `framing_error` and hex fallback | **Baseline-and-stronger Batch 9.** Preserve original raw `rx_framing: slip` one-frame-free result; stronger `protocol: slip` proof owns valid-frame-before-error plus recovery. |
| `native_read_delimiter_framing_decodes` | raw `|pong|` | delimiter decode | **Unchanged.** Add multi-byte split cases in generic matrix. |
| `native_read_length_prefixed_framing_decodes` | raw `04pong` bytes | only raw transport, not RX decoder | **Split.** Preserve raw byte proof only if useful; add real 1-byte RX length decoder assertion. Full matrix adds 1/2/4-byte and endian cases. |
| `native_read_start_end_framing_decodes` | `<<pong>>` | start/end decode | **Unchanged.** |
| `native_write_tx_framing_modes_observed_via_trace` | peer byte trace | delimiter, length, marker, SLIP TX encodings | **Strengthened.** Assert each write result and exact peer bytes with bounded completion; use independent expected vectors. |
| `native_read_explicit_line_endings_split_correctly` | LF, CR, CRLF raw streams | explicit line ending semantics | **Parameterized.** Keep every line-ending case. |
| `native_read_slip_recovers_after_error_on_next_call` | malformed raw SLIP call then valid raw SLIP call | decoder reset/recovery across public reads | **Compared Batch 10.** Direct raw `rx_framing: slip` comparison uses the shared public double-arm/sendraw scaffold; richer `protocol: slip` recovery remains stronger semantic coverage. |
| `native_read_cobs_preset_decodes_frame` | static independent plain-COBS `00 05 70 6f 6e 67 00` | `protocol: {"type":"cobs"}` preset decode with raw parser frame | **Compared Batch 11.** Exact `7/0/0` counters, `timeout`, raw parser, and `52/59/0/0/0/59`; broader zero-containing COBS TX/RX fixture proof remains stronger. |
| `native_read_ndjson_preset_decodes_json_frames` | two records plus blank | NDJSON decode/blank skip | **Compared Batch 15.** Static NDJSON payload `{"a":1}\n\n{"b":2}\n`; `protocol: {"type":"ndjson"}` uses auto line framing, `skip_empty:true`, and JSON parser; exact `17/0/0` timeout result with ordered parsed `a`/`b` frames and positions `52/69/0/0/0/69`; stronger NDJSON fixture proof remains independent. |
| `native_read_ndjson_preset_skips_empty_lines` | records plus blank/whitespace lines | whitespace skipping | **Compared Batch 15.** Static NDJSON payload `{"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n`; `protocol: {"type":"ndjson"}` uses auto line framing, `skip_empty:true`, and JSON parser; exact `30/0/0` timeout result with ordered parsed `a`/`b`/`c` frames and positions `52/82/0/0/0/82`; blank and whitespace-only lines emit no frames; stronger NDJSON fixture proof remains independent. |
| `native_read_nmea0183_preset_decodes_parsed_frame` | locally checksummed GGA | NMEA parser/checksum result | **Compared Batch 16.** Static 67-byte valid GGA sentence `$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n`; `protocol: {"type":"nmea0183"}` uses start/end framing with markers excluded and NMEA parser; exact `67/0/0` timeout result with one UTF-8 `start_end` frame, parsed `talker_id="GP"`, `sentence_type="GGA"`, exact ordered fields `["123519","4807.038","N","01131.000","E","1","08","0.9","545.4","M","46.9","M","",""]`, `checksum_valid:true`, and positions `52/119/0/0/0/119`; stronger NMEA fixture proof remains independent. |
| `native_read_modbus_ascii_preset_decodes_parsed_frame` | official-style `:010300000001FB` response | address/function/data/LRC parse | **Compared Batch 17.** Static valid response `:010300000001FB\r\n`; protocol-only `modbus_ascii` preset; exact `17/0/0` counters, `timeout`, parsed address `1`, function `3`, data `[0,0,0,1]`, `checksum_valid:true`, and positions `52/69/0/0/0/69`; stronger mutable-peer fixture proof remains independent. |
| `native_capture_boot_arm_only_captures_post_arm_command_output` | external ping after private mark | arm-only capture excludes stale banner and preserves shared cursor/history | **Retired.** Source only checks arm-only post-mark output after a consumed stale banner; `device_command_parity::capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor` excludes stale bytes, preserves private-mark replay and shared cursor/history, and captures post-arm output. |

## 6 Lifecycle Tests

| Current test | Firmware command/state | Public MCP behavior proved | Disposition and migration requirement |
|---|---|---|---|
| `native_named_connection_appears_in_list_connections` | none | named real-path connection summary | **Unchanged.** |
| `native_set_flow_control_updates_summary_and_result` | none; current value already `none` | result/summary agree | **Strengthened; mapped.** `device_command_parity::flow_control_none_at_open_and_live_set_are_reflected_in_summary` parameterizes open-time plus live `none`; PTY cannot prove physical hardware/software flow control, so controlled backend remains physical-effect proof. |
| `native_close_while_read_active_returns_normal_result` | readiness-proven marker | structured close and read result after close | **Historical Compared Batch 18.** Readiness-proven public close sequence uses `arm_cmd 1000\r\n`, baseline `get_status.rx_bytes`, secondary unmatched `from={"type":"now"}` read, and primary marker command `sendraw hex 524541442D52454144592D4D41524B45520D0A\r\n` writing `READ-READY-MARKER\r\n` after `rx_bytes` increases by 19 while pending read remains unfinished; primary close returns normal anonymous profile with `source="disabled"`, `profile_persistence.operation="close_snapshot"`, and `profile_persistence.state="transient"`; pending read returns exact marker with `19/19/19`, `connection_closed`, no match/truncation/frames/drops/error, and positions `52/71/0/0/0/71`; stronger fixture proof remains independent. |
| `native_reopen_same_port_after_close_works` | ping then close/reopen | duplicate of stronger following case | **Retired.** Keep strengthened reopen/fresh-output test. |
| `native_reopen_then_match_finds_fresh_output` | ping before and after reopen | same PTY path opens twice | **Historical Baseline-and-stronger Batch 19.** Standard anonymous first `open` at 115200 with `profile_mode: "none"`; boot banner is synchronized once because firmware emits it only at process start; first `write("ping\r\n")` is normal UTF-8 `6/6`, and positioned first literal `pong` read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/truncation, positions `32/38/0/0/0/38`; normal first close retains disabled/transient close-profile checks; same endpoint second `open` requires a distinct raw connection ID, public status verifies `rx_bytes=0` before second command, and pending second `from={"type":"now"}` UTF-8 `pong` read uses bounded 100 ms baseline admission before independent-client `write("ping\r\n")`; second read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/truncation, positions `0/6/0/0/0/6`; the status probe is not normalized; stronger fixture proof remains independent. |
| `native_open_with_flow_control_persists_in_summary` | none; opens with default `none` | open setting appears in summary | **Parameterized; mapped.** Combined with `device_command_parity::flow_control_none_at_open_and_live_set_are_reflected_in_summary`. |

## Firmware Behavior Required by Traceability

Replacement core needs only observable state used above:

- split command assembly and committed-line observation;
- ping/info and unique generation responses;
- deterministic finite/cadenced flood with completion marker;
- bounded output queue with explicit capacity, hold, drain, and drop/block
  policy;
- one-shot response delay and peer readiness barriers;
- exact TX byte observation;
- stateful ACK sequence;
- JSON sensor output;
- arbitrary raw byte emission;
- explicit peer process exit 42 for exit-path test.

Do not reproduce unobserved Zephyr IRQ scheduling, `k_timer`, native board, or
firmware implementation defects. Preserve a defect only if a public product
contract depends on it and a characterization test says so.

## Historical Path-Level NCS Coupling Inventory and Phase F Disposition

| Path | Historical coupling | Phase F disposition |
|---|---|---|
| `.github/workflows/ci.yml` | dedicated NCS cache/install/build/native-test job; release depended on it | removed native job and release dependency; replacement suites remain in normal Rust job |
| `scripts/install-nrfutil-ci.sh` | pinned CI provisioning | deleted |
| `scripts/tests/test_install-nrfutil-ci.sh` | installer-only offline test | deleted with installer |
| `flake.nix` | `nix-nrf-dev`, `mkNrfShell`, NCS v3.3.0, multilib, firmware PATH | replaced with ordinary Rust dev shell |
| `flake.lock` | `nix-nrf-dev` and orphanable transitive nodes | pruned offline; surviving revisions unchanged |
| `firmware/` | complete Zephyr native_sim test device and helpers | deleted active source/config |
| `tests/common/firmware.rs` | build-on-demand `NativeSimFirmware` child harness | deleted |
| `tests/common/mod.rs` | exported firmware helper; independent `PtyPair` also here | removed firmware export; retained PTY fixture |
| `tests/native_sim_validation.rs` and directory | 43 ignored tests | replacement suites retained; wrappers deleted |
| `tests/native_sim_connection_lifecycle.rs` | 6 ignored tests | replacement lifecycle proofs retained; suite deleted |
| `xtask/src/main.rs` | built firmware, ran ignored suites, printed Zephyr path | removed firmware asset logic; runs normal Rust fixture tests |
| `xtask/src/agent_eval/scenarios.rs` | completion references pointed at native tests | repointed to replacement public-boundary tests |
| `tests/doc_drift.rs` | research-plan guard and release-job guard | retained 49-row proof; removed registry/source assertions and added zero-active-NCS guard |
| `opencode.json` and `firmware/.clangd` | firmware C LSP through NCS shell | removed firmware LSP route; Nordic docs MCP remains optional |
| root `README.md`, `AGENTS.md`, development docs | native commands and NCS invariants | active docs updated; historical evidence retained |
| `docs/development/agent-interface-baseline.json` | historical completion references | preserved unchanged as historical metrics |
| `CHANGELOG.md` | historical native/NCS releases | retained unchanged as history |

Cargo has no NCS dependency. Keep `libudev-dev`, `pkg-config`, serialport,
tokio-serial, and Unix PTY support. No unrelated active build/test path requires
NCS.

## Historical Migration Gate

No table row disappeared. The migration mapped every former name to replacement
test ID(s), compared normalized public outcomes during the parity window, made
replacement suites required, and then removed active native fixture source.
Fresh clean-checkout CI acceptance remains the final ADR acceptance gate.
