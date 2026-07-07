# Protocol Guide

serial-mcp ships a framing/parser/preset system that turns a raw serial byte
stream into structured frames you can match against, inspect, and react to.
This guide documents the seven built-in protocol presets, the four-layer
precedence model that decides which framing/parser wins, and the checksum
behavior that governs validated protocols (NMEA-0183, Modbus ASCII).

It is the product's most differentiating feature. If you are an agent
reading this before talking to a device, read at least the
[Presets](#presets) and [Precedence](#precedence) sections.

## Framing and parsers at a glance

Every `read`, `subscribe`, and `write` call can carry three sibling fields:

- **`rx_framing` / `tx_framing`** — how the byte stream is split into
  frames. RX modes: `line`, `delimiter`, `length_prefixed`, `start_end`,
  `slip`, `cobs`. TX modes mirror these minus `auto` line ending and minus
  `max_frames`/`include_terminators`/`parser`.
- **`rx_parser`** — interprets each frame's bytes. Parsers: `at_command`,
  `json_lines`, `shell_prompt`, `raw`, `nmea`, `modbus_ascii`. `rx_parser`
  is a **sibling** to `rx_framing`, not nested inside it. `write` has no
  parser (TX frames are outbound payloads, not parsed).
- **`protocol`** — a named preset that fills in defaults for the above. A
  single field instead of three.

`match` (byte-pattern detection) is independent of framing and can be
combined with any of the above. In framed mode the matcher runs per-frame
(window reset between frames); in raw mode it runs across the whole
accumulated byte stream.

## Presets

A preset is a tag-object: `{"type": "<name>"}`. Seven ship today. The table
shows what each one wires up; the sections below show concrete
`write`/`read` examples and the decoded frame shape.

| Preset | TX framing | RX framing | RX parser | `validate` default |
|---|---|---|---|---|
| `at_command` | line (`\r`) | line (`auto`) | `at_command` | false |
| `slip` | SLIP | SLIP | `raw` | false |
| `json_lines` | line (`\n`) | line (`auto`) | `json_lines` | false |
| `cobs` | COBS | COBS | `raw` | false |
| `ndjson` | line (`\n`) | line (`auto`, `skip_empty`) | `json_lines` | false |
| `nmea0183` | start_end (`$`…`\r\n`) | start_end (`$`/`!`…`\r\n`) | `nmea` | **true** |
| `modbus_ascii` | start_end (`:`…`\r\n`) | start_end (`:`…`\r\n`) | `modbus_ascii` | **true** |

`nmea0183` and `modbus_ascii` default to `validate: true` because their
specs define mandatory checksums. The other presets have no checksum to
validate, so `validate` is inert for them.

### `at_command` — AT-command modem protocol

TX appends `\r`; RX splits on line endings (auto LF/CRLF/bare-CR
detection); RX frames are parsed as AT command responses or URCs
(Unsolicited Result Codes).

```jsonc
// write: send AT+CGMI and read the model name
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

### `slip` — RFC 1055 byte-stuffed framing

SLIP wraps a payload between `END` (0xC0) bytes, escaping `END`/`ESC`
occurrences inside the payload. Raw payload — no parser. Used for IP-style
or binary packet streams where the delimiter must never appear in the data.

```jsonc
{ "connection_id": "radio0",
  "write": { "data": "<hex payload>", "encoding": "hex",
             "protocol": {"type": "slip"} } }
{ "connection_id": "radio0",
  "read":  { "protocol": {"type": "slip"} } }
```

A malformed SLIP escape sequence (0xDB followed by anything other than 0xDC
or 0xDD) is a **stream-fatal** decode error: the call/subscription stops
with `stop_reason: "framing_error"`, returning the frames decoded before
the error as a partial result. See [Checksum and error behavior](#checksum-and-error-behavior).

### `json_lines` / `ndjson` — one JSON value per line

Both use line framing + the `json_lines` parser. `ndjson` additionally sets
`skip_empty: true` so blank/whitespace-only lines are skipped per the
NDJSON spec; `json_lines` emits them (which you usually don't want).

```jsonc
{ "connection_id": "telem0",
  "read": { "protocol": {"type": "ndjson"},
            "match": {"pattern": "\"temp\"", "config": {"mode": "literal_substring"}} } }
```

Decoded `parsed` is the JSON value itself, inlined alongside the `"parser":
"json"` tag:

```jsonc
{ "parser": "json", "sensor": "temp", "value": 25.5 }
```

### `cobs` — Consistent Overhead Byte Stuffing (plain 0x00-delimited)

COBS encodes a payload so that 0x00 never appears inside it, then delimits
frames with a bare 0x00. Useful on noisy serial links where a delimiter
byte must be unambiguous. Raw payload — no parser. Like SLIP, a COBS decode
error is stream-fatal and yields a partial result with
`stop_reason: "framing_error"`. (Note: the canonical plain-COBS decoder
accepts all code bytes 0x01–0xFF as valid, so decode errors are rare in
practice; the error path exists for forward-compatibility.)

### `nmea0183` — marine sentence protocol

StartEnd framing with start markers `$` or `!` (standard and AIS) and end
`\r\n`; the NMEA parser splits the sentence into talker ID, sentence type,
and comma-separated fields, and validates the `*XX` XOR checksum. The
preset defaults to `validate: true`.

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

Proprietary sentences (`$P...`) split as `talker_id: "P"` + the rest as
`sentence_type` (e.g. `$PGRMM` → `P` + `GRMM`), per the NMEA proprietary
convention. AIS sentences start with `!` and are handled by the same
start-marker set.

### `modbus_ascii` — Modbus ASCII mode

StartEnd framing with start `:` and end `\r\n`; the Modbus ASCII parser
hex-decodes the body, exposes slave address + function code + data bytes,
and validates the trailing LRC. Defaults to `validate: true`.

```jsonc
{ "connection_id": "plc0",
  "read": { "protocol": {"type": "modbus_ascii"} }
}
```

Sample input `:010300000001FB\r\n` (read holding registers, address 1,
function 3, start 0, qty 1, LRC 0xFB) decodes to:

```jsonc
{ "parser": "modbus_ascii",
  "address": 1,
  "function_code": 3,
  "data": [0, 0, 0, 1],
  "checksum_valid": true }
```

## Precedence

When a call provides more than one source for a framing/parser field, the
**first non-`None` source wins**, in this order:

1. **explicit call field** — `rx_framing` / `rx_parser` / `tx_framing`
   passed directly on the call.
2. **call-time `protocol` preset** — the `protocol` field on the call,
   mapped through the preset's `preset_*` functions.
3. **connection default** — the default stored on the connection (from
   `open`/`open_profile`/profile defaults).
4. **connection `protocol` preset** — the `protocol` stored on the
   connection.

Example: a call with `protocol: {"type": "nmea0183"}` and an explicit
`rx_framing` of `{"type": "line", "ending": "lf"}` uses the explicit line
framing (layer 1) but the NMEA parser from the preset (layer 2, since no
explicit `rx_parser` was given). This lets you override one layer without
rewriting the whole bundle.

The resolution lives in `src/precedence.rs` (`resolve_field`) and is shared
by `write`, `read`, and `subscribe` so the three cannot drift.

## Checksum and error behavior

This is the part most likely to surprise an agent. Two distinct cases:

### Per-frame checksum mismatch (NMEA, Modbus ASCII with `validate: true`)

A single corrupted sentence in a burst does **not** abort the call. The
frame is:

- **dropped** (not emitted, not counted in `Frame.index`),
- **counted** in `ReadResult.frames_dropped` (read) or the subscription's
  final stop notification `frames_dropped` field (subscribe),
- logged at `WARN` with the expected/received checksum values,

and decoding continues with the next frame. `Frame.index` stays contiguous
across the dropped frame.

This is deliberate: real NMEA streams (marine RS-422) routinely contain
occasional corrupt sentences, and aborting the whole read on one bad
checksum made the preset unusable on exactly the streams it targets. The
drop is always observable — never silent.

With `validate: false` the frame is emitted with `checksum_valid: false`
and nothing is dropped.

### Stream-fatal decode error (SLIP malformed escape, COBS invalid code)

These mean the byte stream itself is corrupt, not just one frame's
payload. The call stops with `stop_reason: "framing_error"` and returns
the frames/bytes decoded **before** the error as a partial result. `read`
returns a normal tool result (not `is_error`) carrying the partial data;
`subscribe` emits a final notification with `stop_reason: "framing_error"`
and an `error` field. The frames already decoded this chunk are preserved
in both cases — they are not discarded.

Summary table:

| Event | `validate` | Frame emitted? | `frames_dropped` | `stop_reason` |
|---|---|---|---|---|
| Checksum valid | either | yes, `checksum_valid: true` | 0 | normal |
| Checksum invalid | `false` | yes, `checksum_valid: false` | 0 | normal |
| Checksum invalid | `true` | **no (dropped)** | +1 | normal (continues) |
| SLIP invalid escape | n/a | no | 0 | `framing_error` (partial result) |
| COBS invalid code | n/a | no | 0 | `framing_error` (partial result) |

## Field reference

- `ReadResult.frames_dropped` — count of frames dropped by the decoder
  (checksum mismatches with `validate: true`) plus frames dropped during
  result encoding (rare; per-frame encoding failure). Always observable.
- `ReadResult.stop_reason` — includes `"framing_error"` for stream-fatal
  decode errors. Not a normal stop; the result still carries partial data.
- `Frame.index` — 0-based, contiguous across dropped frames and across
  `skip_empty` skips. A dropped frame consumes no index.
- `ParsedFrame.Nmea.checksum_valid` / `ParsedFrame.ModbusAscii.checksum_valid`
  — `Some(true)` valid, `Some(false)` invalid (only with `validate: false`),
  `None` no checksum present.

## Choosing a preset

- **AT modem** → `at_command`.
- **Line-delimited JSON** → `ndjson` (skips blank lines; preferred over
  `json_lines` for telemetry streams).
- **NMEA GPS/AIS** → `nmea0183`.
- **Modbus ASCII PLC** → `modbus_ascii`.
- **Binary packets with a reserved delimiter** → `slip` (0xC0-delimited) or
  `cobs` (0x00-delimited). COBS is more robust on links that may strip
  0xC0; SLIP is simpler and widely supported.
- **Custom line protocol** → `rx_framing: {"type": "line", …}` with
  `rx_parser: {"type": "shell_prompt"}` or `raw`.

If none of the presets fit, drop down to explicit `rx_framing` + `rx_parser`
— the preset is just a convenience bundle, and every layer can be set
independently per the [precedence](#precedence) rules.

## References

Normative spec citations for each implemented framing mode, parser, and
preset are in [references.md](protocols/references.md). Cite-only — no spec
text is committed to this repository.