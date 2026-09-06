---
id: PB-015
title: "Persistent per-connection framing decoder"
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
ordinal: 1150
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Framing decoder state is per-call; multi-call protocols requiring state
carried across reads cannot be handled cleanly.

### Desired outcome

Carry framing decoder state across `read` calls bound to the connection.

### Scope

- Bind framing (optionally) to the connection with persistent decoder
  state.

### Non-goals

- Changing the four-layer precedence for existing callers.

### Technical context

Requires rethinking the 4-layer precedence in src/precedence.rs.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Stateful protocols work across multiple `read` calls.
- [ ] #2 Four-layer precedence rethink documented and tested.
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
