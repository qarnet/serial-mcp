---
id: PB-014
title: "Flow-control-aware ring backpressure (pause-on-full)"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:rx'
dependencies: []
priority: p2
type: feature
ordinal: 1140
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

The RX ring wraps when full, silently dropping oldest bytes; hardware
RTS/CTS backpressure semantics are not restored to the sender.

### Desired outcome

`on_full: "wrap" | "pause"` with observable paused-state events,
restoring hardware flow-control backpressure semantics.

### Scope

- Pause-on-full ring mode.
- Observable paused-state events.

### Non-goals

- Per-client cursors (PB-016).

### Technical context

src/rx_ring.rs, src/rx_session.rs.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Pause mode stops consuming without data loss while paused.
- [ ] #2 Paused/resumed states are observable through the public surface.
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
