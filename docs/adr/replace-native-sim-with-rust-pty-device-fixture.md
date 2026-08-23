# Replace `native_sim` with a Rust PTY device fixture

**Status:** Accepted

**Current platform scope:** Production-path real-PTY fixture tests run on Linux
only. On macOS, `serialport` applies `IOSSIOSPEED` while configuring valid baud
rates, and macOS PTYs return `ENOTTY`. macOS still runs normal Rust
fmt/build/test/clippy and controlled-backend tests. This scope adds no baud-0
exception, macOS PTY fallback, or production serial behavior change.

## Context

serial-mcp formerly used an NCS/Zephyr `native_sim` firmware process for 49
public-MCP serial tests. Phase F removed that active source and configuration
coupling after the accepted parity window; fresh clean-checkout CI acceptance is
recorded in the [canonical Phase F acceptance record](../development/native-sim-replacement-research-progress.md).
The former firmware modeled a command parser, timer, and TX ring, but
acceptance needs only observable serial-peer behavior, not MCU or Zephyr
internals.

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
- keep Linux real-PTY evidence, macOS product build/test/clippy plus
  controlled-backend coverage, and state Windows limits honestly;
- minimize new dependencies and lifecycle complexity.

## Considered Options

1. Keep NCS/Zephyr `native_sim`.
2. Use direct `nix::pty::openpty` with an in-process Rust peer fixture.
3. Replace `nix` with `rustix-openpty`/`rustix`.
4. Use Python or `socat` child processes as primary fixture.
5. Use privileged tty drivers or full MCU/system emulators.

## Proposed Decision

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

Acceptance required fixing and proving Linux PTY peer-disconnect classification
so a pending public read reports `connection_closed`. That proof is recorded in
the [traceability record](../development/native-sim-test-traceability.md). Do
not classify every `ErrorKind::Other` as fatal; characterize exact OS error
first.

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
- Linux PTY disconnect classification required product work before migration and
  is now covered by the public `connection_closed` proof;
- stable-symlink reconnect fixture needs careful no-clobber ownership/cleanup;
- production-path real-PTY fixture tests are Linux-only because macOS
  `serialport` baud configuration invokes `IOSSIOSPEED` and macOS PTYs return
  `ENOTTY`; macOS retains normal Rust and controlled-backend coverage;
- optional helper crates add dev dependency/advisory surface.

### Neutral / Explicit Limits

- PTY tests do not validate electrical UART behavior;
- Windows remains compile-tested plus controlled-backend coverage until a
  suitable pre-provisioned signed-driver runner exists;
- changelog and migration records retain historical NCS/native_sim references;
- the differential native/replacement window ran both suites before active
  native source removal.

## Acceptance Evidence

All acceptance requirements are satisfied:

1. **Satisfied:** all 49 traceability rows map to replacement tests with no
   weakened assertion; see the [test traceability record](../development/native-sim-test-traceability.md).
2. **Satisfied:** the disconnect/replacement regression passes 100/100; see the
   [Phase E record](../development/native-sim-replacement-research-progress.md)
   and its repeat-gate evidence.
3. **Satisfied:** every shipped preset/framing/parser has normal, fragmented,
   stateful, and malformed/fault proof plus independent oracle metadata; see the
   [traceability record](../development/native-sim-test-traceability.md) and
   [Phase E record](../development/native-sim-replacement-research-progress.md).
4. **Satisfied:** replacement and native normalized public outcomes match for
   the agreed parity window; see the [Phase E parity record](../development/native-sim-replacement-research-progress.md).
5. **Satisfied:** the fresh clean checkout passes the full required suite
   without NCS installed; see [fresh PR CI run 32653648970](https://github.com/qarnet/serial-mcp/actions/runs/32653648970)
   and the [canonical acceptance record](../development/native-sim-replacement-research-progress.md).
6. **Satisfied:** CI disk/time reduction is measured in the [canonical
   acceptance record](../development/native-sim-replacement-research-progress.md),
   with its one-run observed wall-clock comparison caveat.
7. **Satisfied:** fresh CI passed format/build/test/clippy/Nix gates, including
   [run 32653648970](https://github.com/qarnet/serial-mcp/actions/runs/32653648970).
   Rustdoc passed locally during current acceptance-documentation verification
   via `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked`; rustdoc was
   not a CI step.

Research evidence:

- [`native-sim-test-traceability.md`](../development/native-sim-test-traceability.md)
- [`native-sim-virtual-serial-candidate-survey.md`](../development/native-sim-virtual-serial-candidate-survey.md)
- [`native-sim-boundary-prototype-results.md`](../development/native-sim-boundary-prototype-results.md)
- [`native-sim-protocol-peer-worksheets.md`](../development/native-sim-protocol-peer-worksheets.md)
