# Feature Ideas

## Goal

- Make `serial-mcp` best tool for agent-driven serial work.
- Prioritize features that reduce agent prompt burden, improve reliability, and make hardware identity stable.
- Keep future expansion open, but avoid overcommitting to giant flashing/DFU ecosystem scope.

## Priority legend

- **Now** — should plan or implement soon.
- **Near-term** — important, but after current highest-value work.
- **Later** — expected future feature.
- **Wish** — interesting, but not close to current implementation focus.
- **Needs architecture review** — good idea, but should be designed at system level first.

## Ranked roadmap

### Tier 1 — strongest next features

1. **Device identity** — no-brainer first addition
2. **Hot reconfigure for open ports** — simple, high value, likely easy add
3. **Status/config introspection** — obvious value, low ambiguity
4. **Profile-based target selection** — important, but needs design for minimal user input
5. **Simple statistics** — useful structured counters, no graph-heavy scope

### Tier 2 — high-value, needs more design

6. **Packet decoder** — super useful, needs careful design
7. **Protocol parser** — start simple, built to expand later
8. **Timestamps / log files** — useful, but needs model discussion
9. **Auto reconnect** — definite later add, depends on richer device identity first

### Tier 3 — needs architecture analysis first

10. **Explicit session model** — not important right now, revisit with proper architecture analysis
11. **Expect/script automation** — interesting, but needs higher-level architecture discussion first

### Tier 4 — later or future features

12. **RS-485 options** — definitely worth adding later, but needs serious testing strategy
13. **Filtering/search across captures** — usefulness unclear; discuss versus using grep/glob externally
14. **Recording + replay** — useful but niche
15. **RFC2217 backend support** — valuable later, complex to test
16. **Bridge mode** — powerful, but complex

### Tier 5 — future wish features

17. **Socket sharing / tee / shared live access** — future wish, likely complex
18. **File transfer protocols** — future wish only
19. **Non-intrusive sniffing / proxy observation** — future wish, useful for adjacent tooling

### Explicit skip for now

- **Remote monitor** — skip for now

## Features already listed before this review

These stay on list unless superseded by ranked items below:

- richer matching modes
- multiple public subscriptions per connection
- native_sim test firmware diagnostics
- stronger live TX flush testing support
- better newline / terminal semantics
- safety policies for dangerous commands
- capture bookmarks / annotations
- config import/export
- external decoder/plugin API
- decoder integration / export hooks
- human + agent shared session / tee mode

## Detailed feature notes

### 1. Device identity

**Priority:** Now — likely first new feature

Why:

- unstable `/dev/tty*` numbering hurts agents
- auto reconnect depends on this
- profiles depend on this
- multi-device labs depend on this

What to expose:

- `path`
- `display_name`
- `description`
- `hardware_id`
- `transport` (`usb`, `pci`, `bluetooth`, `unknown`)
- `vid`
- `pid`
- `serial_number`
- `manufacturer`
- `product`
- `interface`
- `location`
- maybe `driver`
- maybe computed stable `fingerprint`

Possible follow-on APIs:

- extend `list_ports`
- allow `open` by `selector`
- later allow named profiles

Design direction:

- expose as much raw metadata as OS provides
- do not overcompress early
- let selectors build on raw fields

### 2. Hot reconfigure for open ports

**Priority:** Now

User decision:

- simple feature
- easy add
- should add

What it means:

- change baud / parity / data bits / stop bits / flow control on open connection
- no close/reopen required

Likely tool shape:

- `reconfigure`
- input: `connection_id` + optional changed fields
- output: previous config + effective new config

Open questions:

- behavior while `read` / `subscribe` active
- rollback on unsupported combination or partial failure
- whether tool accepts only changed fields or full config snapshot

### 3. Status/config introspection

**Priority:** Now

User decision:

- add for sure
- makes sense

Likely tool:

- `get_status` or `get_connection_info`

Useful fields:

- effective config
- connection state (`open`, `disconnected`, `reconnecting` later)
- active subscription state
- byte counters
- truncation / drop counters
- last activity time
- bound device identity metadata
- line state if available

Why important:

- gives agent evidence before acting
- supports debugging
- pairs naturally with reconfigure and reconnect

### 4. Profile-based target selection

**Priority:** Now / Near-term

User decision:

- definite add
- needs discussion
- goal: as little user input as possible; agent configures everything

Why important:

- agent should not need repeated port/config prompts
- profile can hold identity + defaults + policies

Possible profile contents:

- selector rules
- serial defaults
- line ending / terminal defaults
- reconnect policy later
- preferred decoder later
- safety policy hints

Design challenge:

- how profiles are created
- how much user setup needed
- whether agent can propose and save profile after observing device once

Possible direction:

- agent opens raw device first time
- server returns enough identity/config info
- agent/user confirms “save as profile”
- later sessions use profile name only

### 5. Timestamps / log files

**Priority:** Near-term discussion

User decision:

- consider
- discuss how it fits model first

Questions to solve:

- log at connection level, session level, or both
- raw bytes only, rendered text, or event log
- stream to notifications, file, or both
- opt-in vs always-on ring buffer

Useful forms:

- text transcript
- JSONL event log
- raw byte chunks with timestamps

Potential model fit:

- per-connection rolling in-memory log
- optional export tool
- later file sink if user wants persistence

### 6. Expect/script automation

**Priority:** Needs architecture review

User decision:

