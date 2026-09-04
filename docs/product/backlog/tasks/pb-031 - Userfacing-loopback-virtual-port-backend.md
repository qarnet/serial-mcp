---
id: PB-031
title: "User-facing loopback / virtual port backend"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:platform'
dependencies: []
priority: p3
type: feature
ordinal: 1310
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Agents cannot demo or develop flows with no hardware attached.

### Desired outcome

Expose a virtual echo/scripted device as an openable backend.

### Scope

- Loopback/virtual backend openable through the standard surface.

### Non-goals

- Making the Rust PTY fixture (test-only) a production backend.

### Technical context

The Rust PTY fixture stays test-only.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Virtual device usable through open/transact without special flags.
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
