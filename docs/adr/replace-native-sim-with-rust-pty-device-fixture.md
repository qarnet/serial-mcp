# Replace `native_sim` with a Rust PTY device fixture

**Status:** Proposed

## Context

serial-mcp currently uses an NCS/Zephyr `native_sim` firmware process for 49
public-MCP serial tests. Required CI downloads or restores a multi-gigabyte NCS
toolchain, builds C firmware, and runs ignored Rust suites in a dedicated job.
The firmware models a command parser, timer, and TX ring, but acceptance needs
only observable serial-peer behavior, not MCU or Zephyr internals.

Research compared direct `nix::pty::openpty`, `rustix-openpty`, and Python
standard-library PTYs through the same production serial path. All transported
arbitrary bytes, supported split/fragmented streams, flood/hold behavior,
same-path reopen, endpoint replacement, and bounded cleanup for 100 iterations.
Direct `nix` matched alternatives with least added machinery. All candidates
also exposed one product gap: peer-master closure currently ends public reads by
timeout rather than `connection_closed`.

Physical baud timing, parity/stop-bit faults, modem lines, BREAK, and electrical
flow control are outside PTY fidelity. Controlled backends already provide
deterministic line-control and failure injection.

## Forces

- preserve every current public-boundary assertion;
- open a real OS serial pathname through production `tokio_serial` code;
- gain deterministic state, fragmentation, timing, backpressure, disconnect,
  malformed-data, and recovery control;
- keep protocol vectors independent from production codec implementations;
- remove NCS, Zephyr, west, nrfutil, and multilib firmware CI cost;
- keep Linux/macOS portability and state Windows limits honestly;
- minimize new dependencies and lifecycle complexity.

## Considered Options

1. Keep NCS/Zephyr `native_sim`.
2. Use direct `nix::pty::openpty` with an in-process Rust peer fixture.
3. Replace `nix` with `rustix-openpty`/`rustix`.
4. Use Python or `socat` child processes as primary fixture.
5. Use privileged tty drivers or full MCU/system emulators.

## Proposed Decision

Use direct `nix::pty::openpty` as primary Unix serial boundary. Build a reusable
Rust test fixture with:

- owned PTY master and retained slave descriptor;
- explicit async shutdown and bounded task cleanup;
- state-machine peer core with bounded input/output queues;
- scripted emit/chunk/delay/silence/hold/saturate/malformed/close/crash/replace
  actions;
- protocol-specific peers and independent oracle dependencies;
- in-process HTTP `TestServer` for most scenarios;
- small separate-process and spawned-serial-mcp smoke cases;
- stable fixture-owned symlink for true endpoint replacement and reconnect;
- controlled backend retained for modem-line, BREAK, and deterministic I/O
  failure claims.

Before accepting this ADR, fix and prove Linux PTY peer-disconnect
classification so a pending public read reports `connection_closed`. Do not
classify every `ErrorKind::Other` as fatal; characterize exact OS error first.

Recommended independent helper pins:

- `slip-codec = 0.4.0` for valid RFC 1055 bytes;
- `cobs = 0.5.1` for valid and strict plain-COBS vectors;
- `rmodbus = 0.12.2` for Modbus ASCII protocol/server state;
- local spec-derived peers/vectors for AT DCE, JSON/NDJSON scheduling, NMEA,
  generic framing, shell prompt, and raw bytes.

## Rationale

Direct `nix` already exists in repository, returns owned descriptors, exposes
stable slave path while pair lives, and measured identically to alternatives.
Rustix adds dependency churn without measured gain. Python adds process and IPC
variance. `socat` still needs a separate protocol peer and supervision.
Privileged drivers and full emulators violate CI cost or privilege goals.

Rust fixture can model behaviors tests observe more deterministically than
current one-command-latch firmware while staying behind real OS serial boundary.

## Consequences

### Positive

- NCS-free normal Rust build/test path;
- much smaller CI/cache footprint and fewer external provisioning failures;
- explicit state, fault, queue, timing, and cleanup APIs;
- stronger protocol coverage and independent vectors;
- current 49 scenarios become required, non-ignored Rust tests.

### Negative

- fixture and peer library become repository-maintained test infrastructure;
- Linux PTY disconnect classification needs product work before migration;
- stable-symlink reconnect fixture needs careful no-clobber ownership/cleanup;
- macOS needs separate behavioral verification;
- optional helper crates add dev dependency/advisory surface.

### Neutral / Explicit Limits

- PTY tests do not validate electrical UART behavior;
- Windows remains compile-tested plus controlled-backend coverage until a
  suitable pre-provisioned signed-driver runner exists;
- changelog retains historical NCS/native_sim references;
- differential native/replacement window temporarily runs both suites.

## Acceptance Evidence

Required before status changes to Accepted:

1. all 49 traceability rows map to replacement tests with no weakened assertion;
2. disconnect/replacement regression passes 100/100;
3. every shipped preset/framing/parser has normal, fragmented, stateful, and
   malformed/fault proof plus independent oracle metadata;
4. replacement and native normalized public outcomes match for agreed parity
   window;
5. clean checkout passes full required suite without NCS installed;
6. CI disk/time reduction is measured;
7. format/build/test/clippy/rustdoc/Nix gates pass.

Research evidence:

- [`native-sim-test-traceability.md`](../development/native-sim-test-traceability.md)
- [`native-sim-virtual-serial-candidate-survey.md`](../development/native-sim-virtual-serial-candidate-survey.md)
- [`native-sim-boundary-prototype-results.md`](../development/native-sim-boundary-prototype-results.md)
- [`native-sim-protocol-peer-worksheets.md`](../development/native-sim-protocol-peer-worksheets.md)
