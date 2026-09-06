---
id: PB-008
title: "Config import/export"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:profiles'
dependencies: []
priority: p1
type: feature
ordinal: 1080
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

There is no way to snapshot a running server's state for reproduction or
migration. The profiles TOML file covers copy-between-machines but not a
running server's open connections and their framing/parser defaults.

### Desired outcome

Export a running server's full state (open connections + framing/parser
defaults) as importable profiles.

### Scope

- Export open connection state as importable profile data.
- Import path restores defaults.

### Non-goals

- Restoring exact RX buffers or cursors (transient state).

### Technical context

Pairs with the shipped profile system (`src/profile_store.rs`,
`save_profile`).

### Open questions

Needs sharpening before it earns implementation: does exporting a running
server's state add enough value over the existing TOML file to justify the
surface?
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 A running server's defaults round-trip through export/import.
- [ ] #2 Exported data is human-readable TOML compatible with the profile store.
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
