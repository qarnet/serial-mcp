---
id: PB-029
title: "MCP Bundle (MCPB) distribution"
status: Backlog
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:M'
  - 'area:distribution'
dependencies: []
priority: p2
type: feature
ordinal: 1290
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

One-click stdio installation in supporting desktop clients requires
bundling native binaries.

### Desired outcome

Package native release binaries as an MCP Bundle for one-click local
stdio installation.

### Scope

- `server.type = "binary"` bundle packaging.
- Platform-specific command overrides; optional user configuration.

### Non-goals

- Protocol work; separate release/distribution project.

### Technical context

Release workflow owns artifacts (see AGENTS.md release section).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Decide one cross-platform bundle vs per-platform bundles, manifest version, signing, update flow.
- [ ] #2 Clean-machine Claude Desktop tests pass.
- [ ] #3 Validate/pack with a pinned `@anthropic-ai/mcpb` release.
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
