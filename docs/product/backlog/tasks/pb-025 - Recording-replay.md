---
id: PB-025
title: "Recording + replay"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:L'
  - 'area:capture'
dependencies: []
priority: p2
type: feature
ordinal: 1250
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Bugs against real hardware are hard to reproduce; decoder/parser
regression tests cannot use real-device data.

### Desired outcome

Reproducible bugs, test fixtures from real hardware, decoder regression
tests from recordings.

### Scope

- Recording of live sessions.
- Replay into tests as fixtures.

### Non-goals

- Continuous capture lifecycle (separate design).

### Technical context

The safe persistent capture foundation shipped (disabled-by-default
`--capture-dir` store, portable filename-only `export_log`, quotas,
no-overwrite atomic commits, advisory locks). Continuous capture lifecycle
design: `docs/design/PB-025-continuous-capture.md` (recommendation: do not
implement until concrete task evidence).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A recorded session can be replayed deterministically in a test.
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
