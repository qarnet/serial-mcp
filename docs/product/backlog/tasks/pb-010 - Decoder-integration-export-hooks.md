---
id: PB-010
title: "Decoder integration / export hooks"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:capture'
dependencies: []
priority: p1
type: feature
ordinal: 1100
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Captured data has no path to external decoder tools.

### Desired outcome

Export capture or frames to external decoder tools if in-process support
stays small.

### Scope

- Export hooks from capture/frame paths to external tooling.

### Non-goals

- The full plugin API (PB-009).

### Technical context

Related to PB-009 but independently useful; not a dependency.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Frames/captures can be handed to an external decoder without manual copy-paste of payloads.
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
