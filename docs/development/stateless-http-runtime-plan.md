# Stateless server runtime and platform portability plan

Status: in progress — tracked as the server-runtime entry in
[BACKLOG.md](BACKLOG.md); this document is the design.

Target baseline: `main` at `cbdcf8b61527f0dfced5f0fc0f71704774c72c71`. No
runtime behavior changes have landed yet; all baseline citations were
re-verified against current `main`.

## Goal and terms

Make mutable live serial state **server-runtime-scoped**, never
handler-scoped. A *server runtime* is one independently constructed serial-mcp
instance: one stdio subprocess, one HTTP host, or one embedded/test server in a
host process. A *handler* is an MCP dispatch object created by rmcp.

A modern `2026-07-28` HTTP request receives a fresh `SerialHandler`; every
such handler must therefore use one shared TX queue, RX ring, reconnect
supervisor, and event hub for its owning server runtime.

Preserve the current MCP surface, protocol versions, and legacy HTTP lifecycle:

- `2026-07-28` remains discovery-first and stateless at protocol level.
- `2025-11-25` remains initialized/session-compatible.
- `connection_id` remains the explicit application-state reference passed to
  later tool calls.
- One server runtime owns its own live serial connections. No global singleton
  may couple two separately constructed servers.

## Transport and ownership scope

This is not an HTTP-only plan. Three related ideas have different boundaries:

| Concern | HTTP | stdio | Required owner |
| --- | --- | --- | --- |
| Modern MCP statelessness | Each `2026-07-28` request carries its own protocol metadata. | Same protocol rule. Metadata is in the JSON-RPC `_meta` body because stdio has no headers. | Protocol handler. |
| Fresh handler construction | Yes. rmcp calls the HTTP factory for every modern request and every legacy HTTP session. | No. One stdio subprocess currently creates one long-lived handler. | HTTP transport only. |
| Live serial state | Must survive fresh HTTP handlers so later calls can use `connection_id`. | Must survive unrelated requests on one long-lived stdio stream, but process identity is not a protocol session. | `SerialServerRuntime`. |
| Physical-port ownership | Must reject another HTTP-host process opening an owned port. | Must reject another stdio subprocess opening an owned port. | `ConnectionManager` and its lease provider. |

