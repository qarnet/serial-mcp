# `native_sim` Replacement Recommendation

**Status:** Approved on 2026-08-13. Phases A-D fixture/command/protocol parity
and Phase E required replacement/repeat gate complete. The full normalized
differential parity window is accepted on 2026-08-23. Batch 1 executable
 differential comparison remains isolated at eight command/lifecycle rows; Batch 2
 retains six generic matching/framing rows; Batch 3 retains five raw generic-framing
  rows; Batch 4 adds two flood/buffer rows; Batch 5 adds three command-diagnostic
   rows; Batch 6 adds one ACK state-machine row; Batch 7 adds one output-flush row;
   Batch 8 adds one raw SLIP happy-path row; Batch 9 adds one malformed raw SLIP
   baseline-and-stronger row; Batch 10 adds one direct malformed-then-recovery raw
    SLIP row; Batch 11 adds one direct COBS-preset row with a static independent
      wire; Batch 12 adds one direct AT-parser row as historical evidence; Batch 13
       adds one direct AT protocol-default row as historical evidence; Batch 14 adds
        one direct JSON-parser row as historical evidence; Batch 15 adds two direct
         NDJSON-preset rows as historical evidence; Batch 16 adds one direct
          NMEA-0183 preset row as historical evidence; Batch 17 adds one direct
           Modbus ASCII preset row as historical evidence; Batch 18 adds one direct
            close-while-read row as historical evidence; Batch 19 adds one
            reopen/fresh-output baseline-and-stronger row as historical evidence;
             Batch 20 adds one flush-during-armed-delay baseline-and-stronger row as
              historical evidence; Batch 21 adds one direct input-flush backlog Compared
               row as historical evidence; Batch 22 adds one direct bootloader-touch
               process-exit Compared row as historical evidence. Each batch has separate
              report schema/output.
             Global registry status is **26 compared, 16 baseline-and-stronger, 7 retired, and
             0 pending** rows (`26/16/7/0`) with 46 covered rows. Historical Batch 21 counts are 25 compared,
          16 baseline-and-stronger, 3 retired, and 5 pending (`25/16/3/5`) with 41
          covered rows. Historical Batch 20 counts are 24 compared,
         16 baseline-and-stronger, 3 retired, and 6 pending (`24/16/3/6`); Batch 20
         historical registry status is **24 compared, 16 baseline-and-stronger, 3 retired,
         and 6 pending** rows. Historical Batch 19 status was 24 compared,
        15 baseline-and-stronger, 3 retired, and 7 pending (`24/15/3/7`) with 39
        covered rows. Historical Batch 18 status was 24 compared,
       14 baseline-and-stronger, 3 retired, and 8 pending (`24/14/3/8`) with 38
       covered rows. Historical Batch 16 status was 22 compared,
     14 baseline-and-stronger, 3 retired, and 10 pending (`22/14/3/10`) with 36
     covered rows. Historical Batch 15 status was 21 compared, 14
     baseline-and-stronger, 3 retired, and 11 pending (`21/14/3/11`) with 35
     covered rows. Pending-read baselines use fixed 100 ms baseline-only delay
plus independent exact-modern client for later public write, avoiding
same-transport scheduling artifact. Required fixture proofs retain stronger
 readiness, matcher, frame-limit, framed-match, and call-time-precedence
 obligations. Batch 3 preserves complete public raw-read fields and independent
 host-to-peer wire bytes. Batch 4 preserves source-derived live flood bytes and
  exact bounded public metadata. Batch 5 preserves duplicate CRLF diagnostics,
  exact trace bytes, partial-line pending behavior, and six-field cursor tables.
  Batch 6 preserves ACK pre-execution ordering, exact payloads, and six-field
  cursor tables. Batch 7 preserves first matched-pong delivery boundary,
  output-only flush, exact six-field positions, and modeled outcome fields;
  request echoes are not modeled. Batch 8 preserves raw SLIP `rx_framing`
  output, exact hex payloads, frame metadata, six-field positions, and the same
   `elapsed_ms` raw-characterization distinction. Batch 9 preserves the raw
   malformed SLIP fallback hex/error/no-frame result and six-field positions;
   its stronger `protocol: slip` proof retains valid-frame-before-error plus
   recovery. Batch 10 preserves the shared public double-arm/sendraw scaffold,
   exact malformed target, exact recovery target, and six-field positions. Batch
    11 preserves direct `protocol: {"type":"cobs"}` preset output from static
    independent wire `00 05 70 6f 6e 67 00`, raw-parser frame metadata, `7/0/0`
    counters, `timeout`, and positions `52/59/0/0/0/59`; it is distinct from raw
    COBS framing and broader zero-containing COBS TX/RX fixture proof. Batch 11 is
     a historical checkpoint. Batch 12 adds direct explicit-parser AT evidence for
     `native_read_at_parser_parses_pong` as a historical checkpoint. Batch 13 adds
     direct protocol-default AT evidence as a historical checkpoint. Batch 14 adds
       direct JSON-parser evidence as a historical checkpoint; Batch 15 adds direct
        NDJSON-preset evidence as a historical checkpoint; Batch 16 adds direct
         NMEA-0183 preset evidence as a historical checkpoint; Batch 17 adds direct
          Modbus ASCII preset evidence as a historical checkpoint; Batch 18 adds direct
            close-while-read evidence as a historical checkpoint; Batch 19 adds historical
            reopen/fresh-output baseline-and-stronger evidence with 39 covered rows; Batch
             20 adds historical flush-during-armed-delay baseline-and-stronger evidence with
              40 covered rows; Batch 21 adds historical input-flush backlog Compared evidence
              with 41 covered rows; Batch 22 adds historical bootloader-touch process-exit
              Compared evidence with 42 covered rows.
              Native differential oracle was required during parity; Phase F
             subsequently removed its active source/configuration coupling.

Current status is **26 compared, 16 baseline-and-stronger, 7 retired, and 0 pending** (`26/16/7/0`) with 46 covered rows. All 49 registry rows now have compared, baseline-and-stronger, or explicit retired disposition; full normalized differential parity is accepted, active native/NCS source and configuration coupling is removed, and fresh clean-checkout CI acceptance remains pending.

Batch 12 uses standard anonymous public `open` at 115200 with `profile_mode:
"none"`, public boot-banner literal-match `read`, public
`transact("arm_cmd 1000\r\n")` matching exact `arm_cmd delay=1000\r\n`, and
public UTF-8 `write("ping\r\n")` with `bytes_written=decoded_bytes=6`. Target
`read` uses `from={"type":"now"}`, `encoding="utf8"`, `timeout_ms=3000`,
explicit `rx_framing: line`, and explicit `rx_parser: at_command`; setup calls
are validated but excluded from normalized observations. The one-second public
arm barrier replaces source-test sleep/flush behavior.

