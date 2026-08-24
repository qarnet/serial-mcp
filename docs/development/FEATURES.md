# Feature ideas: serial-mcp

> This list is trimmed from the original roadmap. Items already shipped, such
> as device identity, hot reconfigure, `get_status`, profiles, statistics,
> framing and packet decoding, AT/JSON/shell protocol parsers, auto-reconnect,
> richer regex/glob matching modes, the event log, native_sim diagnostics,
> `--version`, and the 7 protocol presets, were removed. See
> [CHANGELOG.md](../../CHANGELOG.md) and the README tool list for shipped work.
>
> The protocol roadmap covers HDLC/PPP framing, Modbus RTU, MIDI, and Firmata.
> All are deferred and tracked per protocol in
> [protocol-matrix.md](protocol-matrix.md). This file tracks other work.
>
> Priorities are Near-term, Later, Wish, and Needs architecture review.
> Non-feature work, including refactors, CI, and release mechanics, is listed
> under Infrastructure / tech debt at the bottom.

## Near-term

### Local-only usage statistics for development

- Collect local metadata about tool-call frequency, stop-reason distribution,
  option usage, which `from` variants agents choose, how often `match` and
  `framing` options are used, and the average configured
  `max_buffered_bytes`. Use it to decide which options to keep, trim, or set as
  defaults.
- Keep this data strictly local. Write it to a host file, such as one under
  `~/.local/share/serial-mcp/` or a configured path. Never transmit it over the
  network. Do not use telemetry or a remote endpoint. Opt out by default and
  require explicit enablement.
- Use the data for development insight into dead-weight tools and options and
  incorrect defaults, not for user tracking. Design the schema around concrete
  questions such as whether `from: {"type":"now"}` is used enough to keep.
- Pair this work with the shipped `configure` tool and connection-default trim.
  The statistics can show which fields to cut, and cutting fields makes the
  remaining statistics cleaner.

### Declarative checksums on generic framing

- Add a `checksum: { algorithm, ... }` option to `Delimiter`, `LengthPrefixed`,
  and `StartEnd` `rx_framing` and `tx_framing`. This generalizes the checksum
  behavior hardcoded by the NMEA and Modbus presets.
- This covers many proprietary vendor protocols without building the full
  plugin API. See External decoder/plugin API below. It is the lighter first
  step.
- This is a natural follow-on to the checksum-helper refactor that landed in
  0.7.3.

### TX pacing / throttling

- Add an inter-chunk or inter-line delay to `write`, with a per-call field and
  a connection default.
- Bootloaders, GRBL controllers, and inexpensive AT modems can drop bytes
  without flow control. Today an agent must hand-loop small writes with sleeps
  it cannot actually perform.

### Modbus ASCII TX auto-LRC

- Add the TX counterpart to shipped RX LRC validation. Hex-encode a binary PDU
  and append the LRC on write as `:` plus hex, LRC, and `\r\n`.
- Keep this separate from the NMEA TX auto-checksum work that landed in 0.7.3.
  It needs hex-encoding of a binary payload, not only checksum appending.
- Refactor trigger (one-consumer rule): when this lands, extract a shared TX
  checksum-append layer
  instead of adding one `TxFramingMode` variant at a time. The shipped
  `TxFramingMode::Nmea` encode arm is the first checksum-appending TX mode.
  Modbus ASCII TX would be the second. Both should share a skeleton that
  computes a checksum, validates or appends it, and wraps the payload with
  markers. A third CRC-16 / FCS-16 mode should then be a one-line diff.
- Refactor trigger (one-consumer rule): when CRC-16 (Modbus RTU) lands, make
  `emit_frame`'s per-frame validation policy pluggable. Today `emit_frame` hard-codes
  `ChecksumMismatch` to drop and count for all parsers. A multi-checksum
  implementation with XOR/LRC now and CRC-16/FCS-16 later should choose
  drop-and-count, stream-fatal, or emit-with-flag from the parser's declared
  checksum width, not from a match on the error variant.

