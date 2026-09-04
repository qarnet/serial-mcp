---
id: PB-006
title: "TX pacing / throttling"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:tx'
dependencies: []
priority: p1
type: feature
ordinal: 1060
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Bootloaders, GRBL controllers, and cheap AT modems drop bytes when the
host writes one large uninterrupted burst. An agent cannot hand-loop small
writes with sleeps it cannot actually perform.

### Desired outcome

An MCP client can request bounded host-side pacing for transmitted data
(inter-chunk or inter-line delay) on `write`, with a per-call field and a
connection default.

### Scope

- Inter-chunk or inter-line delay on TX operations.
- Per-call field + connection default (profile-learned like other defaults).
- Preserve current behavior when pacing is not configured.

### Non-goals

- A general-purpose automation language.
- Device-specific flashing implementations.
- Replacing hardware flow control.

### Technical context

TX paths route through shared TX preparation in
`src/tools/io_ops.rs` (`decode_tx_payload` / `apply_tx_framing`).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Pacing configurable through the intended public interface.
- [ ] #2 Unpaced operations retain current behavior.
- [ ] #3 Cancellation leaves the connection usable.
- [ ] #4 Controlled-backend and PTY tests pass.
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
