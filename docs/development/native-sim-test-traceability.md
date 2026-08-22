# `native_sim` Test Traceability and NCS Coupling

**Status:** Phase E required Rust PTY gate complete on 2026-08-13. All 49
native cases now have one explicit mapping row. Required Linux x86_64 CI runs
the four normal replacement targets and the ignored 100-iteration deterministic
repeat gate; macOS arm64 runs fixture/command/framing real-PTY coverage.
`native_sim` remains required temporary differential oracle. No firmware, NCS,
native job, release dependency, or Nix coupling is removed in this phase.

Batch 15 extends executable native-versus-fixture comparison through shared
public MCP scenarios. Global registry status is **21 compared, 14
baseline-and-stronger, 3 retired, and 11 pending** rows. Batch 1 remains an
isolated eight-case command/lifecycle report; Batch 2 remains six generic
matching/framing cases; Batch 3 remains five raw generic-framing cases; Batch 4
adds two flood/buffer cases; Batch 5 adds three command-diagnostic cases; Batch
6 adds one ACK state-machine case; Batch 7 adds one output-flush case; Batch 8
adds one raw SLIP happy-path case; Batch 9 adds one malformed raw SLIP
baseline-and-stronger case; Batch 10 adds one direct raw SLIP recovery case;
Batch 11 adds one direct COBS-preset case as a historical checkpoint; Batch 12
adds one direct AT-parser case as a historical checkpoint; Batch 13 adds one
protocol-default case as a historical checkpoint; Batch 14 adds one JSON-parser
case as a historical checkpoint; Batch 15 is current and adds two NDJSON-preset
cases. Each batch has a separate report. This covers 35
executable rows, is not full differential parity, and does not
permit Phase F work. Current counts are 21 compared, 14 baseline-and-stronger,
3 retired, and 11 pending (`21/14/3/11`).

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
14 remains historical partial evidence; Batch 15 is current partial evidence;
Phase F blocked.

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
covered rows. Batch 15 is current direct Compared evidence
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
`10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3`. Current
registry is `21/14/3/11` with 35 covered rows; Phase F blocked.

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
| `native_bootloader_touch_exits_42` | `touch_write_causes_small_rust_child_peer_to_exit_42` | real child exits 42 |
| `native_list_ports_after_open` | `call_tool_list_ports_returns_structured_result`; `ports_resource_includes_profile_match_map`; `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports` | **Retired.** Deterministic public tool/resource preview proof is stronger than ambient enumeration. |
| `native_list_ports_includes_identity_fields` | `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`; `list_ports_preview_selected_winner_matches_later_bare_open`; `list_ports_preview_output_validates_against_generated_schema` | strengthened real-PTY + injected-provider identity/schema proof |
| `native_flush_after_write` | `output_flush_after_full_delivery_preserves_later_traffic` | **Retired.** Contract permits queued-output discard; stronger fully delivered output proof retains valid behavior. |
| `native_get_status_after_write_increments_tx_counter` | `status_reports_exact_io_deltas_and_activity` | exact TX/RX/write deltas |
| `native_reconfigure_baud_rate_persists` | `reconfigure_updates_status_and_connection_remains_functional` | transitions, status, post-change traffic |
| `native_ack_command_provides_pre_execution_ack` | `ack_peer_orders_ack_before_response_and_stops_after_disable` | **Compared.** Direct ACK state-machine comparison; existing Rust-PTY proof remains stronger semantic coverage. |
| `native_txbuf_status_reports_pending` | `held_output_reports_nonzero_queue_then_drains_and_recovers` | nonzero held queue and recovery |
| `native_flush_input_clears_host_rx` | `flush_input_discards_known_old_marker_and_keeps_new_marker` | unique old/new markers |
| `native_flush_during_arm_cmd_delay` | `flush_after_command_acceptance_does_not_cancel_delayed_response` | peer acceptance barrier |
| `native_flush_output_after_full_delivery_is_safe` | `output_flush_after_full_delivery_preserves_later_traffic` | **Compared.** First matched `pong` is delivery boundary; output-only flush retains cursor and later traffic. Existing Rust-PTY proof remains stronger semantic coverage. |
| `native_partial_line_buffered_then_completed` | `split_writes_preserve_one_command_and_exact_wire_order` | `pi` stays pending for bounded delay; `ng\r\n` completes command |
| `native_read_regex_matches_pong` | `regex_and_glob_matchers_find_complete_peer_line` | regex mode |
| `native_read_glob_matches_pong_line` | `regex_and_glob_matchers_find_complete_peer_line` | glob mode |
| `native_auto_reconnect_preserves_connection` | `public_mcp_ping_hold_disconnect_replace_and_reconnect` | disappearance, `connection_closed`, replacement, fresh exchange |
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
| `native_read_nmea0183_preset_decodes_parsed_frame` | `nmea0183_preset_parses_valid_independently_checksummed_sentence` | checksum-valid parsed sentence |
| `native_read_modbus_ascii_preset_decodes_parsed_frame` | `modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation` | LRC and mutable peer |
| `native_capture_boot_arm_only_captures_post_arm_command_output` | `capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor` | arm-only stale exclusion/replay |
| `native_named_connection_appears_in_list_connections` | `named_connection_summary_uses_fixture_stable_path` | named public summary |
| `native_set_flow_control_updates_summary_and_result` | `flow_control_none_at_open_and_live_set_are_reflected_in_summary` | live result/summary |
| `native_close_while_read_active_returns_normal_result` | `close_interrupts_readiness_proven_live_read_with_connection_closed` | known-pending close result |
| `native_reopen_same_port_after_close_works` | `reopen_same_path_returns_distinct_id_and_only_fresh_generation` | **Retired.** Stronger distinct-ID and fresh-generation proof retains reopen behavior. |
| `native_reopen_then_match_finds_fresh_output` | `reopen_same_path_returns_distinct_id_and_only_fresh_generation` | distinct IDs and fresh output only |
| `native_open_with_flow_control_persists_in_summary` | `flow_control_none_at_open_and_live_set_are_reflected_in_summary` | open-time result/summary |