The normalized target is anonymous, normal UTF-8 `pong\r\n`, with
`bytes_read/bytes_observed/bytes_returned=6/0/0`, `stop_reason=timeout`, no
match, truncation, drop, or error, one index-0 `line` frame with payload `pong`,
and parsed
`{"parser":"at_command","response_type":"data","command":null,"status":null,"fields":["pong"]}`.
Positions are `52/58/0/0/0/58` in
`from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
order. Schema is
`serial-mcp.native-sim-differential.at-parser-batch.v1`, report is
`at-parser-batch.json`, and retained target characterization is
`target/native-sim-differential/at-parser-characterization.json` with SHA-256
`52b573c8a71da8aa52fa6ce12ce81d63f5f30756839ae8db3a1e4e56a6424eb5`.
This is direct explicit-parser historical evidence; existing
`at_command_connection_default_drives_stateful_transact_and_parser_quirk`
`AtPeer` fixture proof remains stronger stateful AT behavior. Batch 13 is historical
partial evidence; Batch 14 is historical partial evidence; Batch 15 is historical
partial evidence; Batch 16, Batch 17, and Batch 18 are historical partial evidence;
Batch 19 is historical baseline-and-stronger evidence; Batch 20 is historical
baseline-and-stronger evidence; Batch 21 is historical direct Compared evidence;
Batch 22 is historical direct Compared evidence; post-Batch-22 pre-txbuf-retirement
checkpoint is `26/16/4/3` with 43 covered rows; Phase F blocked.

Batch 13 is historical direct Compared evidence for
`native_open_protocol_default_drives_write_and_read`. Open carries a
protocol-only default `protocol: {"type":"at_command"}` and omits
`rx_framing`. Default-framed public setup uses `transact("arm_cmd 1000")` with
stripped framed arm match `arm_cmd delay=1000`, then bare `ping` adds CR (`4→5`).
Target bare read uses only `from={"type":"now"}`, `timeout_ms=3000`, and UTF-8
encoding. It returns `pong\r\n`,
`bytes_read/bytes_observed/bytes_returned=6/0/0`, no match, truncation, drop, or
error, one AT-parsed UTF-8 line frame with fields `["pong"]`, and positions
`52/58/0/0/0/58`. Schema is
`serial-mcp.native-sim-differential.at-protocol-default-batch.v1`, report is
`at-protocol-default-batch.json`, and characterization is
`target/native-sim-differential/at-protocol-default-characterization.json` with
SHA-256
`cce2c8a47d3d23eedfb857b5701428937174ab066bd6b64ce20e544776b68775`.
The stronger `AtPeer` proof remains required; Phase F blocked.

Batch 14 is historical direct Compared evidence for
`native_read_json_parser_decodes_jsonout`. Both endpoints use standard anonymous
public `open` at 115200 with `profile_mode: "none"`, public boot-banner
literal-match `read`, public `transact("arm_cmd 1000\r\n")` matching exact
`arm_cmd delay=1000\r\n`, and public UTF-8 `write("jsonout\r\n")` with
`bytes_written=decoded_bytes=9`. Target `read` uses `from={"type":"now"}`,
UTF-8, 3000 ms, explicit line framing, and explicit `json_lines` parsing. The
static three JSON object response is 140 bytes. The normalized target has
`bytes_read/bytes_observed/bytes_returned=140/0/0`, stop reason `timeout`, no
match, truncation, drops, or error, three ordered parsed objects, and positions
`52/192/0/0/0/192`. Schema is
`serial-mcp.native-sim-differential.json-parser-batch.v1`, report is
`json-parser-batch.json`, and characterization is
`target/native-sim-differential/json-parser-characterization.json` with SHA-256
`f51b5d77bac3904d214e2ea76794cf1d10f4d5aa8849224e750af30a8e9e3a06`. Existing
JSON Lines fixture proof remains stronger. The existing stronger JSON Lines fixture proof
remains stronger. Batch 14 historical registry is 19 compared, 14
baseline-and-stronger, 3 retired, and 13 pending (`19/14/3/13`) with 33 covered
rows.

Batch 15 is historical direct Compared evidence for
`native_read_ndjson_preset_decodes_json_frames` and
`native_read_ndjson_preset_skips_empty_lines`. Both endpoints use standard
anonymous public `open` at 115200 with `profile_mode: "none"`, public boot-banner
literal-match `read`, public `transact("arm_cmd 1000\r\n")` matching exact
`arm_cmd delay=1000\r\n`, and exact static UTF-8 `sendraw` writes of 48 and 74
bytes after the shared one-second arm barrier. Target reads use
`from={"type":"now"}`, UTF-8, 3000 ms, and only
`protocol: {"type":"ndjson"}`. The preset expands to auto line framing,
`skip_empty:true`, and the JSON parser; no explicit framing/parser, sleep, or
flush is used.

The static payloads are `{"a":1}\n\n{"b":2}\n` and
`{"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n`, sent by
`sendraw hex 7B2261223A317D0A0A7B2262223A327D0A` and
`sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A`.
Exact normalized outcomes are
`17/0/0` with positions `52/69/0/0/0/69` and `30/0/0` with positions
`52/82/0/0/0/82`, respectively. Both targets retain exact raw UTF-8 payload,
stop by `timeout`, emit ordered JSON frames only for records, and have no match,
truncation, drops, or error; blank and whitespace-only lines emit no frames.
Schema is `serial-mcp.native-sim-differential.ndjson-preset-batch.v1`, report is
`ndjson-preset-batch.json`, and characterization is
`target/native-sim-differential/ndjson-characterization.json` with SHA-256
`10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3`. Existing
`ndjson_preset_parses_records_and_skips_blank_whitespace_lines` fixture proof
remains stronger. Batch 15 historical checkpoint is `21/14/3/11` with 35
covered rows; Phase F remains blocked.

Batch 16 is historical direct Compared evidence for
`native_read_nmea0183_preset_decodes_parsed_frame`. Both endpoints use standard
anonymous public `open` at 115200 with `profile_mode: "none"`, public boot-banner
literal-match `read`, public `transact("arm_cmd 1000\r\n")` matching exact
`arm_cmd delay=1000\r\n`, and public UTF-8 `write` of the exact 148-byte command
`sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A\r\n`.
The target read uses `from={"type":"now"}`, UTF-8, 3000 ms, and only
`protocol: {"type":"nmea0183"}`. Start/end framing with markers excluded and
NMEA parsing come from the preset; no explicit framing/parser, sleep, flush, or
generic `sendraw` parser is used.

The static raw wire is the exact 67-byte valid GGA sentence
`$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n`.
The normalized target has `67/0/0` bytes read/observed/returned, stop reason
`timeout`, no match, truncation, drops, or error, one UTF-8 `start_end` frame,
parsed `talker_id="GP"`, `sentence_type="GGA"`, exact ordered fields
`["123519","4807.038","N","01131.000","E","1","08","0.9","545.4","M","46.9","M","",""]`,
and `checksum_valid:true`; positions are `52/119/0/0/0/119`.
The schema is `serial-mcp.native-sim-differential.nmea0183-preset-batch.v1`,
report is `nmea0183-preset-batch.json`, and retained characterization is
`target/native-sim-differential/nmea-characterization.json` with SHA-256
`513d4906b285ef35a2b82ab085968de96f51576acc636244f0eb3a44868f1578`.
Existing `nmea0183_preset_parses_valid_independently_checksummed_sentence`
fixture proof remains stronger. Historical Batch 16 registry was `22/14/3/10`
with 36 covered rows; Phase F blocked.

Batch 17 is historical direct Compared evidence for
`native_read_modbus_ascii_preset_decodes_parsed_frame`. Both endpoints use
standard anonymous public `open` at 115200 with `profile_mode: "none"`, public
boot-banner literal-match `read`, public `transact("arm_cmd 1000\r\n")` matching
exact `arm_cmd delay=1000\r\n`, and public UTF-8 `write` of the exact 48-byte
command `sendraw hex 3A30313033303030303030303146420D0A\r\n`. The target read
uses `from={"type":"now"}`, UTF-8, 3000 ms, and only
`protocol: {"type":"modbus_ascii"}`. Start/end framing with markers excluded
and Modbus ASCII parsing come from the preset; no explicit framing/parser,
sleep, flush, or generic `sendraw` parser is used.

The static raw wire is the exact 17-byte valid response `:010300000001FB\r\n`.
The normalized target has `17/0/0` bytes read/observed/returned, stop reason
`timeout`, no match, truncation, drops, or error, one UTF-8 `start_end` frame,
parsed `address=1`, `function_code=3`, `data=[0,0,0,1]`, and
`checksum_valid:true`; positions are `52/69/0/0/0/69`. The schema is
`serial-mcp.native-sim-differential.modbus-ascii-preset-batch.v1`, report is
`modbus-ascii-preset-batch.json` with SHA-256
`97f9c00dc98e5cd83c440b2e90b7d9f72e58428f9e873f7f418475ef3a79ef9b`, and retained characterization is
`target/native-sim-differential/modbus-ascii-characterization.json` with SHA-256
`b390bdc693778be29a06e40033694e1032986a1700995d065f06d8092fc7973c`.
Existing `modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation`
fixture proof remains stronger. Historical Batch 17 registry is `23/14/3/9` with
37 covered rows; Phase F blocked.

Historical Batch 18 direct Compared evidence covers
`native_close_while_read_active_returns_normal_result`. Both endpoints use
standard anonymous public `open` at 115200 with `profile_mode: "none"`, public
boot-banner literal-match `read`, public `transact("arm_cmd 1000\r\n")` matching
exact `arm_cmd delay=1000\r\n`, and public UTF-8 marker write of the exact
command `sendraw hex 524541442D52454144592D4D41524B45520D0A\r\n`. A secondary
modern client starts an unmatched `from={"type":"now"}` read with UTF-8 and
3000 ms; the primary client polls normalized `get_status.rx_bytes` until the
marker increases RX by 19 and proves the pending read remains unfinished.

Primary close returns normal anonymous `source="disabled"` profile with
`profile_persistence.operation="close_snapshot"` and
`profile_persistence.state="transient"`; pending read returns
`READ-READY-MARKER\r\n` with `19/19/19`, `connection_closed`, unmatched, no
truncation, frames, drops, or error, and positions `52/71/0/0/0/71`. Schema is
`serial-mcp.native-sim-differential.close-while-read-batch.v1`, report is
`close-while-read-batch.json` with SHA-256
`fef0499d6504635c104fe3d125fe572c49e2ef4bec69a5b39c32bdeb50361a09`, and
retained characterization is
`target/native-sim-differential/close-while-read-characterization.json` with
SHA-256 `06a7adb2b4f1c1c6f8b3c8fd507ba9e5004df6dd7a9f4ef6b64c4fff87c3f69b`.
Existing `close_interrupts_readiness_proven_live_read_with_connection_closed`
fixture proof remains stronger. Historical Batch 18 registry was `24/14/3/8`
with 38 covered rows; Phase F blocked.

**Historical Baseline-and-stronger Batch 19** is historical evidence for
`native_reopen_then_match_finds_fresh_output`. Standard anonymous public `open`
uses `profile_mode: "none"` at 115200. Boot banner synchronizes only on first
open because firmware emits it once at process start. First `write("ping\r\n")`
is normal UTF-8 `6/6`; first positioned literal `pong` read is exact
`pong\r\n`, `6/6/6`, `match_found`, index 0, no frames/drops/error/truncation,
and positions `32/38/0/0/0/38`; first close retains disabled/transient anonymous
close-profile checks. Same endpoint second `open` requires a distinct raw
connection ID. A status-only public probe requires `rx_bytes=0` before the
second command and is not normalized. Pending second `from={"type":"now"}`
UTF-8 literal-match read uses bounded 100 ms baseline admission before an
independent modern client writes `ping\r\n`; second read is exact `pong\r\n`,
`6/6/6`, `match_found`, index 0, no frames/drops/error/truncation, and positions
`0/6/0/0/0/6`. Existing
`reopen_same_path_returns_distinct_id_and_only_fresh_generation` fixture proof
remains stronger. Batch 19 schema is
`serial-mcp.native-sim-differential.reopen-fresh-output-batch.v1`, report is
`reopen-fresh-output-batch.json` with SHA-256
`7c9d9071156739a0a2bc81a9ef2adba48a40132ba6bd1208b99e8e04a847d02e`. Current
registry is historical `24/15/3/7` with 39 covered rows; Phase F blocked.

**Historical Baseline-and-stronger Batch 20** is historical evidence for
`native_flush_during_arm_cmd_delay`. Standard anonymous public `open` uses
`profile_mode: "none"` at 115200, and boot synchronization remains the first
two normalized observations. Public `transact("arm_cmd 1000\r\n")` validates exact
`arm_cmd delay=1000\r\n` acknowledgement but is excluded from normalized output.
Public `write("ping\r\n")` is normal anonymous UTF-8 `6/6`. The bounded 100 ms
post-write pause is baseline admission only, not a peer-acceptance proof.
Public `flush(target="both")` is normal anonymous with exact target `both`.
Positioned `from={"type":"now"}` UTF-8 literal `pong\r\n` read returns exact
`pong\r\n`, `6/6/6`, `match_found`, index 0, no truncation/frames/drops/error,
and positions `52/58/0/0/52/58` in
`from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
order. `RxRing::clear` sets retained `start_offset` to monotonic live edge 52.
Existing `flush_after_command_acceptance_does_not_cancel_delayed_response`
fixture proof remains stronger. Batch 20 schema is
`serial-mcp.native-sim-differential.flush-during-arm-delay-batch.v1`, report is
`flush-during-arm-delay-batch.json`, and historical registry is `24/16/3/6` with
40 covered rows; Phase F blocked.

**Historical Compared Batch 21** is historical evidence for
`native_flush_input_clears_host_rx`. Standard anonymous public `open` uses
`profile_mode: "none"` at 115200, and boot synchronization remains the first two
normalized observations. Public UTF-8 old-marker write
`sendraw hex 4F4C442D4D41524B45520D0A\r\n` is normal anonymous with exact
`bytes_written=decoded_bytes=38` and emits `OLD-MARKER\r\n`; status-only
`get_status` polling proves `rx_bytes >= 44` and is not normalized. Public `flush(target="input")` is normal
anonymous with exact target `input`. Public UTF-8 new-marker write
`sendraw hex 4E45572D4D41524B45520D0A\r\n` is normal anonymous with exact
`bytes_written=decoded_bytes=38` and emits `NEW-MARKER\r\n`; status-only polling
proves `rx_bytes >= 56` and is not normalized. Positioned
`from={"type":"buffer_start"}` UTF-8 literal
`NEW-MARKER\r\n` read returns only exact `NEW-MARKER\r\n`, `12/12/12`,
`match_found`, index 0, no truncation/frames/drops/error, and positions
`44/56/0/0/44/56`. The returned new-only payload proves old-marker absence.
The existing `flush_input_discards_known_old_marker_and_keeps_new_marker`
fixture proof remains stronger. Batch 21 schema is
`serial-mcp.native-sim-differential.input-flush-batch.v1`, report is
`input-flush-batch.json` with SHA-256
`ff95074207f7de216780ede42a6d21583e4e88e5c5e5c6f81af4b588fa5dcfd8` after two
matching runs, and historical registry is `25/16/3/5` with 41 covered rows; Phase F
blocked.

**Historical Compared Batch 22** is historical evidence for `native_bootloader_touch_exits_42`.
Standard anonymous public `open` uses `profile_mode: "none"` at 115200, and the
matching boot-banner read remains the second normalized observation. Native uses
the existing firmware PTY; fixture uses a real dedicated small Rust child process
with a raw PTY, not `FixtureExit::Crashed`. The child emits exact
`serial-mcp test firmware ready\r\n` before publishing `PTY_PATH`. Public UTF-8
`write("touch\r\n")` is normal anonymous with exact
`bytes_written=decoded_bytes=7`; both endpoints exit exactly 42 and the typed
normalized outcome retains `{"kind":"process_exit","exit_code":42}`. The
terminal `touch exit(42)\r\n` response is not read, so no response-delivery claim
is made, and no public `close` follows peer exit. Existing
`touch_write_causes_small_rust_child_peer_to_exit_42` remains independent and
stronger process-exit fixture proof. Batch 22 schema is
`serial-mcp.native-sim-differential.bootloader-touch-exit-batch.v1`, report is
`bootloader-touch-exit-batch.json` with SHA-256
  `91befb4e3af3edd65c70c58208be03c09c8c29aed04f9b432e18c1d5becd4d9c` after two
  matching runs, and historical registry is `26/16/3/4` with 42 covered rows; Phase F
  blocked.

Post-Batch 22 retirement: `native_list_ports_includes_identity_fields` is **Retired**, not a Batch 23 or differential case. Deterministic injected-provider identity/profile/schema proofs supersede ambient OS field/null checks. The three public real-PTY/injected-provider proofs are `list_ports_preview_empty_store_reports_none_parallel_and_pure_ports`, `list_ports_preview_selected_winner_matches_later_bare_open`, and `list_ports_preview_output_validates_against_generated_schema`. No new differential batch, report, or hash was created. The pre-txbuf-retirement checkpoint is `26/16/4/3` with 43 covered rows (26 compared, 16 baseline-and-stronger, 4 retired, and 3 pending); full parity and Phase F remain blocked.

Post-Batch-22 txbuf retirement: `native_txbuf_status_reports_pending` is **Retired**, not a Batch 23 or differential case. Source did not observe a nonzero pending TX queue; `txbuf` and `hold` are firmware-only commands. `device_command_parity::held_output_reports_nonzero_queue_then_drains_and_recovers` proves nonzero held output, blocked read, drain, and recovery. No Batch 23, report, or hash was created. The historical pre-auto-reconnect-retirement checkpoint is `26/16/5/2` with 44 covered rows; full parity and Phase F remain blocked.

Post-Batch-22 auto-reconnect retirement: `native_auto_reconnect_preserves_connection` is **Retired**, not a Batch 23 or differential case. Source only invokes `reconnect` while already open; strengthened `device_fixture::public_mcp_ping_hold_disconnect_replace_and_reconnect` preserves same ID/open state and post-no-op traffic, then proves peer loss, replacement, reconnect, and fresh exchange. No Batch 23, report, or hash was created. This is the historical pre-capture-boot-retirement checkpoint at `26/16/6/1` with 45 covered rows; full parity and Phase F remain blocked.

Post-Batch-22 capture-boot retirement: `native_capture_boot_arm_only_captures_post_arm_command_output` is **Retired**, not a Batch 23 or differential case. Source only checks arm-only post-mark output after a consumed stale banner; strengthened `device_command_parity::capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor` excludes stale bytes, preserves private-mark replay and shared cursor/history, and captures post-arm output. No Batch 23, report, or hash was created.

## Recommendation

Replace NCS/Zephyr `native_sim` with direct `nix::pty::openpty` plus reusable
in-process Rust device fixture. Keep real OS slave path and public MCP
`open(port=...)` as parity boundary. Keep controlled backend for DTR/RTS, BREAK,
and deterministic I/O failures. Use separate child process only for crash/exit
and small spawned-server smoke.

Protocol peers:

- local AT DCE state machine;
- `slip-codec 0.4.0` valid-vector oracle plus local malformed injection;
- `serde_json` and local state for JSON Lines/NDJSON;
- `cobs 0.5.1` valid/malformed cross-check;
- local NMEA generator and GPSD/AIS/proprietary golden vectors;
- `rmodbus 0.12.2` mutable Modbus core plus local ASCII stream wrapper;
- local byte builders for generic framing, shell prompt, and raw.

Do not retain disposable rustix or Python boundary dependencies after decision.
They showed no fidelity or cleanup advantage.

## Why

Measured 100-run Linux experiment:

- nix: 122.446 s, FD 10→10, direct children 0→0;
- rustix: 122.474 s, same behavior/cleanup;
- Python: 129.584 s, same behavior/cleanup plus child/IPC cost.

All passed arbitrary bytes, split command, fragmented response, bounded
flood/hold, same-path reopen, and distinct endpoint replacement. Nix is already
repository dependency and simplest owner model.

Current CI evidence shows dedicated native job spends minutes on NCS setup and
uploads about 8.1 GB cache. Past failures include nrfutil bootstrap and
duplicated Rust dependency configuration unrelated to firmware behavior.

## Resolved Blocking Finding

Initial candidates produced public read `stop_reason="timeout"` after peer-master
close. Phase A mapped zero-byte serial reads to `UnexpectedEof`, classified that
kind as fatal while leaving generic `Other` nonfatal, and proved 100/100
`connection_closed`. Phase B added stable-symlink atomic retarget and a real
public `reconnect` proof against distinct replacement PTY.

## Fixture Architecture

```text
DeviceFixture
├── PtyBoundary (nix master/slave/path)
├── DeviceCore (input assembly + state dispatch)
├── OutputQueue (capacity/chunk/hold/drop-or-block/drain)
├── FaultScript (emit/chunk/delay/silence/malformed/saturate/close/crash)
├── Clock/Readiness controls
├── ProtocolPeer enum/trait
└── explicit async shutdown report

Public scenario
├── DeviceFixture
├── TestServer or small spawned serial-mcp process
├── real rmcp client
└── MCP open/write/read/transact/capture_boot assertions
```

Ownership:

- fixture retains slave FD for same-pair reopen;
- peer task owns master I/O behind cancellation;
- shutdown cancels, awaits bounded completion, aborts only as fallback, closes
  FDs, and reports reason;
- Drop is best-effort, never acceptance mechanism;
- stable symlink lives in fixture tempdir, created no-clobber and atomically
  retargeted, then removed by explicit shutdown.

## Tradeoffs

| Option | Decision | Reason |
|---|---|---|
| keep NCS | reject | high CI/cache/provisioning cost; no required MCU fidelity |
| direct nix | select | existing dependency, owned FDs, best measured cost/fit |
| rustix challenger | reject after prototype | same behavior; adds second Unix abstraction and bus-factor surface |
| Python primary | reject after prototype | same fidelity; child/IPC/runtime variance and slower |
| socat primary | reject/waive prototype | relay still needs peer process; adds executable/version/symlink supervision |
| portable-pty/process crates | reject | terminal/process policy without required capability gain |
| tty0tty kernel | reject primary | privileged Linux driver; unsuitable hosted CI/macOS |
| QEMU/Renode | reject primary | irrelevant CPU/peripheral emulation and large dependency cost |

## Implementation Phases and Rollback Points

### Phase A — disconnect truth

- Add focused PTY test: peer master closes during readiness-proven pending read.
- Capture exact OS error and public result.
- Apply narrow classification fix only if needed.
- Prove 100/100 `connection_closed` and no FD/task leak.

Public behavior: disappeared peer stops read as connection closure.
Rollback: isolated product fix/test; no fixture migration.

### Phase B — fixture foundation

- Promote direct-nix prototype into `tests/common/device_fixture/`.
- Add input assembler, bounded output queue, action script, injected readiness,
  explicit async shutdown, same-path reopen, stable-symlink replacement.
- Unit-test fixture state/actions and cleanup.
- Add spawned-server smoke.

Public behavior: MCP opens real slave, exchanges bytes, closes/reconnects to real
replacement, all fixture resources terminate.
Rollback: remove new test-only fixture; native suite unchanged.

### Phase C — command parity

- Implement only observed ping/info/delay/ACK/flood/hold/raw/exit behavior.
- Parameterize all 49 cases using traceability table.
- Strengthen/split/retire only as documented, with stronger proofs already
  required.
- Run native and replacement scenarios against normalized MCP outcomes.

Public behavior: all existing user-visible outcomes preserved or strengthened.
Rollback: replacement remains optional; native required.

### Phase D — protocol peers

- Add exact helper pins after advisory/license review.
- Implement every worksheet happy/fragmented/stateful/fault path.
- Add shipped preset/framing/parser coverage metadata and drift guard.
- Characterize parser questions before changing product semantics.

Public behavior: every shipped protocol has real-path stateful peer coverage.
Rollback: protocol additions can land by peer family while native command suite
remains oracle.

### Phase E — required differential gate

- **Complete:** replacement targets were explicit required CI evidence during
  parity; native differential runs are now historical.
- **Complete:** Linux x86_64 runs timing/flood/hold/flush/disconnect/reconnect
  public lifecycle 100 times with fixed seed `0x50484153455f4545`.
- **Current platform scope:** production-path real-PTY fixture tests are
  Linux-only because macOS `serialport` baud configuration invokes
  `IOSSIOSPEED`, which macOS PTYs reject with `ENOTTY`. macOS still gets normal
  Rust fmt/build/test/clippy and controlled-backend tests; Linux-only fixture
  targets compile as zero tests there. Windows remains explicit
  compile/controlled coverage.
- **Accepted:** all 22 differential batches and all 42 executable cases are part
  of the accepted full normalized parity window. The current registry classifies
  all 49 rows as compared, baseline-and-stronger, or retired, with 46 covered
  rows and no pending rows.

> Full normalized differential parity window accepted on 2026-08-23: all 42 executable cases passed in three serial 22-batch runs; two consecutive canonical 22-report manifests matched SHA-256 `b31b3f3da1412d210096a618cc8d6b6acc5bbed167de525c8122704342c3d3fe`.

- Phase F active source/configuration removal is complete locally. Fresh
  clean-checkout no-NCS CI acceptance remains pending.

> Local worktree verification on 2026-08-23 passed `nix flake check --accept-flake-config` and `nix develop --ignore-env`; the shell found no `west`, `nrfutil`, or `nrfutil-sdk-manager` on `PATH` before `cargo test --locked`. This is not fresh clean-checkout CI evidence.

Rollback: revert source-removal change; prior parity evidence remains reviewable.

### Phase F — NCS deletion

- **Complete locally:** delete firmware tree, native harness/wrappers, and
  nrfutil scripts/tests.
- **Complete locally:** remove NCS job/cache/install/disk cleanup and release
  dependency.
- **Complete locally:** remove `nix-nrf-dev`, multilib firmware shell, lock
  nodes, and firmware LSP.
- **Complete locally:** simplify xtask to normal Rust assets/tests.
- **Complete locally:** update evaluator refs, README, AGENTS, active docs, and
  doc-drift guards; retain changelog release history.
- Clean-checkout full gate with no NCS/west/nrfutil present remains pending fresh
  CI evidence.

Rollback: revert deletion PR; previous parity commits remain independently
reviewable.

## Differential Outcome Schema

Compare scenario outputs after removing nondeterministic IDs/timestamps/path
numbers:

```text
scenario_id
tool
is_error
stop_reason
matched
bytes_observed
bytes_returned
frames[{type,data,parsed,checksum_valid}]
frames_dropped
state/config/counter deltas
peer exit/disconnect outcome
```

Do not normalize away payload bytes, ordering, stop reason, frame type, checksum
status, drop count, or offset relationships.

Batch 1 report uses schema ID
`serial-mcp.native-sim-differential.command-lifecycle-batch.v1` and writes
paired typed outcomes to
`target/native-sim-differential/command-lifecycle-batch.json`. It contains no
actual PTY path, connection ID, or timestamp and must serialize byte-identically
across back-to-back successful runs. This report is partial evidence, not a full
parity or Phase F readiness claim.

Batch 2 report uses schema ID
`serial-mcp.native-sim-differential.generic-matching-framing-batch.v1` and
writes six paired outcomes to
`target/native-sim-differential/generic-matching-framing-batch.json`. It retains
effective top-level/frame encodings and bytes, independent counters,
`match_frame_index`, typed AT parsed fields, `frames_dropped`, and framing error
instead of normalizing framed structure away. The reports include current global
  status: 17 compared, 14 baseline-and-stronger, 3 retired, and 15 pending.

Batch 3 report uses schema ID
`serial-mcp.native-sim-differential.raw-generic-framing-batch.v1` and writes five
paired outcomes to
`target/native-sim-differential/raw-generic-framing-batch.json`. Delayed native
`sendraw` characterization showed exact target-only bytes after `from=now`, with
fresh trace sequence zero and one exact `RX[n]=0xhh\r\n` line per target byte.
The report retains complete raw/frame public fields and typed
`PeerWireObservation` values for exact independent host-to-peer delimiter,
length-prefixed, start/end, and SLIP vectors. Length-prefixed RX and TX wire
rows are baseline-and-stronger with their required fixture proofs; the other
three rows are compared.

Batch 4 report uses schema ID
`serial-mcp.native-sim-differential.flood-buffer-batch.v1` and writes two paired
outcomes to `target/native-sim-differential/flood-buffer-batch.json`:

- `native_read_match_on_spam_complete` sends exact `spam 1024 hex\r\n` after a
  100 ms pending-read admission delay. Source-derived xorshift32 bytes produce
  exact UTF-8 payload length 1088 and `Spam complete` match index 1056.
- `native_read_buffer_budget_stops_under_flood` configures connection default
  `max_buffered_bytes: 256`, then sends exact `spam 512 hex\r\n` after the same
  pending-read admission delay. Result retains exact first 256 bytes, all byte
  counters at 256, `max_buffered_bytes`, unmatched/no-frame/no-drop/no-error
  metadata, and no truncation. Both executable target reads retain all six stable
  public offset/backlog fields: `from_offset`, `next_offset`,
  `bytes_lost`, `buffered_remaining`, `start_offset`, and `end_offset`.
  Measured values in that order are `32/1120/0/0/0/1120` for `spam 1024` and
  `32/288/0/31/0/319` for live `spam 512`.

Native characterization retained variable diagnostic `elapsed_ms`, which is
omitted from typed Batch 4 outcomes, and variable prefilled `spam 65536 hex`
backlog fields. The prefilled 65536 sample is excluded from executable parity
because those values varied across fresh native runs. All eight reports remain
isolated, and this evidence does not claim physical UART behavior, clean-checkout
parity without NCS, CI wiring, full differential parity, or Phase F readiness.

Batch 5 report uses schema ID
`serial-mcp.native-sim-differential.command-diagnostics-batch.v1` and writes
three paired outcomes to
`target/native-sim-differential/command-diagnostics-batch.json`:

- `native_framing_reports_single_split_command` compares exact UTF-8 payload
  `LINE len=4 data="ping"\r\nLINE len=4 data="ping"\r\npong\r\n` (54 bytes),
  `match_index=48`, and positioned table `44/98/0/0/0/98`;
- `native_trace_reports_exact_split_byte_sequence` compares six exact lower-case
  `RX[n]=0xhh\r\n` records for `ping\r\n`, from `RX[0]=0x70\r\n` through
  `RX[5]=0x0a\r\n`, then `pong\r\n` (78 bytes),
  `match_index=72`, and positioned table `42/120/0/0/0/120`;
- `native_partial_line_buffered_then_completed` proves `pi` remains pending
  through bounded admission checks before `ng\r\n` produces exact `pong\r\n`
  (6 bytes), `match_index=0`, and positioned table `32/38/0/0/0/38`.

Native CRLF handling intentionally produces duplicate framing diagnostics. The
compatibility peer reproduces this measured public result without changing
firmware, fixture core, or adding raw-byte APIs. All three rows retain
`split_writes_preserve_one_command_and_exact_wire_order` as stronger proof.

Batch 6 report uses schema ID
`serial-mcp.native-sim-differential.ack-state-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/ack-state-batch.json`:

- `native_ack_command_provides_pre_execution_ack` compares five ordinary
  shared-cursor public write/read pairs after the 32-byte boot banner. Exact
  sequence is `ack on\r\n` → `ack on\r\n` (8 bytes, match index 0,
  `32/40/0/0/0/40`), `ping\r\n` → `ack 0\r\npong\r\n` (13 bytes, match
  index 7, `40/53/0/0/0/53`), `ping\r\n` → `ack 1\r\npong\r\n` (13 bytes,
  match index 7, `53/66/0/0/0/66`), `ack off\r\n` →
  `ack 2\r\nack off\r\n` (16 bytes, match index 7, `66/82/0/0/0/82`),
  and final `ping\r\n` → `pong\r\n` (6 bytes, match index 0,
  `82/88/0/0/0/88`).
- Every target read is normal UTF-8 `match_found`, `matched: true`, with no frames,
  no drops, no error, no truncation, and all three byte counters equal
  exact payload length. `ack 2\r\n` remains before `ack off\r\n`; no
  baseline-proof binding is used. The existing
  `device_command_parity::ack_peer_orders_ack_before_response_and_stops_after_disable`
  remains the stronger semantic proof. Phase F remains blocked.

Batch 7 report uses schema ID
`serial-mcp.native-sim-differential.output-flush-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/output-flush-batch.json`:

- `native_flush_output_after_full_delivery_is_safe` opens anonymously with
  `profile_mode: "none"`, `name: null`, and baud 115200, then compares this
  exact order after the 32-byte boot banner: first `write("ping\r\n")`,
  positioned `read(match="pong")`, `flush(target="output")`, second
  `write("ping\r\n")`, and second positioned `read(match="pong")`;
- both writes are normal anonymous UTF-8 six-byte results with
  `bytes_written=decoded_bytes=6`. Both reads return exact `pong\r\n`, have
  6/6/6 byte counters, `match_index=0`, `match_found`, no frames, drops,
  error, or truncation, and positions `32/38/0/0/0/38` then
  `38/44/0/0/0/44` in `from/next/lost/remaining/start/end` order;
- First matched `pong` is first-command fully-delivered/consumed boundary
  before output-only flush. Flush is normal anonymous with exact target
  `output` and retains RX cursor, so second read starts at 38. No sleep,
  readiness shortcut, or weaker timing substitution is used. `elapsed_ms` is the
  only nondeterministic field removed from raw characterization. Typed
  differential reports retain modeled outcome fields; caller-supplied request
  echoes (`timeout_ms`, `no_new_rx_timeout_ms`) are not modeled. This intentional omission
  is separate from the request-echo model boundary. The
  existing `device_command_parity::output_flush_after_full_delivery_preserves_later_traffic`
  remains the stronger Rust-PTY proof. No baseline-proof binding is used. There
  are 26 covered rows at the Batch 7 checkpoint; Phase F remains blocked.

Batch 8 report uses schema ID
`serial-mcp.native-sim-differential.slip-happy-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/slip-happy-batch.json`:

- `native_read_slip_decodes_frame` uses standard anonymous open, then the same
  all-public setup on native and compatibility fixture endpoints:
  `transact("arm_cmd 1000\r\n", match="arm_cmd delay=1000\r\n")`,
  `write("sendraw hex C0706F6E67C0\r\n")`, and `read` with
  `from={"type":"now"}`, `encoding="hex"`, and raw
  `rx_framing={"type":"slip","max_frames":1}`;
- arming acknowledgement is exact and immediate. Compatibility peer consumes
  one private armed one-second delay before the next exact command's output,
  then emits only `c0 70 6f 6e 67 c0`. Setup write is normal UTF-8 with
  `bytes_written=decoded_bytes=26`; no sleep, flush, direct fixture API,
  fixture-core API, native-only branch, product change, or broad sendraw parser
  is used;
- the normalized read keeps modeled outcome fields: anonymous normal
  result, effective hex payload `c0 70 6f 6e 67 c0`, counters `6/0/6` for
  `bytes_read/bytes_observed/bytes_returned`, stop `max_frames`, no
  match/truncation/drop/error, one raw `slip` frame at index zero with hex
  payload `70 6f 6e 67` and no parser, and positions
  `52/58/0/0/0/58` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order. `elapsed_ms` is the only nondeterministic field removed from raw
  characterization. Typed differential reports retain modeled outcome fields;
  caller-supplied request echoes (`timeout_ms`, `no_new_rx_timeout_ms`) are not
  modeled;
- this is raw `rx_framing: slip` happy-path comparison only. It does not claim
  preset, malformed, recovery, or full protocol parity. Existing
  `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
  remains the stronger Rust-PTY SLIP proof. The Batch 8 checkpoint recorded
  registry status as 14 compared, 13 baseline-and-stronger, 3 retired, and 19 pending,
  with 27 covered rows; Phase F remained blocked.

Batch 9 report uses schema ID
`serial-mcp.native-sim-differential.slip-malformed-batch.v1` and writes one
baseline-and-stronger outcome to
`target/native-sim-differential/slip-malformed-batch.json`:

- `native_read_slip_malformed_escape_returns_partial_result` uses standard
  anonymous open and identical public setup on native and compatibility fixture
  endpoints: `transact("arm_cmd 1000\r\n", match exact
  "arm_cmd delay=1000\r\n")`, `write("sendraw hex C0DB41C0\r\n")`, and
  `read(from={"type":"now"}, encoding="utf8",
  rx_framing={"type":"slip"})`. Arming acknowledgement is exact; setup write
  is normal anonymous UTF-8 with `bytes_written=decoded_bytes=22`. Setup is not
  normalized. No sleep, flush, private fixture script, raw fixture API,
  native-only branch, or `max_frames` is used;
- the normalized target is anonymous and normal with effective fallback hex
  payload `c0 db 41 c0`, counters `4/0/0` for
  `bytes_read/bytes_observed/bytes_returned`, stop `framing_error`, no match,
  truncation, frames, or drops, and exact error
  `SLIP framing error: invalid escape byte 0x41`. Positions are
  `52/56/0/0/0/56` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order;
- `elapsed_ms` is the only raw-characterization field removed. unmodeled request echoes
  remain outside typed outcomes. This preserves original raw
  `rx_framing: slip` malformed one-frame-free behavior. The stronger
  `protocol: slip` proof
  `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
   retains valid-frame-before-error plus recovery. Batch 9 does not claim full
   SLIP protocol parity. Batch 9 counts are an explicit historical checkpoint:
   14 compared, 14 baseline-and-stronger, 3 retired, and 18 pending
   (14/14/3/18) with 28 covered rows; they are not current. Phase F remains
   blocked.

Batch 10 report uses schema ID
`serial-mcp.native-sim-differential.slip-recovery-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/slip-recovery-batch.json`:

- `native_read_slip_recovers_after_error_on_next_call` uses standard anonymous
  open and the shared public double-arm/sendraw scaffold on native and
  compatibility fixture endpoints. First public setup is
  `transact("arm_cmd 1000\r\n", match="arm_cmd delay=1000\r\n")`, public
  `write("sendraw hex C0DB41C0\r\n")`, and raw public
  `read(from={"type":"now"}, encoding="utf8", rx_framing={"type":"slip"})`;
  second setup repeats public arm and sends
  `sendraw hex C0706F6E67C0\r\n`, then public recovery read uses exactly
  `from={"type":"now"}`, `encoding="hex"`, `timeout_ms=3000`, and
  `rx_framing={"type":"slip"}`. Both setup calls are validated but excluded
  from normalized observations;
- observation order is open, boot read, exact malformed read, exact recovery
  read. No setup output, sleep, flush, raw fixture API, native-only path,
  `max_frames`, `no_new_rx_timeout_ms`, or protocol preset is normalized;
- malformed target is effective hex `c0 db 41 c0`,
  `bytes_read/bytes_observed/bytes_returned=4/0/0`,
  `framing_error`, exact error `SLIP framing error: invalid escape byte 0x41`,
  no frame/drop/match/truncation, and positions `52/56/0/0/0/56`;
- recovery target is effective hex `c0 70 6f 6e 67 c0`,
  `bytes_read/bytes_observed/bytes_returned=6/0/0`, timeout, no
  drop/match/truncation/error, one raw SLIP frame at index zero with hex payload
  `70 6f 6e 67`, and positions `76/82/0/0/0/82`. Raw consumption
  advances cursor despite zero `bytes_returned`;
- the stronger `protocol: slip`
   `slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery`
   proof retains valid-frame-before-error plus recovery. Batch 10 is direct raw
   `rx_framing: slip` malformed-plus-recovery evidence, not full SLIP protocol
   parity. Batch 10's historical checkpoint was 15 compared, 14
    baseline-and-stronger, 3 retired, and 17 pending with 29 covered rows; it is
    historical only.

Batch 11 report uses schema ID
`serial-mcp.native-sim-differential.cobs-preset-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/cobs-preset-batch.json`:

- `native_read_cobs_preset_decodes_frame` uses standard anonymous open and the
  same public `arm_cmd 1000` then exact `sendraw hex 0005706F6E6700` setup on
  native and compatibility fixture endpoints. The setup write is normal UTF-8
  with `bytes_written=decoded_bytes=28`; setup output is not normalized;
- the target calls `read` with `from={"type":"now"}`, `encoding="hex"`,
  `timeout_ms=3000`, and `protocol: {"type":"cobs"}`. It does not use raw
  `rx_framing: cobs` (raw COBS framing), `max_frames`, `no_new_rx_timeout_ms`, sleep, flush, or a
  direct fixture API. Static independent plain-COBS wire is exactly
  `00 05 70 6f 6e 67 00`;
- the normalized result is normal anonymous hex with
  `bytes_read/bytes_observed/bytes_returned=7/0/0`, `timeout`, no
  match/truncation/drop/error, one `cobs` frame at index 0 with hex payload
  `70 6f 6e 67`, and raw parser `{"parser":"raw"}`. Positions are
  `52/59/0/0/0/59`;
 - broader zero-containing COBS TX/RX fixture proof remains stronger semantic
  coverage. Batch 11 does not claim full COBS protocol parity. Its historical
  checkpoint was 16 compared, 14 baseline-and-stronger, 3 retired, and 16 pending
  (`16/14/3/16`) with 30 covered rows.

Batch 12 report uses schema ID
`serial-mcp.native-sim-differential.at-parser-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/at-parser-batch.json`:

- `native_read_at_parser_parses_pong` uses standard anonymous public `open` at
  115200 with `profile_mode: "none"`, public boot-banner literal-match `read`,
  public `transact("arm_cmd 1000\r\n")` matching exact
  `arm_cmd delay=1000\r\n`, and public UTF-8 `write("ping\r\n")` with
  `bytes_written=decoded_bytes=6` on both endpoints;
- target `read` uses `from={"type":"now"}`, `encoding="utf8"`,
  `timeout_ms=3000`, explicit `rx_framing: line`, and explicit
  `rx_parser: at_command`. Setup calls are validated but excluded from the
  normalized outcome. The one-second public arm barrier replaces source-test
  sleep/flush behavior; no private fixture API, native-only branch,
  `max_frames`, `no_new_rx_timeout_ms`, or protocol preset is used;
- the normalized target is anonymous, normal UTF-8 `pong\r\n`, with
  `bytes_read/bytes_observed/bytes_returned=6/0/0`, `stop_reason=timeout`, no
  match, truncation, drop, or error, one index-0 `line` frame with payload
  `pong`, and parsed
  `{"parser":"at_command","response_type":"data","command":null,"status":null,"fields":["pong"]}`;
- positions are `52/58/0/0/0/58` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order. Target characterization remains at
  `target/native-sim-differential/at-parser-characterization.json` with SHA-256
  `52b573c8a71da8aa52fa6ce12ce81d63f5f30756839ae8db3a1e4e56a6424eb5`;
- this is direct explicit-parser historical evidence. Existing
  `at_command_connection_default_drives_stateful_transact_and_parser_quirk`
  `AtPeer` fixture proof remains stronger stateful AT behavior. Current status is
  historical at 17 compared, 14 baseline-and-stronger, 3 retired, and 15 pending
  (`17/14/3/15`) with 31 covered rows.

Batch 13 historical report uses schema ID
`serial-mcp.native-sim-differential.at-protocol-default-batch.v1` and writes one
direct Compared outcome to `target/native-sim-differential/at-protocol-default-batch.json`:

- `native_open_protocol_default_drives_write_and_read` opens anonymously at
  115200 with `profile_mode: "none"` and the protocol-only default
  `protocol: {"type":"at_command"}`. It omits `rx_framing`, so bare TX and RX
  inherit the open default;
- public setup uses default-framed `transact("arm_cmd 1000")` with stripped
  framed arm match `arm_cmd delay=1000`, then bare `ping` adds one CR (`4→5`);
- target bare read uses only `from={"type":"now"}`, `timeout_ms=3000`, and
  UTF-8 encoding. It returns `pong\r\n`,
  `bytes_read/bytes_observed/bytes_returned=6/0/0`, no match, truncation, drop,
  or error, one AT-parsed UTF-8 `line` frame with fields `["pong"]`, and
  positions `52/58/0/0/0/58`;
- retained characterization is
  `target/native-sim-differential/at-protocol-default-characterization.json`
  with SHA-256
  `cce2c8a47d3d23eedfb857b5701428937174ab066bd6b64ce20e544776b68775`;
- historical registry is 18 compared, 14 baseline-and-stronger, 3 retired, and
  14 pending (`18/14/3/14`) with 32 covered rows. The stronger `AtPeer` proof
  remains required.

Batch 14 report uses schema ID
`serial-mcp.native-sim-differential.json-parser-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/json-parser-batch.json`:

- `native_read_json_parser_decodes_jsonout` uses standard anonymous public `open`
  at 115200 with `profile_mode: "none"`, public boot-banner literal-match
  `read`, public `transact("arm_cmd 1000\r\n")` matching exact
  `arm_cmd delay=1000\r\n`, and public UTF-8 `write("jsonout\r\n")` with
  `bytes_written=decoded_bytes=9`;
- target `read` uses `from={"type":"now"}`, UTF-8, 3000 ms, explicit line
  framing, and explicit `json_lines` parsing. The static three JSON object
  response is 140 bytes;
- the normalized target has `bytes_read/bytes_observed/bytes_returned=140/0/0`,
  stop reason `timeout`, no match, truncation, drops, or error, three ordered
  parsed objects, and positions `52/192/0/0/0/192`;
- retained characterization is
  `target/native-sim-differential/json-parser-characterization.json` with
  SHA-256
  `f51b5d77bac3904d214e2ea76794cf1d10f4d5aa8849224e750af30a8e9e3a06`;
- existing `json_lines_preset_writes_line_and_preserves_object_only_parser_behavior`
  fixture proof remains stronger. Batch 14 historical registry is
  `19/14/3/13` with 33 covered rows.

Batch 15 report uses schema ID
`serial-mcp.native-sim-differential.ndjson-preset-batch.v1` and writes two direct
Compared outcomes to `target/native-sim-differential/ndjson-preset-batch.json`:

- `native_read_ndjson_preset_decodes_json_frames` and
  `native_read_ndjson_preset_skips_empty_lines` use standard anonymous public
  `open` at 115200 with `profile_mode: "none"`, public boot-banner literal-match
  `read`, public `transact("arm_cmd 1000\r\n")` readiness, and exact static
  UTF-8 `sendraw` writes of 48 and 74 bytes;
- target reads use `from={"type":"now"}`, UTF-8, 3000 ms, and only
  `protocol: {"type":"ndjson"}`. Auto line framing, `skip_empty:true`, and the
  JSON parser come from the preset; no explicit framing/parser, sleep, or flush
  is used;
- exact static payloads are `{"a":1}\n\n{"b":2}\n` and
  `{"a":1}\n\n\n{"b":2}\n   \n{"c":3}\n`. Outcomes are
  `17/0/0` with positions `52/69/0/0/0/69` and `30/0/0` with positions
  `52/82/0/0/0/82`; both stop by `timeout`, preserve raw UTF-8 payload, emit
  ordered parsed record frames only, and have no match, truncation, drops, or
  error. Blank and whitespace-only lines emit no frames;
- retained characterization is
  `target/native-sim-differential/ndjson-characterization.json` with SHA-256
  `10c4273edcd2a53a0b5ff0d1ab310d319be8145db2f42aa153d5207c1b372ec3`;
- existing `ndjson_preset_parses_records_and_skips_blank_whitespace_lines`
  fixture proof remains stronger. Batch 15 historical checkpoint is
  `21/14/3/11` with 35 covered rows; Phase F remains blocked.

Batch 16 report uses schema ID
`serial-mcp.native-sim-differential.nmea0183-preset-batch.v1` and writes one
direct Compared outcome to `target/native-sim-differential/nmea0183-preset-batch.json`:

- `native_read_nmea0183_preset_decodes_parsed_frame` uses standard anonymous
  public `open` at 115200 with `profile_mode: "none"`, boot-banner literal-match
  `read`, public `transact("arm_cmd 1000\r\n")` readiness, and exact static UTF-8
  `sendraw` write of 148 bytes:
  `sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A\r\n`;
- target `read` uses `from={"type":"now"}`, UTF-8, 3000 ms, and only
  `protocol: {"type":"nmea0183"}`. The static wire is the exact 67-byte valid
  GGA sentence `$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n`;
- exact normalized output is `67/0/0`, `timeout`, no match/truncation/drop/error,
  one UTF-8 `start_end` frame with parsed `talker_id="GP"`,
  `sentence_type="GGA"`, ordered fields
  `["123519","4807.038","N","01131.000","E","1","08","0.9","545.4","M","46.9","M","",""]`,
  `checksum_valid:true`, and positions `52/119/0/0/0/119`;
- the retained native characterization is
  `target/native-sim-differential/nmea-characterization.json` with SHA-256
  `513d4906b285ef35a2b82ab085968de96f51576acc636244f0eb3a44868f1578`;
- existing `nmea0183_preset_parses_valid_independently_checksummed_sentence`
  fixture proof remains stronger. Historical Batch 16 registry was `22/14/3/10`
  with 36 covered rows; Batch 17 is historical at `23/14/3/9` with 37 covered
  rows; Batch 18 is historical at `24/14/3/8` with 38 covered rows; Phase F
  blocked.

Historical Batch 18 close-while-read report uses schema ID
`serial-mcp.native-sim-differential.close-while-read-batch.v1` and writes one
direct Compared outcome to `target/native-sim-differential/close-while-read-batch.json`:

- `native_close_while_read_active_returns_normal_result` uses standard anonymous
  public `open` at 115200 with `profile_mode: "none"`, boot-banner literal-match
  `read`, public `transact("arm_cmd 1000\r\n")` readiness, and marker command
  `sendraw hex 524541442D52454144592D4D41524B45520D0A\r\n`;
- a secondary modern client starts unmatched `from={"type":"now"}` read with
  UTF-8 and 3000 ms; primary polls normalized `get_status.rx_bytes` until the
  marker increases RX by 19 and proves pending read remains unfinished, then
  close returns normal anonymous `source="disabled"` and transient
  `close_snapshot` persistence;
- exact pending read is `READ-READY-MARKER\r\n` with `19/19/19`,
  `connection_closed`, unmatched, no truncation/frames/drops/error, and
  positions `52/71/0/0/0/71`;
- report SHA-256 is
  `fef0499d6504635c104fe3d125fe572c49e2ef4bec69a5b39c32bdeb50361a09`; retained
  characterization is
  `target/native-sim-differential/close-while-read-characterization.json` with
  SHA-256 `06a7adb2b4f1c1c6f8b3c8fd507ba9e5004df6dd7a9f4ef6b64c4fff87c3f69b`;
- existing `close_interrupts_readiness_proven_live_read_with_connection_closed`
  fixture proof remains stronger. Phase F remains blocked.

Historical Batch 19 reopen/fresh-output report uses schema ID
`serial-mcp.native-sim-differential.reopen-fresh-output-batch.v1` and writes one
baseline-and-stronger outcome to
`target/native-sim-differential/reopen-fresh-output-batch.json`:

- the first standard anonymous open synchronizes the one process-start boot
  banner, writes `ping\r\n`, and reads exact `pong\r\n` at positions
  `32/38/0/0/0/38`; the normal close retains disabled/transient profile checks;
- the same endpoint reopens with a distinct raw connection ID. A status-only
  probe requires `rx_bytes=0` before the second command and is not normalized;
  the pending `from={"type":"now"}` read uses bounded 100 ms baseline admission
  before an independent modern client writes `ping\r\n`;
- second read is exact `pong\r\n`, `6/6/6`, `match_found`, index 0, no
  frames/drops/error/truncation, and positions `0/6/0/0/0/6`;
- report SHA-256 is
  `7c9d9071156739a0a2bc81a9ef2adba48a40132ba6bd1208b99e8e04a847d02e`; the
  existing `reopen_same_path_returns_distinct_id_and_only_fresh_generation`
  fixture proof remains stronger. Historical registry is `24/15/3/7` with 39
  covered rows; Phase F remains blocked.

Batch 20 flush-during-armed-delay report uses schema ID
`serial-mcp.native-sim-differential.flush-during-arm-delay-batch.v1` and writes
one baseline-and-stronger outcome to
`target/native-sim-differential/flush-during-arm-delay-batch.json`:

- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`; boot synchronization and exact public
  `transact("arm_cmd 1000\r\n")` acknowledgement are validated but setup output
  is excluded from normalized observations;
- public `write("ping\r\n")` is normal anonymous UTF-8 with
  `bytes_written=decoded_bytes=6`; the 100 ms post-write pause is the bounded
  baseline admission window only, not a peer-acceptance proof;
- public `flush(target="both")` is normal anonymous with exact target `both`;
  positioned `from={"type":"now"}` UTF-8 literal `pong\r\n` read returns exact
  `pong\r\n`, `6/6/6`, `match_found`, index 0, no truncation/frames/drops/error,
  and positions `52/58/0/0/52/58` in
  `from_offset/next_offset/bytes_lost/buffered_remaining/start_offset/end_offset`
  order. `RxRing::clear` clamps retained `start_offset` to live edge 52;
- existing `flush_after_command_acceptance_does_not_cancel_delayed_response`
  fixture proof remains stronger. Report SHA-256 is
  `4808262720b252c53f95e88a56ea1d8565b238251884aed364c0e43d0b07f500` after two
  matching runs. Historical registry is `24/16/3/6` with 40 covered rows; Phase F
  blocked.

Historical Batch 21 input-flush backlog report uses schema ID
`serial-mcp.native-sim-differential.input-flush-batch.v1` and writes one direct
Compared outcome to `target/native-sim-differential/input-flush-batch.json`:

- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`; boot synchronization is retained as the first two
  normalized observations;
- old marker command `sendraw hex 4F4C442D4D41524B45520D0A\r\n` writes 38 bytes,
  then status-only polling proves `rx_bytes >= 44`; `get_status` is not normalized;
- public `flush(target="input")` is retained as an anonymous `FlushObservation`
  with exact target `input`; new marker command
  `sendraw hex 4E45572D4D41524B45520D0A\r\n` writes 38 bytes, then status-only
  polling proves `rx_bytes >= 56`;
- positioned `from={"type":"buffer_start"}` UTF-8 literal `NEW-MARKER\r\n`
  read returns only `NEW-MARKER\r\n`, `12/12/12`, `match_found`, index 0, no
  truncation/frames/drops/error, and positions `44/56/0/0/44/56`; the stronger
  `flush_input_discards_known_old_marker_and_keeps_new_marker` fixture proof
  remains independent;
- report SHA-256 is
  `ff95074207f7de216780ede42a6d21583e4e88e5c5e5c6f81af4b588fa5dcfd8` after two
  matching runs. Historical registry is `25/16/3/5` with 41 covered rows; Phase F
  blocked.

Batch 22 bootloader-touch process-exit report uses schema ID
`serial-mcp.native-sim-differential.bootloader-touch-exit-batch.v1` and writes one
direct Compared outcome to
`target/native-sim-differential/bootloader-touch-exit-batch.json`:

- native endpoint is existing firmware PTY; fixture endpoint is a real dedicated
  small Rust child process with a raw PTY, not `FixtureExit::Crashed`. The child
  emits exact `serial-mcp test firmware ready\r\n` before `PTY_PATH` is published;
- both endpoints use standard anonymous public `open` at 115200 with
  `profile_mode: "none"`; matching boot-banner read remains the second
  normalized observation. Public UTF-8 `write("touch\r\n")` is normal anonymous
  with exact `bytes_written=decoded_bytes=7`;
- both endpoints exit exactly 42, retained as typed
  `{"kind":"process_exit","exit_code":42}`. Terminal
  `touch exit(42)\r\n` response delivery is not read or claimed, and no public
  `close` follows peer exit;
- existing `touch_write_causes_small_rust_child_peer_to_exit_42` remains
  independent stronger fixture proof. Report SHA-256 is
  `91befb4e3af3edd65c70c58208be03c09c8c29aed04f9b432e18c1d5becd4d9c` after two
  matching runs. Historical registry is `26/16/3/4` with 42 covered rows; Phase F
  blocked.

Batch 3 rows are `native_read_delimiter_framing_decodes`,
`native_read_length_prefixed_framing_decodes`,
`native_read_start_end_framing_decodes`,
`native_write_tx_framing_modes_observed_via_trace`, and
`native_read_explicit_line_endings_split_correctly`.

Batch 4 rows are `native_read_match_on_spam_complete` and
`native_read_buffer_budget_stops_under_flood`; required stronger proofs are
`finite_flood_matcher_reaches_unique_completion_marker` and
`live_buffer_budget_caps_finite_flood_with_exact_stop_metadata`.

Batch 5 rows are `native_framing_reports_single_split_command`,
`native_trace_reports_exact_split_byte_sequence`, and
`native_partial_line_buffered_then_completed`; required stronger proof is
`split_writes_preserve_one_command_and_exact_wire_order` for each row. Phase F
remains blocked.

Batch 6 row is `native_ack_command_provides_pre_execution_ack`; it is direct
Compared evidence with no baseline proof, and the existing ACK PTY proof remains
required. Batch 7 row is `native_flush_output_after_full_delivery_is_safe`; it is
also direct Compared evidence with no baseline proof and retains the stronger
output-only Rust-PTY proof. Batch 8 row is
`native_read_slip_decodes_frame`; it is direct Compared evidence with no
baseline proof and retains the stronger raw/protocol SLIP Rust-PTY proof. Phase
F remains blocked. Batch 9 row is
`native_read_slip_malformed_escape_returns_partial_result`; it is
  baseline-and-stronger evidence with the exact
  `SLIP_MALFORMED_BASELINE_PROOFS` binding and retains the stronger
  valid-frame-before-error plus recovery proof. Phase F remains blocked. Batch
  10 row is `native_read_slip_recovers_after_error_on_next_call`; it is direct
  Compared evidence with no baseline proof and retains the stronger
  `protocol: slip` valid-frame-before-error plus recovery proof. Phase F remains
  blocked.

## Final Deletion PR

Expected removals:

- `firmware/`;
- `tests/common/firmware.rs`;
- native suite wrappers/files after scenario move;
- `scripts/install-nrfutil-ci.sh` and test;
- NCS CI job and release `needs` entry;
- `nix-nrf-dev` flake input/lock graph and firmware helper shell;
- xtask firmware path/build logic;
- firmware clangd route and active native/NCS docs.

Expected retained items:

- `nix` PTY dev dependency;
- controlled backend and PTY physical-limit docs;
- historical changelog mentions;
- ordinary serial build dependencies;
- proposed ADR updated to Accepted only after acceptance evidence.

## Verification Gates

Smallest regression:

```bash
cargo test --locked --test device_fixture public_mcp_ping_hold_disconnect_replace_and_reconnect \
  -- --test-threads=1
```

Migration gate:

```bash
cargo test --locked --test device_fixture -- --test-threads=1
cargo test --locked --test device_command_parity -- --test-threads=1
cargo test --locked --test device_framing_parity -- --test-threads=1
cargo test --locked --test device_protocol_parity -- --test-threads=1
cargo test --locked --test device_parity_repeat phase_e_public_boundary_repeat_gate \
  -- --ignored --test-threads=1
cargo test --locked --test doc_drift
cargo test --locked
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
nix flake check
```

Final clean checkout must pass without NCS, Zephyr, west, or nrfutil on PATH.

## Research Artifacts

- [test traceability and coupling](native-sim-test-traceability.md)
- [candidate survey](native-sim-virtual-serial-candidate-survey.md)
- [prototype results](native-sim-boundary-prototype-results.md)
- [protocol worksheets](native-sim-protocol-peer-worksheets.md)
- [proposed ADR](../adr/replace-native-sim-with-rust-pty-device-fixture.md)
- [resumable progress](native-sim-replacement-research-progress.md)
