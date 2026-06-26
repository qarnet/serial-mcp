# Feature Ideas — serial-mcp

> Trimmed from the original roadmap. Items already shipped (device identity,
> hot reconfigure, `get_status`, profiles, statistics, framing/packet decoder,
> AT/JSON/shell protocol parsers, auto-reconnect, richer matching modes
> regex/glob, event log, native_sim diagnostics) were removed. See
> [CHANGELOG.md](../../CHANGELOG.md) and the README tool list for what landed.
>
> Priorities: **Near-term** · **Later** · **Wish** · **Needs architecture review**.

## Near-term

### Version command / flag
- restore a dedicated `--version` flag (and/or a `version` subcommand) so the
  binary reports its own version directly
- the version is currently only visible via `--help`; release artifacts no
  longer carry the version in their filenames, so a first-class version readout
  is the reliable way to identify an installed binary

### Config import/export
- likely pairs with profiles (already shipped)

### External decoder/plugin API
- useful after the in-process decoders (AT/JSON/shell) shipped
- allow plugging in custom frame decoders / parsers

### Decoder integration / export hooks
- export capture or frames to external decoder tools if in-process support stays small

## Later

### Multiple public subscriptions per connection
- useful if explicit session model grows later
- requires subscription IDs and fanout semantics

### Safety policies for dangerous commands
- optional dangerous-command confirmation patterns

### Capture bookmarks / annotations
- useful if logs/captures grow further

### Expect/script automation *(needs architecture review)*
- interesting, but discuss at a higher architecture level before adding
- can become huge if designed poorly; can conflict with simpler read/write model
- conservative first design if pursued: JSON transaction steps only, bounded
  step types, no shell access, deterministic transcript output

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