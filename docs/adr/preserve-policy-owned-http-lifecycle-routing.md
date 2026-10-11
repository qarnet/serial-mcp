# Preserve policy-owned HTTP lifecycle routing

- Date: 2026-10-10
- Status: Proposed
- Approval: Pending review of dependency-update PR #87

## Context and forces

Upgrading rmcp from 3.1.4 to 3.5.0 changes HTTP routing: `initialize` always
selects legacy session handling, even with a modern protocol-version header.
The existing raw-wire regression test then receives HTTP 200 instead of 404.
rmcp's legacy path allocates a session and attaches its ID before returning
the handler's rejection. serial-mcp's exact policy permits initialize only for
`2025-11-25`; modern requests must remain stateless.

## Options considered

- Accept the SDK behavior and change expectations: loses the existing no-session
  rejection contract.
- Parse and reject initialize in HTTP middleware: duplicates SDK parsing,
  validation, and error precedence.
- Select rmcp transport mode from the existing exact policy table: preserves
  the contract without parsing or rewriting request bodies.

## Decision and rationale

Use `src/http_transport.rs::router` for production and the HTTP test harness.
Clone the service per request and disable legacy session mode only when the
`MCP-Protocol-Version` header exactly matches a discovery/stateless policy row.
Absent, unknown, and legacy headers keep SDK routing. rmcp still validates all
headers, metadata, bodies, and methods; the handler still rejects initialize.
No new protocol support is inferred from SDK versions or date comparisons.

## Consequences

HTTP embeddings should use the shared router to retain this contract. SDK
routing changes require checking the adapter alongside protocol tests. The
adapter shares the service's session manager and handler factory; per-request
configuration cannot disable legacy sessions for other clients.

## References

- [Dependency update PR #87](https://github.com/qarnet/serial-mcp/pull/87)
- [Protocol policy](../reference/mcp-version-compatibility-policy.md)
- `tests/protocol_compatibility.rs::raw_2026_07_28_initialize_is_rejected_with_method_not_found`
  checks HTTP 404, JSON-RPC -32601, absent session header, and subsequent legacy
  initialize/ping on the same server.
