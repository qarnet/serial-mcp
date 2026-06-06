# serial-mcp Test Plan

**Branch:** `preview-pr` → `main`
**Date:** 2026-06-06

This is the single handoff document for validating serial-mcp. It covers all
automated test layers (CI baseline) and the full physical-device test procedure
using the E83 nRF5340 board.

---

## Part 1 — Automated Test Suite

### Run Commands

```bash
# Full CI baseline — no hardware required
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings

# Hardware loopback (TX wired to RX, or two ports crossed)
SERIAL_MCP_TEST_PORT=/dev/ttyUSB0 cargo test --test hardware_loopback -- --ignored

# E83 live board
cargo test --test e83_live_validation -- --ignored

# Schema network check (fetches upstream schemas)
cargo test --test config_schema_validation -- --ignored

# Fuzz (nightly required, run as long as desired)
cargo +nightly fuzz run tool_call_json
```

---

### Layer 1 — Unit Tests (`src/**`)

`cargo test --lib`

| Module | Tests |
|--------|-------|
| `serial.rs` | `baud_rate_zero_rejected`, `baud_rate_over_max_rejected`, `baud_rate_within_range_accepted` |
| `serial.rs` | `write_pushes_bytes_to_peer`, `read_returns_peer_bytes`, `read_times_out_when_no_data` |
| `serial.rs` | `flush_set_dtr_rts_send_break_are_noops_on_loopback` |
| `serial.rs` | `manager_rejects_duplicate_port`, `manager_duplicate_port_error_includes_owner_metadata` |
| `serial.rs` | `manager_close_then_get_returns_connection_not_found`, `manager_get_unknown_id_returns_connection_not_found` |
| `serial.rs` | `close_cancels_inflight_read` |
| `rx_session.rs` | `rx_event_clone_copies_data` |
| `rx_session.rs` | `manager_get_or_create_returns_same_session`, `manager_remove_awaits_pump_exit`, `manager_remove_nonexistent_is_noop` |
| `rx_session.rs` | `session_register_blocking_starts_pump`, `session_shutdown_cancels_pump_token` |
| `rx_session.rs` | `consumer_receives_data_after_registration`, `two_consumers_both_receive_future_data` |
| `rx_session.rs` | `removing_session_awaits_pump_and_drops_consumers`, `connection_close_causes_pump_exit`, `pump_exits_cleanly_on_shutdown_without_hanging` |
| `rx_session.rs` | `no_consumers_means_no_pump`, `shutdown_is_idempotent`, `repeated_create_remove_no_leaked_pump_tasks` |
| `rx_session.rs` | `full_consumer_is_dropped_from_registry`, `dropped_receiver_removed_from_registry` |
| `stop_controller.rs` | `timeout_stops_at_deadline`, `continue_before_deadline`, `no_timeout_without_deadline` |
| `stop_controller.rs` | `match_found_stops_immediately`, `no_match_continues`, `max_buffered_bytes_stops`, `match_found_takes_priority_over_max_bytes` |
| `stop_controller.rs` | `connection_closed_outcome`, `channel_closed_outcome`, `cancelled_outcome`, `read_error_outcome`, `peer_disconnected_outcome`, `data_complete_outcome` |
| `stop_controller.rs` | `is_normal_stop_classifies_correctly` |
| `stop_controller.rs` | `push_data_without_matcher_accumulates`, `timeout_preserves_match_state_from_earlier_data`, `check_max_buffered_bytes_after_record_data`, `record_data_does_not_trigger_stops` |
| `stop_controller.rs` | `silence_timeout_stops_when_expired`, `silence_timeout_continues_with_future_deadline`, `silence_timeout_disabled_when_none`, `notify_data_received_resets_silence_deadline`, `silence_timeout_is_normal_stop`, `silence_timeout_with_data_produces_bytes_in_outcome` |
| `stop_controller.rs` | `both_timeouts_can_be_set_independently`, `no_new_rx_timeout_metadata_preserves_bytes`, `no_new_rx_timeout_with_bytes` |
| `codec.rs` | `encoding_from_str_accepts_aliases`, `encoding_from_str_rejects_unknown` |
| `codec.rs` | `utf8_roundtrip`, `utf8_encode_rejects_invalid_bytes`, `hex_roundtrip`, `hex_odd_length_rejected`, `hex_invalid_chars_rejected` |
| `codec.rs` | `base64_roundtrip_and_padding_variants`, `binary_roundtrips_via_hex_and_base64` |
| `buffer_budget.rs` | `atomic_budget_reserve_and_release`, `atomic_budget_over_tool_limit`, `atomic_budget_zero_request`, `atomic_budget_insufficient_program`, `atomic_budget_concurrent_reserve`, `atomic_budget_rejects_zero_limits` |
| `buffer_budget.rs` | `fake_budget_basic`, `fake_budget_exhaustion`, `unlimited_budget_always_succeeds`, `unlimited_budget_still_rejects_over_tool_limit`, `unlimited_budget_rejects_zero` |

