# Research Plan — NCS-Free Serial Device and Protocol Simulation

**Status:** Recommendation approved and Phases A-E implemented on 2026-08-13.
Direct `nix::pty::openpty` plus a Rust device fixture is the approved
replacement. Phase E evidence is complete, and Phase F removed active
native_sim/NCS source and configuration from serial-mcp. Retained differential
rows, batch handoffs, and research remain historical records. Phase F active
source/configuration removal and fresh clean-checkout CI acceptance are complete;
ADR acceptance is recorded in the [canonical Phase F acceptance record](native-sim-replacement-research-progress.md).

> Full normalized differential parity window accepted on 2026-08-23: all 42 executable cases passed in three serial 22-batch runs; two consecutive canonical 22-report manifests matched SHA-256 `b31b3f3da1412d210096a618cc8d6b6acc5bbed167de525c8122704342c3d3fe`.

> Local worktree verification on 2026-08-23 passed `nix flake check --accept-flake-config` and `nix develop --ignore-env`; the shell found no `west`, `nrfutil`, or `nrfutil-sdk-manager` on `PATH` before `cargo test --locked`. This is not fresh clean-checkout CI evidence.

## Execution Outcome

| Artifact | Result |
|---|---|
| [test traceability and NCS coupling](native-sim-test-traceability.md) | all 49 tests classified; no silent retirement |
| [candidate survey](native-sim-virtual-serial-candidate-survey.md) | source-linked shortlist and rejection record |
| [boundary prototype results](native-sim-boundary-prototype-results.md) | 100-run nix/rustix/Python comparison; direct nix wins; peer-close blocker found |
| [emulator core research](native-sim-emulator-core-research.md) | in-process layered fixture, clock/queue/fault/ownership design; 5 prototype tests pass |
| [protocol peer worksheets](native-sim-protocol-peer-worksheets.md) | every shipped preset/framing/parser has state, vectors, faults, and oracle; 7 peer prototypes pass |
| [replacement recommendation](native-sim-replacement-recommendation.md) | approved dependencies, architecture, tradeoffs, differential phases, rollback points, deletion set |
| [accepted ADR](../adr/replace-native-sim-with-rust-pty-device-fixture.md) | Accepted after fresh clean-checkout CI evidence; see the [canonical Phase F acceptance record](native-sim-replacement-research-progress.md) |
| [resumable progress checkpoint](native-sim-replacement-research-progress.md) | detailed evidence, failures, current conclusions, and resume order |

Initial Stage 2 research did not falsely pass hard acceptance. Every PTY
candidate transported bytes and cleaned up, but peer-master closure produced
public read `timeout` rather than `connection_closed` in all 100 iterations.
Phase A resolved that historical blocker with narrow `UnexpectedEof`
classification and a 100/100 public `connection_closed` proof. Phase B resolved
stable-path reconnect to a distinct replacement endpoint. Python fulfills the
third viable boundary class; `socat` was unavailable and remains a waived
comparator because it adds a relay process around the same PTY primitive without
supplying the stateful peer. The former migration gate required that Full differential outcome comparison precedes Phase F; that blocker is resolved by
the accepted Phase E evidence. Phase F active source/configuration removal
followed; fresh clean-checkout no-NCS acceptance is complete and recorded in the
[canonical Phase F acceptance record](native-sim-replacement-research-progress.md).

## Objective

Select and prove a lightweight test-device foundation that:

1. preserves or improves all behavior now covered by the 43
   `native_sim_validation` tests and 6 `native_sim_connection_lifecycle`
   tests;
2. exercises serial-mcp through a real OS serial/TTY path and the public MCP
   `open(port=...)` boundary;
3. supports deterministic, stateful simulation of every shipped protocol
   preset, framing mode, and parser;
4. supports timing, fragmentation, backpressure, disconnect, malformed-data,
   and recovery scenarios without product-only bypasses;
5. runs in normal Rust CI without an embedded SDK or multi-gigabyte toolchain.

Rust is preferred because it can share build, lifecycle, cancellation, and CI
tooling with this repository. It is not a predetermined result. A non-Rust
native utility may win if measured fidelity, reliability, maintenance, or
portability is materially better.

## Scope and Non-Scope

In scope:

- POSIX PTY or equivalent virtual serial boundary used by public
  `SerialConnection::open`;