### Config import/export

- This likely pairs with profiles, which already ship.
- Sharpen the idea before implementation. The profiles TOML file already
  supports copying profiles between machines. The added value would be
  exporting a running server's full state, including open connections and
  their framing/parser defaults, as importable profiles.

### External decoder/plugin API

- This becomes useful after the in-process AT/JSON/shell decoders have shipped.
- Allow custom frame decoders and parsers to be plugged in.
- Ship Declarative checksums on generic framing first. It covers much of the
  demand with a smaller API surface.

### Decoder integration / export hooks

- Add hooks to export captures or frames to external decoder tools if in-process
  support remains small.

## Later

### MCP Tasks extension for long-running operations

- Use SEP-2663 / `io.modelcontextprotocol/tasks` so a tool can return a task
  handle immediately and expose `tasks/get`, `tasks/update`, and `tasks/cancel`
  with cooperative cancellation and bounded result retention and TTL.
- Candidates include long `read`, `transact`, `capture_boot`, `send_break`, and
  future firmware or file-transfer operations. Keep normal synchronous
  execution for clients that do not declare Tasks support.
- This is not a replacement for open-ended RX availability notifications.
  Current rmcp clients must poll task state because task-status delivery through
  `subscriptions/listen` is not available yet.
- Design ownership and lifecycle behavior for connection close, client
  disconnect, server restart, task expiry, profile learning, and partial serial
  results.

### Positive MCP cache TTL policy

- This is a future feature only. The 2026-07-28 cache fields already ship as
  the non-cacheable baseline, with `ttlMs=0` and `cacheScope: private` on every
  cacheable family. Positive TTL is not shipped and is what this item proposes.
- If pursued, use long/public TTL for static tool, prompt, and
  resource-template catalogs; short/private TTL for live port lists; and
  zero/private TTL for open connections, logs, status, and RX data.
- Enable positive TTL only after resource/list notification invalidation,
  authorization partitioning, pagination keys, and rmcp's stale-on-error
  client behavior have public-boundary tests.

### Standard HTTP parameter headers

- MCP 2026-07-28 and rmcp 3 automatically provide `Mcp-Method` and `Mcp-Name`.
  Selected primitive tool inputs can later opt into `Mcp-Param-*` through
  top-level `x-mcp-header` schema annotations.
- A possible low-risk first field is `connection_id`. Assess `port` and
  `profile` separately because proxies commonly log headers.
- Never promote commands, serial payloads, match data, selectors, credentials,
  or capture filenames into headers visible to infrastructure.

### Flow-control-aware ring backpressure (pause-on-full)

- This follows the 0.8.0 RX ring redesign. The always-on pump drains the
  kernel buffer continuously. With RTS/CTS enabled, the kernel therefore never
  deasserts RTS and the device is never throttled. Sustained unread traffic
  wraps the ring, losing the oldest bytes and reporting the loss through
  `bytes_lost`.
- Add an opt-in per-connection mode, `on_full: "wrap"` (default) or
  `"pause"`. When the ring is full and RTS/CTS is active, pause the pump
  instead of overwriting. The kernel buffer fills, RTS deasserts, the device
  pauses, and hardware backpressure works end to end.
- Emit a log event and MCP notification when entering or leaving the paused
  state so agents know the device is being held off rather than silent.
- This matters only with hardware flow control. With no flow control, pausing
  moves the loss from the ring to the kernel buffer or UART FIFO.

### Persistent per-connection framing decoder

- This was deferred from the 0.8.0 RX ring redesign. Framing is per call and is
  applied to the drained ring window, so a frame split at a call boundary by a
  partial drain or ring wrap can decode as garbage or a SLIP error.
- Carrying decoder state across `read` calls would fix that. It requires binding
  framing configuration to the connection instead of the call and redesigning
  its place in the four-layer precedence model.

