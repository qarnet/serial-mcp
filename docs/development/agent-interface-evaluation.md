# Agent-Interface Evaluation

Deterministic local measurement of the MCP tool surface: no network access,
no user profiles, no hardware, no timestamps. Rerunning the command below
reproduces the report byte-for-byte (except explicitly excluded presentation
paths).

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```

Byte metric: compact `serde_json` serialization of the tool list and of each
schema — no HTTP/SSE headers, no pretty-print whitespace. Request bytes are
compact JSON of fixed-ID MCP `tools/call` envelopes with normalized
placeholders (`/dev/ttyACM0`, fixed UUID).

## Baseline (historical)

`docs/development/agent-interface-baseline.json` is the committed **Phase 4
baseline and stays historical**: it measures the 26-tool catalog (aggregate
compact `tools/list` payload **258964 bytes**) and the then-modeled,
hypothetical `capture_boot`. It is NOT the current catalog — the evaluator
compares the live catalog against it.

## Current catalog (27 tools, 288177 bytes)

- tool count: **27**
- aggregate compact `tools/list` payload: **288177 bytes** — +29213 vs the
  Phase 4 baseline (26 tools / 258964 bytes, +11.3%; the new tool plus
  `export_log` contract hardening account for the bulk)
- post-refinement snapshot: the 27-tool catalog measured **after** the RX
  encoding/hex-fallback documentation refresh is **+1892 bytes** vs the
  pre-refinement 27-tool snapshot (286285). The growth is output-schema
  description growth only, in the shared result types (`data` + `encoding`
  pairs and their lossless-hex-fallback semantics are now documented on the
  wire):
  - `capture_boot`: 25338 → **25905** (+567), output 10421 → 10988
  - `read`: 23555 → **24122** (+567), output 8220 → 8787
  - `transact`: 26394 → **26961** (+567), output 8914 → 9481
  - `subscribe`: 15461 → **15652** (+191), output 427 → 618
- descriptions and input schemas are byte-identical to the pre-refinement
  snapshot; no tool, input field, or result data field was added or removed
- every other tool is byte-identical to the pre-refinement snapshot (and,
  for pre-Phase-5 tools, to the Phase 4 baseline)
- `export_log` (Phase 6 contract hardening): 754 → **2736 bytes** (description
  931, input schema 542, output schema 1111) — the description now states the
  disabled-by-default store, the `--capture-dir` requirement, the portable
  filename-only `path` contract, no-overwrite/symlink policy, and quotas;
  the result gained three uint-schema-annotated fields (`bytes_written`,
  `files_used`, `total_bytes_used`) and an optional post-commit
  `durability_warning` (`skip_serializing_if`-omitted on success, so the
  committed-wire result shape is unchanged unless a warning occurs). Input
  schema still requires only `connection_id` + `path`.
- largest tools: `configure` 39117, `transact` 26961, `capture_boot` 25905,
  `read` 24122, `open` 24010

The evaluator's regression rule flags aggregate growth `>=5%` and reports
status `warning` — the growth is the deliberate Phase 5/6 scope (a new tool
plus contract hardening) plus the approved +1892 output-schema description
growth, not schema bloat on existing tools. The tool-count guard
(`tool_catalog_has_exactly_twenty_seven_tools`) and the uint-format schema
guards (`serial::schema::export_log_result_has_no_uint_formats`,
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
  stale-data race Phase 4 measured is eliminated by the pump gate
  (`src/rx_session.rs`): the pump holds `pump_gate` across one complete read
  + ring append, and `capture_boot` acquires the same gate for its
  purge → mark → assert sequence. Public-behavior proof:
  `tests/http_integration.rs::capture_boot_pump_barrier_appends_inflight_read_before_mark`
  (an in-flight pre-reset pump read's bytes land in `pre_mark_bytes`, never
  in the capture result) and
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
- native_sim's PTY UART has no modem-line callbacks, so DTR/RTS assertion is
  not observable there; the arm-only capture test on native_sim covers the
  real byte pipeline honestly, and the atomic-reset proof comes from the
  controlled `SerialIo` backend over the public HTTP MCP surface. rmcp's
  client discards post-cancel responses, so `stop_reason="cancelled"` is
  proven at the unit level (`read_loop.rs::cancelled_token_read_returns_structured_cancelled_outcome`)
  while HTTP tests assert the observable release and control-lock release.
