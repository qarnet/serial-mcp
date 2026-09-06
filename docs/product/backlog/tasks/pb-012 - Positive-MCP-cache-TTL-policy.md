---
id: PB-012
title: "Positive MCP cache TTL policy"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:mcp'
dependencies: []
priority: p2
type: feature
ordinal: 1120
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Cache entries use TTL 0; no positive-ttl caching exists.

### Desired outcome

Correct positive TTL support.

### Scope

- Positive TTL policy on cacheable families.

### Non-goals

- Caching before the prerequisites below exist.

### Technical context

Only after list-notification invalidation, authorization partitioning,
pagination keys, and stale-on-error tests exist (see
`docs/reference/mcp-version-compatibility-policy.md`).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 List-notification invalidation exists.
- [ ] #2 Authorization partitioning exists.
- [ ] #3 Pagination keys are cache-stable.
- [ ] #4 Stale-on-error behavior is tested.
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
