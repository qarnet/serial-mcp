# Test/Build Unification Plan — `serial-mcp`

> **Status: Implemented.** All phases 1–5 complete.
> See commits `2362d90` through `36e1bdd` on `fix/reset-firmware-state`.
> This document remains as design rationale.

## Goal

Make repo test and firmware workflows deterministic.

End state:

- all integration tests use one well-defined `serial-mcp` binary source
- all firmware tests use one well-defined firmware build source
- plain `native_sim` and USB `native_sim` builds never share stale state
- local dev, CI, and docs use same commands and same asset-discovery rules
- hidden build pollution and PATH ambiguity removed

---

## Problems to solve

### 1. `serial-mcp` binary provenance fragmented

Current repo mixes multiple strategies:

- in-process library-backed test servers
- direct `cargo build --bin serial-mcp` inside individual test files
- hardcoded `target/debug/serial-mcp`
- PATH-discovered system binary during manual workflows

Risks:

- different test suites validate different server surfaces
- tests may accidentally exercise stale artifacts or host-installed binaries
- binary startup / CLI / transport wiring not consistently covered
- duplicated build logic across test files drifts over time

### 2. firmware build provenance fragmented

Current repo mixes multiple firmware assumptions:

- tests default to `build/firmware/zephyr/zephyr.exe`
- `fw-build-native` reuses existing build tree
- `fw-build-native-usb` uses `--pristine`, but shares build root pattern
- plain and USB variants can contaminate each other through reused outputs

Risks:

- stale Kconfig/devicetree state can make bad builds appear green
- ignored firmware tests can fail for reasons unrelated to server logic
- manual verification can unintentionally use wrong variant

### 3. firmware source currently not variant-safe

`firmware/src/usb_cdc.c` unconditionally references USB CDC devicetree state.

Effect:

- pristine plain `native_sim` build fails
- stale USB-enabled build tree hides bug

### 4. docs and helpers do not enforce one workflow

Current docs/helpers describe useful commands, but not one unified source of truth for:

- which `serial-mcp` binary tests must use
- which firmware image each test class must use
- how freshness is guaranteed

---

## Design principles

1. **Single source of truth for each artifact**
   - one rule for test server binary
   - one rule for plain firmware image
   - one rule for USB firmware image

2. **Deterministic build directories**
   - each firmware variant gets its own build directory
   - no variant reuses another variant's outputs

3. **Separation of test layers**
   - unit tests validate library internals
   - integration tests validate spawned real binary behavior
   - firmware tests validate real server + real firmware interaction

4. **One orchestration entry point**
   - local dev and CI should be able to run same high-level commands

5. **Fresh by default**
   - helpers should prefer current repo outputs over host-installed tools
   - tests should never silently fall back to PATH `serial-mcp`

---

## Target architecture

## A. Binary under test: one discovery rule

All integration tests that need a `serial-mcp` executable should resolve it through one shared helper.

### Proposed helper

Create shared test helper module, for example:

- `tests/common/binaries.rs`

Responsibilities:

- resolve path to `serial-mcp` binary used by all spawned-binary integration tests
- build binary once if needed
- canonicalize path
- fail loudly if artifact missing

### Resolution order

Recommended order:

1. `SERIAL_MCP_TEST_SERVER_BIN`
   - explicit override for advanced workflows / CI / debugging
2. build repo binary once via:
   - `cargo build --locked --bin serial-mcp`
3. return canonical path:
   - `target/debug/serial-mcp`

### Rules

- no integration test should use PATH lookup for `serial-mcp`
- no integration test should hardcode `target/debug/serial-mcp` directly
- no individual test file should run its own ad-hoc build logic

### Expected result

All spawned-binary tests exercise same artifact.

---

## B. Firmware images: one discovery rule per variant

All firmware-oriented tests should resolve firmware binaries through one shared helper.

### Proposed helper

Create shared helper, for example:

- `tests/common/firmware.rs`

Responsibilities:

- resolve plain `native_sim` binary
- resolve USB-enabled `native_sim` binary
- optionally build them when missing or stale
- provide clear paths to callers

### Proposed build directories

Use separate build directories permanently:

- plain firmware: `build/native_sim`
- USB firmware: `build/native_sim_usb`