Phase E gate: all 49 native rows now have one required replacement, cited
stronger proof, or explicit retired disposition. Current focused target totals are
`device_fixture` 7/7, `device_command_parity` 19/19,
`device_framing_parity` 8/8, and `device_protocol_parity` 15/15 (7 public
cases + 7 durable peer-oracle units + 1 exact seven-preset coverage registry).
Linux x86_64 additionally invokes
`phase_e_public_boundary_repeat_gate` with `--ignored --test-threads=1`: 100
fixed-order iterations, seed `0x50484153455f4545`. Each iteration uses a real
`DeviceFixture`, `TestServer`, MCP client, and public tools for ping, bounded
flood, held/released output, post-delivery flush, peer disconnect,
stable-path replacement/reconnect, and explicit teardown. Native oracle remains
43/43 validation and 6/6 lifecycle; it remains required until full
differential/migration gate passes.

## Executable Differential Batches

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
`19/14/3/13` with 33 covered rows. Batch 15 is current at `21/14/3/11` with
35 covered rows; Phase F blocked.

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

Summary: 20 unchanged, 6 parameterized, 17 strengthened, 3 split, 3 retired.

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
| `native_bootloader_touch_exits_42` | `touch` → process exit 42 | public write triggers peer process exit | **Strengthened; mapped.** `device_command_parity::touch_write_causes_small_rust_child_peer_to_exit_42` asserts write success and real Rust child exit 42. |
| `native_list_ports_after_open` | none; ambient OS enumeration | only asserts some host port exists | **Retired.** Stronger deterministic public proofs: `http_integration::call_tool_list_ports_returns_structured_result`, `http_integration::ports_resource_includes_profile_match_map`, and `serial_pty::list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`. |
| `native_list_ports_includes_identity_fields` | none; ambient OS enumeration | weak type/null checks | **Strengthened; mapped.** Existing public real-PTY + injected-provider proofs: `serial_pty::list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`, `list_ports_preview_selected_winner_matches_later_bare_open`, and `list_ports_preview_output_validates_against_generated_schema`. |
| `native_flush_after_write` | `ping`, immediate output flush | racy delivery survives output flush | **Retired.** Contract permits queued TX discard. Keep stronger fully-delivered case below. |
| `native_get_status_after_write_increments_tx_counter` | `ping` | status counters/activity exposed | **Strengthened.** Assert monotonic exact deltas from before/after snapshots. |
| `native_reconfigure_baud_rate_persists` | PTY ignores physical baud; ping remains functional | MCP config/state mutation | **Strengthened.** Assert every result and status transition; state explicitly that PTY proves configuration, not wire baud. |
| `native_ack_command_provides_pre_execution_ack` | `ack on/off`, sequence | ordered stateful peer responses | **Compared.** Exact five-step public payload/position comparison, including `ack 2\r\n` before `ack off\r\n`; `device_command_parity::ack_peer_orders_ack_before_response_and_stops_after_disable` remains stronger semantic proof. |
| `native_txbuf_status_reports_pending` | TX hold/ring status | idle queue and hold recovery | **Split.** Separate idle status, nonzero held queue/backpressure, and release/recovery. |
| `native_flush_input_clears_host_rx` | spam then input flush | stale RX discard | **Strengthened.** Use unique pre/post markers; prove old bytes absent and post-flush bytes readable. |
| `native_flush_during_arm_cmd_delay` | one-shot delayed next command | flush race does not cancel peer command already received | **Unchanged.** Use explicit peer barrier around accepted command/delayed response. |
| `native_flush_output_after_full_delivery_is_safe` | ping fully delivered before output flush | safe output flush after delivery, later traffic works | **Compared Batch 7.** First matched `pong` is delivery boundary; existing output-only Rust-PTY proof remains stronger. |
| `native_partial_line_buffered_then_completed` | partial `pi`, later `ng\r\n` | partial line remains pending, then exact `pong\r\n` completes | **Baseline-and-stronger Batch 5.** The differential case proves two bounded unfinished observations; the split-write public proof remains stronger. |
| `native_read_regex_matches_pong` | ping | regex matcher | **Parameterized.** Preserve regex case with glob case. |
| `native_read_glob_matches_pong_line` | ping | per-line glob matcher | **Parameterized.** Preserve glob case with regex case. |
| `native_auto_reconnect_preserves_connection` | no peer loss; `reconnect` called while open | only no-op reconnect success | **Strengthened; mapped.** `device_fixture::public_mcp_ping_hold_disconnect_replace_and_reconnect` proves true disappearance, `connection_closed`, distinct endpoint replacement, reconnect, and fresh exchange. |
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
| `native_read_nmea0183_preset_decodes_parsed_frame` | locally checksummed GGA | NMEA parser/checksum result | **Unchanged.** Add AIS/proprietary/bad/missing vectors in worksheet. |
| `native_read_modbus_ascii_preset_decodes_parsed_frame` | official-style `:010300000001FB` request | address/function/LRC parse | **Unchanged.** Add mutable server/response paths in worksheet. |
| `native_capture_boot_arm_only_captures_post_arm_command_output` | external ping after private mark | arm-only capture excludes stale banner and preserves shared cursor/history | **Unchanged; mapped.** `device_command_parity::capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor` uses stale and post-mark markers, private capture offset replay, retained-history replay, and no reset-line claim. |

