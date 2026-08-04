# Feature Ideas — serial-mcp

> Trimmed from the original roadmap. Items already shipped (device identity,
> hot reconfigure, `get_status`, profiles, statistics, framing/packet decoder,
> AT/JSON/shell protocol parsers, auto-reconnect, richer matching modes
> regex/glob, event log, native_sim diagnostics, `--version`, the 7 protocol
> presets) were removed. See [CHANGELOG.md](../../CHANGELOG.md) and the README
> tool list for what landed.
>
> **Protocol roadmap** (HDLC/PPP framing, Modbus RTU, MIDI, Firmata — all
> deferred) is tracked per-protocol in
> [protocol-matrix.md](protocol-matrix.md); this file tracks everything else.
>
> Priorities: **Near-term** · **Later** · **Wish** · **Needs architecture review**.
> Non-feature work (refactors, CI, release mechanics) lives in
> § Infrastructure / tech debt at the bottom.

## Near-term

### Local-only usage statistics for development
- collect local metadata on tool-call frequency, stop-reason
  distribution, option-usage frequency (which `from` variants agents
  actually pick, how often `match`/`framing` options get used, average
  `max_buffered_bytes` actually configured, etc.) to
  drive evidence-based decisions on which options to keep, trim, or default
  differently
- strictly local: write to a file on the host (e.g. under
  `~/.local/share/serial-mcp/` or a configured path), never transmit
  over the network, no telemetry, no remote endpoint, opt-out by
  default with an explicit enable
- the goal is development insight (which tools/options are dead weight,
  which defaults are wrong), not user tracking — design the schema
  around questions we actually want to answer ("is `from: {"type":"now"}` used
  enough to justify keeping it?")
- pairs with the shipped `configure` tool + connection-default trim (the
  stats inform which fields to cut, and cutting fields makes the remaining
  stats cleaner)

### Declarative checksums on generic framing
- a `checksum: { algorithm, ... }` option on `Delimiter` / `LengthPrefixed` /
  `StartEnd` rx_framing/tx_framing — generalizes what the NMEA/Modbus presets
  hardcode
- covers the long tail of proprietary vendor protocols without building the
  full plugin API (see "External decoder/plugin API" below — this is the
  lighter first step)
- natural follow-on to the checksum-helper refactor that landed in 0.7.3

### TX pacing / throttling
- inter-chunk or inter-line delay on `write` (per-call field + connection
  default)
- bootloaders, GRBL controllers, and cheap AT modems drop bytes without flow
  control; today an agent must hand-loop small writes with sleeps it cannot
  actually perform

### Modbus ASCII TX auto-LRC
- TX-side counterpart to the shipped RX LRC validation: hex-encode a binary
  PDU and append the LRC on write (`:` + hex + LRC + `\r\n`)
- deliberately split out of the NMEA TX auto-checksum work that landed
  in 0.7.3 — needs hex-encoding of a binary payload, not just a
  checksum append
- **refactor trigger (one-consumer rule):** when this lands, extract a shared
  TX checksum-append layer instead of growing `TxFramingMode` variant-by-
  variant. The `TxFramingMode::Nmea` encode arm (shipped) is the first
  checksum-appending TX mode; Modbus ASCII TX would be the second and should
  share the "compute checksum → validate-or-append → wrap with markers"
  skeleton so a third (CRC-16 / FCS-16) is a one-line diff.
- **refactor trigger (one-consumer rule):** when CRC-16 (Modbus RTU) lands,
  make `emit_frame`'s per-frame validation policy pluggable. Today
  `emit_frame` hard-codes the `ChecksumMismatch` → drop-and-count behavior
  for all parsers; a multi-checksum world (XOR/LRC now, CRC-16/FCS-16 future)
  wants the validation outcome (drop+count vs. stream-fatal vs. emit-with-
  flag) driven by the parser's declared checksum width, not a match on the
  error variant.

### Config import/export
- likely pairs with profiles (already shipped)
- needs sharpening before it earns implementation: the profiles TOML file
  already covers copy-between-machines; the added value would be exporting a
  *running* server's full state (open connections + their framing/parser
  defaults) as importable profiles

### External decoder/plugin API
- useful after the in-process decoders (AT/JSON/shell) shipped
- allow plugging in custom frame decoders / parsers
- prefer shipping "Declarative checksums on generic framing" (above) first —
  it covers much of the demand at a fraction of the API surface

### Decoder integration / export hooks
- export capture or frames to external decoder tools if in-process support stays small

## Later

### MCP Tasks extension for long-running operations
- SEP-2663 / `io.modelcontextprotocol/tasks`: let a tool return a task handle
  immediately, then expose `tasks/get`, `tasks/update`, and `tasks/cancel` with
  cooperative cancellation and bounded result retention/TTL
- candidates: long `read`, `transact`, `capture_boot`, `send_break`, and any
  future firmware/file-transfer operation; keep normal synchronous execution
  for clients that do not declare Tasks support
