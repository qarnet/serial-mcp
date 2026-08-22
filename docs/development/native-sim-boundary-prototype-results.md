# `native_sim` Replacement Boundary Prototype Results

**Status:** Linux Stage 2 experiment completed on 2026-08-13 for direct `nix`,
`rustix-openpty`, and Python standard-library PTYs. Recommendation is approved.
Phase A fixed zero-byte peer closure narrowly as `UnexpectedEof`; direct-nix
acceptance rerun passes 100/100 with public `connection_closed`. Phase B also
resolved stable-path replacement/reconnect. Full differential parity and Phase F
deletion remain open.

Generated raw logs and normalized JSON are ignored under
`target/native-sim-research/stage2/`. Disposable experiment source and helpers
were removed after promotion to `tests/common/device_fixture/`; this document
preserves their historical measured results.

## Environment

| Item | Value |
|---|---|
| Git HEAD | `0cb72f81d905933531609321c7cf9282a1ffbab9` |
| Rust | 1.97.1 |
| Host/kernel | x86_64 Linux 7.1.1, NixOS |
| direct Rust | `nix 0.31.3` |
| Rust challenger | `rustix-openpty 0.2.0` + `rustix 1.1.4` |
| scripting comparator | Python 3.13.14 standard library |
| external-native comparator | `socat` unavailable locally; not installed |

Exact invocation per candidate:

```bash
SERIAL_MCP_NATIVE_SIM_RESEARCH_REPETITIONS=100 \
  cargo test --locked --test native_sim_research <candidate-test> \
  -- --ignored --test-threads=1
```

## Identical Experiment

Each iteration:

1. allocates candidate PTY and raw-configures slave;
2. injects slave path through deterministic `PortProvider`;
3. opens only through modern public MCP `open(port=...)` and production
   `tokio_serial::SerialStream`;
4. round-trips all bytes `0x00..=0xFF` in both directions;
5. assembles `PING\r\n` across four public writes;
6. emits `PONG\r\n` in delayed fragments;
7. emits 32 KiB flood under 1024-byte live read default;
8. holds one pending read until explicit peer release marker;
9. closes cleanly and reopens same PTY path with distinct connection ID;
10. destroys pair, reserves old number, creates distinct replacement endpoint,
    and opens replacement through public MCP;
11. closes peer master during known-pending read;
12. explicitly shuts down fixture and compares test-process FD/direct-child
    counts.

## Results

| Candidate | 100 iterations | Wall time | FD start/end | Child start/end | Bytes | split/fragment | flood/hold | same path | replacement | peer close |
|---|---:|---:|---:|---:|---|---|---|---|---|---|
| direct `nix` | completed | 122.446 s | 10/10 | 0/0 | pass | pass | pass | pass | pass | **fail: `timeout`** |
| rustix challenger | completed | 122.474 s | 10/10 | 0/0 | pass | pass | pass | pass | pass | **fail: `timeout`** |
| Python stdlib | completed | 129.584 s | 10/10 | 0/0 | pass | pass | pass | pass | pass | **fail: `timeout`** |

Test exits are intentionally failures after writing normalized results because
hard acceptance requires public disconnect observation. All three reported:

```text
peer close never produced connection_closed; observed ["timeout"]
```

## Interpretation

### Boundary equivalence

All three candidates expose equivalent kernel PTY fidelity for tested
operations. Rustix adds no measured lifecycle, byte, race, or cleanup advantage
over existing `nix`. Python adds about 7.1 seconds across 100 iterations
(roughly 71 ms per iteration), plus interpreter/child protocol complexity.

Direct `nix` therefore wins provisional boundary choice:

- already locked and used by repository;
- no added runtime process;
- direct owned master/slave descriptors;
- same measured behavior as rustix challenger;
- lower dependency and maintenance surface.

Remove disposable `rustix-openpty`/`rustix` dependencies after research artifacts
are accepted. Python may remain only as independent research comparator, not
production test foundation.

Isolated warm-registry/offline clean debug builds provide candidate-only cost
comparison (not full serial-mcp build cost):

| Candidate | Build wall time | isolated target bytes | Compiled packages |
|---|---:|---:|---|
| nix 0.31.3 | 1.300 s | 36,857,490 | bitflags, cfg-if, libc, nix, probe |
| rustix 1.1.4 + rustix-openpty 0.2.0 | 1.799 s | 40,247,429 | bitflags, linux-raw-sys, rustix, wrapper, probe |
| Python helper syntax compile | 0.044 s | system interpreter, no isolated target | stdlib only |

Direct nix saves about 0.5 s and 3.4 MB in this small isolated comparison and
already exists in repository lock graph. Network cost was intentionally zero
(`cargo build --offline`); cold registry download remains CI-cache dependent.

### Disconnect blocker