- replacement of current firmware command/state behavior;
- all seven shipped protocol presets: `at_command`, `slip`, `json_lines`,
  `cobs`, `ndjson`, `nmea0183`, and `modbus_ascii`;
- all shipped RX/TX framing modes: line, delimiter, length-prefixed,
  start/end, SLIP, and COBS;
- all shipped parsers: AT command, JSON lines, shell prompt, raw, NMEA, and
  Modbus ASCII;
- reusable device-state, stream-generation, and fault-injection support;
- CI, Nix, xtask, documentation, and dependency removal needed to eliminate
  NCS completely.

Out of scope for replacement acceptance:

- pretending a PTY validates physical baud timing, parity, stop-bit errors,
  electrical flow control, modem-line behavior, or BREAK on a UART wire;
- deferred protocols in `docs/development/protocol-matrix.md` such as HDLC,
  Modbus RTU, MIDI, or Firmata. They become simulator requirements when their
  product status changes to shipped;
- hardware RF, USB-device-controller, or MCU instruction emulation;
- reusing serial-mcp's production decoder as the only device-side oracle.

PTY limitations must remain explicit. Existing controlled-backend tests own
deterministic DTR/RTS, BREAK, and failure injection where an OS PTY has no
faithful physical model. Hardware or a privileged virtual-serial lane remains
necessary for electrical claims.

## Current Baseline to Preserve

Load-bearing evidence:

- `tests/common/mod.rs::pty::PtyPair` already creates a raw OS PTY with
  `nix::pty::openpty`; serial-mcp opens its slave path through normal
  `tokio_serial` production code.
- `tests/protocol_emulator.rs` already runs a stateful Rust device task on the
  PTY master. This proves basic repository integration, not full replacement
  suitability.
- `tests/native_sim_validation/unix.rs` contains 43 public-MCP tests.
- `tests/native_sim_connection_lifecycle.rs` contains 6 public-MCP lifecycle
  tests.
- `firmware/src/main.c`, `command.c`, `command.h`, `uart_drv.c`, and
  `uart_drv.h` define current test-device behavior.
- `.github/workflows/ci.yml`, `flake.nix`, `xtask`, test helpers, and current
  docs contain NCS/native_sim coupling that final removal must delete.

Current behavior inventory:

| Area | Existing proof that must be traced |
|---|---|
| basic I/O | boot banner, ping, pending read plus later write, split writes, exact byte trace, partial-line completion |
| stream pressure | deterministic spam, bounded read, delayed command, slow consumer, TX hold, queue status, flush races |
| lifecycle | named open, counters/status, reconfigure, flow-control state, close during read, close/reopen, process exit `42`, arm-only boot capture |
| matching | literal, regex, glob, framed match, `max_frames` |
| framing | line variants, delimiter, length prefix, start/end, SLIP, COBS, TX framing observation, malformed SLIP and recovery |
| parsing/presets | AT, JSON lines, NDJSON including blank-line behavior, NMEA checksum parsing, Modbus ASCII LRC parsing, connection-default precedence |
| discovery | valid port-list and identity result behavior while a PTY connection exists |

First research artifact must turn this inventory into a test-level
traceability table. Each of 49 tests needs one disposition: unchanged,
parameterized, strengthened, split, or retired with written evidence that it
duplicates a stronger public-boundary test. No silent coverage deletion.

## Research Questions

Research answered these questions before implementation began; they remain the
source-backed acceptance record for the completed A-E phases and final
acceptance:

### Virtual serial boundary

1. Is direct `openpty` through existing `nix` dependency sufficient, or does a
   maintained Rust PTY crate provide useful lifecycle/portability guarantees?
2. Should emulator run as in-process task, separate Rust test binary, or both?
3. Which design gives deterministic slave-path creation, raw byte transport,
   close/HUP observation, path reopen, process cleanup, and no leaked file
   descriptors?
4. Can endpoint disappearance and restoration be represented honestly? A
   reconnect call on an already-open PTY is not disconnect/reconnect proof.
5. Which behavior differs on Linux and macOS? Is Linux-only core acceptance
   justified, and what remains compile-tested elsewhere?
6. Can Windows use an unprivileged user-space equivalent, or should existing
   compile-only/controlled-backend coverage remain explicit until a suitable
   signed-driver runner exists?
7. What PTY kernel semantics affect `tcflush`, buffering, HUP/EIO, retained
   slave descriptors, and reopen behavior?

### Emulator architecture