`2026-07-28` statelessness applies to both transports. The final MCP
[overview](https://modelcontextprotocol.io/specification/2026-07-28/basic)
requires every request to be self-contained and explicitly says an open stdio
process is not a conversation or session. `server/discover` is the
transport-neutral entry point. The current server proves this already:
`src/mcp_protocol.rs` selects `DiscoverStateless`, `src/server.rs::discover`
is transport-neutral, and
`tests/stdio_integration.rs::stdio_2026_07_28_discovery_lifecycle_negotiates_exact_version`
uses the modern lifecycle over stdio.

Fresh handlers are an HTTP implementation detail. `src/main.rs::run_http`
passes a factory to `StreamableHttpService`; modern HTTP dispatch invokes that
factory per request. `src/main.rs::run_stdio` builds one handler and serves one
bidirectional process stream. Stdio still needs the same runtime object for
correct shutdown, future parity, and port ownership. It does not need an HTTP
`LocalSessionManager` or per-request handler factory.

`connection_id` remains application state, not protocol session state. It is
an explicit handle, valid only in its owning runtime. A modern client may send
unrelated requests on one stdio process or across many HTTP requests; the
server must not infer client identity, authorization, or conversation lifetime
from that carrier.

## Scope

### In scope

- Shared TX session ownership across HTTP handler instances.
- One cancellable reconnect supervisor per server runtime.
- Deterministic runtime shutdown for production HTTP, stdio, and the in-process
  HTTP test harness.
- Atomic reconnect-task admission in `ConnectionManager`.
- Tests for modern HTTP, legacy HTTP, stdio, and two independent server
  runtimes operating concurrently.
- Cross-process physical-port ownership at the shared `ConnectionManager`
  boundary, for HTTP, stdio, and embedded server hosts.
- Platform portability fixes found by this plan's Linux, macOS, and Windows
  audit. Platform-specific test assets may remain platform-specific when no
  equivalent device primitive exists; their commands and gates must say so.
- Documentation of instance ownership and deployment limits.

### Out of scope

- New MCP methods, tool arguments, protocol versions, or session stores.
- Changing serial command semantics into an atomic multi-client transaction
  protocol.
- Horizontal routing of one live serial connection across multiple processes.
- Server-to-server UART proxying, read-only followers, or independent RX
  cursors. Those require a broker and observer design, not a second owner.
- HTTP authentication, TLS termination, or public-network deployment policy.
  Those need a separate security design; `Origin` validation is not
  authentication.
- Per-client authorization, ownership, or independent RX cursors. Today any
  caller that knows a live `connection_id` can address it in that server
  runtime, and concurrent clients share the consuming RX cursor. This plan
  preserves that documented contract rather than disguising it as isolation.
- Changing profile persistence behavior during ordinary process shutdown.

## Cross-process serial-port ownership research

Status: required companion delivery slice. It is not an HTTP feature. It
changes serial connection ownership for every server transport and embedding
path, so it belongs at the shared `ConnectionManager` boundary. It may land as
its own reviewable change after the runtime refactor, but must not be bypassed
by stdio, HTTP, or a public production opener.

### Ownership invariant

One physical serial port has exactly one owning serial-mcp server runtime at a
time for cooperating processes under one OS account. One runtime may own many
ports. A second serial-mcp process must never read or write a port already
owned by another process, even if both processes belong to separate OpenCode
sessions on the same host.

Stateless MCP does not alter this invariant. It removes protocol-level HTTP
session state; it does not make UART bytes non-destructive, portable between
processes, or safe for multiple independent readers.

Within one runtime, `ConnectionManager::open` rejects a duplicate port and one
`RxSession` pump reads hardware into one bounded ring. `read` and `transact`
then use a shared consuming cursor. They are intentionally not per-client,
non-destructive streams. Cross-process ownership needs an additional boundary.

### One shared control point

Production opens currently flow through:

```text
open/open_profile tool
  -> tools::port_ops::open_connection
  -> ConnectionManager::open
  -> SystemConnectionOpener
  -> SerialConnection::open
  -> build_stream
  -> tokio_serial::open_native_async
```

Both `run_http` and `run_stdio` construct a `ConnectionManager`, and the public
tool paths route through it. Put lease acquisition there, after its
same-runtime opening reservation and before native I/O. HTTP handler factories
must clone the same manager through `SerialServerRuntime`; a lease is never
created per request or per MCP session.

Two escape hatches need a deliberate fix before this becomes a product
guarantee:

- `SerialConnection::open` is public and can open a native stream without a
  manager lease.
- `ConnectionManager::insert` accepts an already-created connection for test
  backends.

Make native production opening manager-owned. Either make
`SerialConnection::open` crate-private or require an explicit unmanaged/lease
token in its public API. Keep injected test connections through an explicit
no-op lease provider. Do not let a convenient library constructor silently
bypass the same ownership policy used by the binary.

### Two-layer ownership contract

Use both layers. They protect different things.

1. **Retained serial-mcp lease.** An `fs2` non-blocking exclusive file lock
   rejects competing same-user serial-mcp runtimes on every supported platform.
   This is the strict cross-process product guarantee for normal local agent
   deployments.
2. **Native serial open exclusivity.** The OS driver rejects ordinary competing
   opens, including programs that do not use serial-mcp's lease. This is a
   defensive second layer. Its exact behavior must be an explicit, pinned, and
   tested dependency contract.

Do not describe a file lock as protection from arbitrary programs. `fs2`
documents all of its locks as advisory. Do not describe a transitive crate
default as a complete native guarantee either, especially on Windows.

### Native serial-open evidence by release platform

Current production code is `src/serial/connection.rs::build_stream`. It calls
`tokio_serial::open_native_async`. The lockfile resolves that path through
`tokio-serial 5.5.0`, `mio-serial 5.0.7`, and `serialport 4.9.0`.

| Target | Current native mechanism | Finding and required action |
| --- | --- | --- |
| Linux x86_64 | `serialport::TTYPort::open` uses `TIOCEXCL` and exclusive `flock` when the builder is exclusive. `serialport::new` sets `exclusive: true`; `mio-serial` then changes the existing fd to nonblocking. | Continuous native handle. Add real-PTY contention proof. Do not treat privilege or driver bugs as a security boundary. |
| macOS arm64 | Same pinned POSIX `TTYPort` code path uses `TIOCEXCL` and `flock`. Apple documents `TIOCEXCL` as preventing later opens except by a root-owned process. | Equivalent OS mechanism exists. Add hardware or provisioned-device validation before claiming macOS real-device coverage. A Linux PTY test is not macOS evidence. |
| Windows x86_64 | `serialport::COMPort::open` calls `CreateFileW` with share mode `0`. Microsoft requires zero share mode for communications resources. | Resulting handle is exclusive. But `mio-serial::TryFrom<COMPort>` drops that synchronous handle, then reopens with `FILE_FLAG_OVERLAPPED`, also with share mode `0`. There is a short zero-handle gap. |

The Windows gap matters. The retained `fs2` lease prevents a second
same-user serial-mcp runtime from winning it, so normal local
serial-mcp-to-serial-mcp ownership is safe. An unrelated program can
theoretically acquire the COM port after the first handle drops and before the
overlapped handle opens. Do not claim a continuous kernel-exclusive guarantee
against arbitrary Windows programs until one of these is true:

- upstream `mio-serial` opens the first handle with `FILE_FLAG_OVERLAPPED` and
  no close/reopen gap;
- serial-mcp supplies a reviewed Windows native opener that does so; or
- product documentation limits the native guarantee to the final active handle
  and makes the gap explicit.

The pinned `mio-serial` source also labels its Windows support "present but
largely untested by the author." Cross-platform compilation is necessary, not
native COM evidence.

Calling `SerialStream::set_exclusive(true)` cannot solve this portably. That
method is Unix-only in `mio-serial`; Windows uses `CreateFileW` share mode
instead. Keep platform calls inside the native opener. The rest of the server
must depend on one platform-neutral success or failure contract.

Source evidence:

- Pinned `serialport` source:
  `serialport-4.9.0/src/lib.rs`, `src/posix/tty.rs`, and `src/windows/com.rs`.
- Pinned `mio-serial` Windows reopen path:
  `mio-serial-5.0.7/src/lib.rs::TryFrom<NativeBlockingSerialPort>`.
- Pinned `fs2` source: `fs2-0.4.3/src/lib.rs`, `src/unix.rs`, and
  `src/windows.rs`.
- Pinned `dirs` source: `dirs-6.0.0/src/lib.rs`, `src/lin.rs`, `src/mac.rs`,
  and `src/win.rs`.
- [Apple serial-device guidance](https://developer.apple.com/library/archive/documentation/DeviceDrivers/Conceptual/WorkingWSerial/WWSerial_SerialDevs/SerialDevices.html).
- [Microsoft CreateFileW communications-resource rules](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew).

### Portable advisory lease design

Use the existing pinned `fs2 0.4.3`; do not add a second file-lock crate.
`fs2::FileExt::try_lock_exclusive` maps to `flock(LOCK_EX | LOCK_NB)` on Unix
and `LockFileEx(LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)` on
Windows. Compare contention against `fs2::lock_contended_error()`, not a
hand-written Unix errno or Windows error number. A non-contention filesystem
failure is `port_lease_unavailable`, not `port_owned`.

`PortLease` owns exactly one open `File`. Never clone that file or take nested
locks on it. `fs2` documents lock behavior differences for cloned handles. On
explicit close, release the native stream first, then unlock and drop the lease.
On process death, OS handle cleanup releases both native and lease handles. Do
not delete an old lease filename: unlinking a still-locked Unix file can let a
new process lock a new inode and split coordination.

Use a product-owned local-data root:

```text
dirs::data_local_dir()/serial-mcp/port-leases/<sha256(PortKey)>.lock
```

This works on all release targets. With `dirs 6.0.0`, it resolves to XDG local
data or `~/.local/share` on Linux, `~/Library/Application Support` on macOS,
and `FOLDERID_LocalAppData` on Windows. Do not use a runtime directory: the
same `dirs` implementation reports no runtime directory on macOS or Windows.
Do not use profile configuration, caller-selected capture roots, or generic
`/tmp`.

Create this fixed root on demand. If it cannot be resolved, created, opened, or
locked, fail the open before touching serial hardware. No temp-directory
fallback. Validate an existing lease path is a regular non-symlink file before
opening it. The user-local data directory remains an operator-controlled trust
boundary, like the existing capture root. Cross-platform `std` and `fs2` APIs
cannot supply a directory-handle-relative hostile-filesystem defense.

The local-data root is per user, not a system-wide lock service. Cross-user and
non-cooperating-program exclusion comes from native serial open behavior where
the driver supports it. A system-wide lease would need an administrator-owned
directory or device broker and is out of scope.

Use a deterministic cryptographic digest for the filename. Add a hash crate
such as `sha2` if needed. Do not use `DefaultHasher`: its algorithm and output
are not a persistent cross-version key contract. The lock filename must not
expose a USB serial number or raw device path.

First delivery returns a stable generic `port_owned` operational error on lease
contention. Do not make PID checks, stale-file cleanup, force unlock, or owner
metadata part of the authority decision. Optional metadata can be a best-effort
diagnostic later. PIDs recycle, liveness checks race, and a free lease is the
only valid takeover signal.

### Port-key resolution across platforms

Add one `PortKey::resolve` API behind a platform-neutral type. No tool handler,
profile resolver, or manager should compare raw requested strings as its final
identity check.

1. Use existing `profiles::high_identity` first. Its transport, VID, PID,
   nonempty USB serial number, and optional interface survive `/dev` or COM
   path changes.
2. On Unix, resolve an existing character device through a small `cfg(unix)`
   implementation using device metadata such as `rdev`. This collapses a
   `/dev/serial/by-id/...` symlink and its underlying tty path when both refer
   to the same current device.
3. On Windows, normalize known COM aliases case-insensitively. `COM3` and
   `\\\\.\\COM3` must become one key. Do not apply POSIX path canonicalization
   to a COM resource.
4. For a weak or unavailable identity, use a tagged normalized raw-path
   fallback. Document path-reuse limits. Native exclusivity remains the safety
   backstop for that case.

The same resolver must feed allowlist checks, `PortProvider` lookup,
automatic-profile identity capture, `ConnectionManager` duplicate checks,
lease acquisition, and generated connection names. Today
`tools::port_ops::open` matches `PortInfo.name` with raw string equality,
`SecurityManager` matches only the raw request, `ConnectionManager` compares
raw strings, and `ResolvedOpenSettings` splits only `/` for generated names.
Those paths make `COM3`, `com3`, and `\\\\.\\COM3` behave inconsistently and let
path aliases evade same-runtime duplicate detection.

Allowlist evaluation should see both the requested spelling and a provider- or
resolver-confirmed canonical spelling. A user may intentionally allow either
one. An unknown alias must not gain a canonical identity by guesswork.

### Lease lifecycle and manager transaction

Proposed `ConnectionManager::open` transaction:

```text
reserve local PortKey
  -> resolve PortKey and PortInfo
  -> spawn_blocking: create/open lock file and try_lock_exclusive
  -> native open through SystemConnectionOpener
  -> attach PortLease to SerialConnection
  -> atomically publish connection_id
```

Use `spawn_blocking` for directory creation, file open, and lock work. The
operation must not wait for a competing owner. RAII releases the reservation
and lease on a failed native open or cancelled request. Attach the successful
lease before publishing the connection.

Retain the lease across `mark_disconnected` and reconnect attempts. The logical
connection still owns that port identity. `SerialConnection::reconnect` reuses
its lease and reopens native I/O; it never reacquires a new lease. Explicit
close and manager shutdown must release the lease even when best-effort buffer
clear or stream shutdown reports an error. Capture cleanup errors for the tool
result or log, but do not strand an unreachable lease after removing a
connection from the manager registry.

`ConnectionManager::insert` must either acquire and attach a lease before
publication or be made explicitly test/unmanaged-only. Tests using
`ControlledConnectionOpener` should inject a no-op lease provider, not skip the
manager transaction by accident.

### Required ownership proofs

- One manager rejects a duplicate normalized key without touching hardware.
- Two independent processes using one lease root contend for one key. First
  open succeeds; second gets `port_owned` immediately; normal close lets the
  second open.
- Killing the first process releases the OS lock. Second process acquires the
  retained lock filename without deleting it.
- Failed physical open and cancelled open release the lease and local
  reservation.
- Disconnect retains the lease; reconnect uses it; explicit close releases it.
- Pure resolver tests cover high USB identity, Unix aliases where available,
  and `COM3` / `com3` / `\\\\.\\COM3` normalization on every host.
- Linux real-PTY tests prove current native contention. macOS and Windows need
  a provisioned physical or virtual serial device before they can prove driver
  behavior. Controlled backends prove manager and lease behavior on all hosts;
  they do not prove kernel exclusivity.

### Explicit non-goal: server-to-server read proxy

Do not make a losing serial-mcp process tunnel read-only requests through the
winner in this work. That creates a broker protocol, discovery, authentication,
authorization, observer lifecycle, backpressure, replay, and per-observer
cursor design.

When two OpenCode sessions need one device, preferred topology is both clients
connecting to one owner serial-mcp HTTP server. This still exposes the current
shared destructive cursor. Non-destructive observation, if later required,
belongs inside that owner as named/per-client cursors or read-only snapshots,
not as a second server opening or proxying the UART.

## Platform portability audit

The audit covers requested release targets Linux x86_64, macOS arm64, and
Windows x86_64. CI currently compiles, tests, and runs clippy on all three.
Release builds the same macOS and Windows targets, plus Linux arm64. The audit
does not require identical OS primitives. It requires one documented server
contract, narrow `cfg` implementations where primitives differ, and tests that
do not pretend a Linux-only device proves another OS.

### Production behavior that needs action

| Finding | Evidence | Plan action |
| --- | --- | --- |
| Native port ownership is implicit and has a Windows reopen gap. | `src/serial/connection.rs::build_stream`; pinned `serialport` and `mio-serial` sources above. | Add retained lease on all targets. Resolve or document the Windows native-open gap before claiming external-program exclusion. |
| Port identity is raw-string based in critical paths. | `src/tools/port_ops.rs`, `src/security.rs`, `src/serial/manager.rs`, `src/tools/helpers.rs`. | Add `PortKey` and canonical port resolution. Normalize COM aliases and use POSIX device identity only behind `cfg(unix)`. |
| Capture directory durability differs by OS. | `src/capture_store.rs::sync_root_dir` calls directory `sync_all` only on Unix and is a no-op elsewhere. | Keep behavior explicit. Research a supported Windows durability equivalent before claiming rename crash durability. If none is adopted, expose a machine-readable warning or documented lower guarantee rather than treating results as identical. |
| Profile writes sync the file then rename, but do not sync the parent directory. | `src/profile_store.rs::write_atomic`. | Separate persistence-durability audit. Do not expand this runtime change into a risky file-transaction rewrite without a cross-platform contract and tests. |

No current production source hard-codes a Unix serial API outside dependency
boundaries. Serial I/O, enumeration, files, paths, and locks use portable
crates or `std`. The gaps are implicit behavior and inconsistent identity, not
a need to scatter Windows conditionals through tool handlers.

### Test, firmware, and developer-tooling assumptions

| Finding | Evidence | Plan action |
| --- | --- | --- |
| ~~`native_sim` is Linux-only, but one wrapper uses `cfg(unix)` and xtask always builds and runs it.~~ Resolved before implementation: `native_sim` and the NCS firmware stack were removed from `main` (`8d8d33c5`, merged PR #71) and replaced by Linux-only Rust PTY device fixtures, already explicitly Linux-gated in CI. No runtime action left. | (historical) | None. |
| Real PTY suites are Linux-only, although shared comments still say Linux/macOS. | `tests/serial_pty.rs`, `tests/protocol_emulator*.rs`, `tests/common/mod.rs`. | Label PTY coverage Linux-only. Keep controlled-backend public-MCP tests cross-platform. Add macOS real-device evidence only when a supported fixture exists. |
| Windows real serial E2E is intentionally deferred. | `docs/development/windows-serial-e2e-investigation.md`. | Preserve decision. No unsigned or privileged virtual COM driver on hosted CI. Require a pre-provisioned signed-driver runner or physical loopback device for native COM proof. |
| Compatibility and schema-update scripts assume GNU/Linux tools. | `scripts/test-mcp-compat.sh` requires GNU `timeout`; `scripts/update-config-schemas.sh` calls `sha256sum`; `fuzz/run.sh` assumes Linux/Nix paths. | Keep conformance/fuzz Linux-scoped and state it in help/docs. Make schema checksum use `sha256sum` or `shasum -a 256`, or document Linux-only invocation. |

The right fix is sometimes a portable abstraction and sometimes an honest
platform gate. Do not make Windows or macOS CI install a privileged virtual
serial driver merely to make a Linux PTY test appear universal.

## Baseline evidence

| State or task | HTTP today | stdio today | Effect |
| --- | --- | --- | --- |
| `ConnectionManager` | Created once in `main.rs::run_http`, cloned into the handler factory | Created once in `main.rs::run_stdio` | Correct process/server scope. Live connections remain in memory. |
| `RxSessionManager` | Created once and injected into every HTTP handler | One manager for the one stdio handler | Correct. `tests/resource_subscriptions.rs::stateless_requests_share_session_ring_and_cursor` proves cross-request ring and cursor visibility. |
| `TxSessionManager` | Fresh `Arc<TxSessionManager>` in every `SerialHandlerBuilder::build` | One only because stdio builds one handler | Wrong for HTTP and for every legacy HTTP session: no one server-wide TX queue. TX workers are lazy and usually end when a short-lived handler drops, but queues and workers are still not owned or serialized at server scope. |
| reconnect supervisor | Every `SerialHandlerBuilder::build` spawns an unbounded loop | One only because stdio builds one handler | Wrong for HTTP: each request leaves another polling task alive until process exit. |
| `ResourceEventHub`, profile store, capture store, port provider | Created once and cloned into HTTP handlers | Created once and injected into the stdio handler | Correct process/server scope. |
| `LocalSessionManager` | One per `StreamableHttpService` | Not used | Required only for `2025-11-25`; rmcp serves `2026-07-28` statelessly regardless. |
| physical-port ownership | `ConnectionManager` rejects only the same raw port string inside one process | Same raw-string check | No cross-process lease, no alias normalization, and no Windows COM normalization yet. |

Load-bearing source:

- `src/main.rs::run_http` creates a new handler from the service factory at
  lines 376-387, but shares only the listed injected dependencies.
- `src/server.rs::SerialHandlerBuilder::build` constructs the TX manager at
  line 232 and calls `spawn_reconnect_supervisor` at line 239.
- `src/server.rs::spawn_reconnect_supervisor` has no cancellation token or join
  handle.
- `src/tools/io_ops.rs::{write,transact,flush}` route output through the
  handler's `TxSessionManager`.
- `src/serial/manager.rs::start_reconnect` checks for an existing task, drops
  the lock, then spawns and inserts. Concurrent supervisors can pass that gap.
- `tests/common/mod.rs::TestServer::start_inner` repeats the HTTP factory shape
  and therefore repeats the TX/supervisor defect. It also currently does not
  inject the test RX budget into handlers or enable production's strict modern
  metadata setting.
- rmcp 3.1.0 `StreamableHttpService::get_service`
  (`crates/rmcp/src/transport/streamable_http_server/tower.rs`, lines 982-984)
  invokes the stored factory. The stateless POST path calls it for each request;
  the legacy initialize path calls it before creating a session worker.

Thus current code starts an immortal 200-ms reconnect loop once per modern
request and once per initialized legacy session, not once per server.

## Stateless protocol boundary

MCP statelessness does not mean a serial server cannot retain application
state. It means no protocol session or connection history is required to
interpret a request. The current modern path already meets that boundary on
both transports:

- `src/mcp_protocol.rs` assigns `2026-07-28` the
  `DiscoverStateless` lifecycle.
- `src/server.rs::discover` advertises that modern policy and
  `SerialHandler::initialize` rejects modern initialization.
- `src/main.rs::run_http` enables
  `with_stateless_protocol_metadata_required(true)`, the HTTP-only header and
  `_meta` consistency check.
- `tests/protocol_compatibility.rs::raw_discover_succeeds_without_session_and_lists_2026_07_28_first`
  proves discovery succeeds without `Mcp-Session-Id`.
- `tests/stdio_integration.rs::stdio_2026_07_28_discovery_lifecycle_negotiates_exact_version`
  proves the same discovery lifecycle over stdio without an HTTP session.

This follows MCP's stateless-first model: each modern request supplies protocol
metadata, while tool-level state travels by an explicit reference. HTTP carries
some routing metadata in headers and all transport-neutral metadata in `_meta`;
stdio carries it all in `_meta`. The
[2026-07-28 MCP changelog](https://modelcontextprotocol.io/specification/2026-07-28/changelog)
removes protocol-level sessions and describes server-minted handles as ordinary
tool arguments. Here, `open` returns a UUIDv4 `connection_id`; later `read`,
`write`, `transact`, and control tools carry that ID. No new handle protocol is
needed.

The boundary has an important limit. `ConnectionManager`, RX rings, TX queues,
and reconnect tasks are in memory. A `connection_id` is meaningful only to the
server instance that opened it. Stateless MCP therefore does **not** make one
physical UART safely load-balancerable across independent processes. A future
multi-instance deployment would need sticky routing by connection owner or a
separate device-agent/control-plane design. Do not add a global process static
or shared in-memory registry to simulate this: that would make independent
embedded servers interfere and still would not solve cross-host ownership.

This is a server-ownership boundary, not an authorization boundary.
`tools::helpers::lookup_connection` resolves only by ID in the owning
`ConnectionManager`; it does not bind IDs to an MCP client identity. Keep HTTP
authentication and principal-aware connection ownership as separate security
work.

## Decision and alternatives

Choose an explicit, independently owned `SerialServerRuntime`.

| Option | Result | Decision |
| --- | --- | --- |
| Keep handler-owned TX and reconnect services | Modern requests and legacy sessions multiply queues and immortal supervisors. Lifecycle cannot be joined at transport shutdown. | Reject. |
| One process-global runtime | Masks HTTP factory duplication, but makes two embedded/test servers share ports, RX cursors, events, and shutdown. Does not work across processes. | Reject. |
| Make every HTTP request own a complete runtime | Loses `connection_id`, ring, cursor, and profile-visible connection continuity after one request. | Reject. |
| Explicit `Arc<SerialServerRuntime>` per host/server | Gives handler factories shared live state, preserves stateless protocol behavior, permits deterministic shutdown, and keeps separately constructed servers isolated. | Select. |

## Recommended design

### Introduce an explicit per-server runtime

Add public `SerialServerRuntime` in `src/server_runtime.rs`. It owns mutable
live-connection services for exactly one server runtime:

```text
SerialServerRuntime
├── ConnectionManager
│   └── PortLeaseProvider
├── BufferBudget
├── ResourceEventHub
├── RxSessionManager
├── TxSessionManager
├── shutdown CancellationToken
└── RuntimeTasks (async mutex)
    ├── ReconnectSupervisor { join handle }
    └── optional PortWatcher { join handle }
```

`SerialServerRuntime::builder()` accepts the manager, budget, and event hub
needed for dependency injection. The manager carries its `PortLeaseProvider`,
which uses one machine-local lease root across independently constructed
runtimes. The runtime's `build()` is pure: it always constructs its own RX and
TX managers from those exact dependencies and starts no task. That prevents
callers from combining an RX registry from server A with the budget or
notification hub from server B, and keeps construction valid outside a Tokio
runtime.

Profile store, capture store, port provider, and security policy remain handler
configuration supplied alongside the runtime. They may be independently chosen
or intentionally shared by embedded servers; they are not live connection
state. `PortWatcher` belongs to the runtime lifecycle because it publishes to
the runtime hub, but starts explicitly with a provider and interval rather than
from the pure constructor.

`SerialHandler` receives `Arc<SerialServerRuntime>` and reads its connection,
RX, and TX services from that runtime. HTTP service factories clone this one
`Arc`; they do not construct mutable services, background tasks, or port
leases. Stdio builds one runtime once and passes it to its one handler.

### Preserve builder and embedding compatibility deliberately

`SerialHandler::builder()` is public library API. Do not silently remove its
current automatic-reconnect behavior for one-handler embedders.

Use this migration shape:

1. Add `SerialHandler::for_runtime(Arc<SerialServerRuntime>)`, returning a
   dedicated configuration builder for security, profiles, capture, and port
   provider. Keep live-runtime injection out of that builder, so its behavior
   cannot depend on whether `.connections(...)` was called before or after a
   hypothetical `.runtime(...)` setter.
2. Keep `SerialHandler::builder()` and `SerialHandler::new()` as additive
   compatibility paths. They build a private single-handler runtime from the
   existing options, including the normal system lease provider, and start its
   reconnect supervisor as they do now.
3. Document that a caller creating more than one handler, or requiring
   deterministic shutdown, must create and retain a `SerialServerRuntime` and
   use `for_runtime`. Private fallback runtimes preserve old convenience, not
   host-level lifecycle control.
4. Put idempotent `start_reconnect_supervisor`, optional
   `start_port_watcher`, and async `shutdown_and_join` on the runtime. Only
   these runtime lifecycle methods create or join background tasks. The legacy
   builder invokes `start_reconnect_supervisor` once for its newly created
   private runtime; constructing a handler for an injected runtime never does.

This additive path avoids a semver-breaking replacement of the builder while
giving HTTP factories a correct ownership boundary. A later major-version API
cleanup can make runtime ownership mandatory if real embedders justify it.

### Reconnect supervisor and task admission

Move `SerialHandler::spawn_reconnect_supervisor` into the runtime. It must:

- start once for that runtime;
- receive a runtime cancellation token;
- stop and join before runtime teardown completes;
- use the runtime's exact `ConnectionManager` and `RxSessionManager`;
- never start merely because a stateless request constructed a handler.

Fix `ConnectionManager::start_reconnect` at the same time. Hold its state lock
through duplicate-task admission, `tokio::spawn`, and handle insertion, with no
await inside the critical section. Fetch and validate the connection before the
critical section only when safe, then re-check it while admitting the task.
`ConnectionManager::close` or shutdown can then remove and abort the inserted
task without a check/spawn/insert gap. This closes the current race independently
of the supervisor fix.

Runtime shutdown also needs to own the per-connection reconnect tasks held by
`ConnectionManager`, not only the outer polling supervisor. Add an idempotent
manager `shutdown_all` path that marks the manager stopping, atomically drains
connection and reconnect-task maps, aborts and awaits the drained reconnect
handles, then closes the drained connections and releases their retained port
leases. It must reject a new open after shutdown begins and must not invoke
profile-learning code.

The supervisor is an instance service, not a global singleton. Two
`SerialServerRuntime`s may each supervise their own disconnected connection;
neither may inspect, reconnect, or cancel the other's connection.

### Shutdown contract

`SerialServerRuntime::shutdown_and_join` is idempotent. Hosts call it only
after stopping ingress and waiting for in-flight requests; runtime shutdown
then performs this order:

1. cancel the runtime token and take the reconnect-supervisor and watcher
   handles from `RuntimeTasks`;
2. await those tasks, so no supervisor can schedule a new reconnect;
3. invoke manager `shutdown_all`, which joins per-connection reconnect tasks
   and closes live ports without invoking user-facing profile learning;
4. drain RX and TX session maps, then cancel and join every pump and worker,
   releasing ring-budget reservations and file descriptors;
5. return only after all taken tasks have joined. Never rely on `Drop` for
   asynchronous cleanup.

RX/TX manager bulk shutdown takes sessions out of their maps before awaiting
their joins. `ConnectionManager::shutdown_all` similarly releases its registry
lock before waiting or doing I/O. Current process shutdown does not perform a
synthetic `close` tool call, so runtime teardown must not unexpectedly persist
profile defaults. Connection teardown closes native I/O before it unlocks and
drops each `PortLease`, even if an earlier best-effort buffer cleanup failed.

Extract reusable host shutdown plumbing. `run_http` first asks Axum to stop
accepting work and drain requests, then calls runtime teardown. `run_stdio`
calls the same teardown after its service ends on stdin closure.
`TestServer::shutdown_and_join` follows that graceful path; its `Drop` remains
an abort-only last-resort cleanup for a panicking test, not acceptance evidence.

## Transport effects

### Modern HTTP (`2026-07-28`)

Every request remains self-contained at MCP level. Factory-created handlers
share one runtime, so a request that opens a connection, another that writes,
and a third that closes all operate on the same connection, TX queue, RX ring,
and reconnect policy. Existing strict header and `_meta` checks remain enabled.

### Legacy HTTP (`2025-11-25`)

Do not replace `LocalSessionManager` with a never-session manager. Legacy
clients still need initialized sessions. Their session workers must also clone
the same `SerialServerRuntime`, because separate legacy sessions can access the
same process-owned serial connection. Sharing a runtime makes output ordering
and reconnect behavior consistent across modern and legacy clients.

### stdio

Stdio creates one handler today, so it does not multiply TX managers or
supervisors. Modern stdio is still stateless at protocol level: no
`initialize`, no implicit session state, and each request supplies its own
`_meta` metadata. The long-lived stdin/stdout carrier does not change that.
Legacy stdio keeps the `2025-11-25` initialized lifecycle.

Stdio must construct and own one `SerialServerRuntime`:

- behavior stays identical for normal tool calls;
- the same shutdown contract releases ports when stdin closes;
- future changes cannot make HTTP and stdio diverge;
- modern discovery and per-request metadata remain supported without adding
  HTTP session semantics;
- the same `ConnectionManager` lease blocks a separate stdio or HTTP process
  from opening an owned physical port.

### Concurrent serial operations

One shared TX manager serializes `write` and output `flush` operations. It does
**not** make simultaneous `transact` calls atomic request/response exchanges:
they still share one RX cursor and can compete for replies. This plan must not
advertise transaction isolation or silently add an operation lease. Document
that limitation and keep concurrent `transact` serialization as a separate,
evidence-driven feature if users need it.

`SerialConnection::write` already holds its underlying I/O mutex through one
write-and-flush call. That prevents byte-level overlap today, but it is not a
server-wide request FIFO and does not give independently created TX managers a
shared lifecycle. The runtime change must not claim a pre-existing corruption
bug or invent transaction isolation; it establishes one owner for queueing,
backpressure, and cleanup.

## Test plan

Tests must prove public behavior and lifecycle, not `Arc` identity or helper
call counts.

### Runtime and reconnect unit tests

- Build two independent runtimes with separate controlled connections. Cause
  only A to disconnect; public status/log output must show A's reconnect work
  while B neither reconnects nor stops. Shut down A and prove B remains usable.
- Force concurrent `start_reconnect` admission for one disconnected connection
  with deterministic test synchronization. Assert one reconnect attempt per
  retry window through externally visible status/log effects and bounded
  completion, not `Arc` identity or helper-call counts.
- Verify runtime shutdown joins its outer supervisor, manager reconnect tasks,
  RX pumps, and TX workers within a bounded timeout. A post-shutdown attempt to
  open through that runtime must fail cleanly rather than resurrecting it.

### In-process HTTP integration tests

Extend `TestServer` to create one runtime, inject its one budget into every
handler, and match production's strict modern metadata configuration. Use two
distinct modern clients so calls traverse separate stateless handler instances.

Fixture constraint: `SerialConnection::reconnect` always calls production
`build_stream`, so `ControlledIo` cannot prove a successful reopen. Use a real
PTY plus test setup that obtains the injected manager's connection and calls
`SerialConnection::mark_disconnected` for successful auto-recovery. Use
controlled backends for cross-platform isolation and fault/lifecycle boundaries.

- Open through one modern request, then write and output-flush through later
  requests. A gated device backend must observe all complete frames in their
  causal enqueue order and see output flush after preceding output.
- Construct many modern handlers, then fault-inject a fatal disconnect on a
  real PTY-backed connection through the injected test manager. With
  auto-reconnect enabled, prove the same `connection_id` returns to `open` and
  exchanges bytes without an explicit `reconnect` call. The fault injection is
  setup; status, log, and byte flow are public-boundary assertions.
- Exercise one modern and one legacy HTTP client against the same connection.
  Verify write/flush behavior stays ordered and close from either lifecycle
  stops later output.
- Gracefully stop an HTTP server with an open real PTY, then open that same PTY
  from a new server. Success proves pumps, workers, supervisors, and port
  handles were released.

### Multiple-server isolation tests

Add two levels of evidence.

1. **Cross-platform in-process:** start two `TestServer`s with distinct
    managers, runtimes, hubs, and controlled ports. Run operations concurrently.
    Inject RX into A and B, assert each client receives only its own marker,
    verify A's resource notification does not arrive at B, and send A's live
    `connection_id` to B to assert `Connection ID … not found`. Stop A, then
    prove B still writes and reads.
2. **Linux real process / real PTY:** start two `SpawnedServer` binaries and two
     distinct `PtyPair`s. Open one PTY per server through public MCP calls, drive
     both device ends concurrently, and assert each process receives only its own
     bytes. Add a Linux-only graceful `SpawnedServer` stop that sends `SIGINT` and
     waits for exit; stop one child cleanly and prove the other remains usable.
     Controlled-server coverage proves transport-neutral runtime behavior on
     Windows and macOS. It does not prove their native serial-driver behavior.

Do not test two server instances sharing one serial port as a success case. A
physical port has one owner; operating systems and serial drivers enforce that
boundary. A future deployment design may test the second open fails cleanly,
but it is not runtime sharing.

### Port-ownership and stdio integration tests

- Exercise two independent HTTP child processes and two independent stdio
  children against one lock root and one native Linux PTY. First owner succeeds;
  second gets `port_owned` without a device write; close or process death lets a
  later owner open. The same lease behavior must not depend on transport.
- Run portable child-process lease tests with a controlled/no-op serial opener
  on Linux, macOS, and Windows. These prove `fs2` contention, cleanup, and COM
  key normalization without claiming a virtual device is a real COM port.
- Keep native serial-driver contention tests target-specific: Linux PTY now;
  macOS and Windows only after a provisioned device fixture exists.

- Add a test-only stdio-child wrapper that retains child stdin/stdout and can
  close stdin, then await child exit with a bounded timeout. Existing
  `TokioChildProcess` convenience helpers remain for ordinary protocol tests.
- Exercise open, write, and read through the shipped stdio binary and a real
  PTY. Close client stdin, await orderly child exit, then open the same PTY from
  a fresh server. This proves EOF invokes runtime cleanup rather than relying
  on process abort.
- Run two stdio children on two PTYs concurrently. Closing one stdin must not
  affect the other's open connection or byte flow.
- Send `server/discover` and modern tool calls through one stdio child without
  `initialize`; verify the child continues serving self-contained modern calls.
  Keep the existing legacy stdio initialize test as the permanent compatibility
  counterexample.

### Regression gates

Keep existing modern header/meta, discovery, cache, subscription, legacy
session, profile persistence, and PTY tests. Add the new tests to existing
cross-platform or Linux-specific targets rather than creating hardware-required
coverage. Gate native-sim work explicitly to Linux in xtask and test wrappers.
Ensure `bash scripts/test-mcp-compat.sh` still passes on its documented Linux
scope, because the runtime change must preserve both advertised protocol
versions.

## Planned file map

| Path | Planned responsibility |
| --- | --- |
| `src/server_runtime.rs` | New public per-server runtime, its pure builder, task ownership, startup methods, and idempotent async shutdown. |
| `src/lib.rs` | Export `SerialServerRuntime` without exposing private task internals. |
| `src/server.rs` | Make handlers reference runtime services; add `SerialHandler::for_runtime`; retain private-runtime compatibility builder; remove handler-owned reconnect spawning. |
| `src/main.rs` | Build one runtime per stdio or HTTP host; pass its cancellation token to HTTP service/watcher; drain host then call runtime shutdown. |
| `src/serial/port_lease.rs` | New `PortKey`, portable resolver, retained `PortLease`, `PortLeaseProvider`, local-data lock root, and narrow Unix/Windows identity code. |
| `src/serial/connection.rs` | Attach retained lease, release it after native I/O close on every close path, and resolve the Windows native-open contract. |
| `src/serial/manager.rs` | Acquire lease before native open, prevent raw-key aliases, retain/release leases through reconnect and shutdown, atomic reconnect admission, reconnect-task drain/join, manager stopping state, and no-profile-learning `shutdown_all`. |
| `src/serial/port_info.rs`, `src/tools/port_ops.rs`, `src/security.rs`, `src/tools/helpers.rs` | Resolve requested versus canonical ports consistently for enumeration, allowlists, profiles, duplicate checks, leases, and generated names. |
| `src/rx_session.rs`, `src/tx_session.rs` | Bulk drain-and-join methods that never await while holding session-map locks. |
| `src/resource_events.rs` | Reuse `PortWatcher`; runtime stores its optional lifecycle handle instead of handlers or factories starting it. |
| `Cargo.toml` | Add only a deterministic cryptographic hash dependency if the chosen `PortKey` filename format needs it. Reuse existing `fs2` and `dirs`. |
| `tests/common/mod.rs`, `tests/common/controlled.rs` | Give `TestServer` one injected runtime/budget, production-equivalent strict metadata, graceful `shutdown_and_join`, and injectable no-op/real lease providers. |
| `tests/common/spawned.rs` | Add bounded Linux graceful `SIGINT` stop and independent-child ownership test helpers. |
| `tests/resource_subscriptions.rs`, `tests/http_integration.rs`, `tests/serial_pty.rs` | Add cross-handler runtime, modern/legacy, reconnect, graceful-release, in-process isolation, and Linux native-lease behavior proofs. |
| `tests/stdio_integration.rs` | Add stdin-EOF child lifecycle, two-child PTY isolation, and modern discovery-without-initialize proofs. |
| `tests/native_sim_validation.rs`, `tests/native_sim_connection_lifecycle.rs`, `xtask/src/main.rs` | ~~Mark Linux-only native-sim work with exact target gates and skip it explicitly elsewhere.~~ Obsolete: `native_sim` was removed from `main` (`8d8d33c5`); its replacement PTY fixtures are already Linux-gated. No file changes planned. |
| `scripts/update-config-schemas.sh`, `scripts/test-mcp-compat.sh`, docs | Add portable checksum fallback or clear Linux scope; correct Linux-only PTY/native-sim wording. |
| `README.md`, `docs/agent-config.md`, `AGENTS.md`, `docs/development/README.md` | State protocol versus runtime versus port ownership, process-local `connection_id` limits, platform test coverage, shutdown behavior, and test-map updates without changing tool wire contracts. |

## Implementation order

1. Decide the Windows native-open contract. Audit or patch the `mio-serial`
   close/reopen path before promising uninterrupted driver exclusivity against
   arbitrary programs. This does not block a serial-mcp-to-serial-mcp lease,
   but it blocks a stronger external-owner claim.
2. Add `PortKey`, portable `PortLeaseProvider`, deterministic hash filenames,
   and controlled cross-platform contention/cleanup tests. Normalize COM
   aliases and resolve Unix device aliases behind narrow `cfg` code. Make all
   production open paths use the manager boundary.
3. Add `src/server_runtime.rs`, its runtime builder, pure construction,
   lifecycle state, and bulk manager/RX/TX shutdown helpers. Add focused
   reconnect-admission, lease-retention, and teardown tests.
4. Add `SerialHandler::for_runtime` and migrate handler internals to runtime
   access. Preserve `SerialHandler::builder()` / `new()` private-runtime
   fallback and their documented one-handler reconnect behavior.
5. Convert `run_http`, `run_stdio`, and `TestServer` to one runtime each.
   Extract reusable graceful-host shutdown plumbing; make test HTTP metadata
   validation match production.
6. Add modern/legacy HTTP cross-handler, reconnect, ownership, and
   graceful-PTY-release tests. Keep raw modern header/meta and legacy session
   matrix coverage intact.
7. Add two-runtime in-process isolation, Linux two-process PTY ownership, and
   stdio EOF/lifecycle tests with bounded graceful child-stop helpers. Correct
   native-sim and test-tool platform gates.
8. Update README, `docs/agent-config.md`, AGENTS.md, transport/test-map prose,
   and this plan's status. Add an ADR if public embedding ownership or the
   Windows native-open choice becomes a durable API or deployment contract.

## Acceptance criteria

- No fresh `TxSessionManager` or reconnect-supervisor task per HTTP request.
- Exactly one runtime-owned supervisor per server instance, cancelled and joined
  during orderly shutdown.
- Runtime shutdown joins manager reconnect tasks, RX pumps, TX workers, and
  watcher work; it neither accepts a new open nor writes profile-learning state.
- Modern and legacy HTTP handlers share their owning server's runtime.
- Modern stdio remains protocol-stateless and legacy stdio remains initialized;
  both use the same runtime model without changing public tool behavior.
- Two independent servers run concurrently without sharing connections, RX,
  TX, reconnect work, resource events, or shutdown signals.
- One server instance cannot make a live `connection_id` usable on another.
- Every production HTTP, stdio, and embedded-manager open takes one retained
  cross-platform lease before native I/O. Same physical port or confirmed alias
  cannot be opened by two same-user serial-mcp runtimes.
- Linux, macOS, and Windows use documented native serial-open behavior. The
  Windows close/reopen gap is removed or explicitly limits the claim about
  arbitrary external programs.
- No stale-lock deletion, PID authority, force unlock, or generic `/tmp` lease
  fallback exists.
- Native-sim and PTY commands are explicitly Linux-only; portable controlled
  tests remain runnable on macOS and Windows.
- No MCP tool, schema, protocol-version, cache-policy, or legacy-client
  regression.
- Full required validation passes:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
nix flake check --accept-flake-config --print-build-logs
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --locked
bash scripts/test-mcp-compat.sh
```

## Separate future work

If HTTP remains advertised for remote or public deployment, create a security
design before changing the default bind behavior. It must define per-request
authentication and authorization, TLS/reverse-proxy trust boundaries, allowed
hosts/origins, and audit/rate-limit policy. rmcp's Host and Origin options are
useful DNS-rebinding and browser defenses, but neither authenticates a
non-browser MCP caller.
