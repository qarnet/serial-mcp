---
id: PB-007
title: Modbus ASCII TX auto-LRC
status: Review
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-10-17 23:01'
labels:
  - 'size:S'
  - 'area:protocol'
dependencies: []
priority: p1
type: feature
ordinal: 1070
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

TX-side Modbus ASCII cannot auto-append the LRC; only RX validation is
shipped. An agent must hand-compute the LRC and hex-encode the payload.

### Desired outcome

Hex-encode a binary PDU and append the LRC on write
(`:` + hex + LRC + `\r\n`).

### Scope

- TX framing mode for Modbus ASCII with automatic LRC append.
- Hex-encoding of the binary payload, not just a checksum append.

### Non-goals

- Modbus RTU (CRC-16) — separate future work.

### Technical context

Deliberately split from the NMEA TX auto-checksum that landed in 0.7.3.
Refactor trigger (one-consumer rule): when this lands, extract a shared TX
checksum-append layer instead of growing `TxFramingMode` variant-by-variant;
`TxFramingMode::Nmea` is the first checksum-appending mode, this is the
second, and a third (CRC-16 / FCS-16) should be a one-line diff. When CRC-16
lands, make `emit_frame`'s per-frame validation policy pluggable.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Writing a binary PDU through the mode produces a valid Modbus ASCII frame on the wire (verified by PTY fixture).
- [x] #2 RX validation behavior unchanged.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Add ModbusAscii TX mode and shared checksum frame append helper consumed by NMEA and Modbus. Preserve existing modbus_ascii preset wire behavior; expose auto-LRC via explicit tx_framing. Verify binary payload exact bytes through Linux PTY, regression-test RX validation and NMEA, update docs, and run required gates. No RTU, hardware, version bump, push, PR, merge or publication.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Refined from codecs.rs, config.rs and device_protocol_parity.rs. PB-001 remains In Progress and untouched; only local main branch exists at expected base 70774b079a87ee64e6f7e1d0b185c23ae51666d3. Approved sprint permits this separate localized item. No unresolved product questions.

Acceptance evidence: device_protocol_parity::modbus_ascii_auto_lrc_writes_binary_payload_over_pty observes exactly :01060001002ACE CRLF through public MCP write and Linux PTY; existing preset transact still passes. RX code unchanged; existing valid/bad-LRC decoder and parser tests pass, and the new all-byte TX/RX regression validates binary content. Broad default-parallel cargo test failed in unmodified ACK peer test (missing expected pong); required serial rerun and full serial suite pass. No unrelated fixture changes made.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Implemented explicit `tx_framing: {"type":"modbus_ascii"}` over binary address/function/data: uppercase hex, automatic LRC and colon/CRLF envelope. Shared `checksum_frame` now serves NMEA and Modbus and accepts checksum bytes in wire order. NMEA validation/pass-through, existing Modbus preset TX and RX validation remain unchanged.

Acceptance evidence: the public MCP write/PTY fixture (Linux PTY) observes exactly `:01060001002ACE\r\n`; binary vector tests cover empty, zero, overflow and marker bytes, and all 256 byte values roundtrip through validated RX. Existing valid/bad-LRC RX and preset transaction regressions pass.

Validation (all through `direnv exec .`, Rust gates with `RUSTFLAGS="-D warnings"`):
- `cargo fmt --all -- --check`: passed.
- `cargo build --all-targets --locked`: passed.
- `cargo test --locked`: initially failed the unmodified ACK peer test with missing expected pong under default parallel execution. No fixture changes made.
- `cargo test --locked --test device_command_parity -- --test-threads=1`: 19 passed.
- `cargo test --locked -- --test-threads=1`: 999 passed, 2 intentionally ignored; includes required Linux fixture (7), command (19), framing (8), protocol (16) suites.
- `cargo clippy --all-targets --locked -- -D warnings`: passed.
- `cargo test --locked --test device_parity_repeat public_boundary_repeat_gate -- --ignored --test-threads=1`: passed, 100 lifecycles.
- `nix flake check --no-update-lock-file`: both x86_64-linux checks passed (package and registry builder tests); other systems remain CI coverage.
- `bash scripts/test-mcp-compat.sh`: passed; historical rmcp 1.7.0 stdio/HTTP, 6 legacy + 7 modern conformance scenarios, Inspector smoke.
- `backlog doctor` and `git diff --check`: passed.

Updated protocol guide, Unreleased changelog and repository fast-truth/test map. Version stays 0.9.3: additive feature queued for next release; no policy requires a delivery-time bump. No dependency/toolchain/lock changes, no hardware, no push, PR, merge or publication. PB-001 remains untouched. Review status is implementation-complete awaiting bundling, not official acceptance; Done requires the PR gate. Team Lead owns import verification, independent QA and subsequent PR/CI phases after producer lease release.
<!-- SECTION:FINAL_SUMMARY:END -->