### Per-client RX cursors

- This was deferred from the 0.8.0 RX ring redesign. Multiple HTTP clients share
  one consuming read cursor, so concurrent consuming reads interleave their
  drains.
- Offsets in every result already make this diagnosable. If shared multi-agent
  access becomes real, use named cursor groups with one cursor per client. This
  overlaps with Socket sharing / tee below.

### Baud-rate auto-detection *(deferred)*

- This is a classic serial-tool feature and a useful diagnosis step for an
  unknown device on `/dev/ttyUSB0`. Try candidate rates, score RX for each
  rate, and return ranked guesses.
- Generic host-side detection over an ordinary USB-serial adapter is heuristic,
  not waveform measurement. The adapter's UART has already re-clocked the bits
  at the configured rate, so the host sees decoded garbage that can score well
  at the wrong rate. A built-in tool should return inconclusive rather than
  guess.
- Before building anything, study EXPLIoT's `uart.generic.baudscan`:
  ([repo](https://gitlab.com/expliot_framework/expliot),
  [baudscan.py](https://gitlab.com/expliot_framework/expliot/-/blob/master/expliot/plugins/serial/baudscan.py)).
- The reference cycles candidate rates, reads a bounded sample for each rate,
  and ranks each by printable-ASCII percentage. Default rates are
  1200/2400/4800/9600/19200/38400/57600/115200. It accepts 100% or the best
  rate above 90%. This is useful for continuously emitting ASCII devices and
  weak for silent or binary devices.
- This is a reference only. It implies no dependency, integration, or adoption.

### Modem input lines + UART error counters in `get_status`

- Read CTS/DSR/CD/RI, which the serialport crate exposes. Today the server can
  set DTR/RTS but cannot read any input line.
- Add parity, framing, and overrun error counters where the platform provides
  them.
- These are cheap additive fields on `ConnectionStatus`.

### Per-frame timestamps

- `Frame` carries an index but no time. Correlating frames against the event log
  or across two connections is impossible with the current frame data.
- A timestamp is a small additive field, but it changes the wire format. Decide
  before 1.0.

### GRBL / G-code preset

- Add a line-based `ok`/`error` protocol for the popular CNC, laser, and
  3D-printer controller target.
- This becomes nearly free after TX pacing lands. The `transact` tool already
  ships, so implement this after pacing.

### Safety policies for dangerous commands

- Add optional dangerous-command confirmation patterns.
- Include the profile-level safety-policy intent carried over from the removed
  `ProfileDefaults.safety_policy` field.

### Capture bookmarks / annotations

- Add bookmarks or annotations if logs and captures grow further.

### Expect/script automation *(needs architecture review)*

- Discuss this at a higher architecture level before adding it.
- It can become large if designed poorly and can conflict with the simpler
  read/write model.
- If pursued, start with JSON transaction steps, bounded step types, no shell
  access, and deterministic transcript output.
- The shipped `transact` tool is the minimal kernel for this. Revisit whether
  scripting is still needed.

### Filtering/search across captures

- The value is unclear. An agent may be able to use grep or glob instead.
- Consider this only if searches include direction, timestamps, event types, and
  parsed fields.

### Recording + replay

- This is useful but niche for reproducible bugs, test fixtures from real
  hardware, and decoder/parser regression tests.
- The safe persistent capture foundation has shipped. It includes the
  disabled-by-default `--capture-dir` store, portable filename-only
  `export_log`, quotas, no-overwrite atomic commits, and advisory locks. It is
  a prerequisite, not the recording feature. The continuous raw capture
  lifecycle is specified but not implemented in
  [safe-continuous-capture-design.md](safe-continuous-capture-design.md).
  Do not implement it until concrete task evidence exists.

### RS-485 options

- Add half-duplex bus semantics, direction-control timing, and RTS-based send
  control.
- This needs a new testing strategy and firmware.

### RFC2217 backend support

- Open a remote serial device over the network with control signals.
- This is a backend transport feature, not an MCP transport replacement.

### Bridge mode

- Support proxy observation, reverse engineering, and test harnessing.
- This is very complex.

### MCP Bundle (MCPB) distribution

- Package existing native release binaries as an MCP Bundle for one-click local
  stdio installation in supporting desktop clients.
- This fits `server.type = "binary"`, needs no language runtime, supports
  platform-specific command overrides, and can include optional user
  configuration.
- Treat this as a separate release and distribution project, not protocol work.
  Decide between one cross-platform bundle and per-platform bundles, then
  define the manifest version, signing, update flow, and clean-machine Claude
  Desktop tests before implementation.
- Validate and pack it with a pinned `@anthropic-ai/mcpb` release when
  scheduled.

## Wish

### Earlier MCP protocol revisions (pre-2025-11-25)

- This is a potential future feature, not current support and not a near-term
  commitment. The supported set remains exactly `2026-07-28` (preferred) and
  permanent `2025-11-25`.
- Possible older candidates are `2025-06-18`, `2025-03-26`, and `2024-11-05`.
- Implement an older revision only with concrete user or client demand, never
  merely because rmcp lists it in `KNOWN_VERSIONS`.
- Each version would need an explicit product policy row,
  lifecycle/capability/cache review, raw-wire tests, official conformance
  support where available, and a real historical client fixture.

### User-facing loopback / virtual port backend

- Expose a virtual echo or scripted device as an openable backend. The
  native_sim harness exists but is test-only.
- This would let agents demonstrate and develop flows without hardware.

### Socket sharing / tee / shared live access

- This is not the HTTP MCP transport. It would expose a live serial
  stream/session to another consumer.
- The design seems complicated, so keep it as a future wish.

### File transfer protocols

- Do not turn the project into a full DFU or flashing suite.
- Consider generic serial-native transfer helpers only if they are added later.

### Non-intrusive sniffing / proxy observation

- The most realistic path is proxy or bridge observation, not universal passive
  sniffing.

### Human + agent shared session / tee mode

- This overlaps with socket sharing.

## Explicitly deferred

- MRTR product flows are deferred. rmcp 3 supports SEP-2322 multi-round
  tool/resource/prompt requests (`InputRequiredResult`) for client elicitation
  and retries. Current schemas, defaults, destructive hints, and cancellation
  cover the serial workflows we have. Revisit this only for a concrete need,
  such as physical power-cycle guidance or destructive reset confirmation. Any
  echoed `requestState` must be integrity-protected.
- Remote monitor is deferred and remains off the active roadmap.
- `SECURITY.md` and the vulnerability disclosure policy are deferred. Revisit
  them if outside contributors arrive.

## Infrastructure / tech debt

Non-feature work is listed in rough suggested order. This list comes from the
2026-07-05 repository review.

### `UInt` newtype to remove schemars `uint_schema` boilerplate

- Per-field `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]`
  annotations are spread across the tree. The documented regression history
  includes b12b09fd, bc37a0b0, and the PortInfo miss. Missing one is a known bug
  vector.
- schemars 1.x emits non-standard `"format": "uintN"` for unsigned integer
  fields. The per-field attribute is the workaround.
- Fields with `#[serde(skip_serializing_if = "Option::is_none")]` must also
  carry `#[serde(default)]`. schemars 1.2.2 does not see through `schema_with`
  to the `Option` type. Without `default`, the field enters the schema's
  `required` array while serialization omits it. This caused the PortInfo
  vid/pid/interface miss.
- A `UInt` and `OptionUInt` newtype that derives `JsonSchema` natively without
  the format keyword, or a schemars visitor that strips the format globally,
  would remove this class of bug and the per-struct `check_schema!` maintenance
  burden.
- Coordinate this with any schemars 2.x migration on the roadmap. An upstream
  fix may make the newtype redundant.
