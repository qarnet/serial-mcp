# RX and Reading Data

How `read` (and its close relatives `transact` and `capture_boot`) consume the
incoming byte stream: the always-on ring, the shared cursor, replay, matching,
timeouts, lossless encoding, and the boot-capture path.

## The always-on RX ring

Every connection keeps an **always-on ring buffer** that captures every byte
from `open` to `close`, whether or not any tool is currently reading. This is
what makes `read` feel like `cat`:

- buffered-but-unread bytes are returned **immediately**;
- the same bytes stay available for replay until the ring wraps;
- a `read` can also wait for new data, match a pattern, or stop on silence.

The ring's retention capacity is **fixed at open time** by `rx_buffer_size`
(the connection's RX ring size, default 256 KiB; open-time only, reopen to
resize). Bytes older than the retained window are discarded; ring wrap is
always observable through `bytes_lost` (see below) — never silent.

## The shared read cursor and `from`

`read` reads from the ring via a **shared cursor** on the connection. The
cursor advances only when `read` consumes bytes, so a later `read` without a
`from` continues where the previous one stopped. Resource subscriptions
(`subscriptions/listen`) never move the shared cursor.

`read`'s `from` parameter resolves the start position **non-destructively** and
uses exactly one of these tagged wire forms:

| Form | Meaning |
|---|---|
| `{"type":"cursor"}` | Start at the shared read cursor (the default for `read`) |
| `{"type":"now"}` | Jump to the live edge — skip the buffered backlog, only new data |
| `{"type":"buffer_start"}` | Replay everything retained in the ring, then go live |
| `{"type":"offset","offset":N}` | Start at an absolute stream offset from a prior result's `from_offset` / `next_offset` |

There is no string shorthand — always send the tagged object. `transact`'s read
half defaults to `{"type":"now"}` so it only awaits post-write bytes.

Results report the offsets you need to reason about position:

- `from_offset` / `next_offset` — the stream offsets this read covered;
- `start_offset` / `end_offset` — the retained range of the ring;
- `bytes_lost` — the unavailable gap when the requested start position fell
  before the ring's current `start_offset`; the value is `start_offset` minus
  the requested position (nonzero only when the ring wrapped past it);
- `buffered_remaining` — unread bytes left in the ring.

Re-passing the same `from` re-reads the same bytes: reading is
non-destructive until the ring wraps. Replayed history flows through the same
framing, parsing, and matching pipeline as live data.

## Waiting: timeouts, silence, and match

`read` waits when no buffered data satisfies the stop condition:

- `timeout_ms` — total wall-clock budget for the call.
- `no_new_rx_timeout_ms` — silence timeout: stop when no new data arrives
  within this window. The timer starts immediately and resets on every
  received byte.
- `match` — stop when a pattern is found. Buffered history is checked first,
  then the call waits for live data. `match` configuration carries the pattern
  plus `mode` (`literal_substring`, `regex`, or `glob` — a whole-line match)
  and `pattern_encoding` (`utf8`, `hex`, or `base64`).

When a `match` stops the read, the result includes `matched: true` and
`match_index`. Optional `context_amount_of_matched_bytes` in the match config
returns up to N bytes before the match in the payload.

**Matching and framing interact by design.** In raw mode, matching is
sliding-window over the byte stream, so a pattern split across two RX chunks is
still found. In framed mode (with `rx_framing`), each decoded frame is matched
individually — a pattern spanning two frames is intentionally not matched.

## Ring wrap and `bytes_lost`

The ring's retention window is `rx_buffer_size` bytes, fixed at open. When the
device outpaces reads and the ring wraps, a read whose requested start position
is older than the retained window reports `bytes_lost` equal to the unavailable
gap (`start_offset` minus the requested position) — those bytes were already
discarded. Data loss is never silent. Remedies: increase `rx_buffer_size` at
open, or read sooner/more often. Modern `subscriptions/listen` resource
notifications only wake the caller; they do not consume or preserve payload
bytes.

## Lossless encoding fallback

RX payloads are **lossless**: when the requested `encoding` (default `utf8`)
cannot represent received bytes — for example binary data under `utf8` — the
server re-encodes the **same bytes** as exact lowercase spaced hex and reports
`encoding: "hex"` on that payload. Bytes are never dropped, repeated, or
lossy-converted, and a successful fallback is never counted as a dropped
notification or frame. This applies to raw reads, decoded frames, and match
context payloads.

## Hardware flow-control caveat

With hardware flow control (RTS/CTS) enabled, the always-on pump drains the
kernel RX buffer continuously, so the kernel never deasserts RTS and the device
streams freely. A setup that relied on flow control to pause a device until the
host reads will behave differently — the device no longer pauses.

## `capture_boot`: the atomic boot/reset path

For boot and reset capture (Arduino auto-reset, power-cycle banner, boot
prompt) use `capture_boot` — one atomic call instead of the racy
arm-then-reset-then-read composition:

1. **Purge unread OS input** — the OS receive buffer is flushed first.
2. **Mark the RX live edge atomically** — the mark happens under the pump gate,
   so no byte physically read before the reset can leak in after the mark.
3. **Optionally pulse DTR/RTS** — `reset` asserts the configured lines for
   `hold_ms`, then always releases them, on normal completion, cancellation,
   or failure. `reset=null` arms the capture without touching any line, for
   externally reset or power-cycled devices.
4. **Capture only post-mark bytes** through the same match/framing/parser/
   timeout/silence pipeline as `read`, from a **private cursor**. The shared
   `read` cursor and ring history are untouched.

The result stays **in memory**, bounded by the connection's
`max_buffered_bytes` (the in-memory read-result cap) — there is no file output.
`read.from_offset` equals
`mark_offset` unless the ring wrapped (then `bytes_lost` reports it). Omitted
`timeout_ms` resolves to a bounded 5000 ms default. Capture is transient: it
never triggers profile learning.

> `capture_boot` with a configured `reset` may reboot hardware — treat it as
> destructive.

## Resource subscriptions (`subscriptions/listen`)

Modern MCP clients may use `subscriptions/listen` as an optional wakeup
mechanism: subscribe to `serial://ports`, `serial://connections`, or a concrete
`serial://connections/{id}[/raw|/log]` URI and the server notifies you when
that resource changes (port hotplug, open/close, RX bytes appended, log
cleared). Notifications are **hints only** — they never carry serial payloads
and never move the read cursor. `read` remains the primary lossless data path.
