# Agent interface evaluation

This report describes deterministic local measurement of the MCP tool surface.
It uses no network access, user profiles, hardware, or timestamps. Re-running
the command below reproduces the report byte for byte, apart from explicitly
excluded presentation paths.

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```

The catalog metric is compact `serde_json` serialization of the tool list and
each schema. It excludes HTTP/SSE headers and pretty-print whitespace. Scenario
request bytes are compact JSON of fixed-ID MCP `tools/call` envelopes with the
normalized placeholders `/dev/ttyACM0` and a fixed UUID.

## Historical baseline

`docs/development/agent-interface-baseline.json` is the committed historical
baseline, not the current catalog. It measures 26 tools and an aggregate compact
`tools/list` payload of 258964 bytes. It also contains the then-modeled,
hypothetical `capture_boot`. The evaluator compares the live catalog against
this file when passed with `--baseline`.

## Current catalog

The current catalog has 25 tools and an aggregate compact `tools/list` payload of
267008 bytes. Relative to the historical 26-tool, 258964-byte baseline, the
delta is +8044 bytes, or +3.1%. The `subscribe` and `unsubscribe` tools were
removed with MCP logging during the rmcp 3 server-surface migration. `read`
remains the complete RX path, and `poll_interval_ms` and stream-only schema
helpers are gone. Capture of boot data, `export_log` contract hardening, and
documentation growth account for the increase, while removing those two tools
reduced it from the pre-migration 27-tool, 288177-byte snapshot.

Largest tools are `configure` at 38162 bytes, `transact` at 26450,
`capture_boot` at 25516, `open` at 23635, and `read` at 23598.

The current `export_log` entry is 2725 bytes: 899 for its description, 563 for
its input schema, and 1111 for its output schema. Its description covers the
disabled-by-default store, the `--capture-dir` requirement, the portable
filename-only `path` contract, no-overwrite and symlink policy, and quotas. Its
result carries the uint-schema-annotated fields `bytes_written`, `files_used`,
and `total_bytes_used`, plus the optional post-commit `durability_warning`,
which is omitted on success through `skip_serializing_if`.

Against the historical baseline, aggregate growth is 3.1%, below the evaluator
warning threshold of 5%. The comparison still has warning status because
`export_log` grew from 754 to 2725 bytes, an increase of 1971 bytes or 261.4%.
That per-tool increase comes from the hardened persistent-capture contract.
The tool-count guard `tool_catalog_has_exactly_twenty_five_tools` and the
uint-format schema guards
(`serial::schema::export_log_result_has_no_uint_formats` and
`tools::mod::tool_schemas_have_no_nonstandard_uint_formats`) pin the catalog
shape.

## Accepted decisions

### Automatic profiles

Accepted. Automatic profile reuse saves 1 call and 21.8% of request bytes
compared with explicit management. The thresholds are at least 1 saved call
and at least 20% fewer bytes. Identity rules are unchanged.

### `transact`

Accepted. Write-then-await-response takes 2 calls and 383 bytes, compared with
3 calls and 522 bytes for `write` plus `read`, a 26.6% reduction.

### Atomic `capture_boot`

Accepted. The implemented operation uses 1 call and 360 request bytes, with
`stale_race=false`. The five-call `boot_reset_manual_composition` uses 886
bytes and has `stale_race=true`, so `capture_boot` reduces request bytes by
59.4% and removes the stale-data race in the manual arm/reset composition.

The pump gate in `src/rx_session.rs` provides the ordering. The pump holds
`pump_gate` across one complete read and ring append. `capture_boot` acquires
the same gate for its purge, mark, and assert sequence. The deterministic proof
is `src/rx_session.rs::pump_holds_gate_across_inflight_read_and_ring_append`:
an in-flight pump read holds the gate and appends before any gate acquisition
succeeds. Public behavior is covered by
`capture_boot_stale_bytes_excluded_boot_bytes_captured_cursor_preserved`.

## Shorthand

Rejected under current thresholds. Neither of the 2 shorthand scenarios reaches
the required 20% request-byte reduction, while at least 3 scenarios must meet
it. Projected catalog growth is 0.0%, within the 3% limit.

- `command_response_transact` models 2 calls and 320 bytes instead of the
  current 2 calls and 383 bytes. String forms for `match` and `from` would
  expand to the current tagged objects, including the tagged `{"type":"now"}`
  form when `from` is supplied.
- `at_modem` models 2 calls and 399 bytes instead of the current 2 calls and
  408 bytes. A bare-string `protocol` would expand to the current tagged
  preset object, `{"type":"at_command"}`.

## Initial recipes

Rejected under current thresholds. Two of the 2 recipe scenarios meet the
reduction or advanced-object rule, but at least 3 scenarios must meet it with no
extra calls. Projected catalog growth is 0.0%, within the 2% limit.

- `at_modem_recipe` models 2 calls and 404 bytes instead of the current 2 calls
  and 408 bytes. It removes one repeated advanced object. The current expansion
  uses `protocol: {"type":"at_command"}`. The recipe uses the `at_command`
  preset and bounded timeouts.
- `ndjson_stream` models 2 calls and 303 bytes instead of the current 2 calls
  and 298 bytes. It removes one repeated advanced object but increases request
  bytes by 1.7%. The current expansion uses `protocol: {"type":"ndjson"}`.
  The recipe uses the `ndjson` preset, line framing, the JSON parser, and
  `skip_empty`.

## Versioned facade

Rejected under current thresholds. Common-task median facade savings are 0.0
calls, below the required 1 call. The byte reduction is 20.1%, below the
required 30%, although modeled completion is 100%. The projected catalog-growth
limit is 10%, and modeled growth is 0.0%.

The modeled `command_response_facade` uses 2 calls and 306 bytes instead of the
current 2 calls and 383 bytes. A `command` facade would be a 1:1 alias of
`transact` with string `match`, so it does not reduce call count.

## Modeled candidates

Shorthand, recipe, and facade candidates are hypothetical and not implemented.
The evaluator marks them `modeled` and records their expansion into current
calls. They add no tools, so projected catalog growth is 0%. Growth inside
existing `oneOf` branches is not modeled. The current `capture_boot` is
implemented; only the historical baseline treated it as hypothetical.

## Limitations

- A static harness cannot measure model misunderstanding, invalid-call rates
  from real agents, or how descriptions steer tool choice.
- `invalid calls` and `retries` are plan-level facts for the fixed scenarios,
  not measured agent behavior.
- Request bytes exclude transport framing, including HTTP/SSE headers, and
  result payloads. Only request envelopes and the `tools/list` payload are
  measured.
- Documentation friction is not measurable by a static harness.
- native_sim's PTY UART has no modem-line callbacks, so DTR/RTS assertion is not
  observable there. The arm-only capture test on native_sim covers the real byte
  pipeline. The atomic-reset proof comes from the controlled `SerialIo` backend
  over the public HTTP MCP surface. rmcp's client discards post-cancel
  responses, so `stop_reason="cancelled"` is proven at the unit level by
  `read_loop.rs::cancelled_token_read_returns_structured_cancelled_outcome`.
  HTTP tests assert the observable release and control-lock release.
