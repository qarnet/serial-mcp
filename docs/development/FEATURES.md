# Feature Ideas — serial-mcp

> Trimmed from the original roadmap. Items already shipped (device identity,
> hot reconfigure, `get_status`, profiles, statistics, framing/packet decoder,
> AT/JSON/shell protocol parsers, auto-reconnect, richer matching modes
> regex/glob, event log, native_sim diagnostics) were removed. See
> [CHANGELOG.md](../../CHANGELOG.md) and the README tool list for what landed.
>
> Priorities: **Near-term** · **Later** · **Wish** · **Needs architecture review**.

## Near-term

### SLIP decoder performance — drop O(n²) byte draining
- **Problem.** `slip_decode` (`src/framing.rs`) consumes its input with
  `buf_outer.remove(0)` one byte at a time. `Vec::remove(0)` shifts every
  remaining element, so decoding an `n`-byte buffer is `O(n²)`. The RX pump
  delivers chunks up to `PUMP_READ_SIZE` (4096 bytes, `src/rx_session.rs`), so
  a single large SLIP frame already pays ~8M element-moves worst case, and a
  cross-chunk frame re-pays it on every push. Every other decoder mode
  (line/delimiter/length_prefixed/start_end) is already linear because it uses
  range `drain`/`position` instead of per-byte removal.
- **Goal.** Make SLIP decode `O(n)` in the number of bytes processed, matching
  the other modes.
- **Solution options (ranked):**
  1. **Cursor + single drain (smallest change).** Iterate `buf_outer` by index
     with a local `read_pos` cursor, pushing decoded bytes into the frame
     buffer, and `drain(..read_pos)` exactly once before returning. No
     `remove(0)`. Keeps the existing `SlipState` resync logic intact. `O(n)`
     time, `O(1)` extra moves.
  2. **`VecDeque<u8>` for `buf_outer`.** `pop_front` is amortized `O(1)`, so the
     existing per-byte loop becomes `O(n)` with minimal structural change. Costs
     a type change on the decoder's buffer and loses cheap slice access that the
     other modes rely on — only attractive if SLIP keeps a separate buffer.
  3. **Slice-scan like the other modes.** Find the next `END` with
     `position`, decode the span between markers in one pass (un-stuffing into
     the frame buffer), then `drain` through the marker. Most consistent with
     the rest of the file; slightly more code to handle the cross-chunk escape
     carry (`escaped` at a chunk boundary).
- **Recommendation:** option 1 — it is the least invasive, preserves the
  state-machine and its tests verbatim, and gets the full `O(n)` win.

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