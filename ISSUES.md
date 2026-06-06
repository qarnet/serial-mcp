# serial-mcp Issues

Issues encountered during a real embedded bring-up session with nRF5340,
logic analyzer, and two different UARTs across USBIP.

---

## 1. No way to list open connections

**What was done:** Over a multi-hour session I opened 4--5 serial connections to
`/dev/ttyUSB0` and `/dev/ttyACM0`. Some were explicit opens, some were
leftovers from earlier test cycles. I had no way to inspect which connections
were alive.

**Why it matters:** I tracked connection IDs by copy-pasting UUIDs into the
conversation context. When a port failed with "already open", I guessed which
UUID held it. Once I closed the wrong connection because I misremembered the
ID from 20 minutes earlier.

**Want:** A `list_connections` operation that returns all active connections
with at minimum `connection_id`, `port`, `baud_rate`, and ideally a
human-readable `name`.

---

## 2. UUIDs are the only human-facing identifier

**What was done:** Every connection was identified by UUIDs like
`ede182c8-bd83-4afb-8c63-43fb0b08ca0f`. In a session with two UARTs
(E83 console and PicoProbe), I had to mentally map UUIDs to physical ports.

**Why it matters:** UUIDs are unreadable. My tool-call logs filled with
unrecognizable strings. Debugging which connection I was reading from 10
turns back meant scrolling up and matching UUIDs manually.

**Want:** I want to pass a `name` string on `open` (e.g. `"e83-uart"` or
`"dk-jlink-vcom"`). That name should appear in `list_connections`, read
responses, and error messages.

---

## 3. Port conflict with other processes is opaque

**What was done:** After opening `/dev/ttyUSB0` through serial-mcp, I tried
`cat /dev/ttyUSB0` from bash to send a quick command. Got "Device or resource
busy" with no indication of *who* holds it.

**Why it matters:** In a session with multiple agents/tools touching serial,
the "busy" error tells you nothing. I wasted time checking `lsof` and
`fuser`, then remembered serial-mcp held it. If I hadn't remembered, I would
have killed the wrong process.

**Want:** When `open` fails because a port is already in use, the error
message should name which `connection_id` (and `name`, if set) currently
owns it.

---

## 4. CTS flow control stalls silently — no escape hatch visible

**What was done:** The E83 custom board uses CTS (P0.21) for UART flow
control. During testing, reads from `/dev/ttyUSB0` returned zero bytes
without any error. I assumed the board crashed or the serial bridge died.
It wasn't — CTS was de-asserted and the UART TX was blocked.

**Why it matters:** Silent data loss during bring-up is the worst kind of
failure. I spent minutes debugging firmware that wasn't broken. A flow-control
indicator or an override would have pointed me at the real problem immediately.

**Want:** An affordance to disable flow control on an open connection. This
could be an `open` parameter, a dedicated tool, or `set_dtr_rts` with
documented semantics for RTS/CTS. I want to be able to say "ignore hardware
flow control for this session" without editing kernel settings.

---

## 5. `wait_for` pattern semantics are unclear

**What was done:** I avoided `wait_for` throughout the entire session and
used poll-style `read` + timeout loops instead. Every reference doc says
"pattern" but no one says whether it's a regex, a substring, a glob, or an
exact match.

**Why it matters:** In a bring-up scenario, `wait_for` is the most valuable
tool — wait for boot banner, wait for shell prompt, wait for error message.
Without confidence in what "pattern" means, I cannot trust it will match
when I need it to.

**Want:** `wait_for` documentation to state exactly what kind of matching is
used: substring, regex, glob, or exact. A single sentence in the response
schema or README is sufficient.

---

## 6. Timeout behavior across close-and-reopen

**What was done:** I closed a connection while a read was in flight (timeout
set, waiting for data). After closing, I reopened the same port. The new
connection sometimes received stale bytes from the old session.

**Why it matters:** Reopening a port after a timeout close should give a
clean buffer. Getting old data confuses state machines that expect a fresh
boot log or a known prompt sequence.

**Want:** `close` to flush the RX buffer before releasing the port, so a
subsequent `open` starts from an empty FIFO.
