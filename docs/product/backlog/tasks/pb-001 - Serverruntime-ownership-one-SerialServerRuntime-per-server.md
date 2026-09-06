---
id: PB-001
title: "Server-runtime ownership: one SerialServerRuntime per server"
status: In Progress
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:L'
  - 'area:runtime'
dependencies: []
priority: p1
type: refactor
ordinal: 1010
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

Stateless modern HTTP creates a fresh SerialHandler per request. Every
process-wide dependency (ConnectionManager, ProfileStore, CaptureStore,
PortProvider, event hub, RxSessionManager) must be injected and shared by
hand; nothing owns TX ordering across requests, the reconnect supervision is
ad hoc, and shutdown of the shared runtime is not deterministic. Cross-process
port access has no lease semantics, so two server processes can fight over the
same physical port.

### Desired outcome

One `SerialServerRuntime` per server process owns the shared state: a
shared TX queue with deterministic ordering, a reconnect supervisor,
deterministic shutdown, and cross-process port leases so concurrent processes
coordinate port ownership explicitly. Platform-portable across the supported
targets.

### Scope

- Define and implement `SerialServerRuntime` as the single owner of
  process-wide session state.
- Shared TX queue with deterministic cross-request ordering.
- Reconnect supervisor integrated into the runtime.
- Deterministic runtime shutdown (pump, watcher, listeners).
- Cross-process port lease mechanism.
- Keep stdio transport behavior unchanged.

### Non-goals

- Changing the MCP tool surface or wire formats.
- Multi-host coordination; leases are per-machine.

### Technical context

Full design: `docs/design/PB-001-server-runtime-ownership.md` (moved from
the stateless-http-runtime plan). Windows `mio-serial` close/reopen research
feeds the lease design (PB-003).

### Open questions

Windows lease semantics depend on the mio-serial close/reopen decision
(PB-003).
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Stateless HTTP requests provably share the runtime-owned state.
- [ ] #2 TX ordering is deterministic under concurrent stateless requests.
- [ ] #3 Shutdown completes without leaked tasks or hang; proven by test.
- [ ] #4 Two server processes coordinate port ownership via leases; proven by test on Linux PTYs.
- [ ] #5 Existing compatibility gates stay green.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
See the phased plan in the design document; phases validated against
the current codebase at branch time.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->
Active. Design doc verified accurate against main as of 2026-09;
stale native_sim rows struck through there.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->
<!-- SECTION:FINAL_SUMMARY:END -->
