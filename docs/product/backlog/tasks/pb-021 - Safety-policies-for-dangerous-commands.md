---
id: PB-021
title: "Safety policies for dangerous commands"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:security'
dependencies: []
priority: p2
type: feature
ordinal: 1210
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Nothing stops a destructive command (flash erase, factory reset) being
sent by mistake or by a confused agent.

### Desired outcome

Optional confirmation patterns for dangerous commands, including the
profile-level safety-policy intent from the removed `ProfileDefaults.safety_policy`
field.

### Scope

- Profile-level safety policy for command confirmation.

### Non-goals

- A general sandboxing mechanism.

### Technical context

Removed-field intent recorded in CHANGELOG 0.9.x history.

### Open questions

Confirmation UX over MCP needs design (client elicitation vs
structured error).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A configured pattern blocks its command until explicitly confirmed.
- [ ] #2 Policy lives in the profile, persists, and is visible in status.
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
