---
id: PB-007
title: "Modbus ASCII TX auto-LRC"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
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
- [ ] #1 Writing a binary PDU through the mode produces a valid Modbus ASCII frame on the wire (verified by PTY fixture).
- [ ] #2 RX validation behavior unchanged.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
<!-- SECTION:FINAL_SUMMARY:END -->