---

### Layer 2 — Property Tests (`tests/proptest.rs`)

`cargo test --test proptest`

| Block | Coverage |
|-------|----------|
| Phase A.1 — Arg roundtrips | All 9 tool arg structs survive serde roundtrip with arbitrary valid inputs |
| Phase A.2 — Result schema validation | All 9 result structs validate against their JSON schemas |
| Phase A.3 — Encoding roundtrips | Arbitrary bytes survive hex/base64 roundtrip |
| Phase A.4 — Boundary helpers | Clamp/min helpers never panic on arbitrary input |

---

### Layer 3 — HTTP Integration (`tests/http_integration.rs`)

`cargo test --test http_integration`

Spins up a real streamable-HTTP MCP server in-process; uses a real MCP client over localhost.

| Test | Coverage |
|------|----------|
| `initialize_handshake_succeeds` | MCP handshake |
| `list_tools_returns_all_twelve_tools` | Tool count == 12 |
| `list_resources_returns_two_statics` | Static resource list |
| `list_connections_returns_open_connection_summaries` | `list_connections` returns open connections with names |
| `list_resources_pagination_with_cursor_returns_next_page` | Resource pagination |
| `list_resource_templates_returns_connection_template` | Connection resource template |
| `list_resource_templates_pagination_with_cursor_returns_next_page` | Template pagination |
| `list_prompts_returns_diagnose_and_interactive` | Both prompts advertised |
| `read_serial_ports_resource_returns_json_payload` | `serial://ports` resource |
| `read_unknown_resource_yields_not_found` | Unknown resource URI |
| `read_unknown_connection_yields_not_found` | Unknown connection resource URI |
| `call_tool_open_with_bad_data_bits_returns_is_error` | Invalid arg → `is_error: true`, not protocol error |
| `call_tool_list_ports_returns_structured_result` | `list_ports` returns valid JSON |
| `get_prompt_diagnose_port_returns_user_message` | `diagnose_port` prompt |
| `write_tool_sends_bytes_to_loopback_peer` | `write` delivers bytes |
| `subscribe_then_peer_write_pushes_notification` | `subscribe` data arrives as notifications |
| `subscribe_with_timeout_auto_stops_in_background` | `subscribe(timeout_ms=...)` self-terminates |
| `subscribe_without_timeout_is_fire_and_forget` | `subscribe` without timeout returns immediately |
| `subscribe_closed_from_other_session_stops_streaming_task` | Connection close stops subscribe task |
| `validation_limits_return_tool_errors_over_http` | Over-limit args → `is_error: true` |
| `read_with_no_data_times_out_with_is_error` | `read` timeout → `is_error: true` |
| `read_result_contains_elapsed_ms` | Result includes `elapsed_ms` |
| `send_break_result_includes_actual_duration` | Result includes timing |

---

### Layer 4 — PTY Integration (`tests/serial_pty.rs`)

`cargo test --test serial_pty` — Linux/macOS only, uses real POSIX pseudoterminals.

