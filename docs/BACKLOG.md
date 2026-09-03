# Product backlog

Planned and in-progress work that is not yet a shipped feature. Each entry
links to its plan document under [development/plans/](development/plans/)
when one exists; entries without a plan are one-paragraph ideas.

Priorities: **In progress** · **Near-term** · **Later** · **Wish** ·
**Infrastructure**.

| Status | Entry | Reference |
| --- | --- | --- |
| In progress | Server-runtime ownership: one `SerialServerRuntime` per server (shared TX queue, reconnect supervisor, deterministic shutdown), cross-process port leases, platform portability | [stateless-http-runtime-plan.md](development/plans/stateless-http-runtime-plan.md) |
| Near-term | CI Nix timing measurement and cache investigation: record cold/warm restore-save and `nix flake check` durations, inspect 8 GiB store-cache retention, before any further Nix tuning | (scope below) |
| Near-term | Windows native serial-open gap: decide the `mio-serial` close/reopen contract before claiming external-program exclusion | [stateless-http-runtime-plan.md](development/plans/stateless-http-runtime-plan.md) (ownership research) |

## Features

| Status | Entry | Reference |
| --- | --- | --- |
| Near-term | Local-only usage statistics for development: payload-free tool-call/stop-reason/option-usage records, strictly local and opt-in, to drive evidence-based interface trimming | — |
| Near-term | Declarative checksums on generic framing: `checksum: { algorithm, ... }` on `Delimiter` / `LengthPrefixed` / `StartEnd` — generalizes what the NMEA/Modbus presets hardcode | — |
| Near-term | TX pacing / throttling: inter-chunk or inter-line delay on `write` (per-call field + connection default) for bootloaders, GRBL, cheap AT modems | — |
| Near-term | Modbus ASCII TX auto-LRC: hex-encode a binary PDU and append the LRC on write; extract a shared TX checksum-append layer when it lands (one-consumer rule) | — |
| Near-term | Config import/export: export a *running* server's full state (open connections + framing/parser defaults) as importable profiles — needs sharpening to earn implementation | — |
| Near-term | External decoder/plugin API: pluggable frame decoders/parsers; ship declarative checksums first (lighter, covers much of the demand) | — |
| Near-term | Decoder integration / export hooks: export capture or frames to external decoder tools if in-process support stays small | — |
| Later | MCP Tasks extension (SEP-2663) for long-running operations: task handles for long `read`/`transact`/`capture_boot`, `tasks/get`/`update`/`cancel`; needs ownership/lifecycle design | — |
| Later | Positive MCP cache TTL policy: only after list-notification invalidation, authorization partitioning, pagination keys, and stale-on-error tests exist | — |
| Later | Standard HTTP parameter headers (`Mcp-Param-*`) via `x-mcp-header` annotations; first candidate `connection_id`; never promote payloads or credentials into headers | — |
| Later | Flow-control-aware ring backpressure (`on_full: "wrap" | "pause"`): restore hardware RTS/CTS backpressure semantics with observable paused-state events | — |
| Later | Persistent per-connection framing decoder: carrying decoder state across `read` calls; requires binding framing to the connection and rethinking 4-layer precedence | — |
| Later | Per-client RX cursors: named cursor groups if shared multi-agent access becomes real; overlaps "Socket sharing / tee" | — |
| Later | Baud-rate auto-detection: deferred — host-side detection over USB-serial is heuristic, not waveform measurement; a built-in tool should return inconclusive rather than guess; EXPLIoT `uart.generic.baudscan` is the reference to study first | — |
| Later | Modem input lines + UART error counters in `get_status`: read CTS/DSR/CD/RI; parity/framing/overrun counters; cheap additive fields | — |
| Later | Per-frame timestamps: correlating frames against the event log or across connections; small additive wire-format change — decide before 1.0 | — |
| Later | GRBL / G-code preset: line-based `ok`/`error` protocol; nearly free once TX pacing lands | — |
| Later | Safety policies for dangerous commands: optional confirmation patterns, incl. the profile-level safety-policy intent from the removed `ProfileDefaults.safety_policy` field | — |
| Later | Capture bookmarks / annotations: useful if logs/captures grow further | — |
| Later | Expect/script automation *(needs architecture review)*: conservative first design — JSON transaction steps only, bounded step types, no shell access; the shipped `transact` is the minimal kernel — revisit whether scripting is still needed | — |
| Later | Filtering/search across captures: unclear value — worth it only if searches include direction, timestamps, event types, parsed fields | — |
| Later | Recording + replay: reproducible bugs, test fixtures from real hardware, decoder regression tests | [safe-continuous-capture-design.md](development/plans/safe-continuous-capture-design.md) (foundation) |
| Later | RS-485 options: half-duplex bus semantics, direction control timing, RTS-based send control; needs physical half-duplex testing | — |
| Later | RFC2217 backend support: remote serial device over network (backend transport, not MCP transport replacement) | — |
| Later | Bridge mode: proxy observation, reverse engineering, test harnessing — very complex | — |
| Later | MCP Bundle (MCPB) distribution: package native release binaries for one-click stdio install; separate release/distribution project — decide cross-platform vs per-platform bundles, signing, update flow first | — |
| Wish | Earlier MCP protocol revisions (pre-2025-11-25): only with concrete client demand; each needs an explicit policy row, lifecycle/capability/cache review, raw-wire tests, conformance, and a real historical client fixture | [mcp-version-compatibility-policy.md](development/mcp-version-compatibility-policy.md) |
| Wish | User-facing loopback / virtual port backend: expose a virtual echo/scripted device as an openable backend (the Rust PTY fixture stays test-only) | — |
| Wish | Socket sharing / tee / shared live access: expose a live serial stream to another consumer — complicated, keep as future wish | — |
| Wish | File transfer protocols: no full DFU/flashing suite; only generic serial-native transfer helpers if ever | — |
| Wish | Non-intrusive sniffing / proxy observation: most realistic path is proxy/bridge, not universal passive sniff | — |
| Wish | Human + agent shared session / tee mode: overlaps socket sharing | — |