- not a replacement for open-ended RX availability notifications: rmcp 3.0.1
  clients must poll task state and task-status delivery through
  `subscriptions/listen` is not available yet
- needs ownership/lifecycle design for connection close, client disconnect,
  server restart, task expiry, profile learning, and partial serial results

### Positive MCP cache TTL policy
- future feature only: the 2026-07-28 cache fields already ship, but as the
  non-cacheable baseline (`ttlMs=0`, `cacheScope: private`) on every cacheable
  family — enabling positive TTL is NOT shipped and is what this item proposes
- possible policy if pursued: long/public for static tool, prompt, and
  resource-template catalogs; short/private for live port lists; zero/private
  for open connections, logs, status, and RX data
- only enable positive TTL after resource/list notification invalidation,
  authorization partitioning, pagination keys, and rmcp's stale-on-error client
  behavior have public-boundary tests

### Standard HTTP parameter headers
- MCP 2026-07-28 + rmcp 3 automatically provide `Mcp-Method` and `Mcp-Name`;
  selected primitive tool inputs can later opt into `Mcp-Param-*` through
  top-level `x-mcp-header` schema annotations
- possible low-risk first field: `connection_id`; assess `port` and `profile`
  separately because proxies commonly log headers
- never promote commands, serial payloads, match data, selectors, credentials,
  or capture filenames into infrastructure-visible headers

### Flow-control-aware ring backpressure (pause-on-full)
- follow-up to the 0.8.0 RX ring redesign: the always-on pump
  drains the kernel buffer continuously, so with RTS/CTS enabled the kernel
  never deasserts RTS and the device is never throttled — sustained unread
  traffic wraps the ring (oldest bytes lost, observably via `bytes_lost`)
- opt-in per-connection mode (`on_full: "wrap"` (default) | `"pause"`): when
  the ring is full and RTS/CTS is active, pause the pump instead of
  overwriting — the kernel buffer fills, RTS deasserts, the device pauses,
  and hardware backpressure semantics are restored end-to-end
- emit a log event + MCP notification on entering/leaving the paused state
  so agents know the device is being held off rather than silent
- only meaningful with hardware flow control; with none enabled, pausing
  just moves the loss from our ring to the kernel buffer/UART FIFO

### Persistent per-connection framing decoder
- deferred from the 0.8.0 RX ring redesign: framing is per-call
  and applied to the drained ring window, so a frame torn at a call boundary
  (partial drain, ring wrap) decodes as garbage or a SLIP error
- carrying decoder state across `read` calls fixes that, but requires
  binding framing config to the connection rather than the call and
  rethinking its place in the 4-layer precedence model — needs design

### Per-client RX cursors
- deferred from the 0.8.0 RX ring redesign: multiple HTTP
  clients share the one consuming read cursor, so concurrent consuming reads
  interleave their drains
- offsets in every result already make this diagnosable; the fix, if shared
  multi-agent access becomes real, is named cursor groups (one cursor per
  client) — overlaps with "Socket sharing / tee" below

### Baud-rate auto-detection *(deferred)*
- classic serial-tool feature and a strong agent diagnosis step for
  "unknown device on /dev/ttyUSB0": try candidate rates, score the RX per
  rate, return ranked guesses
- deferred: generic host-side detection over an ordinary USB-serial adapter
  is heuristic, not waveform measurement — the adapter's UART already
  re-clocked the bits at the configured rate, so the host sees decoded
  garbage that can score deceptively well at the wrong rate; a built-in
  tool should return **inconclusive** rather than guess
