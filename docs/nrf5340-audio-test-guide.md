# nRF5340 Audio Device Test Guide

This guide uses the E83 nRF5340 audio receiver board on `/dev/ttyUSB0` to
exercise real serial-mcp behavior under heavy logging.

It is designed to validate:
- open / close lifecycle
- `read`
- `subscribe`
- `match` on `read`
- pre-match context shaping
- `max_buffered_bytes`
- `timeout_ms`
- `no_new_rx_timeout_ms`
- final stop metadata / notification behavior

The device-side log source is the I2S debug stress test documented in
`~/repos/le-audio-receiver/AGENTS.md`.

## Device Setup

- Board: E83 custom nRF5340 board
- Serial console: `/dev/ttyUSB0`
- Baud: `115200 8N1`

Stress logging commands:

- Start flood:
  - `audio i2s-test`
- Stop flood:
  - `audio stop`

Expected high-rate log lines include:
- `Queued TX`
- `supply_next_buffers`

## Safety Notes

- Always stop the stress test with `audio stop` before ending the session.
- If you suspect stale log traffic, run a `flush(target="both")` before the next test.
- If a test intentionally starts a background `subscribe`, remember to `unsubscribe`
  or `close` the connection after verification.

## Recommended Test Flow

### 1. Port discovery

Goal:
- confirm the expected UART is visible

Tool flow:
1. `list_ports`
2. confirm `/dev/ttyUSB0` appears

### 2. Basic open / close

Goal:
- confirm connection lifecycle and metadata

Tool flow:
1. `open(port="/dev/ttyUSB0", name="e83-uart", baud_rate=115200)`
2. `list_connections`
3. confirm returned connection is listed with expected port and settings
4. `close(connection_id=...)`

### 3. Start and stop stress logging

Goal:
- confirm the board is producing a sustained RX stream

Tool flow:
1. `open(...)`
2. `write(data="audio i2s-test\r\n")`
3. `read(max_buffered_bytes=2048, timeout_ms=2000)`
4. confirm returned data contains `Queued TX` or `supply_next_buffers`
5. `write(data="audio stop\r\n")`
6. `close(...)`

### 4. Buffer stress test with `read`

Goal:
- confirm `max_buffered_bytes` stops reads cleanly under heavy traffic

Tool flow:
1. `open(...)`
2. `write("audio i2s-test\r\n")`
3. `read(max_buffered_bytes=256, timeout_ms=5000)`
4. confirm:
   - result is successful
   - `stop_reason` is `max_buffered_bytes` or another expected stop reason if timeout wins first
   - returned `data` length is bounded
5. `write("audio stop\r\n")`
6. `close(...)`

Good follow-up:
- repeat with `max_buffered_bytes=2048`
- repeat with `max_buffered_bytes=65536`

### 5. Match-on-read

Goal:
- confirm `read(match=...)` stops immediately on first match

Tool flow:
1. `open(...)`
2. `write("audio i2s-test\r\n")`
3. call `read` with:

```json
{
  "connection_id": "...",
  "timeout_ms": 3000,
  "max_buffered_bytes": 4096,
  "match": {
    "pattern": "Queued TX",
    "config": {
      "mode": "literal_substring",
      "pattern_encoding": "utf8"
    }
  }
}
```

4. confirm:
   - `matched = true`
   - `match_index` is not `null`
   - `stop_reason = match_found`
5. `write("audio stop\r\n")`
6. `close(...)`

### 6. Pre-match context shaping

Goal:
- confirm `context_amount_of_matched_bytes` returns bytes before the match plus the match itself

Tool flow:
1. `open(...)`
2. `write("audio i2s-test\r\n")`
3. call `read` with:

```json
{
  "connection_id": "...",
  "timeout_ms": 3000,
  "max_buffered_bytes": 4096,
  "match": {
    "pattern": "Queued TX",
    "config": {
      "mode": "literal_substring",
      "pattern_encoding": "utf8",
      "context_amount_of_matched_bytes": 32
    }
  }
}
```

4. confirm:
   - `matched = true`
   - returned `data` starts up to 32 bytes before the match
   - returned `data` includes `Queued TX`
   - `match_index` points at the start of `Queued TX` within the shaped payload
5. `write("audio stop\r\n")`
6. `close(...)`

### 7. Background subscribe under flood

Goal:
- confirm background streaming works under sustained traffic

Tool flow:
1. `open(...)`
2. `subscribe(max_buffered_bytes=2048, poll_interval_ms=50)`
3. `write("audio i2s-test\r\n")`
4. observe `notifications/message` with logger like `serial:<connection_id>`
5. confirm repeated data notifications arrive
6. `write("audio stop\r\n")`
7. `unsubscribe(...)` or `close(...)`

### 8. Subscribe match stop

Goal:
- confirm `subscribe(match=...)` streams data and then stops on first match

Tool flow:
1. `open(...)`
2. `subscribe` with:

```json
{
  "connection_id": "...",
  "max_buffered_bytes": 4096,
  "match": {
    "pattern": "Queued TX",
    "config": {
      "mode": "literal_substring",
      "pattern_encoding": "utf8",
      "context_amount_of_matched_bytes": 32
    }
  }
}
```

3. `write("audio i2s-test\r\n")`
4. confirm:
   - normal stream notifications arrive first
   - final stop notification reports `stop_reason = match_found`
   - final notification includes shaped payload with pre-match context and match
5. `write("audio stop\r\n")` if the board is still flooding
6. `close(...)`

### 9. Silence timeout

Goal:
- confirm `no_new_rx_timeout_ms` stops cleanly without being treated as transport failure

Tool flow:
1. `open(...)`
2. ensure board is quiet or send `audio stop\r\n`
3. call `read` with:

```json
{
  "connection_id": "...",
  "timeout_ms": 5000,
  "no_new_rx_timeout_ms": 300,
  "max_buffered_bytes": 2048
}
```

4. confirm:
   - result is successful
   - `stop_reason = no_new_rx_timeout`
   - returned data may be empty
5. close connection

Repeat with `subscribe`:
- final notification should report `stop_reason = no_new_rx_timeout`

### 10. Close while active

Goal:
- confirm active RX operations terminate correctly on close

Tool flow:
1. `open(...)`
2. start `subscribe(...)` or `read(...)`
3. trigger `audio i2s-test`
4. `close(...)` while activity is ongoing
5. confirm final notification / result reflects connection closure semantics

### 11. Final board health check

Goal:
- confirm stress test did not destabilize audio pipeline

Tool flow:
1. `open(...)`
2. `write("audio status\r\n")`
3. `read(max_buffered_bytes=2048, timeout_ms=1500)`
4. confirm `I2S underruns: 0`
5. `close(...)`

## Stress Test Ideas

- Run repeated `subscribe` / `unsubscribe` loops during `audio i2s-test`
- Repeatedly replace a subscription with a new one using different `match` config
- Sweep `max_buffered_bytes` from very small to large values
- Trigger `read(match=...)` while another `subscribe` is active to verify shared RX pump behavior
- Use `context_amount_of_matched_bytes = 0` to confirm match-only payload shaping

## Expected Good Signs

- no stale bytes after close/reopen
- no spurious tool errors on normal stop conditions like `timeout`,
  `max_buffered_bytes`, or `no_new_rx_timeout`
- `match_index` always points to the matched text in returned/shaped payload
- stop metadata remains internally consistent

## Cleanup Checklist

After the session:
1. send `audio stop`
2. `unsubscribe` active subscriptions if any remain
3. `close` all open serial connections
