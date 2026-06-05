# PLAN 3 - Add Buffer Budgets

## Goal

Bound total program buffering and per-tool buffering with simple global startup settings, while still letting the agent choose per-request `max_buffered_bytes`.

## Startup Settings

- `--max-program-buffered-bytes`
  - default `1 GiB`
  - total bytes all in-flight tools may reserve together
- `--max-tool-buffered-bytes`
  - default `1 MiB`
  - maximum one tool request may reserve

## Runtime Setting

- `max_buffered_bytes`
  - request-level field used by agent
  - default `2048`
  - must be `<= max_tool_buffered_bytes`

## Scope

In scope:
- shared budget manager in server state
- reserve full requested bytes at start of RX operation
- release on finish/cancel/error/close
- pool exhaustion becomes tool error
- unify naming from old `max_bytes` / `max_chunk_bytes`

Out of scope:
- dynamic growth after start
- ring-buffer retention

## Design Notes

- reserve full requested amount up front
- no incremental reserve/grow logic
- read, wait_for, subscribe all participate in same pool
- future tools should reuse same manager

## Documentation Phase

Explain:
- program budget vs tool budget vs request budget
- open connection alone does not reserve from pool
- reservation happens when RX tool starts

## Testing Phase

Cover:
- startup parsing/validation
- request over per-tool ceiling rejected
- request fails when pool exhausted
- reservation released after success/error/cancel
- concurrent requests consume from same pool
