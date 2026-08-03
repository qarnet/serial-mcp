# Windows serial E2E investigation

Decision record for whether serial end-to-end testing can run on Windows CI.
Outcome: **deferred** — no privileged virtual-port driver installation on
GitHub-hosted runners.

## Question

The `native_sim` e2e suite is Unix-only (PTY-based; 43 tests ignored on the
Windows runner). Windows CI only compiles and unit-tests. Can Windows real
serial E2E be added, e.g. with a com0com-style virtual port pair?

## Findings

- Windows real serial E2E needs a real or virtual COM device, not a named
  pipe. The PTY trick used on Linux has no named-pipe equivalent that
  exercises the actual COM port APIs; a pipe is not a COM device and would
  not test the serial code path.
- com0com is a kernel-mode virtual serial driver. The project describes
  driver setup requiring administrative steps; common upstream binaries are
  test-signed, so they are sensitive to Windows driver-signature policy and
  may require test-signing/security configuration, administrator privileges,
  or a reboot.
  - Project: <https://sourceforge.net/projects/com0com/>
  - README (test-signing note, 2017-07-13):
    <https://sourceforge.net/projects/com0com/files/com0com/3.0.0.0/README.txt/download>
    states the x64 build is unsigned (test-signed) and "will not load by
    default on x64 Windows" without `bcdedit -set TESTSIGNING ON` plus a
    reboot, and warns that enabling test signing impairs computer security.
- GitHub-hosted runners are ephemeral VMs recreated per job
  (<https://docs.github.com/en/actions/using-github-hosted-runners/about-github-hosted-runners>).
  Installing and signing a kernel driver there would need to happen on every
  job, with no durable machine state, no reboot persistence guarantee, and
  driver-signature policy controlled by the runner image. That is not an
  acceptable default trust/reliability trade-off for this project.
- A self-hosted, pre-provisioned runner with a signed driver, or physical
  loopback hardware, could support Windows serial E2E later. No such runner
  exists today.

## Decision

- Do not install privileged Windows virtual-port drivers in CI, on hosted or
  self-hosted runners, without a separately approved design.
- Keep Windows serial E2E deferred until a pre-provisioned signed-driver
  runner or an approved design exists.
- Current coverage stands: Windows CI compiles and runs unit tests; injectable
  `SerialIo` and the controlled HTTP tests cover lifecycle behavior without
  OS COM devices on every platform.

## Notes on sourcing

com0com is a community project; its builds are not an official,
Microsoft-signed package. The project's own README distinguishes a
"test-signed" (unsigned) x64 build from a signed one, and the "signed"
3.0.0.0 build carries the project's certificate — not a WHQL/Microsoft
signature (project discussion:
<https://sourceforge.net/p/com0com/discussion/440109/thread/c4d52f1b/>). Do not treat third-party/community distributions, including
community re-signed builds, as vendor-signed or as official Microsoft
support. The GitHub-hosted-runner behavior is cited from official GitHub
documentation.

## Status

- [x] Investigation recorded (2026-08)
- [ ] Windows serial E2E implemented — blocked on pre-provisioned
      signed-driver runner or approved design
