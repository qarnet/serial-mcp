---
id: PB-009
title: "External decoder/plugin API"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:L'
  - 'area:framing'
dependencies:
  - PB-005
priority: p1
type: feature
ordinal: 1090
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

In-process framing/parsers cannot cover every proprietary protocol.

### Desired outcome

Pluggable frame decoders/parsers for protocols the built-in modes do not
cover.

### Scope

- A plugin mechanism for frame decoders/parsers.

### Non-goals

- Shell-out execution of arbitrary local binaries without a security
  design.

### Technical context

Ship PB-005 first; it covers much of the demand.

### Open questions

Plugin ABI/registration mechanism needs a design before READY.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A third-party decoder can be registered and used through the
- [ ] #2 Declarative checksums (PB-005) remain the lighter alternative for
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
