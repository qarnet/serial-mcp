---
id: PB-004
title: "Local-only usage statistics for development"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:mcp'
dependencies: []
priority: p1
type: feature
ordinal: 1040
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Interface trimming decisions (which options to keep, trim, or default
differently) are made without usage evidence.

### Desired outcome

Payload-free tool-call, stop-reason, and option-usage records, written
strictly locally and opt-in, to drive evidence-based interface trimming.

### Scope

- Local metadata: tool-call frequency, stop-reason distribution,
  option-usage frequency (which `from` variants agents pick, how often
  `match`/`framing` options get used, average `max_buffered_bytes`).
- Strictly local: file on the host (e.g. under `~/.local/share/serial-mcp/`
  or a configured path); never transmitted; opt-out by default with an
  explicit enable.
- Schema designed around questions we actually want answered.

### Non-goals

- Telemetry, remote endpoints, user tracking.

### Technical context

Pairs with the shipped `configure` tool + connection-default trim
(PB-001 design discussion).

### Open questions

Exact record schema and retention policy need a first draft before
implementation.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Opt-in recording with zero network transmission.
- [ ] #2 Recorded data can answer at least one concrete trimming question
- [ ] #3 Disabling recording leaves no overhead on the hot path.
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
