---
id: PB-042
title: Migrate GitHub Actions to workflow dependency lockfile
status: Done
assignee: []
created_date: '2026-11-15 00:30'
updated_date: '2026-11-15 00:32'
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
User-authorized migration to readable action references with generated actions.lock. Preserve current direct action commits and release gates; cover every workflow, add read-only CI verification, document Dependabot maintenance. No dependency upgrades or automatic merges.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Every repository workflow is covered by generated actions.lock, with current direct action commits preserved.
- [x] #2 CI verifies lockfile provenance and coverage with a pinned generator; stale lockfiles fail verification.
- [x] #3 Workflow validation and relevant repository gates pass; maintenance documentation and agent instructions describe the new policy.
<!-- AC:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Migrated all seven workflows to generated actions.lock, preserving all 37 previous direct action uses and their eight distinct commits. Release-backed references now show tags; rust-toolchain and rust-cache retain SHAs where no matching release tag exists. Added a read-only, five-minute CI verification job using gh-actions-lock v0.4.1, gating build/Nix/conformance and thus trusted release. Existing monthly Dependabot grouping retained. Added maintenance documentation and agent instructions. Passed online gh actions-lock --verify, offline coverage, direct-commit preservation audit, a negative stale-lockfile check, structural actionlint (-shellcheck=; unchanged release script has existing SC2129 style finding), git diff --check and nix flake check on x86_64-linux using cached derivations. Hosted CI to validate migration in the PR. Existing user AGENTS.md preamble excluded from commit. Human PR merge remains official acceptance.
<!-- SECTION:FINAL_SUMMARY:END -->
