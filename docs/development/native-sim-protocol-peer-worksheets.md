# `native_sim` Replacement Protocol Peer Worksheets

**Status:** Durable fixture-backed protocol parity baseline complete on
2026-08-13. These worksheets define remaining simulator behavior and
independent-oracle strategy for every shipped preset, framing mode, and parser.
Native suites remain required differential coverage; this work does not remove
firmware, NCS, CI, Nix, or xtask dependencies.

Durable independent peer/oracle code lives in
`tests/common/device_fixture/protocol_peers.rs`; seven helper unit tests in
`tests/device_protocol_parity.rs` cover
AT state/error/URC/no-response, SLIP escapes/malformed vector, COBS
zero/max-block/fragment/error, JSON/NDJSON values/malformed recovery, NMEA
standard/AIS/proprietary/bad/missing checksums, mutable/broadcast/bad-LRC Modbus
ASCII, and generic/shell/raw vectors. Public real-PTY coverage lives in
`tests/device_protocol_parity.rs` and covers every shipped preset: AT default
and parser behavior, SLIP TX/happy/partial-error/recovery, JSON Lines TX and
object-only parsing, COBS zero-byte TX/RX, NDJSON whitespace skipping, NMEA
valid parsed checksum, and stateful Modbus ASCII LRC read/write. Historical
prototype paths remain only in prior research history.

## Common Peer Contract

Each peer runs above reusable PTY transport and scripted fault scheduler. Every
scenario states:

- peer state and transition;
- independently sourced valid and invalid bytes;
- explicit fragmentation points;
- MCP operation and expected visible result;
- silence, disconnect, and recovery behavior;
- oracle source independent of production `src/framing/` code.

Common actions:

```text
Expect(bytes)
Emit(bytes)
EmitChunks { chunks, gap }
Delay(duration)
Silence(duration)
Hold
Release
Saturate { bytes, chunk_size, policy }
ClosePeer
Crash(exit_code)
ReplaceEndpoint
```

Fixture time uses explicit actions and readiness barriers. Product timeout
tests keep bounded real wall time; fixture unit tests may use paused Tokio time
only where no OS PTY I/O is awaited.

## Dependency Recommendation

Use exact dev pins only after implementation review:

```toml
slip-codec = "=0.4.0" # MIT
cobs = "=0.5.1"       # MIT OR Apache-2.0
rmodbus = "=0.12.2"   # Apache-2.0
```

Verified on Rust 1.97.1 during research. None needs native libraries, build
scripts, privileged setup, or runtime services. Run full post-lockfile advisory
and license checks before merging dependencies.

## `at_command` Preset and Parser

### Peer role and state

Local DCE/modem state machine, because surveyed Rust AT crates (`atat`,
`at-commands`) model host/DTE behavior rather than modem peer behavior.

State:

- `echo: bool`, changed by `ATE0`/`ATE1`;
- `verbose_errors: bool`, changed by `AT+CMEE=0/1/2`;
- `registered: bool` and signal value for evolving query responses;
- pending URC queue with placement before, between, or after command response;
- command counter and optional one-shot delay/no-response fault.

### Vectors and scenarios

| Case | Peer bytes/state | Fragmentation/fault | MCP-visible proof |
|---|---|---|---|
| normal query | optional echo, `+CSQ: 15,99\r\nOK\r\n` | split after `+`, comma, CR, and before final `OK` | `transact(protocol=at_command)` frames classify data/response and final result in order |
| echo transition | `ATE0` disables later echo; `ATE1` restores | split command host writes | state survives request and output changes observably |
| error | `ERROR`, `+CME ERROR: 10`, `+CMS ERROR: 515` | each final result split across chunks | parser emits intended error classification and code/text fields |
| URC interleave | `+CEREG: 1` before, between data and `OK`, and after response | cadence and arbitrary split | ordered frames preserve URCs without losing command response |
| delayed/no response | accepted command, then delay or silence | bounded silence | `transact` stops by match/silence/wall timeout as configured |
| malformed/recovery | bad `+` line or binary noise, then valid `OK` command | noise before valid line | malformed data remains observable; next command succeeds |

Oracle: ITU-T V.250 command/result grammar plus vendor CME/CMS/URC examples;
small local parser/generator written independently of product parser.

Product question to characterize first: current parser branch order appears to
classify `+CME ERROR` and `+CMS ERROR` through generic `+COMMAND:` handling.
Fixture must expose this behavior; do not encode desired behavior as if shipped.

## SLIP Preset and Framing

### Peer role and state

Packet peer with sequence counter. `slip-codec 0.4.0` supplies independent
valid encoding/decoding. Local action script injects bytes the helper rejects or
normalizes.

### Vectors and scenarios

- RFC 1055 golden: payload `[0xC0, 0xDB]` encodes inside delimiters as
  `C0 DB DC DB DD C0`.
- normal request/response packet through `write` and `read`/`transact`;
- fragmented packet at every byte boundary, especially after ESC;
- two back-to-back frames in one write;
- leading/trailing END and noise policy;
- invalid `DB 41` escape after one valid frame; expected public
  `stop_reason="framing_error"`, partial valid frame retained, raw malformed
  bytes losslessly represented (hex fallback under UTF-8);
