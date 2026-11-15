# Unsafe code rationale

This record documents the explicit unsafe boundary in the Linux integration-test
fixture. It follows the deviation-rationale approach described by
[MathWorks](https://www.mathworks.com/help/ecoder/ug/deviations-rationale-for-misra-c2012-compliance.html):
scope, justification, permitted circumstances, consequences, and safety evidence.
It does not claim MISRA compliance or a deviation from a formal repository rule.

## PTY master registration

### Scope and reason

`tests/common/device_fixture/mod.rs`, `PtyBoundary::open`, calls
`tokio::io::unix::AsyncFd::register` once to register the nonblocking PTY master
with the Tokio reactor. This is Linux integration-test infrastructure, compiled
into test executables; it is not part of the shipped `serial-mcp` binary.
There is no explicit unsafe Rust in production source. This inventory concerns
repository source, not unsafe code inside dependencies or generated output.

The fixture needs readiness-driven PTY I/O with cancellable device tasks.
Tokio 1.54.1 deprecates the previously safe `AsyncFd::new` constructor and makes
the descriptor-lifetime obligation explicit in the unsafe replacement.
[Tokio's upstream rationale](https://github.com/tokio-rs/tokio/pull/8587)
explains why an `AsFd` bound alone cannot guarantee stable descriptor identity.
The applicable contract is `AsyncFd::register`'s Safety section in the locked
Tokio version; recheck that contract when updating Tokio.

### Alternatives considered

- Keeping `AsyncFd::new` with a deprecation suppression would conceal the same
  lifetime obligation rather than resolve it.
- `tokio-serial` 5.5.0's safe PTY constructors internally use the deprecated
  `AsyncFd::new`. Adopting them would move the boundary into a dependency and
  change fixture setup without eliminating the underlying requirement.
- A blocking worker with explicit polling could avoid a local unsafe call,
  but would require a different cancellation, wakeup, and teardown design.
  `tokio::fs::File` also moves I/O to the blocking pool. Neither is an equivalent
  replacement for the existing readiness-driven fixture.

For this fixture, the single local registration boundary has a smaller review
surface than a new I/O lifecycle. This is a scoped engineering choice, not a
claim that safe alternatives are impossible.

### Permitted circumstances and safety argument

1. `openpty` returns a fresh, valid `OwnedFd`. Setup makes the master
   nonblocking, then moves exclusive ownership directly into `AsyncFd`.
2. `OwnedFd` reports a stable descriptor number and retains ownership of the
   same underlying file description. The fixture never manually closes it,
   replaces it through `get_mut`, extracts it, or hands ownership elsewhere.
3. The private `PtyBoundary` retains `AsyncFd<OwnedFd>`. The read/write helpers
   borrow it immutably and pass `get_ref()` to `nix::unistd::read` and `write`;
   these operations do not close or replace the descriptor.
4. On normal shutdown or task cancellation, `AsyncFd` deregisters the master
   before its owned descriptor is dropped. The fixture does not implement a
   separate close path for the registered master.
5. Registration failure returns ownership inside `AsyncFdRegisterError`.
   Conversion to `std::io::Error` drops that owner before attaching `anyhow`
   context, avoiding retention of the descriptor in the error chain.

These constraints must hold for the entire registration lifetime, not just at
construction. The adjacent `SAFETY` comment links back to this record.

### Risk and consequences

Closing or replacing the descriptor while it remains registered can associate
reactor events with the wrong resource or, as described by Tokio, cause a
use-after-free. Test-only scope does not make undefined behavior acceptable:
it could corrupt or crash the test process and invalidate test results.
Exclusive ownership and the absence of descriptor replacement are the primary
safety argument; successful tests alone cannot prove memory safety.

### Verification and review triggers

`tests/device_fixture.rs::repeated_fixture_shutdown_returns_file_descriptors_to_baseline`
checks 100 joined shutdowns and 100 active-fixture drops, waiting for cancellation
to return PTY descriptor counts to baseline. The fixture suites exercise real
PTY reads/writes, and the ignored `public_boundary_repeat_gate` exercises 100
public MCP lifecycles. These checks support teardown correctness; they do not
replace inspection of the ownership argument above.

Run the affected coverage on Linux:

```sh
cargo test --locked --test device_fixture -- --test-threads=1
cargo test --locked --test device_command_parity -- --test-threads=1
cargo test --locked --test device_framing_parity -- --test-threads=1
cargo test --locked --test device_protocol_parity -- --test-threads=1
cargo test --locked --test device_parity_repeat public_boundary_repeat_gate -- --ignored --test-threads=1
```

Re-review this record and the local `SAFETY` comment whenever Tokio's registration
contract, `PtyBoundary` ownership, read/write access, failure handling, or task
teardown changes. Adding descriptor duplication, raw-descriptor access, mutable
inner access, extraction, or a new unsafe site requires its own assessment;
this record does not authorize those changes. Reconsider the implementation if
a maintained safe API can preserve the required PTY and cancellation behavior.