| Test | Coverage |
|------|----------|
| `pty_open_returns_connection_id` | Open a real PTY |
| `pty_client_write_reaches_device_side` | `write` → PTY device side |
| `pty_device_write_then_client_read` | Device write → `read` tool |
| `pty_subscribe_streams_device_writes_as_notifications` | Device writes → subscribe notifications |
| `pty_read_match_finds_real_serial_pattern` | `read(match=...)` stops on regex match |
| `pty_read_match_with_context_returns_shaped_payload` | `context_amount_of_matched_bytes` shapes payload |
| `pty_read_match_with_zero_context_returns_only_matched_bytes` | Zero context → matched bytes only |
| `pty_read_match_without_context_returns_full_accumulated` | No context → full buffer |
| `pty_subscribe_match_with_context_includes_shaped_data` | `subscribe(match=..., context=...)` shaped notification |
| `pty_close_then_use_returns_is_error` | Use closed connection → `is_error: true` |
| `pty_send_break_short_duration_timing` | `send_break` timing |

---

### Layer 5 — stdio Integration (`tests/stdio_integration.rs`)

`cargo test --test stdio_integration`

| Test | Coverage |
|------|----------|
| `stdio_initialize_handshake_succeeds` | Binary starts and handshakes |
| `stdio_list_tools_returns_all_twelve_tools` | Tool count == 12 |
| `stdio_list_resources_returns_statics_and_templates` | Static resources and connection template |
| `stdio_full_connection_lifecycle_with_hardware` *(ignored)* | Full lifecycle with real hardware loopback |

---

### Layer 6 — Allowlist (`tests/allowlist.rs`)

`cargo test --test allowlist`

| Test | Coverage |
|------|----------|
| `empty_allowlist_allows_any_port` | No allowlist = permissive |
| `exact_match_blocks_unauthorized_port` | Exact pattern blocks non-matching port |
| `exact_match_allows_authorized_port` | Exact pattern allows matching port |
| `glob_pattern_matches_multiple_ports` | Glob matches multiple ports |
| `comma_separated_multiple_exact_ports` | Comma-separated list |

---

### Layer 7 — Blob Resources (`tests/blob_resources.rs`)

| Test | Coverage |
|------|----------|
| `blob_resource_template_is_advertised` | `serial://connections/{id}/rx/raw` template exists |
| `resource_uri_parsing_includes_raw_suffix` | URI parser extracts ID and `raw` suffix |

---

### Layer 8 — Resource Subscriptions (`tests/resource_subscriptions.rs`)

| Test | Coverage |
|------|----------|
| `resource_subscription_works` | `resources/subscribe` → change notifications on open/close |
| `resource_subscribe_unsubscribe_roundtrip` | Subscribe then unsubscribe stops notifications |

---

### Layer 9 — Protocol Emulators

| Test | Coverage |
|------|----------|
| `protocol_emulator` | Arbitrary well-formed MCP messages handled without crash |
| `protocol_emulator_binary` | Non-JSON/malformed input handled without crash |

---

### Layer 10 — Config Schema Validation (`tests/config_schema_validation.rs`)

| Test | Coverage |
|------|----------|
| `config_schema_validates_against_local_schemas` | Schemas in `schemas/` match current types |
| `config_schema_validates_against_upstream_schemas` *(ignored, requires network)* | Local schemas match latest published schemas |

---

### Layer 11 — Hardware Loopback (`tests/hardware_loopback.rs`)

Requires a serial loopback device (TX↔RX wired, or two ports crossed). Set `SERIAL_MCP_TEST_PORT`.

```bash
SERIAL_MCP_TEST_PORT=/dev/ttyUSB0 cargo test --test hardware_loopback -- --ignored
```

| Test | Coverage |
|------|----------|
| `hw_loopback_write_then_read_roundtrip` | Write bytes, read them back via hardware |
| `hw_loopback_read_match_echo` | `read(match=...)` finds pattern echoed through real hardware |

---

### Layer 12 — Fuzz (`fuzz/fuzz_targets/tool_call_json.rs`)

```bash
cargo +nightly fuzz run tool_call_json
```

