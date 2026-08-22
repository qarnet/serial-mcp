# `native_sim` Replacement: Virtual Serial Candidate Survey

**Status:** Stage 1 survey complete on 2026-08-13. Prototype candidates are
shortlisted, but no replacement is selected. Scores below are desk-research
priors and must be replaced by measured Stage 2 results.

## Decision Boundary

Primary fixture acceptance requires all of these properties:

- a real OS terminal pathname accepted by public MCP `open(port=...)` and the
  production `tokio-serial` path;
- lossless transport of every byte from `0x00` through `0xFF` after explicit raw
  terminal configuration;
- deterministic peer close/HUP observation, same-pair slave close/reopen, and
  endpoint replacement/recovery tests;
- bounded cancellation and cleanup with no process, task, file descriptor, or
  symlink leak;
- unprivileged, pinned, reproducible Linux CI operation;
- Linux and macOS support where POSIX PTYs are used, with Windows limits stated
  rather than hidden behind an in-memory substitute.

Existing repository evidence already proves basic shape. The Unix
[`PtyPair`](../../tests/common/mod.rs) uses `nix::pty::openpty`, applies
`cfmakeraw`, gets the slave pathname with `ttyname`, retains both ends, and lets
the server open the pathname through production code. This survey asks whether
another boundary improves that implementation enough to justify replacement.

## Evidence Method

Version and publication dates come from package registries or project release
metadata as observed on 2026-08-13. A later docs.rs rebuild date is not treated
as a release date. API claims come from source or API documentation. PTY kernel
claims use Linux and FreeBSD manual pages:

