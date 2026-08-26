# Protocol guide

serial-mcp's framing, parser, and preset system turns a raw serial byte stream
into structured frames for matching and inspection. This guide covers the seven
built-in protocol presets, the four-layer precedence model for choosing framing
and parsers, and checksum behavior for NMEA-0183 and Modbus ASCII.

For a quick overview, start with [Presets](#presets) and
[Precedence](#precedence).

## Framing and parsers at a glance

RX fields `rx_framing`, `rx_parser`, and `protocol` apply to `read`, the
`transact` read half, and `capture_boot`. TX fields `tx_framing` and `protocol`
apply to `write` and the `transact` write half. The fields are siblings.

- `rx_framing` and `tx_framing` split the byte stream into frames. RX modes are
  `line`, `delimiter`, `length_prefixed`, `start_end`, `slip`, and `cobs`. TX
  modes mirror these modes except for the `auto` line ending and
  `max_frames`, `include_terminators`, and `parser`.
- `rx_parser` interprets each frame's bytes. Parsers are `at_command`,
  `json_lines`, `shell_prompt`, `raw`, `nmea`, and `modbus_ascii`. `rx_parser` is
  a sibling of `rx_framing`, not a nested field. `write` has no parser because
  TX frames are outbound payloads.
- `protocol` is a named preset that fills defaults for the fields above. It
  replaces the three individual settings when their bundled defaults are
  sufficient.

`match` is independent of framing and can be combined with any of these fields.
In framed mode, the matcher runs once per frame and resets its window between
frames. In raw mode, it runs across the accumulated byte stream.

## Presets

A preset is a tag object such as `{"type": "<name>"}`. The seven available
presets are listed below. The sections that follow show `write` and `read`
examples and the decoded frame shape.

| Preset | TX framing | RX framing | RX parser | `validate` default |
|---|---|---|---|---|
| `at_command` | line (`\r`) | line (`auto`) | `at_command` | false |
| `slip` | SLIP | SLIP | `raw` | false |
| `json_lines` | line (`\n`) | line (`auto`) | `json_lines` | false |
| `cobs` | COBS | COBS | `raw` | false |
| `ndjson` | line (`\n`) | line (`auto`, `skip_empty`) | `json_lines` | false |
| `nmea0183` | start_end (`$`…`\r\n`) | start_end (`$`/`!`…`\r\n`) | `nmea` | true |
| `modbus_ascii` | start_end (`:`…`\r\n`) | start_end (`:`…`\r\n`) | `modbus_ascii` | true |

`nmea0183` and `modbus_ascii` default to `validate` enabled because their
specifications require checksums. The other presets have no checksum to
validate, so `validate` has no effect for them.

### `at_command` modem protocol

TX appends `\r`. RX splits on line endings. It detects LF, CRLF, and bare-CR
endings automatically. RX frames are parsed as AT command responses or URCs.
URCs are Unsolicited Result Codes.

```jsonc
// Send AT+CGMI and read the model name
{ "connection_id": "cell0",
  "write": { "data": "AT+CGMI", "protocol": {"type": "at_command"} } }
{ "connection_id": "cell0",
  "read":  { "protocol": {"type": "at_command"}, "match": {"pattern": "OK"} } }
```

Decoded frame `parsed` shape:

```jsonc
{ "parser": "at_command",
  "response_type": "data",        // "data" | "urc" | "result_code" | "error"
  "command": "+CGMI",             // omitted for URCs
  "status": "OK",                 // "OK" | "ERROR" | "CME ERROR: <n>" | …
  "fields": ["Quectel", "EC25"] } // response lines between echo and status
```

### `slip` framing

SLIP wraps a payload between `END` bytes, 0xC0. It escapes `END` and `ESC`
occurrences inside the payload. It returns raw payload with no parser. It is
used for IP-style or binary packet streams. The delimiter must not appear in
the data.

```jsonc
{ "connection_id": "radio0",
  "write": { "data": "<hex payload>", "encoding": "hex",
             "protocol": {"type": "slip"} } }
{ "connection_id": "radio0",
  "read":  { "protocol": {"type": "slip"} } }
```

A malformed SLIP escape, 0xDB followed by anything other than 0xDC or 0xDD, is
a stream-fatal decode error. The call or read pipeline stops with
`stop_reason: "framing_error"` and returns frames decoded before the error as a
partial result. See [Checksum and error behavior](#checksum-and-error-behavior).

### `json_lines` and `ndjson`

Both presets use line framing and the `json_lines` parser. The `ndjson` preset
sets `skip_empty` to true. It skips blank or whitespace-only lines as required
by the NDJSON convention. `json_lines` emits those lines.

```jsonc
{ "connection_id": "telem0",
  "read": { "protocol": {"type": "ndjson"},
            "match": {"pattern": "\"temp\"", "config": {"mode": "literal_substring"}} } }
```

The decoded `parsed` value is the JSON value itself, inlined next to the
`"parser": "json"` tag:

```jsonc
{ "parser": "json", "sensor": "temp", "value": 25.5 }
```

### `cobs` framing

COBS encodes a payload so that 0x00 never appears inside it. It then delimits
frames with a bare 0x00. This is useful on noisy serial links where a delimiter
byte must be unambiguous. It returns raw payload with no parser. Like SLIP, a
COBS decode error is stream-fatal. It produces a partial result with
`stop_reason: "framing_error"`.

The canonical plain-COBS decoder accepts every code byte from 0x01 through 0xFF.
Decode errors are rare in practice. The error path remains for
forward-compatibility.

### `nmea0183` marine sentence protocol

Start/end framing uses `$` or `!` as start markers and `\r\n` as the end. The
NMEA parser splits each sentence into talker ID, sentence type, and
comma-separated fields. It validates the `*XX` XOR checksum. The preset
defaults to `validate` enabled.

```jsonc
{ "connection_id": "gps0",
  "read": { "protocol": {"type": "nmea0183"} }
}
```

Sample input `$GPGLL,3751.65,N,12226.54,W*7E\r\n` decodes to:

```jsonc
{ "parser": "nmea",
  "talker_id": "GP",
  "sentence_type": "GLL",
  "fields": ["3751.65", "N", "12226.54", "W"],
  "checksum_valid": true }
```

Proprietary sentences (`$P...`) use `talker_id: "P"` and the remaining text as
`sentence_type`. For example, `$PGRMM` becomes `P` and `GRMM`. This follows the
NMEA proprietary convention. AIS sentences start with `!` and use the same
start-marker set.

### `modbus_ascii` mode

Start/end framing uses `:` as the start marker and `\r\n` as the end. The
Modbus ASCII parser hex-decodes the body. It exposes the slave address, function
code, and data bytes. It validates the trailing LRC. The preset defaults to
`validate` enabled.

```jsonc
{ "connection_id": "plc0",
  "read": { "protocol": {"type": "modbus_ascii"} }
}
```

Sample input `:010300000001FB\r\n` represents a read of holding registers at
address 1, function 3, start 0, quantity 1, with LRC 0xFB. It decodes to:

```jsonc
{ "parser": "modbus_ascii",
  "address": 1,
  "function_code": 3,
  "data": [0, 0, 0, 1],
  "checksum_valid": true }
```

## Precedence

When a call provides more than one source for a framing or parser field, the
first non-`None` source wins:

1. The explicit call field has highest priority. It is `rx_framing`, `rx_parser`, or `tx_framing` passed directly on the call.
2. The call-time `protocol` preset comes next. It is the call's `protocol` field, mapped through the preset's `preset_*` functions.
3. The connection default comes next. It is stored on the connection from `open`, `open_profile`, or profile defaults.
4. The connection `protocol` preset has lowest priority. It is the `protocol` stored on the connection.

For example, a call can set `protocol: {"type":"nmea0183"}` and explicit
`rx_framing` to `{"type":"line", "ending":"lf"}`. The call then uses the
explicit line framing from layer 1. It uses the NMEA parser from layer 2 because
no explicit `rx_parser` was supplied. One layer can be overridden without
replacing the whole bundle.

The resolution lives in `src/precedence.rs` (`resolve_field`) and is shared by
`write`, `read`, `transact`, and `capture_boot`.

## Checksum and error behavior

Checksum handling has two separate cases.

### Per-frame checksum mismatch for validated NMEA and Modbus ASCII

A corrupted sentence in a burst does not abort the call. The frame is dropped,
counted, and logged.

- It is not emitted or counted in `Frame.index`.
- It is counted in `ReadResult.frames_dropped`. `TransactResult.read` and
  `CaptureBootResult.read` carry the same result shape.
- It is logged at `WARN` with the expected and received checksum values.

Decoding continues with the next frame. `Frame.index` remains contiguous across
the dropped frame.

This behavior handles occasional corrupt sentences in real NMEA marine RS-422
streams. Aborting a read on one bad checksum would make the preset unusable for
those streams. The drop remains observable.

With `validate` disabled, the frame is emitted with `checksum_valid: false` and
nothing is dropped.

### Stream-fatal decode error (SLIP malformed escape, COBS invalid code)

These errors indicate corruption of the byte stream, not only one frame's
payload. The call stops with `stop_reason: "framing_error"`. It returns the
frames and bytes decoded before the error as a partial result.

`read` returns a normal tool result, not `is_error`. It includes the partial
data and an `error` field. `TransactResult.read` and `CaptureBootResult.read`
carry the same shape. Frames decoded in the same chunk are preserved.

The cases are summarized below.

| Event | `validate` | Frame emitted? | `frames_dropped` | `stop_reason` |
|---|---|---|---|---|
| Checksum valid | either | yes, `checksum_valid: true` | 0 | normal |
| Checksum invalid | `false` | yes, `checksum_valid: false` | 0 | normal |
| Checksum invalid | `true` | no (dropped) | +1 | normal (continues) |
| SLIP invalid escape | n/a | no | 0 | `framing_error` (partial result) |
| COBS invalid code | n/a | no | 0 | `framing_error` (partial result) |

## Field reference

- `ReadResult.frames_dropped` counts frames dropped for checksum mismatches with
  `validate` enabled. It also counts frames dropped when the hex encoding
  fallback fails. That failure is effectively unreachable because hex encoding is
  total. A successful per-frame fallback re-encodes bytes as `hex`. It emits the
  frame normally and does not increment this counter.
- `ReadResult.encoding` and `FrameResult.encoding` report the effective payload
  encoding. Direct success uses the requested encoding. A lossless fallback uses
  `hex`. Frames are encoded independently, so a valid UTF-8 frame before
  malformed binary data stays UTF-8 while raw bytes use hex.
- `ReadResult.stop_reason` includes `framing_error` for stream-fatal decode
  errors. It is not a normal stop. The result still carries partial data.
- `Frame.index` is zero-based and contiguous across dropped frames and
  `skip_empty` skips. A dropped frame consumes no index.
- `ParsedFrame.Nmea.checksum_valid` and
  `ParsedFrame.ModbusAscii.checksum_valid` use `Some(true)` for a valid checksum,
  `Some(false)` for an invalid checksum when `validate` is false, and `None`
  when no checksum is present.

## Choosing a preset

- For an AT modem, use `at_command`.
- For line-delimited JSON, use `ndjson`. It skips blank lines and is preferred
  for telemetry streams over `json_lines`.
- For NMEA GPS or AIS, use `nmea0183`.
- For a Modbus ASCII PLC, use `modbus_ascii`.
- For binary packets with a reserved delimiter, use `slip` for 0xC0-delimited
  packets or `cobs` for 0x00-delimited packets. COBS is more robust on links
  that may strip 0xC0. SLIP is simpler and widely supported.
- For a custom line protocol, set `rx_framing: {"type": "line", …}`.
  Set `rx_parser: {"type": "shell_prompt"}` or `raw`.

If no preset fits, use explicit `rx_framing` and `rx_parser`. Presets are
convenience bundles, and each layer can be set independently under the
[precedence](#precedence) rules.

## References

Normative citations for each implemented framing mode, parser, and preset are in
[references.md](protocols/references.md). This repository includes citations
only, not specification text.
