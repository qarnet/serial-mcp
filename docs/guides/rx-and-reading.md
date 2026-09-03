# RX and reading data

This guide explains how `read`, `transact`, and `capture_boot` consume incoming
bytes. It covers the always-on ring, shared cursor, replay, matching, timeouts,
lossless encoding, and boot capture.

## The always-on RX ring

Every connection has an always-on ring buffer that captures every byte from
`open` to `close`, whether or not a tool is reading. As a result:

- buffered unread bytes return immediately;
- those bytes remain available for replay until the ring wraps;
- `read` can wait for new data, match a pattern, or stop on silence.

The ring's retention capacity is fixed at open time by `rx_buffer_size`, the
connection's RX ring size. The default is 256 KiB, and resizing requires a
reopen. Bytes older than the retained window are discarded. Ring wrap is always
reported through `bytes_lost` and is never silent.

## The shared read cursor and `from`

`read` reads from the ring through a shared connection cursor. The cursor
advances only when `read` consumes bytes, so a later `read` without `from`
continues where the previous call stopped. Resource subscriptions,
`subscriptions/listen`, never move the shared cursor.

The `from` parameter selects a start position without discarding data. Use one
of these tagged wire forms:

| Form | Meaning |
|---|---|
| `{"type":"cursor"}` | Start at the shared read cursor, the default for `read` |
| `{"type":"now"}` | Jump to the live edge, skip the buffered backlog, and receive only new data |
| `{"type":"buffer_start"}` | Replay everything retained in the ring, then go live |
| `{"type":"offset","offset":N}` | Start at an absolute stream offset from a prior result's `from_offset` or `next_offset` |

There is no string shorthand. Always send the tagged object. The `transact`
read half defaults to `{"type":"now"}` so it waits only for post-write bytes.

Results include the offsets needed to track position:

- The read covers stream offsets in `from_offset` and `next_offset`.
- The retained range is in `start_offset` and `end_offset`.
- `bytes_lost` reports the unavailable gap when the requested start was before
  the ring's current `start_offset`. It equals `start_offset` minus the
  requested position. It is nonzero only after the ring wrapped past that
  position.
- `buffered_remaining` reports unread bytes left in the ring.

Passing the same `from` value again reads the same bytes until the ring wraps.
Replayed history uses the same framing, parsing, and matching pipeline as live
data.

## Waiting for timeouts, silence, and matches

`read` waits when no buffered data satisfies its stop condition:

- The `timeout_ms` value sets the total wall-clock budget for the call.
- The `no_new_rx_timeout_ms` value sets the silence timeout. The timer starts
  immediately. It resets on every received byte.
- The `match` setting stops when a pattern is found. Buffered history is checked
  first. The call then waits for live data. The configuration includes the
  pattern, `mode`, and `pattern_encoding`.
  `mode` can be `literal_substring`, `regex`, or `glob`. `glob` matches a whole
  line. `pattern_encoding` can be `utf8`, `hex`, or `base64`.

When a match stops the read, the result includes `matched: true` and
`match_index`. The optional `context_amount_of_matched_bytes` setting returns up
to N bytes before the match in the payload.

Matching and framing interact by design. In raw mode, matching uses a sliding
window over the byte stream, so a pattern split across two RX chunks is still
found. With `rx_framing`, each decoded frame is matched separately, so a pattern
spanning two frames is not matched.

## Ring wrap and `bytes_lost`

The ring's retention window is `rx_buffer_size` bytes. It is fixed at open. If
the device outpaces reads and the ring wraps, a read whose requested start is
older than the retained window reports `bytes_lost` as the unavailable gap.
That gap is `start_offset` minus the requested position. Those bytes have
already been discarded, and data loss is never silent.

Increase `rx_buffer_size` at open or read sooner and more often. Modern
`subscriptions/listen` notifications only wake the caller. They do not consume
or preserve payload bytes.

## Lossless encoding fallback

RX payloads are lossless. The requested `encoding` defaults to `utf8`.
If it cannot represent received bytes, such as binary data under `utf8`, the
server re-encodes the same bytes as exact lowercase spaced hex. It reports
`encoding: "hex"` for that payload.

Bytes are never dropped, repeated, or lossily converted. A successful fallback
is never counted as a dropped notification or frame. This applies to raw reads,
decoded frames, and match context payloads.

## Hardware flow-control caveat

With hardware flow control, RTS/CTS, enabled, the always-on pump continuously
drains the kernel RX buffer. The kernel therefore never deasserts RTS. The
device streams freely.

A setup that relied on flow control to pause the device until the host read will
behave differently. The device no longer pauses.

## Atomic boot and reset capture with `capture_boot`

Use `capture_boot` for an Arduino auto-reset, power-cycle banner, or boot prompt.
It replaces the racy arm, reset, and read sequence with one operation:

1. Purge unread OS input by flushing the OS receive buffer.
2. Mark the RX live edge atomically under the pump gate. No byte physically read
   before the reset can append after the mark.
3. Optionally pulse DTR/RTS. `reset` asserts the configured lines for `hold_ms`.
   It always releases them on normal completion, cancellation, or failure.
   `reset=null` arms the capture without touching a line. Use this for externally
   reset or power-cycled devices.
4. Capture only post-mark bytes through the same match, framing, parser, timeout,
   and silence pipeline as `read`. The operation uses a private cursor. The
   shared `read` cursor and ring history remain untouched.

The result stays in memory. `max_buffered_bytes` bounds it as the in-memory
read-result cap. It does not write a file. `read.from_offset` equals
`mark_offset` unless the ring wrapped. In that case, `bytes_lost` reports the
gap. An omitted `timeout_ms` resolves to a bounded 5000 ms default. Capture is
transient and never triggers profile learning.

> A configured `reset` may reboot hardware. Treat `capture_boot` as destructive.

## Resource subscriptions (`subscriptions/listen`)

Modern MCP clients may use `subscriptions/listen` as an optional wakeup
mechanism. Subscribe to `serial://ports`, `serial://connections`, or a concrete
`serial://connections/{id}[/raw|/log]` URI.

The server sends notifications for port hotplug, open and close, appended RX
bytes, or cleared logs. Notifications are hints only. They carry no serial
payloads and never move the read cursor. `read` remains the primary lossless
data path.
