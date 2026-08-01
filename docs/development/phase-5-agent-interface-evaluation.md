# Phase 5 Agent-Interface Evaluation Note — Atomic Boot Capture

Deterministic local measurement, same harness as Phase 4: no network, no user
profiles, no hardware, no timestamps. Run with:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```

The Phase 4 baseline (`docs/development/agent-interface-baseline.json`) and
decision record (`docs/development/phase-4-agent-interface-evaluation.md`)
are historical and were NOT rewritten: they measure the 26-tool catalog and
the modeled `capture_boot` hypothesis. This note records the Phase 5 catalog
delta against that baseline now that `capture_boot` is implemented.

## Catalog delta vs Phase 4 baseline

- tool count: **26 → 27** (`capture_boot` added)
- aggregate compact `tools/list` payload: **258964 → 284303 bytes (+9.8%)**
- new tool: `capture_boot` — 25338 bytes total (description 725, input
  schema 13991, output schema 10421)
- existing tools: **no per-tool regressions** (all 26 pre-existing tools are
  byte-identical to the Phase 4 baseline)
- largest tools: `configure` 39117, `transact` 26394, `capture_boot` 25338,
  `open` 24010, `read` 23555

The aggregate growth is entirely the new tool (a new oneOf-heavy input
schema plus a nested `ReadResult` output schema). The evaluator's regression
rule flags `>=5%` aggregate growth — this run reports status `warning` for
exactly that reason; there are no per-tool regressions and the growth is the
deliberate Phase 5 scope, not schema bloat on existing tools.

## Scenario delta

- `boot_reset_prompt_capture` is now ONE implemented `capture_boot` call
  (360 request bytes, `stale_race=false`) — the Phase 4 modeled hypothesis
  is gone from the current report. Completion reference:
  `tests/http_integration.rs::capture_boot_stale_bytes_excluded_boot_bytes_captured_cursor_preserved`
- `boot_reset_manual_composition` (new scenario) preserves the pre-Phase-5
  5-call composition (886 request bytes, `stale_race=true`) as the
  comparison baseline for the `capture_boot` decision.
- Decision re-evaluated from fixed thresholds: **Phase 5 atomic
  `capture_boot`: yes** — the manual composition retains a stale-data/
  arm-reset race (`true`) and `capture_boot` reduces 5 calls to 1
  (360 bytes vs 886, +59.4% request-byte reduction).
- All other scenario metrics are unchanged from Phase 4.

## Behavior proof for the race elimination

The arm/reset race Phase 4 measured (a byte physically read before the reset
could append after a naive `ring.end_offset()` mark) is eliminated by the
pump gate (`src/rx_session.rs`): the pump holds `pump_gate` across one
complete read + ring append, and `capture_boot` acquires the same gate for
its purge → mark → assert sequence. Public-behavior proof:

- `tests/http_integration.rs::capture_boot_pump_barrier_appends_inflight_read_before_mark`
  — an in-flight pre-reset pump read's bytes land in `pre_mark_bytes`, never
  in the capture result.
- `tests/http_integration.rs::capture_boot_stale_bytes_excluded_boot_bytes_captured_cursor_preserved`
  — retained pre-mark bytes never appear; post-assertion bytes do.
- `src/rx_session.rs` unit test — while the gate is held the pump appends
  nothing, regardless of injected bytes.

## Limitations

- native_sim's PTY UART has no modem-line callbacks, so DTR/RTS assertion is
  not observable there; the arm-only capture test on native_sim covers the
  real byte pipeline honestly, and the atomic-reset proof comes from the
  controlled `SerialIo` backend over the public HTTP MCP surface.
- rmcp's client resolves its own `notifications/cancelled` request handle
  with `Err(Cancelled)` and discards the server response, so the structured
  `stop_reason="cancelled"` outcome is proven at the unit level
  (`helpers.rs::cancelled_token_read_returns_structured_cancelled_outcome`)
  while the HTTP tests assert the observable release and control-lock
  release.
- As in Phase 4: a static harness cannot measure model misunderstanding, and
  request bytes exclude transport framing.
