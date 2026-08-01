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
  `max_buffered_bytes` / `poll_interval_ms` actually configured, etc.) to
  drive evidence-based decisions on which options to keep, trim, or default
  differently
- strictly local: write to a file on the host (e.g. under
  `~/.local/share/serial-mcp/` or a configured path), never transmit
  over the network, no telemetry, no remote endpoint, opt-out by
  default with an explicit enable
- the goal is development insight (which tools/options are dead weight,
  which defaults are wrong), not user tracking — design the schema
  around questions we actually want to answer ("is `from: {"type":"now"}` used
  enough to justify keeping it?", "do agents ever set
  `poll_interval_ms`?")
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
- becomes nearly free once TX pacing lands — the `transact` tool already
  ships; implement after pacing

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
- a single file holds config types, two codecs, six parsers, the decoder
  state machine, and a test region of its own — past its scaling limit
- target shape: `framing/` with `config.rs`, `decoder.rs`, `codecs.rs`,
  `parsers/`, tests alongside their subjects
- major rework — sequence AFTER the review-hardening work that rewrites
  chunks of the same file; a split first would create painful conflicts

### Split `src/serial.rs` and `src/tools/helpers.rs`
- `src/serial.rs` holds `SerialConnection`, `ConnectionManager`,
  `ConnectionConfig`, six enums, `PortInfo`, the `SerialIo` trait, and a
  `test_support` module — god-file; split into `serial/config.rs`,
  `serial/connection.rs`, `serial/manager.rs`, `serial/port_info.rs`,
  `serial/test_support.rs`
- `src/tools/helpers.rs` mixes validation, ring driving, result building,
  sinks, encoding fallback, and open-arg parsing — split into
  `tools/rx_validate.rs`, `tools/read_loop.rs`, `tools/result_builders.rs`;
  `frame_outcome_to_stop` and the shared-cursor wrapper around
  `read_from_private_cursor` are the first step in that direction
- `read_from_private_cursor` (the extracted read core in `helpers.rs`) and
  `stream_rx_from_ring` (`stream_ops.rs`) are the longest functions in the
  codebase; further decompose into `read_initial_slice` / `read_wait_loop` /
  `handle_frame_outcome` once the god-file split unblocks finer module
  boundaries

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

### Hex fallback + matcher-truncation parity between read and subscribe
- `read` falls back to hex encoding when framing-error bytes can't be
  represented in the requested encoding; `subscribe` drops the
  notification and emits a warning — binary SLIP/COBS data under utf8
  subscribe is just lost (documented asymmetry in AGENTS.md, not
  abstracted)
- `subscribe` bounds matcher memory with `truncate_front` when the
  buffered window exceeds `max_buffered_bytes`; `read` does not —
  asymmetric and undocumented
- decide: unify on hex fallback (subscribe learns it) or drop it (read
  stops falling back and surfaces the error like subscribe); unify
  matcher truncation (read learns it) or document why read is unbounded
- small behavior change, needs a design call before implementing

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

### Vendor the `models.dev` model schema for hermetic schema tests
- `schemas/opencode.schema.json` refs
  `https://models.dev/model-schema.json#/$defs/Model` externally; the
  `jsonschema` crate resolves external refs eagerly in `validator_for`, so
  the network-less Nix build sandbox cannot compile it
- until vendored, `flake.nix`'s source filter excludes
  `schemas/opencode.schema.json` from the Nix source — that fixture still
  silently skips in `nix flake check` (network-enabled CI covers it), while
  the self-contained Claude Code / Codex fixtures now validate for real
- follow-up: vendor `model-schema.json` under `schemas/`, rewrite the four
  `$ref`s to a local resource, and register it in the validator so the test
  is hermetic on every runner (see the filter comment in `flake.nix`)

### Windows e2e test path — investigate
- CI builds and unit-tests Windows, but the native_sim e2e suite is
  Unix-only (PTY-based; 57 tests ignored on the Windows runner)
- investigate whether a Windows equivalent exists (e.g. com0com-style
  virtual port pairs, or a named-pipe loopback backend); there may be a
  sound reason this was skipped — document it if so, close the gap if not