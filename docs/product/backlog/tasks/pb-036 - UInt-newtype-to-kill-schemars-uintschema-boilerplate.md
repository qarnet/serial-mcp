---
id: PB-036
title: "UInt newtype to kill schemars uint_schema boilerplate"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:schema'
dependencies: []
priority: p1
type: tech-debt
ordinal: 1360
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Per-field `#[schemars(schema_with = "crate::schema_helpers::uint_schema")]`
annotations are sprinkled across the tree; missing one is a known bug vector
(b12b09fd, bc37a0b0, and the PortInfo vid/pid/interface miss). schemars 1.x
emits non-standard `"format": "uintN"` for unsigned integer fields, and
validators log a warning per call and drop the constraint.

### Desired outcome

A newtype or global schemars visitor that collapses the whole class of
missing-annotation bugs.

### Scope

- UInt newtype or global schemars visitor.
- Coordinate with any schemars 2.x migration.

### Non-goals

- Changing wire formats.

### Technical context

AGENTS.md invariants section documents the annotation rules and history.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 No per-field uint annotations remain (or a documented reason they must).
- [ ] #2 Schema regression tests (serial::schema check_schema! list) extended to catch the class.
- [ ] #3 Validators no longer log uint-format warnings.
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