- next read receives valid frame, proving per-call recovery;
- peer silence and close while incomplete frame remains buffered.

Expected frame type `slip`, parser `raw`, independently encoded payload exact.

## JSON Lines Preset and Parser

### Peer role and state

Local line peer using existing `serde_json` only as independent JSON syntax
implementation. Sensor state increments sequence and values after each query.

### Vectors and scenarios

| Case | Data | Expected behavior |
|---|---|---|
| object | `{"sensor":"temp","seq":1}` | parsed object frame |
| array | `[1,2,3]` | characterize current product behavior; JSON Lines permits it |
| scalar | `42`, `true`, `"text"`, `null` | characterize current product behavior |
| fragmented | split UTF-8 code point, JSON token, CRLF, and LF | no premature frame; exact value after terminator |
| multiple | object lines in one burst | ordered frames and `max_frames` stop |
| empty | empty line | not skipped by `json_lines`; emitted raw/parser outcome per current contract |
| malformed recovery | `{bad}\n{"ok":true}\n` | malformed record remains observable, next record parses |
| evolving state | repeated query increments `seq` | proves peer is stateful, not static playback |

Operations: TX line framing through `write`/`transact`; RX through `read` and
`transact`; `capture_boot` only when JSON records represent boot telemetry.

Product question: parser currently structures only JSON objects. Arrays and
scalars need characterization and separate product decision.

## COBS Preset and Framing

### Peer role and state

Packet peer with sequence state. `cobs 0.5.1` supplies independent plain-COBS
encoding and strict streaming decode. Use Cheshire/Baker paper examples as
static cross-checks.

### Required cases

- zero-containing payload, including leading/trailing/consecutive zeros;
- empty payload/frame;
- 254 nonzero bytes requiring `0xFF` maximum code block;
- fragmented input at code byte, final data byte, and zero delimiter;
- back-to-back packets;
- early zero, truncated block, incomplete packet then silence;
- malformed packet followed by valid packet to define recovery;
- exact TX bytes observed by peer and RX payload exposed through raw parser.

Expected frame type `cobs`; arbitrary binary under requested UTF-8 falls back
to exact lowercase spaced hex. Helper's strict malformed semantics are an
oracle input, not permission to redefine current production behavior. Current
decoder's `CobsInvalidCode` reachability needs characterization.

## NDJSON Preset

### Peer role and state

Same stateful JSON peer, but explicit NDJSON record policy.

### Required cases

- object/array/scalar characterization from JSON worksheet;
- empty and whitespace-only lines skipped and excluded from frame count;
- CRLF and LF, with delimiters split across writes;
- cadence stream with increasing `seq` and bounded `max_frames` stop;
- malformed record followed by valid record;
- multiple blank lines between valid records;
- silence and peer close between partial JSON and newline.

Oracle: NDJSON 1.0 specification plus RFC 8259. Expected parsed frame count must
exclude blank/whitespace-only records because shipped preset sets
`skip_empty=true`.

## NMEA-0183 Preset and Parser

### Peer role and state

Local marine instrument generator with changing position, heading, depth, and
sequence. Generate checksum through a local spec-derived XOR function that does
not call product checksum code. Use committed cite-compatible golden strings
from GPSD NMEA/AIVDM references; do not commit copyrighted standard text.

### Required cases

| Case | Example role | Expected MCP fields |
|---|---|---|
| standard | `$GPGGA,...*XX\r\n` with changing fix | `talker_id="GP"`, `sentence_type="GGA"`, fields, checksum valid |
| AIS | `!AIVDM,...*XX\r\n` | alternate `!` marker, `AI`/`VDM` classification, valid checksum |
| proprietary | `$PTEST,...*XX\r\n` | shipped convention `talker_id="P"`, remainder as sentence type |
| bad checksum | valid body with altered `XX` | dropped/count increment under preset validation; following valid sentence parses |
| missing checksum | sentence without `*XX` | characterize shipped acceptance with `checksum_valid=null` |
| noise | bytes before `$`/`!` | start-marker resync; noise policy observable in raw data |
| fragmentation/burst | split marker, `*`, checksum digits, CRLF; then burst | one exact sentence per frame, order preserved |

Cover NMEA TX auto-checksum through `write`, peer verifies body/checksum; RX via
`read`/`transact`; cadence/boot sentences through `capture_boot` where useful.

## Modbus ASCII Preset and Parser

### Peer role and state

`rmodbus 0.12.2` protocol/server core with mutable coils and holding/input
registers. Local ASCII transport wrapper owns colon/CRLF framing, hex conversion,
fragmentation, silence, and malformed LRC injection. `tokio-modbus` is rejected:
its shipped transports are RTU/TCP, not ASCII.

### Required cases

- read coils, discrete inputs, holding registers, and input registers;
- write single/multiple coil and register, followed by read proving mutation;
- exception response for illegal function/address/value;
- broadcast request to address 0 mutates state and emits no response where
  applicable;