Resulting binaries:

- plain: `build/native_sim/zephyr/zephyr.exe`
- USB: `build/native_sim_usb/zephyr/zephyr.exe`

### Resolution order

For plain firmware:

1. `SERIAL_MCP_NATIVE_SIM_BIN`
2. default path `build/native_sim/zephyr/zephyr.exe`

For USB firmware:

1. `SERIAL_MCP_NATIVE_SIM_USB_BIN`
2. default path `build/native_sim_usb/zephyr/zephyr.exe`

### Rules

- plain and USB variants must never share build directories
- helpers must never assume `build/firmware/zephyr/zephyr.exe`
- tests should not duplicate path defaults in multiple files

### Expected result

Variant contamination removed.

---

## C. Test-layer model

Adopt explicit repo testing model.

### Layer 1 — unit tests

Scope:

- pure library logic
- internal managers, parsing, schemas, matching, helpers, etc.

Execution:

- `cargo test --lib`

Allowed style:

- direct in-process Rust calls

### Layer 2 — binary integration tests

Scope:

- startup behavior
- CLI parsing
- transport behavior
- stdio transport
- HTTP transport
- resource and tool surfaces

Execution model:

- spawn same built `serial-mcp` binary from helper

Rules:

- no in-process `SerialHandler` for binary integration coverage
- use real process for stdio and HTTP integration

### Layer 3 — firmware integration tests

Scope:

- real spawned `serial-mcp`
- real spawned `native_sim` firmware
- PTY interactions
- USB/IP bootloader-touch path

Execution model:

- binary path from shared server helper
- firmware path from shared firmware helper

### Why this split

This gives clear responsibility boundaries:

- unit failures mean library regression
- integration failures mean process/transport/runtime regression
- firmware failures mean cross-system interaction regression

---

## D. Unify integration tests around spawned real binary

Current repo has HTTP integration tests using in-process server wiring.

### Proposed change

Refactor integration tests so all process-level integration tests use the same spawned binary artifact.

#### `tests/stdio_integration.rs`

Change from:

- file-local `cargo build`
- file-local `target/debug/serial-mcp`

To:

- shared `serial_mcp_bin()` helper

#### `tests/blob_resources.rs`

Same change.

#### `tests/http_integration.rs`

Current behavior:

- spins in-process `SerialHandler` behind axum

Proposed behavior:

- spawn real `serial-mcp --transport=http --bind=127.0.0.1:0`
  or equivalent supported flags
- discover chosen port reliably
- connect real rmcp HTTP client to spawned server

### Keep in-process coverage where appropriate

Do **not** remove all in-process testing.

Keep in-process validation in:

- library tests under `src/`
- narrowly scoped internal tests where transport/process startup is not part of what is being validated

### Expected result

Integration surface validates real shipped server process, not custom in-test assembly.

---

## E. Fix firmware variant safety in source

This is prerequisite work.

### Problem

`firmware/src/usb_cdc.c` assumes USB CDC node always exists.

### Required design

Compile USB support only when USB variant is enabled.

### Recommended implementation pattern

Use Kconfig / devicetree guards.

Possible pattern:

- if USB CDC support enabled and node exists:
  - compile full implementation
- otherwise:
  - compile stub `usb_cdc_init()` returning success/no-op

### Requirements

- plain `native_sim` pristine build succeeds
- USB `native_sim` pristine build succeeds
- app code can always call `usb_cdc_init()` without scattered ifdefs

### Expected result

Variant-specific behavior becomes source-safe instead of build-dir-dependent.

---

## F. Replace shared firmware build path with dedicated helpers

## Proposed helper behavior

### `fw-build-native`

Should build plain firmware into dedicated dir:

```bash
west build -b native_sim firmware/ -d build/native_sim --pristine
```

### `fw-build-native-usb`

Should build USB firmware into dedicated dir:

```bash
west build -b native_sim firmware/ -d build/native_sim_usb --pristine -- \
  -DEXTRA_CONF_FILE=boards/native_sim_usb.conf \
  -DEXTRA_DTC_OVERLAY_FILE=boards/native_sim_usb.overlay
```

### `fw-run-native`

Should run:

- `build/native_sim/zephyr/zephyr.exe`