Feeds arbitrary JSON into all tool arg structs. No panic = pass. Covers: `OpenArgs`, `CloseArgs`, `WriteArgs`, `ReadArgs`, `FlushArgs`, `SetDtrRtsArgs`, `SendBreakArgs`, `SubscribeArgs`, `UnsubscribeArgs`, `SetFlowControlArgs`.

---

### Known Coverage Gaps

From `docs/plans/CLEANUP.md` — not yet tested:

1. `subscribe(match=...)` matcher memory stays within `max_buffered_bytes` budget
2. Replacing an active subscribe doesn't spuriously fail due to budget exhaustion
3. Final subscribe stop notification has internally consistent `bytes_read`, `bytes_observed`, `bytes_returned`
4. `no_new_rx_timeout_ms=0` rejected at runtime but schema allows it — boundary not tested
5. `no_new_rx_timeout` stop reason not covered by proptest

---

## Part 2 — Physical Device Test Procedure

### Hardware

- **Board:** E83 custom nRF5340 audio receiver
- **Port:** `/dev/ttyUSB0`
- **Baud:** `115200 8N1`
- **Device repo:** `~/repos/le-audio-receiver` (see `AGENTS.md` for I2S debug stress test)

### Stress Log Commands

Send via `write` tool:

| Command | Effect |
|---------|--------|
| `audio i2s-test\r\n` | Start high-rate I2S log flood |
| `audio stop\r\n` | Stop flood |
| `audio status\r\n` | Query audio pipeline health |

Expected log patterns during flood: `Queued TX`, `supply_next_buffers`

### Safety Rules

- Always send `audio stop` before ending the session
- If stale log traffic is suspected, run `flush(target="both")` before the next test
- Always `unsubscribe` or `close` after a background `subscribe`

---

### Test 1 — Port Discovery

**Goal:** confirm the UART is visible before attempting anything else.

```
list_ports
```

Confirm `/dev/ttyUSB0` is in the result.

---

### Test 2 — Basic Open / Close

**Goal:** connection lifecycle and named-connection metadata.

```
open(port="/dev/ttyUSB0", name="e83-uart", baud_rate=115200)
list_connections
```

Confirm the connection appears with correct port, baud, and name `"e83-uart"`.

```
close(connection_id=<id>)
```

---

### Test 3 — Start and Stop Stress Logging

**Goal:** confirm the board produces a sustained RX stream and `read` returns real data.

```
open(port="/dev/ttyUSB0", name="e83-uart", baud_rate=115200)
write(connection_id=<id>, data="audio i2s-test\r\n")
read(connection_id=<id>, max_buffered_bytes=2048, timeout_ms=2000)
```

Confirm returned `data` contains `Queued TX` or `supply_next_buffers`.

```
write(connection_id=<id>, data="audio stop\r\n")
close(connection_id=<id>)
```

---

### Test 4 — Buffer Budget Under Flood

**Goal:** `max_buffered_bytes` stops reads cleanly without error.

```
open(...)
write(..., data="audio i2s-test\r\n")
read(connection_id=<id>, max_buffered_bytes=256, timeout_ms=5000)
```

Confirm:
- result is **not** `is_error`
- `stop_reason` is `max_buffered_bytes` (or `timeout` if timeout wins first)
- `data` length ≤ 256 bytes

Repeat with `max_buffered_bytes=2048` and `max_buffered_bytes=65536`.

```
write(..., data="audio stop\r\n")
close(...)
```

---

### Test 5 — Match on Read

**Goal:** `read(match=...)` stops immediately on first pattern match.

```
open(...)
write(..., data="audio i2s-test\r\n")
read(connection_id=<id>, timeout_ms=3000, max_buffered_bytes=4096, match={
  "pattern": "Queued TX",
  "config": {
    "mode": "literal_substring",
    "pattern_encoding": "utf8"
  }
})
```

Confirm:
- `matched = true`
- `match_index` is not null
- `stop_reason = match_found`

```
write(..., data="audio stop\r\n")
close(...)
```

---

### Test 6 — Pre-Match Context Shaping

**Goal:** `context_amount_of_matched_bytes` returns the bytes before the match plus the match itself.

