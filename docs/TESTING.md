# Testing Guide — serial-mcp

---

## Quick Reference

```bash
# Orchestrator (preferred — runs all CI-critical suites in order)
cargo run --manifest-path xtask/Cargo.toml -- test
cargo run --manifest-path xtask/Cargo.toml -- test-all   # + HTTP integration
cargo run --manifest-path xtask/Cargo.toml -- build-test-assets

# Individual suites
cargo test --lib                    # 149 unit tests
cargo test --test http_integration  # 23 HTTP tests (13 via spawned binary)
cargo test --test stdio_integration # 3 + 1 hw-skipped stdio tests
cargo test --test blob_resources    # 2 blob resource tests

# Firmware tests (require native_sim firmware, see firmware/AGENTS.md)
cargo test --test native_sim_validation -- --ignored
cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1
```

---

## Test Layers

Tests are layered from fastest/no-hardware to slowest/hardware-required.

### Layer 1 — Unit Tests (in-source, `src/**`)

Run via `cargo test --lib`.

| Module | Tests | What they cover |
|--------|-------|-----------------|
| `src/serial.rs` | `baud_rate_zero_rejected`, `baud_rate_over_max_rejected`, `baud_rate_within_range_accepted` | Baud rate validation rejects 0 and out-of-range values |
| `src/serial.rs` | `write_pushes_bytes_to_peer`, `read_returns_peer_bytes`, `read_times_out_when_no_data` | In-memory loopback I/O and timeout |
| `src/serial.rs` | `flush_set_dtr_rts_send_break_are_noops_on_loopback` | Control ops on in-memory backend are safe no-ops |
| `src/serial.rs` | `manager_rejects_duplicate_port` | `ConnectionManager` blocks double-open of same port |
| `src/serial.rs` | `manager_duplicate_port_error_includes_owner_metadata` | Error message includes connection ID and name |
| `src/serial.rs` | `manager_close_then_get_returns_connection_not_found` | Get after close returns `ConnectionNotFound` |
| `src/serial.rs` | `manager_get_unknown_id_returns_connection_not_found` | Unknown connection ID returns `ConnectionNotFound` |
| `src/serial.rs` | `close_cancels_inflight_read` | Closing a connection unblocks a blocked `read` |
| `src/rx_session.rs` | `rx_event_clone_copies_data` | `RxEvent` clones correctly |
| `src/rx_session.rs` | `manager_get_or_create_returns_same_session`, `manager_remove_awaits_pump_exit`, `manager_remove_nonexistent_is_noop` | Session manager lifecycle |
| `src/rx_session.rs` | `session_register_blocking_starts_pump`, `session_shutdown_cancels_pump_token` | Per-session pump start/stop |
| `src/rx_session.rs` | `consumer_receives_data_after_registration`, `two_consumers_both_receive_future_data` | Fan-out: multiple consumers get same data |
| `src/rx_session.rs` | `removing_session_awaits_pump_and_drops_consumers`, `connection_close_causes_pump_exit`, `pump_exits_cleanly_on_shutdown_without_hanging` | Graceful shutdown paths |
| `src/rx_session.rs` | `no_consumers_means_no_pump`, `shutdown_is_idempotent`, `repeated_create_remove_no_leaked_pump_tasks` | Edge cases: no consumers, idempotent shutdown, no task leak |
| `src/rx_session.rs` | `full_consumer_is_dropped_from_registry`, `dropped_receiver_removed_from_registry` | Consumer cleanup when channel fills or receiver drops |
| `src/stop_controller.rs` | `timeout_stops_at_deadline`, `continue_before_deadline`, `no_timeout_without_deadline` | Wall-clock timeout stop condition |
| `src/stop_controller.rs` | `match_found_stops_immediately`, `no_match_continues`, `max_buffered_bytes_stops`, `match_found_takes_priority_over_max_bytes` | Matcher and buffer-budget stop conditions |
| `src/stop_controller.rs` | `connection_closed_outcome`, `channel_closed_outcome`, `cancelled_outcome`, `read_error_outcome`, `peer_disconnected_outcome`, `data_complete_outcome` | All stop-reason outcome variants |
| `src/stop_controller.rs` | `is_normal_stop_classifies_correctly` | Normal vs. error stop classification |
| `src/stop_controller.rs` | `push_data_without_matcher_accumulates`, `timeout_preserves_match_state_from_earlier_data`, `check_max_buffered_bytes_after_record_data`, `record_data_does_not_trigger_stops` | Data accumulation logic |
| `src/stop_controller.rs` | `silence_timeout_stops_when_expired`, `silence_timeout_continues_with_future_deadline`, `silence_timeout_disabled_when_none`, `notify_data_received_resets_silence_deadline`, `silence_timeout_is_normal_stop`, `silence_timeout_with_data_produces_bytes_in_outcome` | `no_new_rx_timeout_ms` silence timeout |
| `src/stop_controller.rs` | `both_timeouts_can_be_set_independently`, `no_new_rx_timeout_metadata_preserves_bytes`, `no_new_rx_timeout_with_bytes` | Interaction between wall-clock and silence timeout |
| `src/codec.rs` | `encoding_from_str_accepts_aliases`, `encoding_from_str_rejects_unknown` | `Encoding` parsing |
| `src/codec.rs` | `utf8_roundtrip`, `utf8_encode_rejects_invalid_bytes` | UTF-8 encode/decode; invalid bytes return error (not lossy replacement) |
| `src/codec.rs` | `hex_roundtrip`, `hex_odd_length_rejected`, `hex_invalid_chars_rejected` | Hex encode/decode edge cases |
| `src/codec.rs` | `base64_roundtrip_and_padding_variants`, `binary_roundtrips_via_hex_and_base64` | Base64 roundtrip and binary fidelity |
| `src/buffer_budget.rs` | `atomic_budget_reserve_and_release`, `atomic_budget_over_tool_limit`, `atomic_budget_zero_request`, `atomic_budget_insufficient_program`, `atomic_budget_concurrent_reserve` | `AtomicBudget` reserve/release and limit enforcement |
| `src/buffer_budget.rs` | `atomic_budget_rejects_zero_limits` | Budget rejects zero-size limits |
| `src/buffer_budget.rs` | `fake_budget_basic`, `fake_budget_exhaustion`, `unlimited_budget_always_succeeds`, `unlimited_budget_still_rejects_over_tool_limit`, `unlimited_budget_rejects_zero` | Test-double budget implementations |

