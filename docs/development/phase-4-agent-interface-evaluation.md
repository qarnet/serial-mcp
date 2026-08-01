# Agent-Interface Evaluation Report (Phase 4)

Deterministic local measurement: no network access, no user profiles, no hardware, no timestamps. Rerunning `xtask agent-eval` reproduces this report byte-for-byte (except explicitly excluded presentation paths).

## Tool catalog (`tools/list` payload)

- tool count: **26**
- aggregate compact payload: **258964 bytes** (whole `{"tools":[...]}` result)

Byte metric: compact `serde_json` serialization of the tool list and of each schema — no HTTP/SSE headers, no pretty-print whitespace.

| tool | total | description | input schema | output schema |
|---|---|---|---|---|
| clear_log | 522 | 69 | 152 | 152 |
| close | 6805 | 36 | 152 | 6463 |
| compute_checksum | 1723 | 366 | 714 | 483 |
| configure | 39117 | 584 | 16480 | 21895 |
| delete_profile | 534 | 75 | 150 | 150 |
| export_log | 754 | 117 | 237 | 249 |
| flush | 1817 | 375 | 628 | 654 |
| get_log | 5399 | 228 | 469 | 4560 |
| get_status | 9234 | 241 | 152 | 8682 |
| list_connections | 4649 | 52 | 33 | 4399 |
| list_ports | 7224 | 607 | 33 | 6429 |
| list_profiles | 19368 | 181 | 33 | 19000 |
| open | 24010 | 496 | 16788 | 6574 |
| open_profile | 7784 | 280 | 771 | 6574 |
| read | 23555 | 748 | 14382 | 8220 |
| reconfigure | 7322 | 233 | 454 | 6470 |
| reconnect | 941 | 207 | 152 | 432 |
| rollback_profile | 22022 | 445 | 504 | 20909 |
| save_profile | 17802 | 212 | 474 | 16961 |
| send_break | 896 | 160 | 228 | 318 |
| set_dtr_rts | 773 | 136 | 214 | 248 |
| set_flow_control | 6864 | 125 | 200 | 6354 |
| subscribe | 15461 | 484 | 14320 | 427 |
| transact | 26394 | 475 | 16800 | 8914 |
| unsubscribe | 656 | 82 | 152 | 231 |
| write | 7301 | 542 | 6104 | 502 |

Largest tools:

- `configure`: 39117 bytes
- `transact`: 26394 bytes
- `open`: 24010 bytes
- `read`: 23555 bytes
- `rollback_profile`: 22022 bytes

## Scenario metrics

Fixed normalized placeholders (`/dev/ttyACM0`, fixed UUID). Request bytes = compact JSON of fixed-ID MCP `tools/call` envelopes.

| scenario | calls | bytes | invalid | retries | advanced fields | stale/race | completion reference |
|---|---|---|---|---|---|---|---|
| first_console_discovery_open | 2 | 203 | 0 | 0 | 0 | no | `tests/serial_pty.rs::auto_session_first_open_creates_generated_profile_and_pty_traffic_flows` |
| returning_known_console_automatic | 2 | 383 | 0 | 0 | 1 | no | `tests/serial_pty.rs::list_ports_preview_selected_winner_matches_later_bare_open` |
| explicit_profile_management | 3 | 490 | 0 | 0 | 1 | no | `tests/serial_pty.rs::open_profile_explicit_binding_reports_matched_port_confidence` |
| command_response_transact | 2 | 383 | 0 | 0 | 1 | no | `tests/serial_pty.rs::pty_transact_writes_then_reads_response` |
| command_response_write_read | 3 | 522 | 0 | 0 | 1 | no | `tests/serial_pty.rs::pty_transact_writes_then_reads_response` |
| line_capture | 2 | 314 | 0 | 0 | 1 | no | `tests/native_sim_validation/unix.rs::native_read_line_framing_splits_lines` |
| at_modem | 2 | 408 | 0 | 0 | 2 | no | `tests/native_sim_validation/unix.rs::native_read_at_parser_parses_pong` |
| at_modem_recipe | 2 | 408 | 0 | 0 | 2 | no | `tests/native_sim_validation/unix.rs::native_read_at_parser_parses_pong` |
| ndjson_stream | 2 | 298 | 0 | 0 | 1 | no | `tests/native_sim_validation/unix.rs::native_read_ndjson_preset_decodes_json_frames` |
| rollback_recovery | 6 | 787 | 0 | 0 | 0 | no | `tests/serial_pty.rs::rollback_with_no_active_connections_reports_zero` |
| boot_reset_prompt_capture | 5 | 886 | 0 | 0 | 1 | yes | `tests/serial_pty.rs::pty_transact_from_now_skips_pre_write_buffer` |
| permission_busy_disconnected | 4 | 525 | 0 | 1 | 0 | no | `tests/native_sim_validation/unix.rs::native_auto_reconnect_preserves_connection` |
| command_response_facade | 2 | 383 | 0 | 0 | 1 | no | `tests/serial_pty.rs::pty_transact_writes_then_reads_response` |

