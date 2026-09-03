# Future design: continuous raw capture lifecycle

Status: planned (deferred) — tracked by the recording + replay entry in
backlog item `PB-025` (`docs/product/backlog/active/PB-025-recording-and-replay.org`). It is not implemented. No
`start_capture`/`stop_capture` tool exists. Current code ships only the safe
persistent capture foundation (`--capture-dir` + `CaptureStore` + hardened
`export_log`).

This document describes the lifecycle that a future continuous raw-capture
feature would need. It must use the existing containment, symlink, quota,
atomicity, and lock policy rather than reintroduce unrestricted writes. This is
not a plan to implement the feature now. See the conclusion.

## Registry and IDs

- Process-wide `CaptureRegistry`, like `CaptureStore` and `ProfileStore`,
  injected through `SerialHandlerOptions`/builder into every stdio/HTTP
  session.
- Stable capture IDs use `capture:<connection_id>:<nonce>`. Initially, there is
  one capture per connection, keyed by connection ID. The ID stays stable across
  reconnect and server restart, as described under orphan recovery, until the
  capture stops.
- A capture is owned by its connection's lifecycle: closing the connection
  or shutting down the server must bound-stop and finalize the capture.

## States

- `starting`: registration is done, but the live-edge mark is not committed
- `running`: segments are being written
- `stopping`: a bounded stop was requested; segments are being flushed and
  finalized
- `stopped`: the final segment is finalized and the result is available
- `failed`: terminal error caused by quota, I/O, or ring overrun

Transitions are `starting → running` after the mark is committed,
`running → stopping → stopped`, and any of `starting`, `running`, or `stopping`
to `failed` on an unrecoverable error. Stop is idempotent. A second stop during
`stopping` waits for the same final result.

## Tools

These are future tools. They are not built.

- `start_capture(connection_id, ...)` registers the capture and atomically
  marks the live edge. It uses the same private-cursor and pump-gate semantics
  as `capture_boot`. It returns the capture ID. Cancellation before the start
  response leaves NO orphan task.
- `stop_capture(capture_id)` performs an idempotent bounded stop and finalizes
  the capture.
- `capture_status(capture_id)` and `list_captures()` report offsets, bytes
  observed, written, and lost, segments, quota usage, and errors.

## Data path

- Private RX cursor: capture never moves the shared `read` cursor or ring
  history. The live-edge mark is atomic under the `pump_gate`, so no pre-start
  byte can be captured.
- Raw bytes are stored exactly as received. Framing, parser, and protocol
  processing are excluded initially. There is no per-frame transformation and
  no parser error in the pipeline.
- Bounded queue with explicit backpressure: a bounded channel connects the
  pump and writer. On overflow, the capture records `bytes_lost` and a
  `ring_overrun` gap. NEVER allow silent loss.

## Stop reasons

`match` (pattern found), `timeout`, `silence` (`no_new_rx_timeout_ms`),
`cancelled`, `quota` (file/total/count), `io` (write failure),
`ring_overrun`, `connection_closed`, `server_shutdown`.

## Disconnect and reconnect

- Initially, disconnect stops the capture. It finalizes what was captured and
  records `connection_closed`. Reconnect continuation, meaning pause and
  resume of the same capture, is deferred. It needs its own gap and lost-byte
  semantics and is not assumed.

## Rotation, finalization, and orphans

- Rotate segments before the per-file quota is reached. Use the SAME advisory
  lock, quotas, and `persist_noclobber` finalization as `CaptureStore`
  `write_new`. One lock scope covers scanning, quota checks, temp-file creation,
  and renaming the closed segment.
- Internal partial-file names for in-progress segments carry the reserved
  `.serial-mcp-capture-` prefix. They COUNT toward file and total quotas while
  open. They also have an explicit orphan policy. On restart, recognize and
  validate reserved-prefix entries, then finalize them by renaming to a managed
  `.jsonl` name or discard them according to policy. Never delete arbitrarily.
- Cancellation before the start response leaves no orphan task and no file.

## Trust, durability, and cleanup

- The trust boundary is the same as `CaptureStore`. The configured root and
  its ancestors are operator-controlled. Advisory locks protect cooperating
  serial-mcp processes only.
- Do not automatically delete completed captures until a deterministic
  retention policy exists. `CaptureStore` deliberately never deletes.

## Testing a future implementation would require

- cancellation before start response (no orphan task/file)
- disconnect mid-capture (bounded stop + finalize)
- ring wrap with explicit `bytes_lost` gap
- disk-full / write failure (`io` stop, capture remains queryable)
- per-file quota rotation and total/count quota exhaustion
- restart with orphan partial segments (finalize or discard per policy)
- concurrent clients (registry lookup, stop idempotence)
- quota accounting across cooperating processes (advisory lock)

## Conclusion

Current evidence supports bounded in-memory boot capture (`capture_boot`), not
continuous disk capture. The safe persistent capture foundation removes the
last unrestricted filesystem write and supplies the lifecycle's needed
primitives: containment, symlink policy, portable names, quotas, advisory
locks, and atomic no-clobber finalization. There is still no concrete task
evidence for an always-on disk capture stream.

Recommendation: do not implement continuous capture until concrete task
evidence exists. If such evidence arrives, implement the lifecycle
incrementally with a registry, start/stop/status, one capture per connection,
segments, and an orphan policy. Reuse `CaptureStore` as the commit primitive
and `capture_boot`'s private-cursor and pump-gate semantics as the mark
primitive.
