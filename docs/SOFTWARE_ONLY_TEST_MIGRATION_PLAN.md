# Software-Only Test Migration Plan

> **Status: Implemented.**
> See `docs/TEST_BUILD_UNIFICATION_PLAN.md` for the successor plan (also implemented).
> This document remains as design rationale and historical reference.

## Goal

Move repo to **software-only validation of serial-mcp server quality**.

Target state:

- No required physical boards
- No required USB-serial dongles
- No required PicoProbe / SWD / `pyocd`
- No required XIAO BLE / UF2 / Nordic hardware workflows
- CI and local contributors can run the important test coverage on a normal Linux host

Out of scope after this migration:

- Real-board bring-up confidence
- Native USB enumeration on physical hardware
- UF2 bootloader / flash layout validation
- PicoProbe bridge validation

This matches repo goal: **serial-mcp server quality**, not hardware demo quality.

---

## Current state

### Already solved in software

- `tests/native_sim_validation.rs` covers the old XIAO UART behavior set in software.
- `tests/bootloader_touch_emulated.rs` covers 1200-baud touch → bootloader-entry behavior in software via USB/IP.
- CI now runs native_sim firmware build + test.

### Remaining hardware-dependent pieces

| Area | Current artifact | Why it exists today | Keep? |
|---|---|---|---|
| XIAO UART behavior | `tests/xiao_ble_validation.rs` | Historical hardware validation of firmware command path | **No** |
| Generic real serial loopback | `tests/hardware_loopback.rs` | End-to-end OS serial port sanity on real adapter | **No** |
| E83 live board | `tests/e83_live_validation.rs` | Regression test for connection lifecycle on a specific board | **No** |
| XIAO board support | `firmware/boards/xiao_ble*` | Hardware firmware target | **No** |
| UF2 flash layout | `firmware/pm_static.yml` | XIAO bootloader-safe app address | **No** |
| XIAO flashing helpers | `firmware/bin/fw-build-xiao*`, `fw-flash-xiao` | Hardware build/flash workflow | **No** |
| Bootloader blob | `firmware/bootloader/*.hex` | XIAO UF2 recovery | **No** |
| Hardware bootloader plan | `firmware/UF2_BOOTLOADER_PLAN.md` | XIAO-specific future work | **No** |

---

## Principle for replacements

Do **not** port hardware tests 1:1 if the hardware-specific behavior is irrelevant to server quality.

Instead:

1. Identify what server behavior the hardware test was actually protecting.
2. Re-test that behavior with `native_sim`, PTY integration, or in-memory loopback.
3. Delete the hardware artifact once equivalent or better software coverage exists.

Priority order for new coverage:

1. `native_sim` firmware tests
2. PTY integration (`tests/serial_pty.rs`)
3. in-memory loopback / HTTP integration if native_sim is unnecessary

Use non-native_sim fallback only when native_sim does not add signal.

---

## Coverage mapping: old hardware signal → software replacement

### 1. `tests/xiao_ble_validation.rs`

This is already effectively replaced.

`firmware/UNIFIED_FIRMWARE_PLAN.md` and `firmware/AGENTS.md` state the native_sim suite carries the same command-path coverage. Current native_sim tests already cover:

- ping roundtrip
- pending read + later write
- split writes preserving order
- framing diagnostics
- byte trace diagnostics
- read(match=...) stop behavior
- subscribe(match=...) stop behavior
- subscribe silence timeout
- buffer budget under flood
- subscribe timeout under flood
- close while subscribe active

### 2. `tests/hardware_loopback.rs`

Existing hardware tests:

- `hw_loopback_write_then_read_roundtrip`
- `hw_loopback_read_match_echo`

Software equivalents already exist:

- `tests/serial_pty.rs`
  - `pty_client_write_reaches_device_side`
  - `pty_device_write_then_client_read`
  - `pty_read_match_finds_real_serial_pattern`
- `tests/native_sim_validation.rs`
  - `native_pending_read_then_write_ping_roundtrip`
  - `native_read_match_on_spam_complete`

Conclusion: `hardware_loopback.rs` can be removed once we explicitly document PTY/native_sim as the replacement layer.

### 3. `tests/e83_live_validation.rs`

This is the only remaining area with real coverage gaps.

What it actually protects:

1. Named connection appears in `list_connections`
2. `set_flow_control` tool returns success and updates tracked config
3. Closing while a `read` is pending produces a close-related error
4. Reopening the same port after close works
5. Fresh reads/matches work after reopen

What is hardware-specific and should **not** be preserved:

- `audio stop`
- `audio i2s-test`
- matching `Queued TX` / `supply_next_buffers`
- anything tied to E83 firmware behavior rather than MCP server behavior

Those board-specific commands should be replaced by generic native_sim firmware commands such as `ping`, `spam`, `trace`, and `framing`.

---

## New tests to add before deleting the last hardware-only suites

Create a small new software-only suite, either:

- append to `tests/native_sim_validation.rs`, or
- create `tests/native_sim_connection_lifecycle.rs`

Recommended test list:

### A. `native_named_connection_appears_in_list_connections`

Purpose:

- open native_sim PTY with explicit `name`
- call `list_connections`
- assert entry contains:
  - `connection_id`
  - `name`
  - `port`
  - `baud_rate`
  - `flow_control == "none"`

Why:

- replaces the first half of `e83_live_validation`
- makes named-connection summary coverage explicit in software

### B. `native_set_flow_control_updates_summary_and_result`

Purpose:

- open connection
- call `set_flow_control`
- assert tool result returns expected flow-control mode
- call `list_connections`
- assert summary shows updated `flow_control`

Implementation note:

- If PTY/native_sim backend rejects software/hardware flow control, this test should use the integration harness over a backend that accepts the operation.
- Best fallback: test-support loopback (`src::serial::test_support::loopback_connection`) behind an integration/server harness.
- Goal is server contract validation, not kernel TTY feature validation.

### C. `native_close_while_read_active_returns_close_error`

Purpose:

- start `read(timeout_ms=...)` on open native_sim connection
- close connection from another task before data arrives
- assert read tool result has `is_error: true`
- assert message contains `closed` or `Connection closed`

Why:

- current unit test covers internal primitive (`close_cancels_inflight_read`)
- missing full MCP integration coverage
- replaces the most important part of `e83_live_validation`

### D. `native_reopen_same_port_after_close_works`

Purpose:

- open native_sim PTY
- sync boot banner
- close connection
- reopen same PTY path
- verify `ping` → `pong`

Why:

- replaces E83 reopen behavior without board-specific firmware

### E. `native_reopen_then_match_finds_fresh_output`

Purpose:

- reopen same PTY after close
- issue deterministic command after reopen (`spam ...`, `trace on`, or `ping`)
- assert `read(match=...)` sees fresh post-reopen output only

Why:

- replaces E83 post-reopen `Queued TX` behavior
- makes “fresh stream after reopen” explicit

### Optional F. `pty_set_flow_control_roundtrip_updates_summary`

If native_sim path is awkward for flow-control testing, use a PTY-backed integration test instead of forcing firmware into it.

Reason:

- flow-control behavior is server/connection bookkeeping, not firmware behavior

---

## Tests that should be deleted after replacements land

### Delete outright

- `tests/xiao_ble_validation.rs`
- `tests/hardware_loopback.rs`
- `tests/e83_live_validation.rs`

### Update docs that mention them

- `README.md`
- `docs/TESTING.md`
- `AGENTS.md`
- `firmware/AGENTS.md`
- `CHANGELOG.md`

---

## Firmware/tooling removal plan

## Phase 1 — prove software-only parity

1. Add the new software-only lifecycle tests listed above.
2. Run:

```bash
cargo test --test native_sim_validation -- --ignored
cargo test --test serial_pty
cargo test --test http_integration
cargo test --lib
```

3. Confirm no remaining server-quality requirement needs physical hardware.

Exit criteria:

- lifecycle coverage from `e83_live_validation.rs` exists in software
- hardware_loopback value is clearly subsumed by PTY/native_sim tests

## Phase 2 — remove hardware tests from Rust test tree

Delete:

- `tests/xiao_ble_validation.rs`
- `tests/hardware_loopback.rs`
- `tests/e83_live_validation.rs`

Then update `docs/TESTING.md`, `README.md`, and root `AGENTS.md` so “software-only baseline” becomes the default message everywhere.

Exit criteria:

- `cargo test --all-targets`
- `cargo clippy --all-targets -- -D warnings`
- CI green

