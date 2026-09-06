---
id: PB-002
title: "CI Nix timing measurement and cache investigation"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:S'
  - 'area:ci'
dependencies: []
priority: p1
type: tech-debt
ordinal: 1020
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

`nix flake check` CI duration and cache behavior are unmeasured. Any
future Nix tuning (cache keys, store limits, source filtering) would be
guesswork.

### Desired outcome

Recorded cold/warm measurements of cache restore/save and `nix flake
check` durations, plus knowledge of whether the 8 GiB store cache retains
dependency outputs, before any further Nix tuning is attempted.

### Scope

- Record cache restore/save duration and `nix flake check` duration over
  several clean runs and runs with only source changes.
- Inspect whether the 8 GiB Nix-store cache retains dependency outputs or is
  repeatedly evicted under the repository cache quota.
- Record cold versus warm time before changing cache keys, store limits, or
  source filtering.

### Non-goals

- Any Nix configuration change; measurement only.

### Technical context

Scope preserved verbatim from the implemented-and-removed CI
runtime-reduction plan (kept so this follow-up is self-contained).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Cold/warm timings recorded for clean and source-change-only runs.
- [ ] #2 Store-cache retention behavior documented with evidence.
- [ ] #3 Findings recorded so tuning decisions can cite them.
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
