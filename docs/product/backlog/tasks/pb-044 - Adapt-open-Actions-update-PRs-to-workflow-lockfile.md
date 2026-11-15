---
id: PB-044
title: Adapt open Actions update PRs to workflow lockfile
status: Review
assignee: []
created_date: '2026-11-15 00:51'
updated_date: '2026-11-15 00:54'
labels:
  - 'size:M'
  - 'area:ci'
dependencies: []
priority: p2
type: tech-debt
ordinal: 2390
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
User-authorized updates to existing Dependabot Actions PRs #93, #94 and #95. Integrate current main and regenerate actions.lock for each intended action upgrade, without unrelated dependency changes. Push to the existing branches and retain reviewable diffs.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Each Actions PR includes current main and a generated, verified lockfile matching its intended action update.
- [x] #2 Per-PR lockfile audit proves unrelated action commits unchanged; workflow lint and relevant Nix checks pass.
- [x] #3 All three existing PR branches are updated, with validation evidence and no unrelated user edits.
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Adapted existing Actions PRs #93/#94/#95 to merged main and generated actions.lock. Dependabot had already rebased #94/#95; regenerated to use readable generator-selected branch refs while preserving each intended commit. Resolved #93 workflow merge conflicts against current main, reapplied only setup-node/upload-artifact/download-artifact upgrades and regenerated lockfile. Audited each lockfile against main: #93 changes exactly its three intended commits; #94 only rust-toolchain; #95 only rust-cache. All three pass online gh actions-lock --verify, structural actionlint (-shellcheck=; unchanged release shell has existing SC2129 style finding), and whitespace checks. Nix flake check passed on current main-based runtime source with cached derivations, including final #93 snapshot. Generator branch refs are documented as immutable via the lockfile; no extra action upgrades. Existing branches updated directly; hosted CI reruns. Review record is retained in #93 for this multi-PR task; human merges remain acceptance. User AGENTS preamble stashed temporarily and restored afterward.
<!-- SECTION:FINAL_SUMMARY:END -->
