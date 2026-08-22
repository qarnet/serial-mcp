# `native_sim` Replacement Recommendation

**Status:** Approved on 2026-08-13. Phases A-D fixture/command/protocol parity
and Phase E required replacement/repeat gate complete. Batch 1 executable
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
       NDJSON-preset rows as current evidence. Each batch has separate report
       schema/output.
   Global registry status is **21 compared, 14 baseline-and-stronger, 3 retired, and
   11 pending** rows (`21/14/3/11`). Pending-read baselines use fixed 100 ms baseline-only delay
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
      NDJSON-preset evidence with 35 covered rows. Native differential oracle and
      Phase F deletion remain blocked.

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
partial evidence; Batch 14 is historical partial evidence; Batch 15 is current
partial evidence; Phase F blocked.

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

Batch 15 is current direct Compared evidence for
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
remains stronger. Current registry is `21/14/3/11` with 35 covered rows; Phase F
blocked.

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

- **Complete:** replacement targets are explicit required CI evidence; native
  still runs as temporary differential oracle.
- **Complete:** Linux x86_64 runs timing/flood/hold/flush/disconnect/reconnect
  public lifecycle 100 times with fixed seed `0x50484153455f4545`.
- **Complete:** macOS arm64 has compact fixture/command/framing real-PTY run;
  Windows remains explicit compile/controlled coverage.
- Still required before Phase F: agreed full differential outcome comparison and
  resolution of every mismatch. Batch 1 through Batch 15 cover 35 rows; 11
  registry rows remain pending before full parity evidence exists.

Rollback: disable replacement required status; native unchanged.

### Phase F — NCS deletion

- Delete firmware tree, native harness/wrappers, nrfutil scripts/tests.
- Remove NCS job/cache/install/disk cleanup and release dependency.
- Remove `nix-nrf-dev`, multilib firmware shell, lock nodes, firmware LSP.
- Simplify xtask to normal Rust assets/tests.
- Update evaluator refs, README, AGENTS, active docs, changelog release entry,
  and doc-drift guards.
- Clean-checkout full gate with no NCS/west/nrfutil present.

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
  fixture proof remains stronger. Current registry is `21/14/3/11` with 35
  covered rows; Phase F blocked.

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
