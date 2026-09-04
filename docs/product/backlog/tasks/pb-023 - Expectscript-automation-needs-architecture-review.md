---
id: PB-023
title: "Expect/script automation (needs architecture review)"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:L'
  - 'area:mcp'
dependencies: []
priority: p2
type: feature
ordinal: 1230
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Repeated interactive sequences (send, wait, respond) must be hand-looped
by the agent each time.

### Desired outcome

Bounded transaction scripting if architecture review approves it.

### Scope

- Conservative first design only: JSON transaction steps, bounded step
  types, no shell access, deterministic transcript output.

### Non-goals

- Shell access. General-purpose language.

### Technical context

The shipped `transact` is the minimal kernel of this; revisit whether
scripting is still needed at all.

### Open questions

Does the shipped `transact` + agent-side looping already cover the
demand?
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Architecture review decision recorded before any implementation.
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
