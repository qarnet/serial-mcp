---
id: PB-005
title: "Declarative checksums on generic framing"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:framing'
dependencies: []
priority: p1
type: feature
ordinal: 1050
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Checksum handling is hardcoded inside the NMEA and Modbus presets.
Proprietary vendor protocols with different checksums get no validation.

### Desired outcome

A `checksum: { algorithm, ... }` option on `Delimiter`, `LengthPrefixed`,
and `StartEnd` framing that generalizes what the presets hardcode.

### Scope

- Declarative checksum configuration on generic framing modes.
- Covers the long tail of proprietary protocols without a full plugin API
  (lighter first step than PB-009).

### Non-goals

- The external decoder/plugin API (PB-009).
- CRC-16 unless the declarative layer makes it nearly free.

### Technical context

Natural follow-on to the checksum-helper refactor that landed in 0.7.3
(`src/checksums.rs`, `compute_checksum` tool).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Checksums configurable on the three generic framing modes.
- [ ] #2 Preset behavior unchanged (NMEA/Modbus presets keep working).
- [ ] #3 Controlled-backend and framing parity tests cover the new option.
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