```
open(...)
write(..., data="audio i2s-test\r\n")
read(connection_id=<id>, timeout_ms=3000, max_buffered_bytes=4096, match={
  "pattern": "Queued TX",
  "config": {
    "mode": "literal_substring",
    "pattern_encoding": "utf8",
    "context_amount_of_matched_bytes": 32
  }
})
```

Confirm:
- `matched = true`
- `data` starts up to 32 bytes before the match
- `data` contains `Queued TX`
- `match_index` points at the start of `Queued TX` within the shaped payload

```
write(..., data="audio stop\r\n")
close(...)
```

---

### Test 7 — Background Subscribe Under Flood

**Goal:** background streaming works under sustained traffic.

```
open(...)
subscribe(connection_id=<id>, max_buffered_bytes=2048, poll_interval_ms=50)
write(..., data="audio i2s-test\r\n")
```

Observe `notifications/message` from `serial:<connection_id>`. Confirm repeated data notifications arrive.

```
write(..., data="audio stop\r\n")
unsubscribe(connection_id=<id>)
close(...)
```

---

### Test 8 — Subscribe Match Stop

**Goal:** `subscribe(match=...)` streams then self-stops on first match.

```
open(...)
subscribe(connection_id=<id>, max_buffered_bytes=4096, match={
  "pattern": "Queued TX",
  "config": {
    "mode": "literal_substring",
    "pattern_encoding": "utf8",
    "context_amount_of_matched_bytes": 32
  }
})
write(..., data="audio i2s-test\r\n")
```

Confirm:
- normal stream notifications arrive first
- final stop notification has `stop_reason = match_found`
- final notification includes shaped payload with pre-match context and match text

```
write(..., data="audio stop\r\n")   # if board still flooding
close(...)
```

---

### Test 9 — Silence Timeout

**Goal:** `no_new_rx_timeout_ms` stops cleanly when the line goes quiet — not treated as transport failure.

Ensure board is quiet first:

```
write(..., data="audio stop\r\n")
```

Then:

```
read(connection_id=<id>, timeout_ms=5000, no_new_rx_timeout_ms=300, max_buffered_bytes=2048)
```

Confirm:
- result is **not** `is_error`
- `stop_reason = no_new_rx_timeout`
- `data` may be empty

Repeat using `subscribe` — final notification should report `stop_reason = no_new_rx_timeout`.

---

### Test 10 — Close While Active

**Goal:** active RX operations terminate correctly on connection close.

```
open(...)
subscribe(connection_id=<id>, max_buffered_bytes=4096)
write(..., data="audio i2s-test\r\n")
```

While notifications are arriving:

```
close(connection_id=<id>)
```

Confirm the subscribe task terminates and the final notification (if any) reflects connection closure, not an error.

---

### Test 11 — Board Health Check

**Goal:** stress testing didn't destabilize the audio pipeline.

```
open(...)
write(..., data="audio status\r\n")
read(connection_id=<id>, max_buffered_bytes=2048, timeout_ms=1500)
close(...)
```

Confirm response contains `I2S underruns: 0`.

---

### Stress Test Ideas

Run these during `audio i2s-test` flood for deeper validation:

- Repeated `subscribe` / `unsubscribe` loops
- Rapidly replace an active subscribe with a new one using different `match` config
- Sweep `max_buffered_bytes` from very small (64) to very large (65536)
- Trigger `read(match=...)` while a `subscribe` is active to stress the shared RX pump
- Use `context_amount_of_matched_bytes=0` to confirm match-only payload (no pre-match bytes)

---

### Expected Good Signs

- No stale bytes after close → reopen
- No `is_error` on normal stop conditions (`timeout`, `max_buffered_bytes`, `no_new_rx_timeout`)
- `match_index` always points to the matched text in the shaped payload
- Stop metadata is internally consistent across `bytes_read`, `bytes_observed`, `bytes_returned`
- Final subscribe notification always arrives; no silent task leak

---

### Session Cleanup Checklist

Before disconnecting the board:

1. `write(..., data="audio stop\r\n")`
2. `unsubscribe` any active subscriptions
3. `close` all open serial connections
4. Verify with `list_connections` — result should be empty
