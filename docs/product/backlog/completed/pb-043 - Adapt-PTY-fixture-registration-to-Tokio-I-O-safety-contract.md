---
id: PB-043
title: Adapt PTY fixture registration to Tokio I/O safety contract
status: Done
assignee: []
created_date: '2026-11-15 00:36'
updated_date: '2026-11-15 00:47'
labels:
  - 'size:S'
  - 'area:testing'
dependencies: []
priority: p1
type: tech-debt
ordinal: 2390
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
User-authorized fix on Dependabot PR #96. Investigate AsyncFd deprecation and adopt supported registration with explicit descriptor lifetime guarantees. Preserve nonblocking readiness and bounded fixture teardown; no warning suppression or dependency rollback.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 PTY registration uses the supported Tokio API with a justified and enforced descriptor lifetime contract.
- [x] #2 Rust build/test/Clippy with warnings denied, required PTY suites and repeat gate, and full MCP compatibility gate pass.
- [x] #3 The fix and validation evidence are pushed to the existing PR #96 without unrelated user edits.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Confirmed Tokio 1.54.1 AsyncFd::register safety requirements from source and upstream PR #8587. PTY master is a private OwnedFd moved into AsyncFd and only borrowed for I/O; no get_mut, descriptor replacement, extraction or manual close. Adopt supported unsafe constructor with explicit lifetime proof, preserve read/write interests and nonblocking mode, convert registration error into io::Error for immediate OwnedFd cleanup. Raise dev-only Tokio lower bound to 1.54.1. Extend existing FD regression with 100 active-write-then-drop cancellation cycles in addition to 100 graceful shutdowns. Full Rust/PTY/compatibility and Nix validation running. User AGENTS preamble preserved and excluded.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Adopted Tokio AsyncFd::register with an explicit OwnedFd lifetime proof; preserved nonblocking mode, readable/writable interests, and I/O loops. Converted registration errors into io::Error to close the descriptor immediately on failure. Raised only the test Tokio minimum to 1.54.1. Added 100 active-task drop/abort cycles to the existing 100 graceful-shutdown FD regression. Full testing exposed a pre-existing ACK-test prefix/CRLF race; fixed its match boundary to require the complete asserted line and configured one-byte output chunks without relaxing ACK-before-PONG assertions. Passed fmt, locked all-target build and full Rust suite, warnings-denied Clippy, all required Linux PTY suites, ignored 100-lifecycle repeat gate, full MCP compatibility gate (historical stdio/HTTP, both conformance sets, Inspector), and final-source nix flake check on x86_64-linux. Evidence: /tmp/serial-mcp-pr96-fix-gates.log and /tmp/serial-mcp-pr96-fix-nix-final.log. Submitted as a follow-up commit in existing PR #96; hosted CI must rerun and human merge remains acceptance. User AGENTS preamble excluded.
<!-- SECTION:FINAL_SUMMARY:END -->
