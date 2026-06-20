# Testing Gaps — serial-mcp

## Purpose

Document known test coverage gaps that are currently acceptable —
things that are **not bugs** but are not yet tested. Each gap has a
reason for not being filled and a note on when it might become worth
addressing.

---

## Device identity (PortInfo)

| Gap | Reason not filled | When to revisit |
|---|---|---|
| No `PortInfo` serde roundtrip test | Covered by schema validation tests and integration tests that check JSON fields | If identity schema changes |
| No USB VID/PID/serial populated-path test | Requires real USB serial hardware, not available in CI | When hardware-in-loop CI exists |
| `ListPortsResult` schema not in `all_result_types_have_valid_schema` | Tool schema assertion tests already cover it, result struct is simple | If tool result type changes |

---

## get_status tool

| Gap | Reason not filled | When to revisit |
|---|---|---|
| `is_open=false` never verifiable | `ConnectionManager::close()` removes the connection from the map; get_status returns "not found" error before `is_open` can be read as `false`. The field exists in the schema but is always `true` when reachable. | If auto-reconnect keeps closed connections alive |
| `last_activity_ms` precision not wall-clock verified | Tested as non-null after I/O but not compared against real elapsed time. Acceptable for a status field. | If time precision becomes critical for reconnect timing |
| `GetStatusResult` schema not in proptest | Covered by tool schema assertion tests; result type is a flat projection of `ConnectionStatus` | If result type gains complex nested fields |
| No counter overflow test | `AtomicU64` wraps at 2^64 bytes. Not reachable in any reasonable test duration. | Never — theoretical only |

---

## reconfigure tool

| Gap | Reason not filled | When to revisit |
|---|---|---|
| Reconfigure while subscription active | Low-priority edge case; reconfigure does not touch the RX pump. Behavior is harmless but untested. | If reconfigure starts affecting stream task state |
| Reconfigure during connection close window | `ConnectionManager` uses `closing_ports` set for a brief window. Reaching this in a test requires tight timing. Acceptable for now. | If close/proxy mode makes this window wider |
| Reconfigure when hardware rejects valid value | Would need a mock `SerialIo` that returns `Err` for specific reconfigure calls. Acceptable — real hardware rarely rejects valid values. | If a target platform is known to reject certain baud rates |
| `ReconfigureResult` schema not in proptest | Covered by tool schema assertion tests | If result type gains complex fields |
| `parse_string_stop_bits` / `parse_string_parity` unit-level only | Error paths tested via integration (bogus values return `is_error: true`). Unit-level parse tests exist for flow_control only. | If parse logic grows complex helper-specific edge cases |

---

## Profile-based target selection

| Gap | Reason not filled | When to revisit |
|---|---|---|
| `open_profile` "no port matches selector" error | Requires injecting a profile with a selector that matches nothing into the server. Test infrastructure (`TestServer`) does not yet support profile injection. | When `TestServer` gains a `start_with_profiles` constructor |
| `open_profile` with real config file | Only empty-config tested. No test loads actual `profiles.toml` via `list_profiles` because server reads from user config dir, not test fixture. | When test infrastructure supports injecting a config path |
| `open_profile` name override | `OpenProfileArgs.name` takes precedence over `ProfileDefaults.name` prefix. Code path untested. | Low priority — simple string logic, unlikely to regress |
| `open_profile` defaults propagation | Verified indirectly (open via profile → get_status confirms config). No direct assertion that all 5 defaults are correctly forwarded to `open()`. | If defaults handling grows complexity |
| `open_profile` allowlist interaction | Allowlist may block the matched port before open. Untested because allowlist allows all in most tests. | If allowlist becomes more restrictive by default |
| `ProfileDefaults` TOML defaults | No test verifies that missing TOML fields default to `115200`/`8`/`1`/`none`/`none`. | Low priority — covered by Rust struct defaults and serde defaults |

---

## Cross-cutting

| Gap | Reason not filled | When to revisit |
|---|---|---|
| No PTY-level get_status/reconfigure/profile tests in `serial_pty.rs` | PTY tests are lower-level transport tests; native_sim covers the same paths with real firmware behavior | If PTY-only platform bugs are suspected |
| No `ReadResult` / `WriteResult` schema roundtrip in proptest for new fields | Existing result types unchanged; new tools have their own result types which are schema-asserted | If existing result types gain new identity/config fields |

---

## Notes

- All listed gaps are **acceptable** — no bug is indicated.
- Each gap has been reviewed against the current test coverage (162 unit, 41 HTTP, 18 native_sim, 6 lifecycle, 13 PTY, 51 prop, plus schema + protocol emulator tests).
- Gaps marked "when to revisit" should be reconsidered when the listed condition becomes true.
