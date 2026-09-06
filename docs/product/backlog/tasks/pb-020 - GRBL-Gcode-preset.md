---
id: PB-020
title: "GRBL / G-code preset"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:S'
  - 'area:protocol'
dependencies:
  - PB-006
priority: p2
type: feature
ordinal: 1200
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

GRBL controllers need line-based `ok`/`error` protocol handling; today
each agent re-invents it.

### Desired outcome

A GRBL preset: line-based `ok`/`error` protocol semantics.

### Scope

- Protocol preset for GRBL/G-code.

### Non-goals

- Full CNC job management.

### Technical context

Nearly free once TX pacing (PB-006) lands.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Preset usable end-to-end against a PTY fixture emulating GRBL responses.
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
