---
id: PB-003
title: "Windows native serial-open gap: decide the mio-serial close/reopen contract"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:S'
  - 'area:platform'
dependencies: []
priority: p1
type: research
ordinal: 1030
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

On Windows, `mio-serial`'s close/reopen behavior differs from Unix
expectations. The server-runtime port-lease design (PB-001) cannot claim
external-program exclusion on Windows until this contract is decided.

### Desired outcome

A decided, documented contract for Windows serial close/reopen semantics
that the port-lease design can rely on, or an explicit documented limitation
with a workaround.

### Scope

- Investigate mio-serial close/reopen behavior on Windows.
- Decide how port leases must treat Windows port handles.
- Record the decision where PB-001 implementation can consume it.

### Non-goals

- Windows E2E test infrastructure (separate deferred decision, see
`docs/reports/windows-serial-e2e-investigation.md`).

### Technical context

Ownership research lives in
`docs/design/PB-001-server-runtime-ownership.md`. Windows E2E decision
record: `docs/reports/windows-serial-e2e-investigation.md`.

### Open questions

Whether a pre-provisioned signed-driver CI runner changes the answer.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 mio-serial close/reopen behavior on Windows documented with
- [ ] #2 Lease contract decision recorded and linked from PB-001.
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