---

### Layer 2 — Property Tests (`tests/proptest.rs`)

Run via `cargo test --test proptest`. Uses `proptest` to generate random inputs.

| Block | Tests | What they cover |
|-------|-------|-----------------|
| Phase A.1 — Schema roundtrips | `open_args_roundtrip`, `close_args_roundtrip`, `write_args_roundtrip`, `read_args_roundtrip`, `flush_args_roundtrip`, `set_dtr_rts_args_roundtrip`, `send_break_args_roundtrip`, `subscribe_args_roundtrip`, `unsubscribe_args_roundtrip` | All tool arg structs survive serde roundtrip with arbitrary valid inputs |
| Phase A.2 — Result schema validation | `open_result_schema_valid`, `close_result_schema_valid`, `write_result_schema_valid`, `read_result_schema_valid`, `flush_result_schema_valid`, `set_dtr_rts_result_schema_valid`, `send_break_result_schema_valid`, `subscribe_result_schema_valid`, `unsubscribe_result_schema_valid` | All result structs validate against their JSON schemas |
| Phase A.3 — Encoding roundtrips | `hex_encode_decode_roundtrip`, `base64_encode_decode_roundtrip` | Arbitrary byte slices survive hex/base64 roundtrip |
| Phase A.4 — Boundary helpers | `clamp_or_err_never_panics`, `require_min_or_err_never_panics` | Clamp/min helpers never panic on arbitrary input |

---

### Layer 3 — HTTP Integration (`tests/http_integration.rs`)

Run via `cargo test --test http_integration`. 13 tests spawn a real `serial-mcp --transport=http` child process; 10 tests that inject a custom `ConnectionManager` still run in-process. Uses a real MCP client over localhost.

