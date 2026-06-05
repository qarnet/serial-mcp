# PLAN 4 - Add No-New-RX Timeout

## Goal

Add `no_new_rx_timeout_ms` to all RX tools so they can stop after a period of silence, independent of total timeout.

## Scope

Tools:
- `read`
- `wait_for`
- `subscribe`

Default:
- disabled / unlimited

## Behavior

- if set, operation stops when no new bytes arrive for given period
- total timeout and silence timeout may both exist
- earliest stop condition wins

For background subscribe:
- stream task stops
- emits final informational notification
- not treated as error

## Documentation Phase

Document difference between:
- `timeout_ms` = total wall-clock budget
- `no_new_rx_timeout_ms` = silence budget

## Testing Phase

Cover:
- read stops on silence timeout
- wait_for stops on silence timeout
- subscribe background task stops on silence timeout
- metadata/notifications identify silence reason
