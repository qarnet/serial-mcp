# `native_sim` Replacement Emulator Core Research

**Status:** Stage 3 research complete and reusable fixture foundation implemented
under `tests/common/device_fixture/` on 2026-08-13. Command/protocol migration is
not complete.

## Observable Model

Current firmware behavior needed by tests reduces to:

- byte input assembly into CR/LF-terminated commands;
- stateful dispatch (ping/info, ACK sequence, delay, flood, hold, raw output,
  explicit exit);
- bounded output queue and chunked drain;
- scheduled delays/cadence/silence;
- malformed output, saturation, close, crash, and endpoint replacement;
- explicit ownership and shutdown.

Zephyr IRQ, scheduler, `k_timer`, board, Kconfig, and native UART internals are
not observable acceptance requirements. Replacement should improve command
assembly rather than copy current one-bit command latch that can merge commands.

## Recommended Layers

```text
PtyBoundary
  owns master, retained slave, slave path, optional stable symlink

DeviceCore
  input assembler -> protocol/command dispatcher -> actions

OutputQueue
  byte capacity, drain chunk size, hold, accepted/dropped counts, drain signal

FaultScheduler
  Emit, EmitChunks, Delay, Silence, Malformed, Saturate, Close, Crash, Replace

DeviceFixture
  task cancellation, readiness barriers, async shutdown, exit report

PublicScenario
  TestServer/spawned server + real rmcp client + MCP assertions
```

Generic fixture must not know serial-mcp frame decoder internals. Protocol peers
produce/consume bytes independently.

## Runner Shape

### In-process default

Use Tokio task over PTY master for most tests:

- fastest lifecycle;
- direct state/fault API;
- deterministic readiness barriers;
- easy TestServer dependency injection;
- exact task/FD cleanup.

### Separate process only where observable

Use small Rust peer binary for:

- crash/exit-code 42 behavior;
- process disappearance;
- small spawned-serial-mcp startup composition smoke.

Do not run all protocol cases through child IPC.

### Controlled backend remains separate

Keep current controlled I/O for exact DTR/RTS, BREAK, release-on-cancel, and
injected OS failures. It complements PTY; it cannot replace real-path parity.

## Clock Strategy

Use injected logical scheduler for peer actions and readiness, not sleeps:

- action `Delay(duration)` advances scheduled emission point;
- cadence is explicit sequence of deadline/chunk pairs;
- `Silence(duration)` is state, not blocked worker;
- fixture exposes barrier when command accepted, output queued, hold active,
  or peer closed.

Use Tokio paused time only in pure fixture tests. OS PTY I/O and public MCP
timeouts require real bounded time because kernel readiness does not advance
with Tokio virtual clock. This hybrid avoids accidental scheduler ordering while
still tests real timeout behavior.

## Queue and Backpressure

Queue contract must expose:

- capacity in bytes;
- max drain chunk size;
- hold/release;
- policy (`DropNew`, future optional `BlockProducer`);
- cumulative accepted/dropped/drained counts;
- empty/drained notification.

Default command-parity mode can model current firmware's drop-new behavior, but
protocol tests should choose policy explicitly. PTY kernel buffer is transport
after fixture queue and is not treated as deterministic backpressure oracle.

## Fault Script

Bounded typed actions:

```rust
enum Action {
    Emit(Vec<u8>),
    EmitChunks(Vec<Vec<u8>>),
    Delay(Duration),
    Silence(Duration),
    Malformed(Vec<u8>),
    Saturate(Vec<u8>),
    Close,
    Crash(i32),
    ReplaceEndpoint,
}
```

Scripts have max action count and max total emitted bytes. No unbounded loop or
implicit sleep. Stateful peers may generate next action list from request and
peer state.

## Ownership and Shutdown Contract

1. Fixture owns all PTY descriptors, task handles, cancellation token, tempdir,
   and symlink.
2. Retained slave FD stays open only while same-pair reopen is required.
3. `shutdown()` cancels peer, closes master/slave, awaits task within bound,
   aborts only as fallback, removes owned symlink/tempdir, and returns report.
4. `Drop` performs best-effort cancellation only; acceptance calls
   `shutdown().await`.
5. Crash is an explicit terminal state with exit code; already-exited child is
   reaped exactly once.
6. Tests compare FD/direct-child baseline after short teardown settle.

## Disposable Prototype Evidence

Superseded disposable pure-core prototype proved five behaviors under Rust
1.97.1:

- fragmented input preserves multiple complete commands without latch loss;
- stateful ping sequence and explicit crash 42;
- queue capacity, hold, drop, chunked drain;
- emit chunks, logical delay/silence, malformed bytes, saturation, close;
- cancellation is terminal and suppresses late output.

Prototype source was deleted after behavior moved into durable fixture and
`tests/device_fixture.rs`. Historical result remains here for traceability.

## Implemented Foundation

`tests/common/device_fixture/{mod.rs,core.rs}` now provides:

- direct `nix::pty::openpty` with raw nonblocking master registered through
  Tokio `AsyncFd`;
- one device event loop owning PTY master lifetime, command assembly, peer
  dispatch, output queue, delays, scripts, and control messages;
- byte-bounded input, scripts, and output, with explicit `DropNew` and
  `BlockProducer` queue policy;
- readiness snapshots instead of test sleeps for command acceptance, queued or
  drained output, hold state, generation, and terminal state;
- no-clobber stable symlink plus atomic retarget to a distinct replacement PTY;
- explicit async shutdown with bounded join and observable abort fallback;
- `DevicePeer` abstraction and stateful `PingPeer` reference peer.

`tests/device_fixture.rs` proves pure boundaries, scripted hold/fragment/delay/
malformed/crash behavior, public MCP disconnect and true same-path reconnect,
real spawned-binary composition, and 100-run Linux FD cleanup.

## Remaining Fixture/Parity Tests

- arbitrary multi-command OS-read behavior through public MCP, beyond pure input
  assembler proof;
- delayed and cadence ordering independent of host scheduler;
- cancellation during input, delay, hold, and output drain;
- close/crash while public read pending;
- stable-symlink hostile collision/no-clobber negative proof;
- 100-run task/child cleanup beyond current FD cleanup proof;
- panic/error propagation with raw diagnostic preservation.

## Recommendation

Implement in-process Rust fixture with typed bounded core and explicit async
shutdown. Do not port Zephyr implementation chronology. Derive peer state from
public test contracts and protocol worksheets. Add child runner only for
process-specific outcomes.