- interesting
- discuss on higher architecture level before adding

Why:

- can become huge if designed poorly
- can conflict with simpler read/write model
- should not become arbitrary scripting VM too early

Conservative first design if pursued:

- JSON transaction steps only
- bounded step types
- no shell access
- deterministic transcript output

### 7. Socket sharing / tee / shared live access

**Priority:** Wish

User decision:

- seems complicated
- keep as wish feature for future

Clarification:

- not same as current HTTP transport
- HTTP exposes MCP API
- this would expose live serial stream/session to another consumer

### 8. RS-485 options

**Priority:** Later

User decision:

- awesome feature
- future feature
- needs a lot of new testing strategy/firmware

Why separate from UART:

- half-duplex bus semantics
- direction control timing
- RTS-based send control on some platforms

### 9. File transfer protocols

**Priority:** Wish

User decision:

- definitely future wish
- not for now

Scope guard:

- do not turn project into full DFU/flashing suite
- only consider generic serial-native transfer helpers if ever added

### 10. RFC2217 backend support

**Priority:** Later

User decision:

- later feature
- testing looks complex

Meaning:

- server can open remote serial device over network with control signals
- backend transport feature, not MCP transport replacement

### 11. Non-intrusive sniffing / proxy observation

**Priority:** Wish

User decision:

- interesting for developing other tools
- useful for things like DFU observation
- future feature

Most realistic path:

- proxy/bridge observation
- not universal passive sniff of any local serial port

### 12. Packet decoder

**Priority:** Near-term design feature

User decision:

- super useful
- needs discussion on how to add

Purpose:

- split stream into meaningful frames/messages
- support protocol-aware future features

Likely first targets:

- line-based text
- delimiter-based binary/text frames
- length-prefixed frames

Design goal:

- decoder layer simple first
- leaves room for richer parsers later

### 13. Protocol parser

**Priority:** Near-term design feature

User decision:

- add in addition to packet decoder
- simple parser first
- future expansion possible

Distinction:

- decoder finds frame boundaries
- parser interprets frame fields/meaning

Likely first parser candidates:

- line command shell
- AT command responses/URCs
- JSON lines
- very simple tagged text protocols

### 14. Filtering/search across captures

**Priority:** Later discussion

User decision:

- not sure how useful
- maybe LLM can use grep/glob instead
- needs discussion

Key question:

- do structured capture searches add enough beyond file export + grep?

Possible answer later:

- yes if searches include direction, timestamps, event types, parsed fields
- maybe no if plain text logs are enough

### 15. Recording + replay

**Priority:** Later / Wish

User decision:

- useful but niche
- future wish

Why still valuable:

- reproducible bugs
- test fixtures from real hardware
- decoder/parser regression tests

### 16. Simple statistics

**Priority:** Now / Near-term

User decision:

- simple statistics feature makes sense
- good feature

Scope:

- structured counters only
- not full graph UI

Candidate fields:

- rx bytes
- tx bytes
- read operations
- write operations
- truncation count
- dropped notification count
- reconnect count later

### 17. Bridge mode

**Priority:** Later

User decision:

- future consideration
- very complex

Why interesting:

- can support proxy observation
- reverse engineering
- test harnessing

### 18. Explicit skip: Remote monitor

- skip for now
- keep off active roadmap

## Older ideas still worth tracking

### Richer matching modes

**Priority:** Near-term

- regex for `read` / `subscribe`
- glob matching
- keep substring default
- add multi-pattern later
- pair naturally with decoder/parser work

### Multiple public subscriptions per connection

**Priority:** Later

- useful if explicit session model grows later
- requires subscription IDs and fanout semantics

### native_sim test firmware diagnostics

**Priority:** Ongoing

- keep growing firmware diagnostics that make software-only PTY tests stronger
- current useful commands include:
  - `framing on|off`
  - `trace on|off`
  - `write cmd <id> <rest>`
  - `arm_cmd <delay_ms>`
  - `slow on|off [<us>]`
  - `touch`

### Stronger live TX flush testing support

**Priority:** Ongoing

- add RX throttle / holdoff style firmware support
- improve proof of `flush(target="output")` behavior

### Better newline / terminal semantics

**Priority:** Later

- configurable newline mode
- terminal-oriented output normalization

### Safety policies for dangerous commands

**Priority:** Later

- optional dangerous-command confirmation patterns

### Capture bookmarks / annotations

**Priority:** Later

- useful if logs/captures land first

### Config import/export

**Priority:** Later

- likely pairs with profiles

### External decoder/plugin API

**Priority:** Later

- useful after first simple decoder/parser lands

### Decoder integration / export hooks

**Priority:** Later

- export capture or frames to external decoder tools if in-process support stays small

### Human + agent shared session / tee mode

**Priority:** Wish

- overlaps with socket sharing

## Recommended execution order

### Phase 1

1. device identity
2. hot reconfigure
3. status/config tool
4. simple statistics in status output

### Phase 2

5. profile-based target selection
6. richer matching modes
7. initial timestamp/log model discussion and minimal implementation

### Phase 3

8. packet decoder
9. simple protocol parser
10. auto reconnect

### Phase 4

11. explicit session architecture review
12. expect/script automation design
13. decide capture search vs external grep model

### Phase 5

14. RS-485
15. RFC2217
16. bridge/proxy observation
17. recording/replay

### Long-shot / wish list

18. socket sharing
19. file transfer helpers
20. non-intrusive sniffing