Master closure is not enough to satisfy current public disconnect contract.
Linux PTY commonly reports `EIO` on slave/master edge transitions, but
`src/serial/config.rs::is_fatal_disconnect` recognizes only NotFound,
PermissionDenied, ConnectionReset, ConnectionAborted, BrokenPipe, and
Interrupted. Experiment sees public wall timeout rather than
`connection_closed` for every candidate and iteration.

Next implementation phase must first add a focused PTY characterization test
that captures exact lower-level error and then decide a narrow Linux PTY
classification fix. Do not broadly classify all `ErrorKind::Other` as fatal.
After fix, rerun this identical gate and require `connection_closed` 100/100.

### Endpoint replacement and reconnect

At Stage 2, creating a distinct replacement endpoint and opening its new path
worked, but it did not prove an existing connection's `reconnect` could recover
because stored port remained old `/dev/pts/N`. The experiment identified an
owned stable symlink, atomically retargeted from old pair to new, as the honest
same-configured-path design. Raw PTY number reuse is not reliable evidence:
initial experiment observed immediate reuse of old `/dev/pts/N` until a spare
PTY reserved that number.

Phase B implemented that owned stable symlink with bounded temporary-directory
cleanup. `public_mcp_ping_hold_disconnect_replace_and_reconnect` now proves
peer disappearance, a distinct physical replacement behind the stable path,
public `reconnect`, and fresh traffic.

### Backpressure limitation

Experiment proves result bound and explicit held-output release. It does not
claim PTY kernel buffering equals physical UART backpressure. Final fixture
needs its own bounded output queue with explicit hold/drop/block policy; PTY
transports only drained chunks.

## Unsupported Physical Semantics

No candidate validates physical baud timing, parity/stop-bit errors, RTS/CTS,
DTR/RTS reset semantics, BREAK on wire, or USB UART behavior. Keep controlled
backend tests for deterministic line control/failure injection and optional
pre-provisioned hardware/virtual-driver lane for electrical claims.

Linux result exists. macOS run remains unavailable in current environment;
macOS CI should run compact boundary characterization before migration is
declared portable. Windows remains compile/controlled-backend only because
unprivileged ConPTY is not a COM device.

## Re-scored Survivors

Scores use research-plan weights and measured Linux evidence. Disconnect
failure lowers lifecycle and 49-test coverage for every PTY candidate.

| Candidate | Fidelity 25 | Lifecycle 20 | 49 tests 15 | Peer extension 15 | CI cost 10 | Maintenance 5 | Linux/macOS 5 | Risk 5 | Weighted /5 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| direct `nix` | 5 | 3 | 4 | 5 | 5 | 5 | 4 | 5 | **4.45** |
| rustix challenger | 5 | 3 | 4 | 5 | 4 | 3 | 4 | 5 | **4.25** |
| Python stdlib | 5 | 3 | 4 | 5 | 3 | 5 | 4 | 4 | **4.20** |

Rust preference is not needed as tie-breaker: direct `nix` has same fidelity,
less machinery, and existing repository adoption.

## Recommendation

Use direct `nix::pty::openpty` as approved replacement boundary foundation.
Phase A resolved public peer-disconnect classification and Phase B resolved
stable-path reconnect. Runner shape:

- reusable in-process Rust fixture for most public-MCP tests;
- explicit async shutdown and FD/task ownership;
- small separate Rust peer process only for crash/exit-code and spawned-server
  smoke;
- stable-symlink subfixture for true reconnect-to-replacement test;
- controlled backend retained for physical-line/failure semantics.

Smallest required regression test:

> Allocate `nix` PTY, open slave through public MCP, exchange arbitrary bytes,
> close peer master during a readiness-proven pending read, require
> `stop_reason="connection_closed"`, explicitly shut down fixture, and prove FD
> and direct-child counts return to baseline.

## Approved Phase A Result

Implemented after user approval:

- zero-byte serial read maps to `UnexpectedEof("serial peer closed")`;
- `UnexpectedEof` is fatal-disconnect class; generic `Other` remains nonfatal;
- focused public PTY test proves pending read ends as `connection_closed`;
- direct-nix experiment passes 100/100 in 48.352 s, FD 10/10, children 0/0,
  with only `connection_closed` peer-close result.

Phase A resolved the conditional boundary blocker. Phase B resolved
stable-symlink reconnect with public replacement and `reconnect` coverage. Full
differential parity and Phase F NCS deletion remain open.

## Reproducibility Gaps

- `socat` comparator was unavailable. Surveyed source shows it wraps same PTY
  primitives while adding child/symlink supervision and protocol peer still
  needs another process. Measured equivalence among two Rust wrappers and Python
  makes installing it low-value; record this as waived prototype class unless
  reviewer requires a pinned Nix `socat` run.
- Peak RSS/disk per candidate was not isolated from shared Cargo target. Added
  rustix artifacts are visible in target but direct nix adds none. Final CI cost
  should be measured on migration PR from clean cache, not inferred from this
  warm local workspace.
- macOS run remains required where runner access exists.
