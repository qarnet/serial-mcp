---
id: PB-041
title: Patch November security alerts in runtime and compatibility dependencies
status: Done
assignee: []
created_date: '2026-11-10 20:30'
updated_date: '2026-11-10 20:38'
labels:
  - 'size:M'
  - 'area:security'
dependencies: []
priority: p1
type: tech-debt
ordinal: 2390
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
User-authorized security maintenance: inventory current alerts, patch affected runtime and compatibility dependencies, preserve both MCP lifecycles and the exact historical rmcp fixture. No protocol retirement or package release bump.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Current open alerts are patched or individually assessed with evidence; npm lockfile audit is clean.
- [x] #2 Historical rmcp version, checksum and features remain unchanged and the complete MCP compatibility gate passes.
- [x] #3 Required Rust format, build, test and Clippy checks pass with warnings treated as errors; changes and limitations are documented.
<!-- AC:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
All 26 open Dependabot alerts (#48-73) are addressed by patched versions. npm audit is clean. Direct MCP pins, sha512 hashes and unchanged historical fixture verified. Existing AGENTS.md edits predate this task and are excluded. Local logs: /tmp/serial-mcp-pb041-gates.log, /tmp/serial-mcp-pb041-pty.log, /tmp/serial-mcp-pb041-nix.log.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
Updated root rustls 0.23.45 to 0.23.46 and patched npm validation transitives plus fast-uri/qs overrides. Exact MCP tool pins and historical rmcp fixture preserved; no new alert dismissals. Passed npm audit and npm ci --ignore-scripts, Rust format/build/test/Clippy with warnings denied, all four required Linux PTY suites, ignored 100-cycle repeat gate, complete scripts/test-mcp-compat.sh (both historical-client transports, 6 legacy and 7 modern conformance scenarios, Inspector), and nix flake check on x86_64-linux. GitHub alerts close after merge to the default branch. Done transition is submitted with the implementation PR; human merge remains acceptance.
<!-- SECTION:FINAL_SUMMARY:END -->
