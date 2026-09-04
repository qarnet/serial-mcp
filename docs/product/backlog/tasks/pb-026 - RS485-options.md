---
id: PB-026
title: "RS-485 options"
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
ordinal: 1260
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Half-duplex RS-485 buses need direction-control timing that the current
backend does not expose.

### Desired outcome

Half-duplex bus semantics: direction control timing, RTS-based send
control.

### Scope

- RS-485 mode options and RTS direction control.

### Non-goals

- Multi-drop addressing.

### Technical context

Needs physical half-duplex and direction-control testing.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Verified against physical half-duplex hardware (not just PTY).
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
