# Replace `native_sim` with a Rust PTY device fixture

**Status:** Implemented

**Current platform scope:** Production-path real-PTY fixture tests run on Linux.
macOS and Windows run normal Rust fmt/build/test/clippy plus controlled-backend
coverage. This scope adds no PTY fallback or production serial behavior change.

## Context

serial-mcp formerly used an NCS/Zephyr `native_sim` firmware process for public
serial tests. The replacement uses an in-process Rust peer behind a Linux PTY,
so tests exercise the production serial pathname and transport while modeling
observable serial-device behavior rather than MCU or Zephyr internals.

Direct `nix::pty::openpty` was selected over `rustix-openpty`/`rustix` and
Python PTY alternatives because `nix` already exists in the repository, returns
owned descriptors, exposes a stable slave path, and adds the least machinery.
The fixture covers arbitrary bytes, fragmented streams, flood/hold behavior,
reopen, endpoint replacement, bounded cleanup, malformed data, and peer
disconnect. Peer-master closure is classified as `connection_closed` by the
public read path.

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
- keep Linux real-PTY evidence, macOS product build/test/clippy plus
  controlled-backend coverage, and state Windows limits honestly;
- minimize new dependencies and lifecycle complexity.

## Considered Options

1. Keep NCS/Zephyr `native_sim`.
2. Use direct `nix::pty::openpty` with an in-process Rust peer fixture.
3. Replace `nix` with `rustix-openpty`/`rustix`.
4. Use Python or `socat` child processes as primary fixture.
5. Use privileged tty drivers or full MCU/system emulators.

## Decision

Use direct `nix::pty::openpty` as primary Linux production-path serial boundary.
Build a reusable Rust test fixture with:

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

Recommended independent helper pins:

- `slip-codec = 0.4.0` for valid RFC 1055 bytes;
- `cobs = 0.5.1` for valid and strict plain-COBS vectors;
- `rmodbus = 0.12.2` for Modbus ASCII protocol/server state;
- local spec-derived peers/vectors for AT DCE, JSON/NDJSON scheduling, NMEA,
  generic framing, shell prompt, and raw bytes.

## Rationale

Direct `nix` already exists in repository, returns owned descriptors, and
exposes a stable slave path while pair lives. Rustix adds dependency churn.
Python adds process and IPC variance. `socat` still needs a separate protocol
peer and supervision.
Privileged drivers and full emulators violate CI cost or privilege goals.

Rust fixture can model behaviors tests observe more deterministically than the
former one-command-latch firmware while staying behind real OS serial boundary.

## Consequences

### Positive

- NCS-free normal Rust build/test path;
- much smaller CI/cache footprint and fewer external provisioning failures;
- explicit state, fault, queue, timing, and cleanup APIs;
- stronger protocol coverage and independent vectors;
- current 49 scenarios become required, non-ignored Rust tests.

### Negative

- fixture and peer library become repository-maintained test infrastructure;
- Linux PTY disconnect classification is covered by the public
  `connection_closed` proof;
- stable-symlink reconnect fixture needs careful no-clobber ownership/cleanup;
- production-path real-PTY fixture tests run on Linux; macOS and Windows retain
  normal Rust and controlled-backend coverage;
- optional helper crates add dev dependency/advisory surface.

### Neutral / Explicit Limits

- PTY tests do not validate electrical UART behavior;
- Windows remains compile-tested plus controlled-backend coverage until a
  suitable pre-provisioned signed-driver runner exists;
- changelog and migration records retain historical NCS/native_sim references;
