---
id: PB-017
title: "Baud-rate auto-detection"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:serial'
dependencies: []
priority: p2
type: feature
ordinal: 1170
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Agents cannot determine an unknown device's baud rate; guessing is
wrong.

### Desired outcome

Host-side baud detection that returns inconclusive rather than guessing,
or no detection at all.

### Scope

- Evaluate host-side detection approaches.
- A built-in tool should return inconclusive rather than guess.

### Non-goals

- Waveform measurement (needs hardware).

### Technical context

Host-side detection over USB-serial is heuristic, not waveform.

### Open questions

Whether heuristic detection is worth shipping at all.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Detection approach studied from EXPLIoT `uart.generic.baudscan`.
- [ ] #2 Decision recorded: implement with honest inconclusive results, or drop
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