1. Can one bounded state-machine core model current command behavior without
   reproducing Zephyr scheduler/IRQ details that tests do not observe?
2. Which clock model gives deterministic delays, cadence, timeouts, and race
   tests: Tokio paused time, real bounded time, or an injected clock?
3. How should output queues expose chunk size, capacity, hold, drop, and drain
   behavior without depending on host scheduler accidents?
4. How should scripted faults express delayed chunks, malformed frames,
   silence, peer close, crash, endpoint swap, and queue saturation?
5. Which parts belong in reusable fixture library versus protocol-specific
   peer implementations?

### Protocol peers and independent oracles

1. Which maintained Rust crates can act as peer implementations or independent
   encoders for each shipped protocol?
2. Where no good crate exists, is a small spec-derived implementation safer
   than adding a dependency?
3. Which official vectors or independently generated fixtures prove encoding,
   escaping, checksum, fragmentation, malformed-input, and recovery behavior?
4. How will stateful behavior be represented, not only static byte playback?
5. How will tests avoid validating production codec with bytes generated by
   that same codec?

### Dependency and CI result

1. What exact build time, disk, cache, and network cost does each candidate
   add?
2. Does candidate support Rust 1.97.1, repository licenses, locked/offline
   builds, and current Linux CI?
3. What security and maintenance risks exist: unsafe code, native libraries,
   kernel modules, privileged setup, abandoned releases, or install scripts?
4. Which files/jobs/inputs can be deleted after parity, and what still depends
   on NCS for unrelated reasons?

## Candidate Classes to Research

Candidate list is discovery seed, not endorsement. Add candidates found from
official package indexes, source repositories, and current maintenance data.

| Class | Initial candidates/questions |
|---|---|
| direct Rust POSIX API | existing `nix::pty::openpty`; compare `rustix`/`libc` approaches only if they improve ownership, portability, or dependency cost |
| Rust PTY abstraction | investigate maintained PTY crates such as `portable-pty`; verify whether API exposes stable slave path and raw byte semantics needed by `tokio-serial` |
| Rust virtual-serial abstraction | investigate `virtual-serialport` and similar crates; reject if server cannot open a real OS path or if peer fault/lifecycle control is insufficient |
| external native utility | `socat` PTY pairs, a minimal C `openpty` helper, or another unprivileged userspace utility; measure process cleanup, availability, and version pinning |
| scripting runtime | Python standard-library `pty` plus protocol scripts; compare development speed against interpreter/runtime variance and dependency drift |
| kernel/privileged virtual serial | `tty0tty` or equivalent; research only as optional modem-control lane because hosted CI privilege and driver installation are likely constraints |
| full system/MCU emulator | QEMU, Renode, or similar; include only to document whether any required behavior truly needs hardware emulation rather than a serial peer |

For protocol helpers, investigate at minimum:

- SLIP: maintained encoder/decoder crates versus a small RFC 1055 peer;
- COBS: maintained plain-COBS crates and independently published vectors;
- JSON lines/NDJSON: `serde_json` plus independent line/blank-line behavior;
- AT: modem/DCE simulation libraries versus a small scripted AT state machine
  supporting command echo, final result codes, errors, and URCs;
- NMEA-0183: maintained NMEA parser/generator crates, talker/AIS/proprietary
  sentence support, checksum generation, and bad/missing checksum control;
- Modbus ASCII: `rmodbus`, `tokio-modbus`, and smaller alternatives; verify
  ASCII transport, mutable server context, exception responses, and LRC rather
  than assuming RTU/TCP support implies ASCII support;
- shell/raw/generic framing: small independent builders and golden vectors,
  because these are framing contracts rather than full device protocols.

## Candidate Evaluation Method

### Evidence sources

For every serious candidate, record:

- canonical repository and package page;
- current release and release date;
- license;
- supported OSes and Rust/MSRV requirements;
- dependency tree and native-system requirements;
- maintenance activity, open critical issues, and bus factor signals;
- unsafe-code and security-advisory status;
- API evidence for exact required operations;
- benchmark/prototype results from this repository.

Prefer official docs and source over blog summaries. Archive URLs and checked
versions in research report so recommendation can be reproduced.

### Hard rejection criteria

Reject candidate as primary fixture if any applies:

- requires NCS, Zephyr, a board SDK, a privileged kernel module, or a hosted-CI
  driver install;
