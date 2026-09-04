---
id: PB-011
title: "MCP Tasks extension (SEP-2663) for long-running operations"
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
ordinal: 1110
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Long `read`/`transact`/`capture_boot` calls hold a request open with no
task handle; a client cannot poll or cancel through the standardized
mechanism.

### Desired outcome

Task handles for long-running operations with
`tasks/get`/`update`/`cancel`.

### Scope

- Task lifecycle for long-running tool calls.
- Client-facing task query/cancel surface.

### Non-goals

- Task persistence across server restarts.

### Technical context

Needs ownership/lifecycle design; interacts with PB-001 runtime
ownership.

### Open questions

How tasks interact with the stateless modern HTTP request model needs
design.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A long read can be observed and cancelled through the tasks
- [ ] #2 Existing cancellation semantics preserved.
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
