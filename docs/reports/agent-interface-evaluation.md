# Agent-Interface Evaluation

Deterministic local measurement of the MCP tool surface: no network access,
no user profiles, no hardware, no timestamps. Rerunning the command below
reproduces the report byte-for-byte (except explicitly excluded presentation
paths).

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/reports/agent-interface-baseline.json
```

Byte metric: compact `serde_json` serialization of the tool list and of each
schema — no HTTP/SSE headers, no pretty-print whitespace. Request bytes are
compact JSON of fixed-ID MCP `tools/call` envelopes with normalized
placeholders (`/dev/ttyACM0`, fixed UUID).

## Baseline (historical)

`docs/reports/agent-interface-baseline.json` is the committed **historical
baseline**: it measures the 26-tool catalog (aggregate
compact `tools/list` payload **258964 bytes**) and the then-modeled,
hypothetical `capture_boot`. Its five native_sim completion references are
historical evidence and remain unchanged in that JSON. It is NOT the current
catalog — the evaluator compares the live catalog against it.

## Current catalog (25 tools, 267742 bytes)

- tool count: **25** — the `subscribe`/`unsubscribe` tools were removed with
  MCP logging in the rmcp 3 server-surface migration (`read` remains the
  complete RX path; `poll_interval_ms` and stream-only schema helpers are gone)
- aggregate compact `tools/list` payload: **267742 bytes** — **+8778** vs the
  historical baseline (26 tools / 258964 bytes, +3.4%; `capture_boot`,
  `export_log` contract hardening, and documentation growth account for most
  growth; removing the two subscription tools shrank the catalog from the
  pre-migration 27-tool / 288177-byte snapshot)
- largest tools: `configure` 38236, `transact` 26637, `capture_boot` 25757,
  `read` 23841, `open` 23635
- `export_log` (contract hardening): 2704 bytes (description 899, input schema
  542, output schema 1111) — the description states the disabled-by-default
  store, the `--capture-dir` requirement, the portable filename-only `path`
  contract, no-overwrite/symlink policy, and quotas; the result carries three
  uint-schema-annotated fields (`bytes_written`, `files_used`,
  `total_bytes_used`) and an optional post-commit `durability_warning`
  (`skip_serializing_if`-omitted on success).

The aggregate is **3.4%**, below the **5%** aggregate threshold. The evaluator
still reports a warning because `export_log` grew from **754** to **2704** bytes
(+258.6%, +1950), while other tool sizes, notably `configure`, shrank.
The tool-count guard (`tool_catalog_has_exactly_twenty_five_tools`) and the
uint-format schema guards
(`serial::schema::export_log_result_has_no_uint_formats`,
`tools::mod::tool_schemas_have_no_nonstandard_uint_formats`) pin the shape.

## Decisions (fixed thresholds, evaluated after measurement)

Accepted:

- **Automatic profiles: yes** — automatic reuse saves 1 call and 21.8%
  request bytes vs explicit management (thresholds: >=1 call, >=20% bytes);
  identity rules unchanged.
- **`transact`: yes** — write-then-await-response: 2 calls vs 3 for
  write+read, 383 bytes vs 522 (+26.6%).
- **Atomic `capture_boot`: yes** — one implemented call (360 request bytes,
  `stale_race=false`) vs the 5-call `boot_reset_manual_composition` (886
  bytes, `stale_race=true`, +59.4% request-byte reduction). The arm/reset
  stale-data race in the manual composition is eliminated by the pump gate
  (`src/rx_session.rs`): the pump holds `pump_gate` across one complete read
  + ring append, and `capture_boot` acquires the same gate for its
  purge → mark → assert sequence. Deterministic proof:
  `src/rx_session.rs::pump_holds_gate_across_inflight_read_and_ring_append`
  (an in-flight pump read holds the gate and appends before any gate
  acquisition succeeds) plus the public-behavior
  `capture_boot_stale_bytes_excluded_boot_bytes_captured_cursor_preserved`.

Rejected (with reasons):

- **Shorthand now: no** — 0 of 2 shorthand scenarios reach the >=20%
  request-byte reduction (need >=3); projected catalog growth 0.0% (limit
  3%). String forms for `match`/`from`/`protocol` would expand to the current
  tagged objects.
- **Initial recipes now: no** — 2 of 2 recipe scenarios meet the
  reduction/advanced-object rule (need >=3); projected catalog growth 0.0%
  (limit 2%).
- **Versioned facade now: no** — common-task median facade call savings 0.0
  (need >=1), byte reduction 20.1% (need >=30%); modeled completion 100%.
  A facade `command` would be a 1:1 alias of `transact` with string `match`.

Modeled (hypothetical, NOT implemented) candidates are marked `modeled` in
the report with their expansion into current calls; their projected catalog
growth is 0% (no new tools) — oneOf-branch growth inside existing schemas is
not modeled.

## Limitations (static harness)

- Cannot measure model misunderstanding, invalid-call rates from real agents,
  or how descriptions steer tool choice.
- `invalid calls`/`retries` are plan-level facts for the fixed scenarios, not
  measured agent behavior.
- Request bytes exclude transport framing (HTTP/SSE headers) and result
  payloads; only request envelopes and the `tools/list` payload are measured.
- Documentation friction is not measurable by a static harness.
- Real PTYs do not provide modem-line callbacks, so DTR/RTS assertion remains
  covered by the controlled `SerialIo` backend over the public HTTP MCP
  surface; the real-PTY arm-only test covers the byte pipeline. rmcp's client
  discards post-cancel responses, so `stop_reason="cancelled"` is proven at
  the unit level (`read_loop.rs::cancelled_token_read_returns_structured_cancelled_outcome`)
  while HTTP tests assert the observable release and control-lock release.