## 6 Lifecycle Tests

| Current test | Firmware command/state | Public MCP behavior proved | Disposition and migration requirement |
|---|---|---|---|
| `native_named_connection_appears_in_list_connections` | none | named real-path connection summary | **Unchanged.** |
| `native_set_flow_control_updates_summary_and_result` | none; current value already `none` | result/summary agree | **Strengthened; mapped.** `device_command_parity::flow_control_none_at_open_and_live_set_are_reflected_in_summary` parameterizes open-time plus live `none`; PTY cannot prove physical hardware/software flow control, so controlled backend remains physical-effect proof. |
| `native_close_while_read_active_returns_normal_result` | boot synchronized, then read/close race | structured read result after close | **Strengthened; mapped.** `device_command_parity::close_interrupts_readiness_proven_live_read_with_connection_closed` starts an unmatched `from=now` public read and requires `connection_closed`, not drained/timeout alternatives. |
| `native_reopen_same_port_after_close_works` | ping then close/reopen | duplicate of stronger following case | **Retired.** Keep strengthened reopen/fresh-output test. |
| `native_reopen_then_match_finds_fresh_output` | ping before and after reopen | same PTY path opens twice | **Strengthened.** Use unique generation marker, distinct connection IDs, and offset/from-now assertions to rule out stale pong. |
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

## Path-Level NCS Coupling Inventory

| Path | Current coupling | Final parity-gated action |
|---|---|---|
| `.github/workflows/ci.yml` | dedicated NCS cache/install/build/native-test job; release depends on it | move replacement suite to normal Rust job, remove NCS job and release dependency |
| `scripts/install-nrfutil-ci.sh` | pinned CI provisioning | delete |
| `scripts/tests/test_install-nrfutil-ci.sh` | installer-only offline test | delete with installer |
| `flake.nix` | `nix-nrf-dev`, `mkNrfShell`, NCS v3.3.0, multilib, firmware PATH | replace with ordinary Rust dev shell |
| `flake.lock` | `nix-nrf-dev` and orphanable transitive nodes | regenerate after input removal |
| `firmware/` | complete Zephyr native_sim test device and helpers | delete after differential parity |
| `tests/common/firmware.rs` | build-on-demand `NativeSimFirmware` child harness | delete after replacement fixture owns lifecycle |
| `tests/common/mod.rs` | exports firmware helper; independent `PtyPair` also here | remove firmware export; keep/strengthen PTY fixture |
| `tests/native_sim_validation.rs` and directory | 43 ignored tests | move scenarios to required replacement suites, then delete wrappers |
| `tests/native_sim_connection_lifecycle.rs` | 6 ignored tests | move scenarios, then delete |
| `xtask/src/main.rs` | builds firmware, runs ignored suites, prints Zephyr path | remove firmware asset logic; run normal Rust fixture tests |
| `xtask/src/agent_eval/scenarios.rs` | completion references point at native tests | repoint to replacement public-boundary tests |
| `tests/doc_drift.rs` | research-plan guard and release-job guard | replace with shipped simulator coverage and zero-active-NCS guards |
| `opencode.json` and `firmware/.clangd` | firmware C LSP through NCS shell | delete firmware LSP route; Nordic docs MCP is optional, not build coupling |
| root `README.md`, `AGENTS.md`, development docs | native commands and NCS invariants | rewrite to replacement fixture after implementation |
| `docs/development/agent-interface-baseline.json` | historical completion references | preserve historical metrics; repoint or explicitly archive references |
| `CHANGELOG.md` | historical native/NCS releases | retain unchanged as history |

Cargo has no NCS dependency. Keep `libudev-dev`, `pkg-config`, serialport,
tokio-serial, and Unix PTY support. No unrelated active build/test path requires
NCS.

## Migration Gate

No table row may disappear. Migration PR must mechanically map every current
name to replacement test ID(s), compare normalized public outcomes against
native fixture during parity window, and make replacement suite required before
deleting native fixture.