Explicit skip for now (not backlog entries): MRTR product flows (revisit only
for a concrete elicitation need such as power-cycle guidance; echoed
`requestState` must be integrity-protected), remote monitor, and
SECURITY.md / vulnerability disclosure (revisit if outside contributors
arrive).

## Infrastructure / tech debt

| Status | Entry | Reference |
| --- | --- | --- |
| Near-term | `UInt` newtype to kill schemars `uint_schema` boilerplate: per-field annotations are a known bug vector (b12b09fd, bc37a0b0, PortInfo miss); a newtype or global schemars visitor would collapse the class; coordinate with any schemars 2.x migration | — |

## Entry lifecycle

- A plan or research document lives under `docs/development/plans/` while its
  backlog entry is pending or in progress, and the backlog references it.
- When the work ships, the entry moves out of the table and the plan document
  is deleted; `AGENTS.md`, `CHANGELOG.md`, and the guides under `docs/` own
  shipped behavior.
- Abandoned work: delete the entry together with its plan document.
- Decision records that outlive their plan (ADRs, investigations with a
  durable verdict such as
  [windows-serial-e2e-investigation.md](development/windows-serial-e2e-investigation.md))
  do not live in `plans/` and may stay.

## Nix measurement scope

From the implemented-and-removed CI runtime-reduction plan, kept so the
follow-up is self-contained:

1. Record cache restore/save duration and `nix flake check` duration over
   several clean runs and runs with only source changes.
2. Inspect whether the 8 GiB Nix-store cache retains dependency outputs or is
   repeatedly evicted under the repository cache quota.
3. Record cold versus warm time before changing cache keys, store limits, or
   source filtering.