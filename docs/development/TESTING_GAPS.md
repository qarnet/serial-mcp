# Testing Gaps — serial-mcp

Known coverage gaps for the frame pipeline (PR #27 / v0.7.0). Overall coverage
is strong — decoder unit tests for every mode, TX/RX round-trips, the shared
`consume_frames` sink, `read_bytes_via_session` integration tests, and ~51
native_sim e2e tests. The items below are the remaining holes, ordered by value.

---

## 1. Four-layer precedence resolution — no unit test

- **What's untested:** The per-field precedence ladder
  *explicit call > call `protocol` preset > connection default > connection
  `protocol` preset*. The chains live in three places that are near-duplicates:
  - `tx_framing` in `io_ops::write`
  - `rx_framing` + `rx_parser` in `io_ops::read`
  - `rx_framing` + `rx_parser` in `stream_ops::subscribe`
- **Current coverage:** Only via native_sim e2e (presets + override precedence +
  connection defaults), which exercises happy paths end-to-end but not each
  layer-boundary in isolation.
- **Why it matters:** The ladder is hand-written `else if` three times; a subtle
  edit (e.g. swapping a layer, or filling `rx_parser` from the wrong source) can
  pass e2e while breaking a specific precedence case. This is the highest-value
  gap.
- **Suggested tests:** Pure unit tests over the resolution logic for each field:
  - explicit beats call-protocol beats connection-default beats
    connection-protocol, one assertion per boundary;
  - mixed case: explicit `rx_framing` + connection `protocol` default still
    pulls the parser from the connection protocol preset (gap-fill);
  - the three call sites agree (parameterize or share a helper so they can't
    drift). Consider extracting the ladder into a single testable function and
    calling it from all three sites — kills the duplication and the gap at once.

## 2. TX length-prefixed overflow — only `prefix_size=1` covered

- **What's untested:** The `u16` overflow branch in `TxFramingMode::encode`
  (`src/framing.rs`) that rejects payloads `> 65535` for `prefix_size=2`.
- **Current coverage:** `tx_length_prefixed_u8_overflow` covers the
  `prefix_size=1` (`> 255`) branch only.
- **Why it matters:** Each overflow guard is a distinct error path with its own
  message; the `u16` one is unexercised.
- **Suggested tests:** A `prefix_size=2` payload of length `65536` asserting the
  "exceeds maximum 65535" error. (`prefix_size=4` has no practical overflow
  bound, so no test needed there.)

## 3. Subscribe matching-frame peer-disconnect mid-emit — author-acknowledged

- **What's untested:** The quirk in `SubscribeFrameSink::on_frame`
  (`src/tools/stream_ops.rs`) where a failed emit of the *matching* frame still
  reports the match (logs + `record_notification_drop`), distinct from the
  non-matching path that returns `PeerDisconnected`.
- **Current coverage:** None — the author flagged it inline as a KNOWN GAP
  (requires simulating a peer disconnect at the exact moment the matching frame
  is emitted).
- **Why it matters:** Low. It's a faithfully-preserved legacy quirk, not new
  behavior, and the inline comment documents the intent.
- **Suggested tests:** Defer until a proper failure-injection harness exists for
  forcing notification emit failure on the matching frame. Do **not** add a
  sleep/timing-based test for this — it would be flaky and would not prove the
  intended behavior deterministically. Once the harness exists, assert that the
  matching-frame emit-failure path records the notification drop and preserves
  the documented legacy "matched despite failed emit" behavior, while the
  non-matching path still returns `PeerDisconnected`.

---

### Priority

- **Do first:** #1 (precedence) — highest risk, and pairs naturally with the
  duplication cleanup in [TECH_DEBT.md](TECH_DEBT.md).
- **Cheap add:** #2 (u16 overflow) — one small unit test.
- **Defer:** #3 — needs tooling that doesn't exist yet; documented in code.