- cannot expose a real OS path accepted by public `open(port=...)`;
- silently substitutes an in-memory serial backend for parity acceptance;
- cannot transport arbitrary bytes losslessly;
- cannot deterministically stop and release tasks/processes/file descriptors;
- license is incompatible with repository use or redistribution plan;
- cannot be pinned and built reproducibly in supported CI.

Candidate may still serve an optional specialized lane if limitation is named
and primary fixture remains independent.

### Weighted comparison

Score survivors from 0–5 with evidence, then apply weights:

| Criterion | Weight |
|---|---:|
| public-boundary and OS-serial fidelity | 25% |
| deterministic lifecycle/fault control | 20% |
| ability to cover all 49 current tests | 15% |
| protocol-peer extensibility | 15% |
| dependency/build/CI cost | 10% |
| maintenance and API stability | 5% |
| Linux/macOS portability | 5% |
| license/security/unsafe-code risk | 5% |

Rust preference breaks a close result; it does not override materially better
fidelity or reliability from a native alternative.

## Required Proof-of-Concept Experiments

Prototype at least three viable boundary classes when available:

1. direct Rust `openpty` using current `nix` dependency;
2. strongest maintained Rust abstraction found by survey;
3. strongest unprivileged non-Rust/native alternative.

Run same black-box experiment against each:

- create endpoint and obtain slave path;
- open slave only through MCP `open` and production serial connection code;
- round-trip all byte values `0x00..=0xFF` without terminal transformation;
- preserve command assembled across multiple host writes;
- emit response in controlled fragments and delays;
- sustain bounded flood plus explicit backpressure/hold behavior;
- close peer and prove public read observes connection closure;
- restore a valid endpoint and prove real reopen/reconnect behavior;
- cancel/drop fixture during pending read and prove bounded cleanup;
- repeat lifecycle and race cases 100 times with no unexpected failures;
- measure wall time, peak disk, build time, process count, and FD baseline/final
  count.

Prototype output belongs under ignored `target/native-sim-research/` and must
include command, host/kernel, tool versions, candidate versions, and normalized
results. Do not commit generated logs or downloaded tools.

## Protocol Simulation Coverage Contract

Replacement must move beyond current static `sendraw` responses. Each shipped
preset needs a peer with normal, fragmented, stateful, and fault behavior.

| Surface | Minimum simulated behavior |
|---|---|
| `at_command` | command echo option, data response, `OK`, `ERROR`, CME/CMS error, URC before/between/after response, delay, no response |
| `slip` | request/response packet, END/ESC payload bytes, fragmented frame, back-to-back frames, malformed escape, recovery/noise policy |
| `json_lines` | object/array/scalar values, fragmented lines, malformed JSON, empty line, multiple values, evolving sensor state |
| `cobs` | zero-containing payload, empty/max-code blocks, fragmented and back-to-back packets, malformed/truncated block, recovery policy |
| `ndjson` | JSON-lines cases plus blank/whitespace skipping, cadence stream, malformed record followed by valid record |
| `nmea0183` | changing talker state, standard/AIS/proprietary sentence, correct/bad/missing checksum, fragmented/burst cadence, noise |
| `modbus_ascii` | mutable coils/registers, common reads/writes, broadcast/no-response rule where applicable, exception response, correct/bad LRC, fragmented/back-to-back frames |
| generic framing | all line endings, multi-byte delimiter splits, 1/2/4-byte length prefixes and both endian modes, alternate start markers, payload boundary/noise cases |
| shell/raw parser | common/custom prompts split across writes; arbitrary binary passthrough and encoding fallback |

For every row, define:

- peer role and state model;
- independently sourced valid vectors;
- invalid and boundary vectors;
- expected MCP-visible result fields and stop reason;
- TX and RX coverage through `write`, `read`, `transact`, and where relevant
  `capture_boot`;
- fragmentation points, cadence, silence, disconnect, and recovery cases;
- whether helper crate, local spec-derived code, or static vector is oracle.

`docs/development/protocol-matrix.md` is scope source. Add doc-drift test that
fails when a new shipped protocol lacks simulator coverage metadata.

## Step-by-Step Research TODO (historical plan and remaining gates)

### Storage hygiene prerequisite

Native-simulator replacement experimentation previously created one top-level
integration-test target per disposable characterization. Cargo retained those
large debug and incremental artifacts after source tests were deleted. Before
adding another temporary native characterization, follow
[storage-hygiene-plan.md](storage-hygiene-plan.md): use an existing integration
target, keep bulk test artifacts disposable on success, and preserve only
explicitly protected evidence artifacts. This is a development-storage policy,
not permission to remove native_sim/NCS dependencies before parity proof.

