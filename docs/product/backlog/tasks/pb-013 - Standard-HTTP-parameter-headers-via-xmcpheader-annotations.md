---
id: PB-013
title: "Standard HTTP parameter headers via x-mcp-header annotations"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:S'
  - 'area:mcp'
dependencies: []
priority: p2
type: feature
ordinal: 1130
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Common per-call parameters ride in JSON bodies; HTTP-level proxies and
routing cannot see them.

### Desired outcome

Standard `Mcp-Param-*` HTTP headers derived from `x-mcp-header`
annotations; first candidate `connection_id`.

### Scope

- `x-mcp-header` annotation support.
- `connection_id` as the first promoted parameter.

### Non-goals

- Promoting payloads or credentials into headers.

### Technical context

SEP-2243 header work in the dual lifecycle (src/server.rs).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 `connection_id` usable as an HTTP header on modern stateless
- [ ] #2 Conformance/compatibility gates stay green.
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
