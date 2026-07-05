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

## Near-term

### `transact` tool (write-then-await-response)
- one tool call: register the RX consumer FIRST, then write, then await
  match/frames/timeout — the request/response primitive for AT, Modbus,
  GRBL-style traffic
- fixes a race agents hit today: `read` returns only future bytes, so a
  device can answer in the gap between separate write and read calls; also
  halves round trips
- composes existing plumbing (`tx_session` + `read_bytes_via_session`); no
  new concepts
- this is the minimal, safe kernel of "Expect/script automation" (§ Later) —
  ship it first, revisit scripting after

### `compute_checksum` utility tool
- pure tool: compute crc16-modbus, crc32, xor, lrc, sum8 over caller-supplied
  bytes (hex/base64 in, value out)
- LLMs cannot reliably compute checksums by hand; any agent hand-crafting
  binary frames for a protocol without a preset needs this immediately
- `src/checksums.rs` is the natural home; nearly free to implement

### Declarative checksums on generic framing
- a `checksum: { algorithm, ... }` option on `Delimiter` / `LengthPrefixed` /
  `StartEnd` rx_framing/tx_framing — generalizes what the NMEA/Modbus presets
  hardcode
- covers the long tail of proprietary vendor protocols without building the
  full plugin API (see "External decoder/plugin API" below — this is the
  lighter first step)
- natural follow-on to the checksum-helper refactor in
  [review-hardening-plan.md](review-hardening-plan.md) § 2F

### TX pacing / throttling
- inter-chunk or inter-line delay on `write` (per-call field + connection
  default)
- bootloaders, GRBL controllers, and cheap AT modems drop bytes without flow
  control; today an agent must hand-loop small writes with sleeps it cannot
  actually perform

### Modbus ASCII TX auto-LRC
- TX-side counterpart to the shipped RX LRC validation: hex-encode a binary
  PDU and append the LRC on write (`:` + hex + LRC + `\r\n`)
- deliberately split out of the NMEA TX auto-checksum work
  ([review-hardening-plan.md](review-hardening-plan.md) § 3B) — needs
  hex-encoding of a binary payload, not just a checksum append

### Profile-configurable reconnect policy
- let profiles set the (already shipped and enforced) per-connection
  `ReconnectPolicy` — carried over from the removed `ProfileDefaults.reconnect_policy`
  string field, which was never wired up
- pure wiring: profile field → `open_profile` → existing policy

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

### Baud-rate auto-detection
- try candidate rates, score the RX per rate (ASCII ratio, framing-error
  rate, parser validity), return ranked guesses
- classic serial-tool feature and a strong agent diagnosis step for
  "unknown device on /dev/ttyUSB0"

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
- becomes nearly free once TX pacing and the `transact` tool (§ Near-term)
  exist — implement after those

### Multiple public subscriptions per connection
- useful if explicit session model grows later
- requires subscription IDs and fanout semantics

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
- the `transact` tool (§ Near-term) is the minimal kernel of this — ship it
  first and revisit whether scripting is still needed

### Filtering/search across captures
- unclear value — maybe an LLM can use grep/glob instead
- worth it only if searches include direction, timestamps, event types, parsed fields

### Recording + replay
- useful but niche: reproducible bugs, test fixtures from real hardware,
  decoder/parser regression tests

### RS-485 options
- half-duplex bus semantics, direction control timing, RTS-based send control
- needs a new testing strategy/firmware

### RFC2217 backend support
- server opens a remote serial device over network with control signals
- backend transport feature, not MCP transport replacement

### Bridge mode
- proxy observation, reverse engineering, test harnessing
- very complex

## Wish

### Hotplug watch
- subscribe-style notification when serial ports appear/disappear
- pairs with profiles + auto-reconnect for flaky USB adapters
- low priority: agents can poll `list_ports` today

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

- **Remote monitor** — skip, keep off active roadmap