- correct LRC request/response, bad LRC with no response/drop behavior, then
  valid request recovery;
- split colon, each hex pair, LRC, CR, and LF;
- back-to-back request/response frames;
- silence and disconnect after partial frame.

Expected parser fields include address, function code, decoded data, and
`lrc_valid=true` for valid frames. TX peer independently checks ASCII hex and
LRC. Use official Modbus serial specification and independent published vectors
such as existing `:010300000001FB\r\n`.

## Generic Framing Modes

Local spec-derived builders are preferable to new dependencies because these
are product configuration contracts rather than full protocols.

### Line

- RX auto LF/CRLF and bare-CR promotion;
- explicit LF preserves preceding CR;
- explicit CR and CRLF;
- terminator split across writes;
- include/exclude terminators;
- empty/whitespace skip and `max_frames`;
- pending bare CR resolved by silence/stream end.

### Delimiter

- one-byte and multi-byte delimiters;
- delimiter split at every boundary;
- adjacent delimiters, empty frames, skip-empty;
- payload prefix/suffix and trailing partial delimiter;
- exact TX delimiter append.

### Length-prefixed

- prefix sizes 1, 2, and 4;
- big and little endian;
- zero-length and boundary payload sizes;
- prefix and payload split at every boundary;
- multiple frames, truncated prefix, declared length followed by silence;
- `initial_offset` behavior;
- exact TX prefix bytes independently built with integer byte-order APIs.

### Start/end

- alternate start marker list and earliest-marker selection;
- marker split across writes;
- pre-start noise;
- payload containing partial markers;
- include/exclude markers;
- missing end then silence; next complete frame recovery;
- exact TX uses first start marker.

For every mode, run public `write`, `read`, and `transact`; use
`capture_boot` for fragmented boot/banner framing cases.

## Shell Prompt Parser

Local shell peer state:

- prompt mode (`$`, `#`, `>`, or custom regex);
- command counter and current directory/user string;
- optional command echo, output, and prompt delay.

Cases:

- common prompts and custom prompt;
- prompt split across chunks and prompt after multiline output;
- prompt-like bytes in body that must not terminate early;
- mode transition from `$` to `#`;
- delayed/no prompt and disconnect;
- invalid custom regex remains configuration/tool error before stream work.

Expected parsed frame identifies prompt versus output according to shipped
parser contract.

## Raw Parser and Encoding

Local binary peer emits every byte `0x00..=0xFF`, fragmented and back-to-back.
Cases cover UTF-8 success, malformed UTF-8 exact hex fallback, explicit hex and
base64, empty payload, max-buffer truncation, silence, peer close, and recovery.
No helper decoder may transform raw bytes.

## Protocol Coverage Metadata and Drift Guard

Implemented coverage registry lives in `tests/device_protocol_parity.rs`, keyed
by independently listed exact wire names. It guards all seven shipped names:
`at_command`, `slip`, `json_lines`, `cobs`, `ndjson`, `nmea0183`, and
`modbus_ascii`. Each row names its public fixture-backed case. Future expansion
can add richer metadata beside that registry. Required fields remain:

```text
preset
peer
oracle
normal_test
fragmented_test
stateful_test
malformed_test
tx_test
rx_test
```

Doc-drift test should compare:

1. exact shipped `ProtocolPreset` enum variants/preset expansion registry;
2. exact `shipped` rows in `protocol-matrix.md` that map to presets;
3. exact simulator coverage metadata keys.

Failure on missing, duplicate, or stale key. Generic parser/framing metadata
gets parallel exact enum coverage checks for `RxFramingMode`, `TxFramingMode`,
and `ParserType`. This guard lands with simulator registry, not as speculative
test before implementation.

## Acceptance Matrix

| Surface | Normal | Fragmented | Stateful | Malformed/fault | Independent oracle |
|---|---:|---:|---:|---:|---|
| AT | yes | yes | echo/errors/registration/URCs | error/no response/noise/recovery | V.250 + local DCE |
| SLIP | yes | every escape boundary | packet sequence | invalid escape/noise/recovery | RFC 1055 + `slip-codec` |
| JSON lines | values | UTF-8/token/newline | sensor sequence | malformed then valid | RFC 8259 + serde_json |
| COBS | zero/max blocks | every code boundary | packet sequence | early/truncated/recovery | paper + `cobs` |
| NDJSON | values/blanks | whitespace/newline | cadence | malformed then valid | NDJSON spec + serde_json |
| NMEA | standard/AIS/proprietary | marker/checksum/CRLF | moving instrument | bad/missing/noise/recovery | GPSD vectors + local XOR |
| Modbus ASCII | reads/writes | colon/hex/LRC/CRLF | mutable data model | exception/bad LRC/broadcast | official vectors + `rmodbus` |
| generic framing | all modes | every delimiter/prefix/marker boundary | frame sequence | partial/noise/silence/recovery | local byte builders |
| shell/raw | common/custom/binary | prompt and arbitrary chunks | prompt mode/counter | timeout/disconnect/fallback | local peer/static bytes |

No required row remains unspecified.
