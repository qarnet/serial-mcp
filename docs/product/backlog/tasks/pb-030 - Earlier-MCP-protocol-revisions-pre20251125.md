---
id: PB-030
title: "Earlier MCP protocol revisions (pre-2025-11-25)"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:L'
  - 'area:mcp'
dependencies: []
priority: p3
type: feature
ordinal: 1300
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Some clients may need older protocol revisions than the permanently
supported `2025-11-25`.

### Desired outcome

Support for pre-2025-11-25 revisions only with concrete client demand.

### Scope

- Possible candidates: `2025-06-18`, `2025-03-26`, `2024-11-05`.
- Each version requires: an explicit product policy row, lifecycle/capability/
  cache review, raw-wire tests, official conformance support where available,
  and a real historical client fixture.

### Non-goals

- Supporting a revision merely because rmcp lists it in KNOWN_VERSIONS.

### Technical context

docs/reference/mcp-version-compatibility-policy.md owns the contract.

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Concrete user/client demand documented before any version row is
- [ ] #2 Permanent `2025-11-25` retention never weakened (policy invariant).
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