- existing solution/reference worth studying before building anything:
  EXPLIoT's `uart.generic.baudscan`
  ([repo](https://gitlab.com/expliot_framework/expliot),
  [baudscan.py](https://gitlab.com/expliot_framework/expliot/-/blob/master/expliot/plugins/serial/baudscan.py))
  cycles candidate rates, reads a bounded sample per rate, and ranks each
  by printable-ASCII percentage (default rates
  1200/2400/4800/9600/19200/38400/57600/115200; accepts 100% or the best
  rate above 90%) — useful for continuously emitting ASCII devices, weak
  for silent or binary devices
- reference only: no dependency, integration, or adoption implied

### Modem input lines + UART error counters in `get_status`
- read CTS/DSR/CD/RI (the serialport crate exposes them) — today the server
  can *set* DTR/RTS but cannot *read* any input line
- parity/framing/overrun error counters where the platform provides them
- cheap additive fields on `ConnectionStatus`

### Per-frame timestamps
- `Frame` carries an index but no time; correlating frames against the event
  log or across two connections is currently impossible
- small additive field, but it changes the wire format — decide before 1.0

### GRBL / G-code preset
- line-based `ok`/`error` protocol; popular real-world target (CNC, laser,
  3D-printer controllers)
- becomes nearly free once TX pacing lands — the `transact` tool already
  ships; implement after pacing

### Safety policies for dangerous commands
- optional dangerous-command confirmation patterns
- includes the profile-level safety policy intent carried over from the
  removed `ProfileDefaults.safety_policy` field

### Capture bookmarks / annotations
- useful if logs/captures grow further

### Expect/script automation *(needs architecture review)*
- interesting, but discuss at a higher architecture level before adding
- can become huge if designed poorly; can conflict with simpler read/write model
- conservative first design if pursued: JSON transaction steps only, bounded
  step types, no shell access, deterministic transcript output
- the shipped `transact` tool is the minimal kernel of this — revisit
  whether scripting is still needed

### Filtering/search across captures
- unclear value — maybe an LLM can use grep/glob instead
- worth it only if searches include direction, timestamps, event types, parsed fields

### Recording + replay
- useful but niche: reproducible bugs, test fixtures from real hardware,
  decoder/parser regression tests
- **The safe persistent capture foundation shipped** (disabled-by-default
  `--capture-dir` store, portable filename-only `export_log`, quotas,
  no-overwrite atomic commits, advisory locks) — a prerequisite, not the
  feature. Continuous raw capture lifecycle is specified (NOT implemented)
  in [safe-continuous-capture-design.md](safe-continuous-capture-design.md);
  recommendation: do not implement until concrete task evidence.

### RS-485 options
- half-duplex bus semantics, direction control timing, RTS-based send control
- needs a new testing strategy/firmware

### RFC2217 backend support
- server opens a remote serial device over network with control signals
- backend transport feature, not MCP transport replacement

### Bridge mode
- proxy observation, reverse engineering, test harnessing
- very complex

### MCP Bundle (MCPB) distribution
- package existing native release binaries as an MCP Bundle for one-click local
  stdio installation in supporting desktop clients
- promising fit: `server.type = "binary"`, no language runtime, platform-specific
  command overrides, and optional user configuration
- separate release/distribution project, not protocol work; decide one
  cross-platform bundle versus per-platform bundles, manifest version, signing,
  update flow, and clean-machine Claude Desktop tests before implementation
- validate/pack with a pinned `@anthropic-ai/mcpb` release when scheduled

## Wish

### Earlier MCP protocol revisions (pre-2025-11-25)
- potential future feature only — NOT current support and not a near-term
  commitment: the supported set remains exactly `2026-07-28` (preferred) and
  permanent `2025-11-25`;
- possible older candidates are `2025-06-18`, `2025-03-26`, and `2024-11-05`;
- implement only with concrete user/client demand — never merely because
  rmcp lists the version in `KNOWN_VERSIONS`;
- each version would require: an explicit product policy row,
  lifecycle/capability/cache review, raw-wire tests, official conformance
  support where available, and a real historical client fixture.

### User-facing loopback / virtual port backend
- expose a virtual echo/scripted device as an openable backend (the
  native_sim harness exists but is test-only)
- lets agents demo and develop flows with no hardware attached

### Socket sharing / tee / shared live access
- not the HTTP MCP transport — this exposes a live serial stream/session to
  another consumer
- seems complicated; keep as future wish

### File transfer protocols
- do not turn project into a full DFU/flashing suite
- only consider generic serial-native transfer helpers if ever added

### Non-intrusive sniffing / proxy observation
- most realistic path: proxy/bridge observation, not universal passive sniff

### Human + agent shared session / tee mode
- overlaps with socket sharing

## Explicit skip for now

- **MRTR product flows** — rmcp 3 supports SEP-2322 multi-round tool/resource/
  prompt requests (`InputRequiredResult`) for client elicitation and retries,
  but current schemas, defaults, destructive hints, and cancellation cover the
  serial workflows we have. Revisit only for a concrete need such as physical
  power-cycle guidance or destructive reset confirmation; any echoed
  `requestState` must be integrity-protected.
- **Remote monitor** — skip, keep off active roadmap
- **SECURITY.md / vulnerability disclosure policy** — not important at the
  current project size; revisit if outside contributors arrive

## Infrastructure / tech debt

Non-feature work, roughly in suggested order. From the 2026-07-05 repo review.

### Decompose the longest read/stream functions
- Read now runs through `ReadAccumulator` + centralized result finalization
  (`src/tools/read_loop.rs`); subscription delivery runs through named
  raw/partial/context helpers (`src/tools/stream_ops.rs`).
- Remaining debt, only if a concrete maintenance need justifies extra
  control-flow boundaries: further async wait-loop decomposition inside the
  read loop.

### `UInt` newtype to kill schemars `uint_schema` boilerplate
- per-field `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]`
  annotations are sprinkled across the tree, with a documented
  regression history (b12b09fd, bc37a0b0, PortInfo miss) — missing one
  is a known bug vector
- schemars 1.x emits non-standard `"format": "uintN"` for unsigned
  integer fields; the per-field attribute is the workaround
- a `UInt` (and `OptionUInt`) newtype that derives `JsonSchema` natively
  without the format keyword, OR a schemars visitor that strips the
  format globally, would collapse the entire class of bug and remove
  the per-struct `check_schema!` maintenance burden
- coordinate with any schemars 2.x migration if one is on the roadmap;
  the upstream fix may make the newtype redundant