Modeled (hypothetical, NOT implemented) variants and their expansion into current calls:

| scenario | kind | calls | bytes | expansion calls | expansion bytes | note |
|---|---|---|---|---|---|---|
| command_response_transact | shorthand | 2 | 320 | 2 | 383 | String forms for `match`/`from` would expand to the current tagged objects. |
| at_modem | shorthand | 2 | 399 | 2 | 408 | A bare string `protocol` would expand to the current tagged preset object. |
| at_modem_recipe | recipe | 2 | 404 | 2 | 408 | A `recipe` would replace the repeated protocol-preset object (at_modem = at_command preset + bounded timeouts). |
| ndjson_stream | recipe | 2 | 303 | 2 | 298 | ndjson_stream recipe = ndjson preset (line framing + JSON parser, skip_empty). |
| boot_reset_prompt_capture | capture_boot | 1 | 216 | 5 | 886 | One server-side operation would snapshot the live edge, pulse DTR/RTS, and capture only post-reset bytes — removing the arm/reset race between the seek and the reset. |
| command_response_facade | facade | 2 | 306 | 2 | 383 | A facade `command` would be a 1:1 alias of `transact` with string `match` — same call count, fewer bytes. |

## Comparisons

- automatic profile reuse vs explicit management: 2 calls vs 3 (savings 1), 383 bytes vs 490 (+21.8%)
- `transact` vs `write`+`read`: 2 calls vs 3 (savings 1), 383 bytes vs 522 (+26.6%)
- shorthand (command_response_transact) vs current: 2 calls vs 2 (savings 0), 320 bytes vs 383 (+16.4%)
- shorthand (at_modem) vs current: 2 calls vs 2 (savings 0), 399 bytes vs 408 (+2.2%)
- recipe (at_modem_recipe) vs current: 2 calls vs 2 (savings 0), 404 bytes vs 408 (+1.0%), repeated advanced objects removed: 1
- recipe (ndjson_stream) vs current: 2 calls vs 2 (savings 0), 303 bytes vs 298 (-1.7%), repeated advanced objects removed: 1
- facade (command_response_facade) vs current: 2 calls vs 2 (savings 0), 306 bytes vs 383 (+20.1%)
- capture_boot vs current composition: 1 calls vs 5 (savings 4), 216 bytes vs 886 (+75.6%), stale/race window: yes

## Decisions (fixed thresholds, evaluated after measurement)

- automatic profiles: **yes** — automatic reuse saves 1 call(s) (need >=1) and 21.8% request bytes (need >=20%) vs explicit management; identity rules unchanged
- shorthand now: **no** — 0 of 2 shorthand scenarios reach >=20% request-byte reduction (need >=3); projected catalog growth 0.0% (limit 3%)
- initial recipes now: **no** — 2 of 2 recipe scenarios meet the reduction/advanced-object rule (need >=3) with no extra calls; projected catalog growth 0.0% (limit 2%)
- versioned facade now: **no** — common-task median facade call savings 0.0 (need >=1), byte reduction 20.1% (need >=30%), modeled completion 100%: yes
- Phase 5 atomic `capture_boot`: **yes** — boot capture composition has a stale-data/arm-reset race: true; capture_boot reduces 5 calls to 1

## Dominant friction

**schema size** — chosen by the fixed rule: schema size if the aggregate `tools/list` payload is >= 64 KiB; else call shape if the median common-task call count is >= 3; else setup if first-connect needs >= 4 calls; else orchestration if any scenario retries. Documentation friction is not measurable by a static harness.

## Catalog regression

Status: **no_baseline**
No per-tool regressions (warning at >=10% or +2 KiB per tool).

## Limitations

- A static harness cannot measure model misunderstanding, invalid-call rates from real agents, or how descriptions steer tool choice.
- `invalid calls`/`retries` are plan-level facts for the fixed scenarios, not measured agent behavior.
- Modeled candidates are hypothetical shapes with explicit expansions into current calls; they are NOT implemented and their projected catalog growth is reported as 0% (no new tools) — oneOf-branch growth inside existing schemas is not modeled.
- Request bytes exclude transport framing (HTTP/SSE headers) and result payloads; only request envelopes and the `tools/list` payload are measured.