### `fw-run-native-usb-attached`

Should run:

- `build/native_sim_usb/zephyr/zephyr.exe`

### Additional optional helpers

Optional but useful:

- `fw-path-native`
- `fw-path-native-usb`
- `fw-clean`

These can reduce duplicated path logic in scripts and docs.

### Expected result

Manual flows and automated flows use same artifact paths.

---

## G. Introduce single orchestrator for build/test assets

Repo needs one command family that prepares current artifacts.

## Recommended approach: `xtask`

Create small Rust orchestration crate, e.g.:

- `xtask/`

### Why `xtask`

- structured path/env handling
- easier than duplicating shell logic across docs, CI, and tests
- integrates naturally with cargo workflows
- easier to add subcommands over time

### Proposed commands

#### `cargo xtask build-test-assets`

Builds current required artifacts:

- `serial-mcp` binary
- plain native_sim firmware
- optional USB firmware via flag

Example:

```bash
cargo xtask build-test-assets
cargo xtask build-test-assets --usb
```

#### `cargo xtask test`

Runs default repo tests:

- build `serial-mcp`
- build plain firmware
- run `cargo test --locked`
- run plain ignored native_sim suites

#### `cargo xtask test-all`

Runs everything:

- build `serial-mcp`
- build plain firmware
- build USB firmware
- run normal suite
- run plain ignored firmware suites
- run USB ignored suite if host prerequisites present

#### `cargo xtask print-paths`

Prints canonical resolved paths for:

- server binary
- plain firmware binary
- USB firmware binary

Useful for debugging CI and local setup.

### Environment variables orchestrator should export/pass

- `SERIAL_MCP_TEST_SERVER_BIN`
- `SERIAL_MCP_NATIVE_SIM_BIN`
- `SERIAL_MCP_NATIVE_SIM_USB_BIN`

### Important rule

CI and docs should use orchestrator commands instead of hand-rolled sequences wherever practical.

---

## H. Normalize stale-asset policy

Need explicit repo policy for freshness.

## Recommended policy

### For test server binary

- build current repo binary once before integration suites
- never use PATH fallback

### For firmware images

- dedicated build dirs by variant
- `--pristine` for variant builds from helper/orchestrator
- tests consume explicit path from env/helper

### For ignored tests

Ignored tests should assume assets already prepared by orchestrator or explicit env vars.

This avoids each ignored test trying to rebuild independently.

### Optional advanced freshness check

Later enhancement:

- helper checks mtimes or marker files
- rebuild only if source newer than artifact

Not required for phase 1. Simpler rule first:

- orchestrator builds assets before suites

---

## I. CI alignment plan

CI should converge on same artifact story as local dev.

### Current CI issue

CI currently wires firmware path directly and builds pieces manually.

### Proposed CI state

Replace ad-hoc build/test steps with orchestrator where possible.

Example shape:

1. setup Rust + NCS env
2. `cargo xtask build-test-assets`
3. `cargo test --locked`
4. `cargo test --test native_sim_validation -- --ignored`
5. `cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1`

Or, once stable:

```bash
cargo xtask test
```

USB/IP workflow can stay separate if privilege setup still special.

### Expected result

Local and CI failures reproduce with same commands.

---

## J. Documentation alignment plan

Update docs after implementation.

### Files to update

- `docs/TESTING.md`
- root `AGENTS.md`
- `firmware/AGENTS.md`
- any helper comments referencing old paths

### Documentation changes needed

1. replace old shared path `build/firmware/zephyr/zephyr.exe`
2. document dedicated build dirs
3. document orchestrator commands
4. document env vars for explicit artifact overrides
5. document that spawned-binary tests use repo-built `serial-mcp`, not PATH

### Expected result

Humans and agents stop reintroducing old workflow drift.

---

## K. Suggested implementation phases

## Phase 1 — unblock correctness

Goal: remove current false-green behavior.

Tasks:

1. fix `usb_cdc.c` variant guard / stub path
2. change firmware helper scripts to separate build dirs
3. update tests to default to new firmware paths
4. verify both pristine builds:
   - plain native_sim
   - USB native_sim

Exit criteria:

- `west build -b native_sim firmware/ -d build/native_sim --pristine` passes
- USB variant build in `build/native_sim_usb` passes
- no helper points at shared `build/firmware/...` path

## Phase 2 — unify test artifact discovery

Goal: one `serial-mcp` binary source, one firmware path source.

Tasks:

1. add `tests/common/binaries.rs`
2. add `tests/common/firmware.rs`
3. migrate `stdio_integration.rs`
4. migrate `blob_resources.rs`
5. migrate native_sim tests to shared firmware helper/path constants

Exit criteria:

- no test file shells out to `cargo build` directly
- no test file hardcodes `target/debug/serial-mcp`
- firmware-path defaults live in one place only

## Phase 3 — unify integration style

Goal: integration tests validate real process behavior.

Tasks:

1. convert `http_integration.rs` to spawned binary HTTP mode
2. keep internal in-process coverage only in library/internal tests
3. ensure all process integration tests use shared binary helper

Exit criteria:

- integration suites exercise same real binary artifact
- no process-level integration test uses custom in-process server assembly unless explicitly intentional and documented

## Phase 4 — orchestrator

Goal: one command family for local + CI.

Tasks:

1. add `xtask`
2. implement `build-test-assets`
3. implement `test`
4. implement `test-all`
5. optionally implement `print-paths`

Exit criteria:

- local dev can run one high-level command
- CI can use same command(s)

## Phase 5 — docs + CI convergence

Goal: prevent drift from returning.

Tasks:

1. update docs
2. update AGENTS files
3. update CI commands
4. remove stale comments and examples

Exit criteria:

- docs, helpers, CI, and tests describe same workflow

---

## Validation matrix

After implementation, validate at least this matrix.

### Binary provenance

- prove integration tests use helper-resolved repo binary
- prove PATH `serial-mcp` is ignored unless explicitly passed as override path

### Firmware provenance

- plain firmware path resolves to `build/native_sim/zephyr/zephyr.exe`
- USB firmware path resolves to `build/native_sim_usb/zephyr/zephyr.exe`
- switching variant does not mutate other variant output

### Build correctness

- plain pristine build passes
- USB pristine build passes

### Test correctness

- `cargo test --lib` passes
- `cargo test --locked` passes
- `cargo test --test native_sim_validation -- --ignored` passes
- `cargo test --test native_sim_connection_lifecycle -- --ignored --test-threads=1` passes
- `cargo test --test bootloader_touch_emulated -- --ignored --test-threads=1` passes when host privilege prerequisites satisfied

### Orchestrator correctness

- `cargo xtask build-test-assets` produces current artifacts
- `cargo xtask test` runs default repo validation successfully

---

## Risks and mitigations

### Risk 1 — converting HTTP integration tests may reduce speed

Mitigation:

- keep unit/internal tests in-process
- only process-level integration tests spawn real binary

### Risk 2 — `xtask` adds maintenance surface

Mitigation:

- keep it small and focused on orchestration only
- no business logic duplication

### Risk 3 — USB/IP suite still has host privilege complexity

Mitigation:

- keep USB suite as explicit opt-in path in orchestrator
- separate default `test` from full `test-all`

### Risk 4 — stale external docs/examples may reintroduce old paths

Mitigation:

- repo-wide grep for `build/firmware/zephyr/zephyr.exe`
- repo-wide grep for `target/debug/serial-mcp`
- repo-wide grep for file-local `cargo build --bin serial-mcp`

---

## Concrete acceptance criteria

This plan is complete when all are true:

1. plain `native_sim` pristine build succeeds
2. USB `native_sim` pristine build succeeds
3. plain and USB firmware use separate build dirs
4. spawned-binary integration tests use one shared binary helper
5. no integration test depends on PATH `serial-mcp`
6. no individual test file builds `serial-mcp` ad hoc
7. firmware tests use one shared firmware path helper
8. orchestrator exists for local + CI build/test flows
9. docs and AGENTS files describe new workflow accurately
10. CI uses same artifact-prep logic as local dev wherever practical

---

## Recommendation

Implement in this order:

1. firmware guard fix
2. split firmware build dirs/helpers
3. shared test artifact helpers
4. integration test migration
5. `xtask` orchestration
6. docs/CI convergence

This order removes current false-green behavior first, then prevents future drift.
