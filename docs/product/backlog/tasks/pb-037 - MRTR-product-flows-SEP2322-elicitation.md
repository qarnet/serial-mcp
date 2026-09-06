---
id: PB-037
title: "MRTR product flows (SEP-2322 elicitation)"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:mcp'
dependencies: []
priority: p3
type: feature
ordinal: 1370
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Physical power-cycle guidance and destructive reset confirmation would
benefit from client elicitation, but current schemas, defaults, destructive
hints, and cancellation cover the serial workflows we have.

### Desired outcome

MRTR flows only when a concrete elicitation need appears.

### Scope

- Revisit only for a concrete need such as power-cycle guidance or
  destructive reset confirmation.

### Non-goals

- Eager adoption of SEP-2322 without a concrete need.

### Technical context

rmcp 3 supports SEP-2322 (InputRequiredResult).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Any echoed `requestState` must be integrity-protected if adopted.
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