### Stage 0 — Freeze scope and evidence

- [ ] Record current `HEAD`, Rust version, host/kernel, NCS version, and exact
      commands used by native suite.
- [ ] Run 43 + 6 native tests once and save normalized pass/failure/duration
      baseline under ignored `target/native-sim-research/`.
- [ ] Generate 49-row traceability inventory from test names and assertions.
- [ ] Map each test to firmware commands/state and public MCP behavior.
- [ ] Mark weak existing claims, especially
      `native_auto_reconnect_preserves_connection`, which reconnects an open
      endpoint rather than proving disappearance/recovery.
- [ ] Inventory every NCS/native_sim reference in Cargo, Nix, CI, xtask,
      scripts, firmware, docs, evaluator completion references, and test
      helpers.
- [ ] Record current CI cost: SDK/cache size, cold/warm setup time, firmware
      build time, native test time, and failure history available from CI.

**Exit:** research report records baseline inventory and no unexplained native
test.

### Stage 1 — Survey virtual-serial candidates

- [x] Search crates.io/docs.rs/source repositories for Rust PTY and virtual
      serial candidates.
- [x] Search official package/source docs for unprivileged native alternatives.
- [x] Complete evidence record and hard-rejection review for each candidate.
- [x] Verify APIs from source; do not infer capabilities from package names.
- [x] Build weighted comparison table with unresolved questions visible.
- [x] Select prototype candidates from at least Rust-direct, Rust-abstraction,
      and non-Rust classes, or document why a class has no viable candidate.

**Exit:** shortlist names strongest candidates but makes no recommendation
without prototypes.

### Stage 2 — Prototype PTY boundary

- [ ] Implement disposable prototypes outside product modules.
- [ ] Run identical proof-of-concept experiment list against each candidate.
- [ ] Test Linux first; compile/run macOS where runner access exists.
- [ ] Measure setup/build/runtime/disk/FD/process cleanup.
- [ ] Capture unsupported semantics explicitly: flush, HUP, path replacement,
      DTR/RTS, BREAK, baud/parity.
- [ ] Run 100-repeat lifecycle/race loop.
- [ ] Re-score candidates with measured evidence.

**Exit:** recommend boundary foundation and runner shape, with rejected options
and tradeoffs. Recommendation must identify smallest public-boundary regression
test: MCP opens candidate slave, exchanges bytes, peer closes, and all fixture
resources terminate.

### Stage 3 — Research emulator core

- [ ] Derive device state machine from firmware behavior and test assertions,
      not from implementation chronology.
- [ ] Compare in-process task, separate Rust binary, and hybrid runner.
- [ ] Prototype deterministic input assembly, command dispatch, output queue,
      delays/cadence, cancellation, and explicit exit state.
- [ ] Prototype bounded fault script with emit/chunk/delay/silence/close/crash/
      malformed/saturate actions.
- [ ] Decide clock strategy and prove timeout tests do not depend on accidental
      scheduler order.
- [ ] Prove arbitrary binary transport and queue/backpressure behavior.
- [ ] Define fixture API and ownership/drop contract.

**Exit:** architecture recommendation plus behavior tests for fixture itself.

### Stage 4 — Research every shipped protocol peer

- [ ] Create one research worksheet per protocol/preset and generic parser.
- [ ] Review official specification/reference named in
      `docs/protocols/references.md`.
- [ ] Survey candidate crates and verify exact transport/mode support from
      source.
- [ ] Select independent oracle strategy and vectors.
- [ ] Prototype one happy path, one fragmented path, one state transition, and
      one malformed/fault path per protocol.
- [ ] Verify checksum generation independently for NMEA and Modbus ASCII.
- [ ] Verify AT URC interleaving and Modbus mutable server state; static replay
      alone does not pass.
- [ ] Fill protocol simulation coverage table and identify gaps.

**Exit:** every shipped protocol has recommended peer implementation, evidence,
and test scenarios. No `TBD` remains for required coverage.

### Stage 5 — Produce replacement recommendation

- [x] Combine boundary, emulator-core, and protocol findings into one report.
- [x] Recommend exact dependencies with versions/features/licenses, or explain
      why local code is preferable.
