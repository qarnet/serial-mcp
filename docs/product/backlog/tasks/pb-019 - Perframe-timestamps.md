---
id: PB-019
title: "Per-frame timestamps"
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
ordinal: 1190
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Frames cannot be correlated against the event log or across
connections.

### Desired outcome

Timestamps on decoded frames.

### Scope

- Small additive wire-format change for frame timestamps.

### Non-goals

- Absolute time synchronization across machines.

### Technical context

Decide before 1.0 (wire-format change).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Timestamps present on frames; correlated against the event log in
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
