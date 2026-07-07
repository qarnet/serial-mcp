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

### Flow-control-aware ring backpressure (pause-on-full)
- follow-up to the RX ring redesign
  ([rx-ring-redesign-plan.md](rx-ring-redesign-plan.md)): the always-on pump
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
- deferred from the RX ring redesign
  ([rx-ring-redesign-plan.md](rx-ring-redesign-plan.md)): framing is per-call
  and applied to the drained ring window, so a frame torn at a call boundary
  (partial drain, ring wrap) decodes as garbage or a SLIP error
- carrying decoder state across `read` calls fixes that, but requires
  binding framing config to the connection rather than the call and
  rethinking its place in the 4-layer precedence model — needs design

### Per-client RX cursors
- deferred from the RX ring redesign
  ([rx-ring-redesign-plan.md](rx-ring-redesign-plan.md)): multiple HTTP
  clients share the one consuming read cursor, so concurrent consuming reads
  interleave their drains
- offsets in every result already make this diagnosable; the fix, if shared
  multi-agent access becomes real, is named cursor groups (one cursor per
  client) — overlaps with "Multiple public subscriptions per connection"
  and "Socket sharing / tee" below

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
- **SECURITY.md / vulnerability disclosure policy** — not important at the
  current project size; revisit if outside contributors arrive

## Infrastructure / tech debt

Non-feature work, roughly in suggested order. From the 2026-07-05 repo review.

### Split `src/framing.rs` into a module tree
- at ~5.5k lines (config types, two codecs, six parsers, decoder state
  machine, ~3.4k lines of tests) the single file is past its scaling limit
- target shape: `framing/` with `config.rs`, `decoder.rs`, `codecs.rs`,
  `parsers/`, tests alongside their subjects
- **major rework — sequence AFTER
  [review-hardening-plan.md](review-hardening-plan.md) Phases 1–2 land**;
  those rewrite chunks of the same file and a split first would create
  painful conflicts

### Proper dependabot / renovate setup
- dependency updates (cargo crates + pinned GitHub Actions) are currently
  manual; some dependabot experimentation happened but nothing landed
- monthly cadence, cargo + github-actions ecosystems; the 4-OS CI matrix is
  strong enough to catch bad bumps automatically

### Scheduled mutation testing + fuzz smoke in CI
- `cargo-mutants` and the `fuzz/` targets exist but run only on demand — the
  NMEA parser panic found in review is exactly the class a scheduled fuzz
  run would have caught first
- weekly `cargo-mutants` job (scope to `framing.rs` + `checksums.rs` to keep
  runtime sane) + a short scheduled fuzz smoke
  (`cargo fuzz run codec_roundtrip -- -max_total_time=300`)
- follows the `schema-drift.yml` precedent for scheduled jobs

### Release-flow guard: version bump ⇒ CHANGELOG roll
- releases key off the `Cargo.toml` version on main, but nothing enforces
  that a bump comes with a rolled `[Unreleased]` section — a stale changelog
  can ship silently
- small CI check: if `Cargo.toml` version changed vs the last release tag,
  `CHANGELOG.md` must contain a section for that version

### Explicit `doc_drift` gate in CI
- `tests/doc_drift.rs` (tool-count and preset-list guards across README,
  Cargo.toml, server.json) already runs implicitly via `cargo test --locked`
  in CI, but nothing names it — a failure shows up as a generic test failure,
  and a future test-filtering change could silently drop it
- add an explicit named step (`cargo test --locked --test doc_drift`)
  following the `config_schema_validation` precedent in `ci.yml`, so doc
  drift is its own visible, required check

### Toolchain single source of truth
- `rust-toolchain.toml` pins 1.88.0 while workflows install
  `dtolnay/rust-toolchain@stable`; rustup resolves the pin correctly but the
  intent is ambiguous and the two can silently diverge (schema-drift job
  runs whatever stable is that day)
- pick one source (recommend: the toolchain file) and make the workflows
  honor it explicitly

### Windows e2e test path — investigate
- CI builds and unit-tests Windows, but the native_sim e2e suite is
  Unix-only (PTY-based; 56 tests ignored on the Windows runner)
- investigate whether a Windows equivalent exists (e.g. com0com-style
  virtual port pairs, or a named-pipe loopback backend); there may be a
  sound reason this was skipped — document it if so, close the gap if not