- [x] Document rejected alternatives and material tradeoffs.
- [x] Draft architecture decision record because complete NCS removal changes
      test architecture and CI dependency model.
- [x] Produce implementation phases sized for review, each with public behavior
      test and rollback point.
- [x] Define temporary differential strategy: same scenario against native and
      replacement until parity passes.
- [x] Define final deletion PR contents and CI simplification.

**Exit:** user approved recommendation before production-quality replacement
implementation started.

### Stage 6 — Parity implementation and migration

- [x] Add selected fixture behind shared test interface.
- [x] Map all 49 scenarios to required replacement or stronger public proof
      without weakening assertions.
- [x] Run native and replacement fixtures against normalized public outcomes.
- [x] Add missing true disconnect/recovery and fixture cleanup tests.
- [x] Add full protocol peers and coverage-drift guard.
- [x] Run timing/flood/hold/flush/close cases 100 times with recorded seed and
      zero unexpected failures.
- [x] Make replacement suite required in CI while native remains temporary
      differential oracle.
- [x] Gather agreed parity window and resolve every mismatch.

**Exit:** Phase E evidence is complete and its replacement targets remain the
required active coverage. Phase F source/configuration removal followed; fresh
clean-checkout no-NCS acceptance is complete in the [canonical Phase F
acceptance record](native-sim-replacement-research-progress.md).

### Stage 7 — Remove native_sim and NCS completely

- [x] Delete `firmware/` and native firmware build helpers.
- [x] Delete `NativeSimFirmware` harness and retain migrated coverage in
      tests to device-fixture/protocol suites.
- [x] Remove NCS/nrfutil/native_sim CI provisioning, cache, disk-reclamation,
      build, and cleanup steps.
- [x] Remove `nix-nrf-dev` input, NCS shell configuration, multilib-only
      requirements, firmware LSP config, and stale lock entries.
- [x] Remove xtask firmware build/path logic and replace commands with normal
      Rust test-asset handling if needed.
- [x] Remove NCS installer scripts/tests if no other repository path uses them.
- [x] Update README, contributor docs, `AGENTS.md`, evaluator completion refs,
      and development indexes; retain changelog history.
- [x] Search active source/config for `native_sim`, `NativeSimFirmware`, `NCS`,
      `nrfutil`, `west build`, `fw-build-native`, and `nix-nrf-dev`; classify
      retained historical documentation rather than deleting history.
- [x] Run complete local and CI gates from clean checkout without NCS installed;
      fresh PR CI evidence is recorded in the [canonical Phase F acceptance
      record](native-sim-replacement-research-progress.md).

**Exit:** clean checkout builds and runs full required suite using Rust/system
dependencies only; no active test, build, dev-shell, or CI path requires NCS.

## Final Acceptance Criteria

Native_sim removal is approved only when all hold:

1. All 49 tests have traceable replacement coverage; no assertion was relaxed
   solely to accommodate new fixture.
2. Every shipped protocol/preset, framing mode, and parser meets protocol
   simulation coverage contract.
3. Public-boundary tests open real OS slave path; in-memory backends remain
   unit/fault tools, not parity substitute.
4. True peer disappearance and valid recovery are tested separately from
   no-op reconnect of open endpoint.
5. Fixture cleanup, cancellation, flood, timing, and close races pass 100
   consecutive runs with zero unexplained failures.
6. Protocol vectors are independent of production codec implementation.
7. Full required suite passes from clean checkout with no NCS, Zephyr, Nordic
   toolchain, `west`, or `nrfutil` present.
8. CI no longer downloads/caches NCS and reports measured disk/time reduction.
9. PTY physical-layer limitations remain documented and covered by controlled
   backend or optional hardware lane where relevant.
10. Full format/build/test/clippy/doc/Nix gates pass.

All ten criteria are satisfied. Current CI gate coverage and the canonical
one-run timing/cache measurement are recorded in
[native-sim-replacement-research-progress.md](native-sim-replacement-research-progress.md).

## Research Deliverables

Research phase produces these reviewable artifacts before implementation:

1. current-test traceability and NCS-coupling inventory;
2. source-linked candidate survey and weighted scorecard;
3. reproducible prototype procedure and normalized results;
4. per-protocol peer/oracle worksheets and coverage matrix;
5. recommendation with exact dependency and architecture choice;
6. proposed ADR and staged implementation/removal plan.

No candidate selection should be treated as settled until these artifacts and
prototype evidence exist.
