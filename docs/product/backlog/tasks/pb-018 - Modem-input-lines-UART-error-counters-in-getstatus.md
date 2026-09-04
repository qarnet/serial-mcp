---
id: PB-018
title: "Modem input lines + UART error counters in get_status"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:S'
  - 'area:serial'
dependencies: []
priority: p2
type: feature
ordinal: 1180
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

CTS/DSR/CD/RI states and parity/framing/overrun counters are invisible.

### Desired outcome

Read modem input lines and UART error counters exposed as cheap additive
`get_status` fields.

### Scope

- CTS/DSR/CD/RI read.
- Parity/framing/overrun counters.

### Non-goals

- Changing control-line output behavior.

### Technical context

Cheap additive wire-format change; get_status result struct.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Fields present on platforms that expose them; absent (not error) on platforms that do not.
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