| Test | What it covers |
|------|---------------|
| `initialize_handshake_succeeds` | MCP init/handshake completes |
| `list_tools_returns_all_twelve_tools` | Server advertises exactly 12 tools |
| `list_resources_returns_two_statics` | Static resource list is correct |
| `list_connections_returns_open_connection_summaries` | `list_connections` returns open connections with names |
| `list_resources_pagination_with_cursor_returns_next_page` | Resource pagination cursor works |
| `list_resource_templates_returns_connection_template` | Connection resource template advertised |
| `list_resource_templates_pagination_with_cursor_returns_next_page` | Template pagination cursor works |
| `list_prompts_returns_diagnose_and_interactive` | Both prompts are advertised |
| `read_serial_ports_resource_returns_json_payload` | `serial://ports` resource returns JSON |
| `read_unknown_resource_yields_not_found` | Unknown resource URI returns not-found error |
| `read_unknown_connection_yields_not_found` | Unknown connection resource URI returns not-found error |
| `call_tool_open_with_bad_data_bits_returns_is_error` | Invalid arg returns `is_error: true` (not a protocol error) |
| `call_tool_list_ports_returns_structured_result` | `list_ports` returns valid structured JSON |
| `get_prompt_diagnose_port_returns_user_message` | `diagnose_port` prompt returns a user message |
| `write_tool_sends_bytes_to_loopback_peer` | `write` delivers bytes to in-memory peer |
| `subscribe_then_peer_write_pushes_notification` | `subscribe` receives data as MCP notifications |
| `subscribe_with_timeout_auto_stops_in_background` | `subscribe(timeout_ms=...)` self-terminates |
| `subscribe_without_timeout_is_fire_and_forget` | `subscribe` without timeout returns immediately |
| `subscribe_closed_from_other_session_stops_streaming_task` | Connection close stops the subscribe task |
| `validation_limits_return_tool_errors_over_http` | Over-limit args return `is_error: true` |
| `read_with_no_data_times_out_with_is_error` | `read` timeout returns `is_error: true` |
| `read_result_contains_elapsed_ms` | `read` result includes `elapsed_ms` field |
| `send_break_result_includes_actual_duration` | `send_break` result includes timing |

---

### Layer 4 — PTY Integration (`tests/serial_pty.rs`)

Run via `cargo test --test serial_pty`. Uses real POSIX PTYs (pseudoterminals) to simulate serial hardware. **Linux/macOS only.**

| Test | What it covers |
|------|---------------|
| `pty_open_returns_connection_id` | `open` returns a connection ID for a real PTY |
| `pty_client_write_reaches_device_side` | Bytes written via `write` tool appear on PTY device side |
| `pty_device_write_then_client_read` | Bytes written by device side are returned by `read` tool |
| `pty_subscribe_streams_device_writes_as_notifications` | Device writes arrive as subscribe notifications |
| `pty_read_match_finds_real_serial_pattern` | `read(match=...)` stops on a regex match against real serial data |
| `pty_read_match_with_context_returns_shaped_payload` | `context_amount_of_matched_bytes` shapes pre-match context |
| `pty_read_match_with_zero_context_returns_only_matched_bytes` | Zero context returns only matched bytes |
| `pty_read_match_without_context_returns_full_accumulated` | No context field returns full accumulated buffer |
| `pty_subscribe_match_with_context_includes_shaped_data` | `subscribe(match=..., context=...)` includes shaped pre-match data in notification |
| `pty_subscribe_match_stops_without_context` | `subscribe(match=...)` self-stops with `match_found` (no context); exercises subscribe stop-bug fix |
| `pty_subscribe_silence_timeout_stops` | `subscribe(no_new_rx_timeout_ms=300)` fires on a silent PTY within expected window |
| `pty_close_then_use_returns_is_error` | Using a closed connection returns `is_error: true` |
| `pty_send_break_short_duration_timing` | `send_break` on a PTY completes in expected time window |

---

### Layer 5 — stdio Integration (`tests/stdio_integration.rs`)

Run via `cargo test --test stdio_integration`. Spawns the binary and communicates over stdin/stdout.

| Test | What it covers |
|------|---------------|
| `stdio_initialize_handshake_succeeds` | Binary starts and completes MCP handshake |
| `stdio_list_tools_returns_all_twelve_tools` | Binary advertises exactly 12 tools |
| `stdio_list_resources_returns_statics_and_templates` | Binary advertises static resources and connection template |
| `stdio_full_connection_lifecycle_with_hardware` *(ignored)* | Full open/write/read/close with real hardware loopback |

---

### Layer 6 — Allowlist (`tests/allowlist.rs`)

Run via `cargo test --test allowlist`. Verifies port allowlist security policy.

| Test | What it covers |
|------|---------------|
| `empty_allowlist_allows_any_port` | No allowlist = permissive |
| `exact_match_blocks_unauthorized_port` | Exact pattern blocks non-matching port |
| `exact_match_allows_authorized_port` | Exact pattern allows matching port |
| `glob_pattern_matches_multiple_ports` | Glob pattern (e.g. `/dev/ttyUSB*`) matches multiple ports |
| `comma_separated_multiple_exact_ports` | Comma-separated list allows each listed port |