- [`pty(7)`](https://man7.org/linux/man-pages/man7/pty.7.html) describes the
  master/slave byte channel and UNIX 98 allocation model.
- [`openpty(3)`](https://man7.org/linux/man-pages/man3/openpty.3.html) describes
  pair allocation and `libutil`; the corresponding
  [FreeBSD page](https://man.freebsd.org/cgi/man.cgi?query=openpty) warns that
  the caller-supplied name buffer has no size check and recommends `ptsname`.
- [`termios(3)`](https://man7.org/linux/man-pages/man3/termios.3.html) defines
  raw-mode controls.
- [`ttyname(3)`](https://man7.org/linux/man-pages/man3/ttyname.3.html) defines
  pathname lookup from an open terminal descriptor.

Source inspection proves API availability, not behavioral acceptance. Every
surviving candidate still needs the identical Stage 2 black-box experiment.

## PTY Lifecycle Semantics

Three different claims must not be conflated:

1. **Same-pair reopen:** close serial-mcp's slave descriptor and reopen the same
   slave pathname while fixture still owns the PTY pair. Keeping an extra slave
   descriptor open avoids Linux master-side `EIO` during the gap and keeps pair
   allocation alive. Existing `PtyPair` uses this pattern. Exact master-read
   behavior after the last slave closes varies across Unix systems; Linux
   commonly reports `EIO` where other systems may report EOF.
2. **Peer disconnect:** close every fixture-owned master descriptor while
   serial-mcp still owns the slave. Product read must observe EOF/HUP or an OS
   error within a bound.
3. **Endpoint disappearance and recovery:** destroy one pair, create another,
   and make public open/reconnect reach the replacement. A retained slave path
   proves neither disappearance nor replacement. If one stable caller-facing
   name is required, fixture must own an atomic symlink retarget and cleanup
   contract; raw `/dev/pts/N` allocation does not promise reuse of `N`.

Closing the PTY pair releases kernel resources. Symlinks are separate filesystem
objects and need explicit no-clobber ownership and cleanup, including crash
cleanup. PTYs do not validate physical baud timing, parity faults, modem lines,
BREAK, or electrical flow control.

## Candidate Summary

| Candidate | Release | License | Real path | Linux/macOS | Raw bytes | Lifecycle control | Status |
|---|---|---|---|---|---|---|---|
| direct `nix` | 0.31.3, 2026-05-11 | MIT | yes | yes/yes | yes, with `cfmakeraw` | direct FD ownership | advance |
| `rustix-openpty` + `rustix` | 0.2.0, 2025-03-06; 1.1.4, 2026-02-22 | Apache-2.0 variants or MIT | yes, via `rustix::termios::ttyname` | yes/yes | yes, with termios | direct `OwnedFd` ownership | advance |
| direct `libc` | 0.2.186, 2026-04-23 | MIT OR Apache-2.0 | yes | yes/yes | yes | custom unsafe ownership | deprioritize |
| `portable-pty` | 0.9.0, 2025-02-11 | MIT | yes on Unix | yes/yes | probable; prototype required | terminal/child abstraction | deprioritize |
| `rust-pty` | 0.6.0, 2026-07-31 | MIT OR Apache-2.0 | reachable on Unix | yes/yes | probable; prototype required | async terminal abstraction | deprioritize |
| `pty-process` | 0.5.3, 2025-07-12 | MIT | reachable through its PTS FD | yes/yes | probable; prototype required | process-focused abstraction | deprioritize |
| `virtualport` | 0.1.3, 2025-02-07 | MIT | yes on Unix | Unix/unclear | configurable | external CLI process | deprioritize |
| `virtual-serialport` | 0.1.3, 2024-10-06 | MIT OR Apache-2.0 | no | in-memory | yes internally | mock-pipe only | reject primary |
| `openpty` | 0.2.0, 2021-11-23 | MIT OR Apache-2.0 | yes | Unix | probable | low-level | reject primary |
| `pty` | unmaintained | varies | Unix | Unix | unknown | abandoned | reject primary |
| `socat` | 1.8.1.3, 2026-06-26 | GPL-2.0-or-later | yes | Unix, including Linux/macOS | yes with `cfmakeraw` | supervised child + symlinks | advance comparator |
| Python stdlib `pty` | CPython 3.14.7, 2026-08-05 | PSF-2.0 | yes | yes/yes | yes with `tty.setraw` | explicit FDs in script | advance |
| minimal C helper | local source | project-chosen | yes | yes/yes | yes | custom process + FD ownership | deprioritize |
| tty0tty | 1.4, 2023-01-31 | GPL-2.0-or-later | yes | Linux/no | yes | driver or relay process | reject kernel; deprioritize PTY relay |
| QEMU | 11.1.0, 2026-08-11 | GPL-2.0 overall | yes on Unix | yes/yes | yes | emulator process | reject primary |
| Renode | 1.16.1, 2026-02-16 | MIT | yes on Linux/macOS | yes/yes | yes | .NET emulator process | reject primary |

`Real path` means an OS pathname production serial code can open. Windows
ConPTY handles are not COM ports and do not satisfy this boundary.

## Rust Candidates

### Direct `nix::pty::openpty`

- Canonical sources: [`nix` on crates.io](https://crates.io/crates/nix),
  [repository](https://github.com/nix-rust/nix),
  [`openpty`](https://docs.rs/nix/0.31.3/nix/pty/fn.openpty.html), and
  [`ttyname`](https://docs.rs/nix/0.31.3/nix/unistd/fn.ttyname.html).
- Current release: 0.31.3 on 2026-05-11. License MIT. Declared MSRV 1.69,
  below repository Rust 1.97.1.
- OS support: documented targets include Linux, macOS, FreeBSD, NetBSD,
  illumos, Android, and others. PTY support is gated by `term` and excludes
  AIX.
- Exact API fit: `openpty` returns master and slave `OwnedFd`s; `ttyname`
  returns `PathBuf`; termios offers `tcgetattr`, `cfmakeraw`, and `tcsetattr`.
  Drop gives deterministic descriptor cleanup.
- Dependencies: `libc`, `bitflags`, `cfg-if`, and build-time `cfg_aliases` for
  selected features. Repository already declares `nix` as Unix-only dev
  dependency and lockfile resolves 0.31.3, so prototype adds no crate family.
- Maintenance: long-running crate, team-owned on crates.io, with many human
  contributors. RustSec lists old
  [`RUSTSEC-2021-0119`](https://rustsec.org/advisories/RUSTSEC-2021-0119.html)
  for `getgrouplist`; 0.31.3 is in patched range and fixture does not call that
  API.
- Safety: public path is safe Rust around platform FFI. Fixture still owns
  semantic hazards such as raw-mode order, Linux `EIO`, and which descriptors
  remain alive.
- Status: **advance as direct-Rust baseline**. It is smallest change and already
  proves basic public-path integration. Prototype must add true peer close and
  endpoint replacement, not only repeat existing round-trip coverage.

### `rustix-openpty` plus `rustix`

- Canonical sources: [`rustix-openpty`](https://crates.io/crates/rustix-openpty),
  [repository](https://github.com/sunfishcode/rustix-openpty),
  [`openpty` API](https://docs.rs/rustix-openpty/0.2.0/rustix_openpty/fn.openpty.html),
  [`Pty` fields](https://docs.rs/rustix-openpty/0.2.0/rustix_openpty/struct.Pty.html),
  [`rustix`](https://crates.io/crates/rustix), and
  [`ttyname`](https://docs.rs/rustix/1.1.4/rustix/termios/fn.ttyname.html).
- Current releases: `rustix-openpty` 0.2.0 on 2025-03-06 and `rustix` 1.1.4
  on 2026-02-22. Both use `Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR
  MIT`; both declare MSRV 1.63.
- OS support: wrapper uses `rustix::pty` on Linux and platform `openpty` through
  libc elsewhere. API references Linux, Apple, and FreeBSD. Exact target matrix
  still needs compile proof.
- Exact API fit: returns public `controller` and `user` `OwnedFd`s with
  close-on-exec intent. `rustix::termios::ttyname(&user, Vec::new())` returns a
  pathname; termios APIs configure raw mode. Linux `ttyname` depends on `/proc`.
- Dependencies: `rustix`, `errno`, and platform `libc`. Feature selection can
  keep surface small, but this adds a second Unix abstraction beside existing
  `nix` unless final migration removes `nix` from test code.
- Maintenance: `rustix` is active and broadly used. `rustix-openpty` has one
  crates.io owner and one repository contributor, so wrapper bus factor is one
  even though its foundation is stronger.
- Safety: I/O-safety `OwnedFd` model is strong; platform FFI stays inside
  crates. `openpty` documentation names platform `grantpt`/`SIGCHLD` caveat.
- Status: **advance as strongest low-level Rust challenger**. It gives useful
  ownership guarantees without process/terminal automation baggage. Benchmark
  whether those gains justify dependency churn versus direct `nix`.

### Direct `libc`

- Canonical sources: [`libc`](https://crates.io/crates/libc) and
  [repository](https://github.com/rust-lang/libc).
- Current release: 0.2.186 on 2026-04-23. License MIT OR Apache-2.0. Declared
  MSRV 1.65.
- Exact API fit: platform `openpty`, `ttyname_r`/`ptsname`, `tcgetattr`,
  `cfmakeraw`, `tcsetattr`, `fcntl`, `read`, `write`, and `close` expose every
  required primitive.
- Dependencies/native needs: no Rust abstraction beyond raw bindings, but
  platform C library and often `libutil` supply `openpty`.
- Maintenance: Rust project-owned, active, and broad. Risk lies in local unsafe
  code, not crate maintenance.
- Safety: every allocation, raw descriptor conversion, error path, close, and
  pathname buffer is fixture responsibility. Do not pass a non-null name buffer
  to `openpty`; manual pages provide no length argument and warn about overflow.
  Get pathname from returned slave FD instead.
- Status: **deprioritize**. It cannot improve fidelity over `nix` enough to pay
  for custom unsafe ownership. Keep only as minimal-C-equivalence reference.

### `portable-pty`

- Canonical sources: [`portable-pty`](https://crates.io/crates/portable-pty),
  [WezTerm repository](https://github.com/wezterm/wezterm), and
  [Unix source](https://docs.rs/portable-pty/0.9.0/src/portable_pty/unix.rs.html).
- Current release: 0.9.0 on 2025-02-11. License MIT. No declared crates.io MSRV;
  repository toolchain compatibility must be tested against Rust 1.97.1.
- OS/API fit: Unix source calls `libc::openpty`, stores tty pathname on master,
  exposes it through `MasterPty::tty_name`, and owns descriptors. Linux `EIO`
  on master read is translated to EOF. Windows backend is ConPTY and does not
  expose a COM path.
- Dependencies: `anyhow`, `downcast-rs`, `filedescriptor`, `libc`, `log`, an
  older `nix`, `serial2`, `shell-words`, plus platform-specific crates. This is
  much larger than boundary allocation needs.
- Maintenance: embedded in active WezTerm repository, substantial adoption,
  dominant maintainer plus broader contributor base.
- Safety: Unix implementation contains reviewed unsafe FFI and process setup.
  Writer drop may emit newline plus terminal EOF byte, behavior undesirable for
  an arbitrary-byte device fixture unless bypassed.
- Status: **deprioritize, not hard reject**. Unix slave pathname is available;
  earlier concerns that it cannot expose one are incorrect. Terminal-child
  semantics and dependency weight add risk without clear fixture benefit.

### `rust-pty`

- Canonical sources: [`rust-pty`](https://crates.io/crates/rust-pty) and
  [`rust-expect` repository](https://github.com/praxiomlabs/rust-expect).
- Current release: 0.6.0 on 2026-07-31. License MIT OR Apache-2.0. MSRV 1.88.
- OS/API fit: supports Unix PTYs and Windows ConPTY. Unix
  `UnixPtyMaster::open` uses `rustix` to allocate the master and returns its
  slave pathname; `slave_name` can query it again. ConPTY is not a COM device.
- Dependencies: Tokio with `full`, `rustix`, `libc`, `thiserror`,
  `signal-hook`, and `signal-hook-tokio`; Windows adds `windows-sys`.
- Maintenance: first published 2026-01-05, six releases from one publisher,
  roughly two thousand downloads at review time. Active but young, with
  single-maintainer/bus-factor risk.
- Safety: low-level Unix and Windows implementation necessarily uses unsafe;
  workspace only warns on `unsafe_code` rather than forbidding it.
- Status: **deprioritize, not hard reject**. Async API matches test runtime, but
  process/session/signal and Windows terminal support are outside required
  boundary and increase dependency surface.

### `pty-process`

- Canonical sources: [`pty-process`](https://crates.io/crates/pty-process),
  [repository](https://git.tozt.net/pty-process), and
  [API source](https://docs.rs/pty-process/0.5.3/src/pty_process/lib.rs.html).
- Current release: 0.5.3 on 2025-07-12. License MIT. No declared MSRV.
- OS/API fit: Unix-only, with docs builds for Linux and macOS. `open` returns
  PTY/PTS objects; caller can derive pathname from PTS descriptor with another
  Unix API. Blocking mode uses `rustix`; optional `async` adds Tokio. API goal
  is spawning a child with controlling terminal, not exposing serial device
  fixtures.
- Dependencies: `rustix`; optional Tokio with `fs`, `process`, and `net`.
- Maintenance: active releases and high adoption, but one crates.io owner.
- Safety: low-level ownership mostly delegated to `rustix`; public unsafe
  `Pty::from_fd` exists for caller-owned descriptors.
- Status: **deprioritize, not hard reject**. Endpoint is reachable, but process
  abstraction adds no fidelity over direct allocation.

### `virtualport`

- Canonical sources: [`virtualport`](https://crates.io/crates/virtualport),
  [repository](https://github.com/s00d/virtualport), and
  [PTY source](https://github.com/s00d/virtualport/blob/main/src/pty.rs).
- Current release: 0.1.3 on 2025-02-07. License MIT. No declared MSRV.
- OS/API fit: Unix CLI wraps `nix::openpty`, reports slave name through
  `libc::ttyname`, supports raw-ish terminal configuration, and owns symlink
  cleanup. Android has a separate unsafe allocation path.
- Dependencies: `clap`, `ctrlc`, `libc`, and `nix`; binary-only crate.
- Maintenance: one release, one publisher, under one thousand downloads at
  review time.
- Safety/behavior: source contains unsafe raw-FD and `ttyname` handling, panic
  paths, retries/sleeps, heartbeat, logging, and interactive CLI behavior.
  These policies conflict with deterministic fixture ownership.
- Status: **deprioritize**. It proves a Rust CLI can wrap same primitives, but
  adds behavior and dependency risk instead of a reusable boundary.

### Rejected Rust Options

- [`virtual-serialport` 0.1.3](https://crates.io/crates/virtual-serialport),
  2024-10-06, MIT OR Apache-2.0, MSRV 1.59: source uses in-memory
  `mockpipe::MockPipe`; `SerialPort::name()` returns `None`. It cannot cross
  public pathname boundary, so reject as primary parity fixture. It may remain
  useful for isolated unit tests, which are outside this decision.
- [`openpty` 0.2.0](https://crates.io/crates/openpty), 2021-11-23, MIT OR
  Apache-2.0: tiny Unix implementation, but stale and redundant with maintained
  `nix`/`rustix-openpty`. Reject as primary dependency.
- [`pty`](https://crates.io/crates/pty): RustSec
  [`RUSTSEC-2022-0015`](https://rustsec.org/advisories/RUSTSEC-2022-0015.html)
  says repository has been inactive since 2017, author unresponsive, and no
  patched version exists. Reject.

## Non-Rust and Native Candidates

### Python Standard Library

- Canonical sources: [Python 3.14.7 release](https://www.python.org/downloads/release/python-3147/),
  [`pty`](https://docs.python.org/3/library/pty.html),
  [`os.openpty` and `os.ttyname`](https://docs.python.org/3/library/os.html), and
  [`tty.setraw`](https://docs.python.org/3/library/tty.html).
- Current release: CPython 3.14.7 on 2026-08-05. License PSF-2.0.
- OS/API fit: `pty.openpty()` returns master/slave FDs, usually through
  `os.openpty`; `os.ttyname(slave)` gives path; `tty.setraw(slave)` controls
  line discipline. Module is Unix-only and mainly tested on Linux, FreeBSD, and
  macOS.
- Dependencies: system Python runtime only; no third-party packages needed.
  CI must pin allowed interpreter range and record exact runtime version.
- Maintenance: Python Software Foundation, large contributor/maintenance base.
- Safety: Python script avoids local Rust unsafe but CPython and platform libc
  remain native code. Linux master reads can raise `EIO` after slave closure;
  script must classify this explicitly. Interpreter/process startup and signal
  cleanup add variance versus an in-process Rust task.
- Status: **advance as scripting-runtime prototype**. It is strongest option for
  rapid stateful peer scripting and provides an independent implementation
  language. Measure startup, cancellation, pipe logging, and version variance.

### `socat`

- Canonical sources: [project site](http://www.dest-unreach.org/socat/),
  [repository](https://repo.or.cz/socat.git), and
  [manual](http://www.dest-unreach.org/socat/doc/socat.html).
- Current release: 1.8.1.3 on 2026-06-26. License GPL-2.0-or-later. Runs on
  Unix platforms including Linux and macOS; no Rust MSRV.
- OS/API fit: `PTY` allocates a real endpoint; `cfmakeraw` applies platform raw
  settings; `link=`, `wait-slave`, ownership/mode, and unlink options support
  named endpoints and startup coordination. A pair such as two `PTY` addresses
  gives a ready-made relay, supervised as a child process. Manual marks `raw`
  obsolete in favor of `rawer` or `cfmakeraw`, and says `wait-slave` depends on
  undocumented PTY behavior and does not work on every OS.
- Dependencies/native needs: installed C executable plus its configured system
  libraries. CI must pin package version and fail clearly when unavailable.
- Maintenance/security: mature project with active 2026 releases. Versions
  1.8.0.0 through 1.8.1.1 had SOCKS5 heap overflow
  [`CVE-2026-56123`](https://nvd.nist.gov/vuln/detail/CVE-2026-56123), fixed in
  1.8.1.2; 1.8.1.3 corrected related test portability. Local PTY use does not
  exercise SOCKS5, but prototype must require 1.8.1.3 or later.
- Safety/lifecycle: child supervision, stderr path discovery, symlink ownership,
  graceful termination, forced-kill fallback, and stale-link cleanup remain
  harness responsibilities. GPL utility execution is separate from linking or
  redistributing Rust product, but distribution policy still needs review if CI
  images bundle it.
- Status: **advance as external-native relay comparator**. It is strongest
  ready-made byte relay, but protocol state machine still needs another process
  or endpoint and dependency availability may outweigh saved code.

### Minimal C `openpty` Helper

- Canonical API sources are platform `openpty(3)`, `ttyname_r(3)`, and
  `termios(3)` pages cited above. Version is local source, compiler, and libc,
  not an independently versioned package.
- OS/API fit: a small helper can return slave pathname, apply `cfmakeraw`,
  retain descriptors, relay bytes, and close deterministically on Linux/macOS.
- Dependencies: C compiler, libc, and `libutil` where required. Build scripts,
  cross-compilation, sanitizer policy, and binary discovery become repository
  responsibilities.
- Maintenance/safety: minimal code surface but full manual memory, buffer,
  signal, partial-I/O, and FD ownership burden. Avoid `openpty` name buffer;
  call `ttyname_r` on returned slave descriptor.
- Status: **deprioritize**. It is useful as independent prototype/control, not
  favored production fixture while safe Rust wrappers already expose same OS
  semantics.

### tty0tty

- Canonical active source reviewed:
  [`lcgamboa/tty0tty`](https://github.com/lcgamboa/tty0tty), release
  [v1.4](https://github.com/lcgamboa/tty0tty/releases/tag/v1.4) on 2023-01-31,
  GPL-2.0-or-later. This corrects an earlier discovery note that treated an
  older fork/tag as current.
- Kernel module: creates paired Linux tty devices but needs kernel headers,
  compilation, module installation/loading, permissions/udev setup, and usually
  root. Hosted CI and macOS cannot use it. **Hard reject as primary fixture**
  under privilege criterion; retain only as possible future specialized
  modem-control lane on pre-provisioned runners.
- `pts/tty0tty` userspace helper: unprivileged Linux code allocates two UNIX 98
  PTYs, applies `cfmakeraw`, optionally creates symlinks, and relays with
  `select`. Source explicitly lacks handshake lines. It uses fixed buffers,
  non-thread-safe `ptsname`, polling sleeps, and drops data when destination
  stays full.
- Status: **deprioritize userspace helper**. It duplicates `socat` with weaker
  portability, backpressure behavior, lifecycle control, and maintenance.

### QEMU

- Canonical sources: [QEMU 11.1.0 release](https://www.qemu.org/2026/08/11/qemu-11-1-0/),
  [download](https://www.qemu.org/download/),
  [repository](https://github.com/qemu/qemu), and
  [`-chardev pty` documentation](https://www.qemu.org/docs/master/system/invocation.html).
- Current release: 11.1.0 on 2026-08-11. QEMU as a whole is GPL-2.0. Large,
  mature, multi-maintainer project.
- OS/API fit: Unix `-chardev pty,id=...,path=...` allocates a PTY and optional
  symlink. It is unavailable on Windows. QEMU removes symlink on graceful exit,
  but official docs warn crashes and some startup failures can leave it.
- Cost/risk: full emulator process, machine configuration, large package/build
  surface, many native dependencies, and broad attack surface. No required
  current test behavior needs CPU, device, or guest emulation.
- Status: **reject as primary fixture** due dependency/build cost and
  irrelevant emulation. Reconsider only if future acceptance requires running
  unmodified firmware or hardware model behavior.

### Renode

- Canonical sources: [Renode project](https://opensource.antmicro.com/projects/renode/),
  [release 1.16.1](https://github.com/renode/renode/releases/tag/v1.16.1), and
  [UART PTY documentation](https://renode.readthedocs.io/en/latest/host-integration/uart.html).
- Current release: 1.16.1 on 2026-02-16. License MIT. Organization-maintained by
  Antmicro with broad project contribution.
- OS/API fit: `CreateUartPtyTerminal` exposes a simulated UART through host PTY
  on Linux/macOS and can use a caller-selected path. Windows lacks this PTY
  path.
- Cost/risk: large .NET/native emulator distribution, platform model and
  monitor process, startup/configuration complexity, and firmware/model inputs.
  It solves MCU simulation, not lightweight stateful byte-peer needs.
- Status: **reject as primary fixture** on cost and scope. Reconsider only
  if future tests need CPU/peripheral timing or unmodified firmware execution.

## Provisional Weighted Scorecard

Scores are 0-5 and use research-plan weights. They represent API/source fit,
not measured reliability. `49` means apparent ability to host all 49 scenarios;
no candidate has proved that yet.

| Candidate | Fidelity 25 | Lifecycle 20 | 49 tests 15 | Peer extension 15 | CI cost 10 | Maintenance 5 | Linux/macOS 5 | Risk 5 | Weighted /5 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| direct `nix` | 5 | 4 | 5 | 5 | 5 | 5 | 5 | 5 | 4.80 |
| `rustix-openpty` + `rustix` | 5 | 4 | 5 | 5 | 4 | 3 | 5 | 5 | 4.60 |
| direct `libc` | 5 | 4 | 5 | 5 | 5 | 5 | 5 | 2 | 4.65 |
| Python stdlib | 5 | 4 | 5 | 5 | 3 | 5 | 5 | 4 | 4.55 |
| minimal C helper | 5 | 4 | 5 | 4 | 4 | 2 | 5 | 2 | 4.25 |
| `socat` | 5 | 4 | 5 | 3 | 3 | 5 | 4 | 3 | 4.15 |
| `pty-process` | 4 | 3 | 5 | 5 | 3 | 4 | 5 | 4 | 4.05 |
| `portable-pty` | 4 | 3 | 5 | 5 | 2 | 5 | 5 | 4 | 4.00 |
| `rust-pty` | 4 | 4 | 5 | 5 | 2 | 2 | 5 | 3 | 4.00 |
| tty0tty userspace | 5 | 2 | 4 | 3 | 2 | 2 | 1 | 2 | 3.15 |
| `virtualport` | 4 | 2 | 3 | 2 | 3 | 1 | 3 | 2 | 2.75 |

Direct `libc` score shows limitation of numeric table: maintained bindings and
low dependency cost score well, while local unsafe ownership is concentrated in
one 5% criterion. Qualitative review therefore deprioritizes it despite high
total. Prototype data may similarly expose score-model blind spots.

## Stage 2 Shortlist

Run identical black-box prototype against these candidates:

1. **Direct `nix::pty::openpty`**: incumbent Rust baseline and smallest change.
2. **`rustix-openpty` + `rustix`**: strongest low-level maintained Rust
   challenger, selected over terminal/process abstractions because it exposes
   exact owned resources without unrelated policy.
3. **Python standard-library PTY**: strongest scripting-runtime candidate and
   fastest independent stateful-peer implementation.
4. **`socat` 1.8.1.3+**: external-native relay comparator. Run if available in
   pinned CI environment; it may expose whether a mature utility materially
   improves HUP, symlink, or process cleanup behavior.

Minimal C can serve as diagnostic control only if Rust and Python results
disagree on kernel semantics. `portable-pty`, `rust-pty`, and `pty-process` do
not enter first prototype round because source shows no required capability
missing from lower-level candidates; this is deprioritization, not hard API
rejection.

## Prototype Questions Left Open

- Does Linux and macOS public read report peer master close consistently enough
  for one assertion, or must normalized disconnect outcomes differ by OS?
- Can stable symlink retarget be atomic and no-clobber while serial-mcp holds or
  reopens old endpoint?
- Which owner should retain extra slave FD for same-pair reopen, and when must it
  be dropped to prove endpoint disappearance?
- Does `tcflush` behavior differ materially among direct APIs and external
  relays?
- Can every candidate terminate pending reads and all child/task work within the
  same bound for 100 consecutive lifecycle runs?
- What build, disk, process, FD, and wall-time cost does each add on cold and
  warm CI?
- Does macOS require candidate-specific EOF/EIO handling or pathname logic?
- Can any Windows user-space option expose a real COM path without a signed
  driver? Current evidence says no; controlled-backend and compile coverage stay
  explicit.

No candidate becomes final until Stage 2 records exact versions, commands,
host/kernel, all-byte round trip, HUP, same-path reopen, endpoint replacement,
bounded cleanup, 100-run stability, and cost measurements under
`target/native-sim-research/`.
