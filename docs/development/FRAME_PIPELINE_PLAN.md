# Frame Pipeline Plan

This document plans the next framing work before implementation. The goal is to
make serial byte handling easier for agents by exposing the protocol building
blocks clearly, without adding a high-level `protocol` preset yet.

## Target mental model

```text
TX:
agent JSON string
→ payload decode (`encoding`: utf8/hex/base64)
→ `tx_framing` creates framed bytes
→ UART write

RX:
UART bytes
→ `rx_framing` extracts frame payloads
→ result encode (`encoding`: utf8/hex/base64)
→ optional RX parser annotates payloads
→ agent receives data/frames
```

The shared concept is **framing**: creating or extracting frames around raw
payload bytes. The RX parser is intentionally separate: it interprets an already
extracted payload frame. Current parsers (`at_command`, `json_lines`,
`shell_prompt`, `raw`) remain RX-only for now.

## API direction

Breaking changes are allowed. Prefer clarity for agents over compatibility.

### Rename RX framing

Replace the current `framing` request field on `read` and `subscribe` with
`rx_framing`.

Old shape:

```json
{
  "framing": {
    "mode": { "type": "line" },
    "parser": { "type": "at_command" }
  }
}
```

New shape:

```json
{
  "rx_framing": {
    "type": "line",
    "ending": "auto",
    "parser": { "type": "at_command" }
  }
}
```

The `mode` nesting goes away. Frame type fields live directly in `rx_framing`.
`parser` remains nested under `rx_framing` for this plan, because parser
relocation is not required for TX/RX frame symmetry.

### Add TX framing

Add `tx_framing` to `write`.

Example:

```json
{
  "connection_id": "abc",
  "data": "AT+CGMI",
  "encoding": "utf8",
  "tx_framing": {
    "type": "line",
    "ending": "cr"
  }
}
```

`tx_framing` converts payload bytes into bytes written to the serial port.
`write` results should distinguish original payload length from bytes actually
written after framing.

## Frame modes in scope

Phase 1 should support these modes for both RX and TX where directionally
meaningful:

### `raw`

No framing. TX writes payload bytes as-is. RX has no frame extraction.

### `line`

Human-oriented line framing.

TX endings:

- `lf` => `\n`
- `cr` => `\r`
- `crlf` => `\r\n`

RX endings:

- `auto` (default): recognize LF and CRLF. Bare CR support is deferred.
- `lf`: split only on `\n`; do not strip a preceding `\r`.
- `cr`: split only on `\r`.
- `crlf`: split only on exact `\r\n`.

Do not add `any` in phase 1. `auto` can grow later.

Future `auto` expansion idea: detect bare-CR streams adaptively. If a bare `\r`
is seen, temporarily wait for the next byte. If the next byte is `\n`, treat as
CRLF. If not, or if a short timeout expires, mark the current RX framing session
as CR-line mode and split on `\r` thereafter. This is intentionally out of
scope for phase 1.

Implementation should avoid duplicated line/delimiter logic where exact line
endings can map to delimiter matching. `auto` needs line-specific logic.

### `delimiter`

Exact arbitrary byte delimiter.

Example TX:

```json
{ "type": "delimiter", "delimiter": "END", "delimiter_encoding": "utf8" }
```

TX appends delimiter bytes. RX emits payload before delimiter. Delimiter matching
is exact; it has no line-ending auto behavior.

### `length_prefixed`

Binary frame with a length header.

Frame format:

```text
[length prefix][payload]
```

Options:

- `prefix_size`: 1, 2, or 4 bytes
- `endianness`: `big` or `little`

Example with one-byte length:

```text
05 68 65 6c 6c 6f
```

The prefix `05` means five payload bytes: `hello`.

TX writes prefix + payload. RX reads the prefix, waits until the full payload is
available, then emits the payload frame.

### `start_end`

Frame wrapped in start and end markers.

Example:

```json
{ "type": "start_end", "start": "<", "end": ">", "marker_encoding": "utf8" }
```

TX writes start + payload + end. RX discards bytes before start, then emits bytes
until end. This mode assumes payload does not contain the end marker. Escaping
and byte-stuffing are not phase-1 scope.

## Out of scope for phase 1

- SLIP.
- COBS.
- Protocol presets such as `at_command`, `json_lines`, or `slip_json`.
- Moving parser out of `rx_framing`.
- AT-command TX builder.
- JSON serialization helper for TX.
- Profile defaults for framing.
- Adaptive bare-CR `auto` mode.
- Escaping in `start_end` payloads.

## Later phases

### Phase 2: line auto improvements

Evaluate adaptive bare-CR handling for RX `line` with `ending: "auto"`.
Questions to answer first:

- Should auto promotion to CR mode be per read/subscribe call, per connection,
  or per profile?
- What timeout should prove a trailing `\r` is bare CR rather than pending CRLF?
- How should long-lived `subscribe` emit a final line ending in bare `\r` when
  no further bytes arrive?

### Phase 3: SLIP

Add shared SLIP frame encoder/decoder.

TX:

```json
{ "tx_framing": { "type": "slip" } }
```

RX:

```json
{ "rx_framing": { "type": "slip" } }
```

This needs byte-stuffing tests and malformed-frame behavior decisions.

### Phase 4: parser relocation / protocol presets

Consider moving parser to an explicit `rx_parser` field:

```json
{
  "rx_framing": { "type": "line", "ending": "auto" },
  "rx_parser": { "type": "at_command" }
}
```

Then add optional protocol presets that expand to explicit framing/parser
settings. Example future preset:

```json
{ "protocol": { "type": "at_command" } }
```

This could imply TX line CR, RX line auto, and RX AT parser. Presets should be
wrappers over explicit primitives, not replacements for them.

### Phase 5: profile defaults

Let saved profiles carry default `tx_framing`, `rx_framing`, and eventually
`rx_parser` or protocol presets. This avoids repeating framing options on every
tool call.

## Orchestration workflow

Insight acts as orchestrator, not implementer, for this feature.

For each phase:

1. Plan the phase and clarify open questions with the user.
2. Define tests that prove the intended behavior.
3. Write a handoff document for the implementation agent.
4. Hand off and wait for the user to return with implementation results.
5. Evaluate the implemented changes.
6. If changes are needed, discuss with the user before asking the agent for
   follow-up work.
7. If changes look good, summarize findings and ask whether to continue to the
   next phase.

Each phase should be reviewed before the next starts.

## Phase 1 validation ideas

Tests should cover at least:

- `write` with default/raw TX framing preserves current exact-byte behavior.
- `write` with line LF/CR/CRLF writes exact bytes.
- `write` with delimiter appends exact decoded delimiter bytes.
- `write` with length prefix writes correct prefix size and endianness.
- `write` with start/end writes exact markers around payload.
- RX `line auto` recognizes LF and CRLF.
- RX `line lf` does not strip preceding CR.
- RX `line cr` splits on bare CR.
- RX `line crlf` waits for exact CRLF.
- RX delimiter, length-prefixed, and start/end still behave as before under the
  flattened `rx_framing` shape.
- Round-trip tests: TX frame creation output fed into RX frame extraction returns
  original payload for delimiter, length-prefixed, start/end, and exact line
  modes.
- Tool schemas expose `rx_framing` and `tx_framing` and do not expose old
  `framing`.
- Schema regression tests still reject non-standard unsigned integer formats.