---

### Layer 7 — Blob Resources (`tests/blob_resources.rs`)

Run via `cargo test --test blob_resources`.

| Test | What it covers |
|------|---------------|
| `blob_resource_template_is_advertised` | `serial://connections/{id}/rx/raw` template exists |
| `resource_uri_parsing_includes_raw_suffix` | URI parser extracts connection ID and `raw` suffix correctly |

---

### Layer 8 — Resource Subscriptions (`tests/resource_subscriptions.rs`)

Run via `cargo test --test resource_subscriptions`.

| Test | What it covers |
|------|---------------|
| `resource_subscription_works` | MCP `resources/subscribe` receives change notifications on open/close |
| `resource_subscribe_unsubscribe_roundtrip` | Subscribe then unsubscribe stops notifications |

---

### Layer 9 — Protocol Emulators

Run via `cargo test --test protocol_emulator` / `cargo test --test protocol_emulator_binary`.

| Test | What it covers |
|------|---------------|
| `protocol_emulator` | MCP protocol-level fuzz: validates server handles arbitrary well-formed MCP messages |
| `protocol_emulator_binary` | Binary protocol: server handles and ignores non-JSON/malformed input without crashing |

---

### Layer 10 — Config Schema Validation (`tests/config_schema_validation.rs`)

Run via `cargo test --test config_schema_validation`.

| Test | What it covers |
|------|---------------|
| `config_schema_validates_against_local_schemas` | Generated JSON schemas in `schemas/` match current types |
| `config_schema_validates_against_upstream_schemas` *(ignored, requires network)* | Local schemas match latest upstream published schemas |

---

### Layer 11 — native_sim Firmware Lifecycle (`tests/native_sim_connection_lifecycle.rs`)

Run via `cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1`.
Requires the native_sim firmware binary (see `firmware/AGENTS.md`).

| Test | What it covers |
|------|---------------|
| `native_named_connection_appears_in_list_connections` | A named connection is correctly reported in `list_connections` |
| `native_set_flow_control_updates_summary_and_result` | `set_flow_control` tool returns the requested mode and `list_connections` reflects the update |
| `native_close_while_read_active_returns_close_error` | Closing while a `read` is pending surfaces a close-related error to the MCP caller |
| `native_reopen_same_port_after_close_works` | Reopening the same PTY after a clean close works and serves new commands |
| `native_reopen_then_match_finds_fresh_output` | After reopen, a fresh `read(match=...)` returns the response to a new command |
| `native_open_with_flow_control_persists_in_summary` | `flow_control` provided at `open` time is reflected in the connection summary |

The bootloader touch flow is exercised via the `touch` command in this
same suite — no separate test binary or USB/IP setup required.

---

### Layer 12 — Fuzz (`fuzz/`)

Not run by default. Uses `cargo-fuzz` (nightly required).

```bash
cargo +nightly fuzz run tool_call_json
```

| Target | What it covers |
|--------|---------------|
| `tool_call_json` | All tool arg structs survive arbitrary JSON input without panicking |

Currently fuzzes: `OpenArgs`, `CloseArgs`, `WriteArgs`, `ReadArgs`, `FlushArgs`, `SetDtrRtsArgs`, `SendBreakArgs`, `SubscribeArgs`, `UnsubscribeArgs`, `SetFlowControlArgs`.

---

## Coverage Gaps (from CLEANUP.md)

Known gaps that should be addressed in follow-up work:

1. **`subscribe(match=...)` memory stays within budget** — no test verifies that matcher state doesn't grow beyond `max_buffered_bytes`.
2. **Subscribe replacement budget ordering** — no test verifies that replacing an active subscribe doesn't spuriously fail due to budget exhaustion.
3. **Subscribe final stop counters consistency** — no test verifies `bytes_read`, `bytes_observed`, `bytes_returned` are internally consistent in the final notification.
4. **`no_new_rx_timeout_ms=0` schema vs. runtime mismatch** — schema allows `0`, runtime rejects it; no test covers this boundary.
5. **Proptest stop-reason coverage** — `no_new_rx_timeout` stop reason not yet covered by proptest.

---

## CI Requirements

All PRs must pass these in order:

```
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
```

See `.github/workflows/ci.yml` for the full matrix (includes nix flake check and multi-platform builds).