## Phase 3 — remove XIAO firmware target

Delete:

- `firmware/boards/xiao_ble.conf`
- `firmware/boards/xiao_ble_usb.conf`
- `firmware/boards/xiao_ble_usb.overlay`
- `firmware/pm_static.yml`
- `firmware/bin/fw-build-xiao`
- `firmware/bin/fw-build-xiao-usb`
- `firmware/bin/fw-flash-xiao`
- `firmware/bootloader/Seeed_XIAO_nRF52840_bootloader-0.6.1_s140_7.3.0.hex`
- `firmware/UF2_BOOTLOADER_PLAN.md`

Rewrite:

- `firmware/AGENTS.md`
- `firmware/UNIFIED_FIRMWARE_PLAN.md` (or replace with native_sim-only plan)

Exit criteria:

- no `xiao_ble` refs outside changelog/history notes
- no `pyocd` workflow in docs/helpers

## Phase 4 — simplify firmware source

### `firmware/src/usb_cdc.c`

Remove device-next / hardware path:

- delete `CONFIG_USB_DEVICE_STACK_NEXT` branch
- delete GPREGRET / NVIC reset branch
- keep native_sim legacy USB stack only
- keep `exit(42)` bootloader entry behavior

### `firmware/src/main.c` and `firmware/src/usb_cdc.h`

Rewrite comments from “dual-target firmware” to “native_sim test firmware”.

Potential rename later:

- `UNIFIED_FIRMWARE_PLAN.md` → `NATIVE_SIM_FIRMWARE_PLAN.md`

Exit criteria:

- firmware code no longer mentions XIAO, PicoProbe, UF2, or hardware reset registers

## Phase 5 — simplify Nix/dev tooling

Candidates to remove from `flake.nix` once XIAO support is fully gone:

- `pyocd`
- `segger-jlink.acceptLicense = true`
- possibly `allowUnfree = true` if nothing else still requires it

Keep if still needed:

- `nrfutil`
- multilib GCC (`gccMultiStdenv.cc`) for native_sim

Re-check whether NCS source + toolchain are still the lightest build path. If not, a later follow-up could reduce further toward upstream Zephyr-only native_sim.

---

## Recommended file end-state after migration

### Keep

- `tests/native_sim_validation.rs`
- `tests/bootloader_touch_emulated.rs`
- `tests/serial_pty.rs`
- `tests/http_integration.rs`
- native_sim firmware board files:
  - `firmware/boards/native_sim.conf`
  - `firmware/boards/native_sim_usb.conf`
  - `firmware/boards/native_sim_usb.overlay`
- firmware helpers:
  - `fw-build-native`
  - `fw-build-native-usb`
  - `fw-run-native`
  - `fw-run-native-usb-attached`

### Remove

- all XIAO-specific boards
- all XIAO-specific helpers
- all UF2/bootloader materials
- all hardware-only tests

---

## Risk notes

### Risk 1 — flow-control coverage may not fit native_sim cleanly

Mitigation:

- use PTY or loopback integration for `set_flow_control` contract
- do not block de-hardware migration on kernel TTY capability differences

### Risk 2 — deleting hardware tests removes “real iron” confidence

Accepted trade-off:

- repo goal is server quality
- hardware bring-up belongs in separate board-specific repos or private validation docs

### Risk 3 — docs drift during staged removal

Mitigation:

- do not delete hardware files before docs and AGENTS are updated in the same change set

---

## Success criteria

Migration is done when all are true:

1. No test in repo requires physical hardware.
2. CI covers all important serial-mcp server-quality behaviors.
3. No XIAO / UF2 / `pyocd` / PicoProbe workflow remains in normal contributor docs.
4. Firmware test target is native_sim only.
5. A new contributor can run the meaningful validation path without buying anything.

---

## Recommended implementation order

1. Add new software-only lifecycle tests replacing `e83_live_validation.rs`
2. Delete `hardware_loopback.rs`
3. Delete `e83_live_validation.rs`
4. Delete `xiao_ble_validation.rs`
5. Remove XIAO board files / helpers / UF2 artifacts
6. Simplify firmware source to native_sim-only
7. Simplify flake/devshell
8. Rewrite docs to present native_sim as the only supported firmware target

This order keeps signal high and avoids deleting evidence before replacement coverage exists.
