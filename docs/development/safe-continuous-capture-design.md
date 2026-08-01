# Future Design — Continuous Raw Capture Lifecycle

**Status: design only. NOT implemented. No `start_capture`/`stop_capture`
tool exists; Phase 6 ships only the safe persistent capture *foundation*
(`--capture-dir` + `CaptureStore` + hardened `export_log`).**

This document specifies the lifecycle a future continuous raw-capture
feature would need so that when (or if) it is built, it composes with the
Phase 6 containment, symlink, quota, atomicity, and lock policy instead of
reintroducing unrestricted writes. It is deliberately NOT a plan to
implement now — see the conclusion.

## Registry and IDs

- Process-wide `CaptureRegistry` (like `CaptureStore`/`ProfileStore`),
  injected through `SerialHandlerOptions`/builder into every stdio/HTTP
  session.
- Stable capture IDs: `capture:<connection_id>:<nonce>` — one capture per
  connection initially, keyed by connection id; the ID is stable across
  reconnect and server restart (orphan recovery, below) until stopped.
- A capture is owned by its connection's lifecycle: closing the connection
  or shutting down the server must bound-stop and finalize the capture.

## States

- `starting` — registration done, live-edge mark not yet committed
- `running` — writing segments
- `stopping` — bounded stop requested; flushing + finalizing segments
- `stopped` — final segment finalized, result available
- `failed` — terminal error (quota, I/O, ring overrun)

Transitions: `starting → running` (mark committed); `running → stopping →
stopped`; any of `starting/running/stopping → failed` on unrecoverable
error. Stop is idempotent: a second stop during `stopping` waits for the
same final result.

## Tools (future surface, not built)

- `start_capture(connection_id, ...)`: registers the capture and atomically
  marks the live edge (same private-cursor + pump-gate semantics as
  `capture_boot`, Phase 5). Returns the capture id. Cancellation before the
  start response leaves NO orphan task.
- `stop_capture(capture_id)`: idempotent bounded stop + finalize.
- `capture_status(capture_id)` / `list_captures()`: offsets, bytes
  observed/written/lost, segments, quota usage, error.

## Data path

- Private RX cursor: the shared `read` cursor and ring history are never
  moved by capture. The live-edge mark is atomic under the pump gate (Phase
  5 `pump_gate`) so no pre-start byte can be captured.
- Raw bytes exactly as received; framing/parser/protocol are excluded
  initially (no per-frame transformation, no parser errors in the pipeline).
- Bounded queue with explicit backpressure: a bounded channel between the
  pump and the writer; on overflow the capture records `bytes_lost` and a
  `ring_overrun` gap — NEVER silent loss.

## Stop reasons

`match` (pattern found), `timeout`, `silence` (`no_new_rx_timeout_ms`),
`cancelled`, `quota` (file/total/count), `io` (write failure),
`ring_overrun`, `connection_closed`, `server_shutdown`.

## Disconnect and reconnect

- Initially: disconnect stops the capture (finalize what was captured,
  record `connection_closed`). Reconnect continuation (pause + resume the
  same capture) is deferred — it needs its own gap/lost-byte semantics and
  is not assumed.

## Rotation, finalization, and orphans

- Segment rotation before hitting the per-file quota, using the SAME
  advisory lock, quotas, and `persist_noclobber` finalization as Phase 6
  `write_new` — one lock scope for scan → quota → temp → rename of the
  closed segment.
- Internal partial-file names (in-progress segments) carry the reserved
  `.serial-mcp-capture-` prefix, COUNT toward the file/total quotas while
  open, and have an explicit orphan policy: on restart, recognize
  reserved-prefix entries, validate them, and finalize (rename to a
  managed `.jsonl` name) or discard per policy — never arbitrary deletion.
- Cancellation before the start response leaves no orphan task and no file.

## Trust, durability, and cleanup

- Same trust boundary as Phase 6: configured root and ancestors are
  operator-controlled; advisory locks protect cooperating serial-mcp
  processes only.
- No automatic deletion of completed captures until a deterministic
  retention policy exists (Phase 6 deliberately never deletes).

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

Phase 4/5 evidence supports **bounded in-memory boot capture**
(`capture_boot`), not yet continuous disk capture. The safe persistent
capture foundation shipped in Phase 6 removes the last unrestricted
filesystem write and supplies every primitive the lifecycle above needs
(containment, symlink policy, portable names, quotas, advisory locks,
atomic no-clobber finalization), but there is still no concrete task
evidence for an always-on disk capture stream.

**Recommendation: do not implement continuous capture until concrete task
evidence exists.** If such evidence arrives, implement the lifecycle above
incrementally (registry + start/stop/status + one capture per connection +
segments + orphan policy), reusing `CaptureStore` as the commit primitive
and `capture_boot`'s private-cursor/pump-gate semantics as the mark
primitive.
