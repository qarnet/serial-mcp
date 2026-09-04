---
id: PB-016
title: "Per-client RX cursors"
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
ordinal: 1160
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

The shared read cursor means concurrent clients consume each other's
positions.

### Desired outcome

Named cursor groups if shared multi-agent access becomes a real usage
pattern.

### Scope

- Named cursor groups.

### Non-goals

- Full multi-client session management.

### Technical context

Overlaps socket sharing / tee (PB-032). Trigger: real multi-agent usage
evidence.

### Open questions

Is the shared multi-agent usage pattern real yet?
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Two clients can read independently over the same connection.
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
