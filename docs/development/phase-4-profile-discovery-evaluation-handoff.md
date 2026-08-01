# Phase 4 Handoff — Profile Discovery, Agent Teaching, and Evaluation

## Goal

Make the normal profile-led workflow discoverable from `list_ports`, rewrite
agent guidance as a short decision tree, and add a repeatable local evaluator
that measures real tool-catalog size plus representative call shapes. Use the
results to decide whether recipes, shorthand, a facade, and atomic boot capture
are justified.

Phases 1–3 are complete. Do not change their identity, persistence, revision,
or learning semantics.

## Public behavior

1. `list_ports` returns a parallel profile-match entry for every `PortInfo`.
2. A high-confidence unique winner previews the same profile bare `open` would
   select, without marking it used.
3. Equal-ranked profiles, duplicate live fingerprints, and weak identity are
   explicit; no unsafe candidate becomes an automatic selection.
4. Server instructions, common tool descriptions, README, and prompts teach:
   `list_ports` → bare `open` → `transact`/`read`/`write` → inspect profile and
   persistence → advanced escalation.
5. One local command produces deterministic schema/call-shape metrics without
   network access, telemetry, user profile access, or hardware.
6. Checked-in decision record states whether recipes, shorthand, facade, and
   Phase 5 `capture_boot` meet fixed thresholds.

## Scope

- Flat `ListPortsResult.profile_matches` parallel map.
- Fresh profile-store preview and pure match computation.
- Common-path descriptions and prompt/README drift fixes.
- Public generated tool catalog shared by schema tests and evaluator.
- `xtask agent-eval` with deterministic baseline/report output.
- Executed current call-shape scenarios where no device is needed; modeled
  candidate forms with explicit labels and expansions.
- Decision record and baseline metrics.

## Non-scope

- Do not add recipes, shorthand, facade tools, or `capture_boot` in this phase.
- Do not hide advanced tools or add common/full modes.
- Do not add remote telemetry or run an LLM benchmark.
- Do not claim modeled candidates are implemented behavior.
- Do not implement continuous capture or arbitrary file export changes.
- Do not change `PortInfo`.

## `list_ports` profile match model

Add flat wire types (snake_case enums):

```rust
pub enum ProfileMatchOutcome {
    Selected,
    Ambiguous,
    Ineligible,
    Duplicate,
    None,
}

pub struct ProfileMatchCandidate {
    pub profile_name: String,
    pub generated: bool,
    pub revision: u64,
    pub last_used_at_ms: Option<u64>,
}

pub struct PortProfileMatch {
    pub port: String,
    pub confidence: IdentityConfidence,
    pub outcome: ProfileMatchOutcome,
    pub selected_profile: Option<String>,
    pub candidates: Vec<ProfileMatchCandidate>,
}
```

Annotate unsigned fields with schema helpers. Extend:

```rust
pub struct ListPortsResult {
    pub count: usize,
    pub ports: Vec<PortInfo>,
    pub profile_matches: Vec<PortProfileMatch>,
}
```

`profile_matches` always has same length/order as `ports`, keyed additionally
by exact `port` name. Do not skip an empty vector on serialization; predictable
parallel shape is easier for agents. This is additive; `PortInfo` remains pure
OS enumeration.

## Preview semantics

Add `ProfileStore::list_fresh()` or equivalent using existing `run_read` so one
`list_ports` call sees cross-process disk truth and republishes cache. Do not
perform one lock/reload per port. Provider failure or corrupt profile store is a
tool error rather than silently claiming no matches.

Use pure computation over one `Vec<PortInfo>` + one fresh `Vec<Profile>`:

- High identity duplicated among live ports: `Duplicate`; never selected.
- High identity, zero eligible profiles: `None`.
- High identity, one eligible profile: `Selected`.
- High identity, multiple eligible profiles with unique maximum
  `last_used_at_ms`: `Selected` winner.
- High identity, equal top rank: `Ambiguous`, no selected profile.
- Medium/low/none: never automatically selected. Include non-empty selectors
  that explicitly match as candidates and return `Ineligible`; otherwise
  `None`.

For high eligibility, exactly reuse Phase 3 rules:

- `Profile::matches(port)`
- selector carries matching high identity fields
- `None` last-used sorts oldest
- candidate order is deterministic: newest first, then profile name only for
  display; name must never break a selection tie

Add `ProfileSelector::is_empty()` to prevent empty profile-mode selectors from
appearing as weak candidates for every port.

`list_ports` is preview only: no `mark_used`, revision, or file mutation.

Update `serial://ports` resource to include same match map. Pass shared store to
tool/resource path.

## Behavior-first discovery tests

Use real PTY paths plus injected `StaticPortProvider` and public MCP calls:

- empty store: one `None` entry per port
- generated/saved high profile: `Selected` with matching revision/name
- unique last-used winner matches later bare `open` selection
- equal timestamps: `Ambiguous`, both candidates, no selected profile
- duplicate live high fingerprint: `Duplicate` for both
- medium VID/PID profile: `Ineligible`, candidate visible
- empty selector excluded from weak candidates
- delete profile changes preview back to `None`
- second process/store writes a profile; `list_ports` fresh read sees it
- `ports` elements serialize identically regardless of match metadata
- output validates against generated schema and uint-format guard

## Agent decision-tree teaching

Rewrite server instructions concisely:

1. call `list_ports`; inspect `profile_matches`
2. normally call bare `open` with port only (115200/8-N-1 fallback,
   profile auto-selection/generation is observable)
3. use `transact` for request/response, `read` for buffered/unsolicited data,
   `write` for send-only, `subscribe` only for ongoing notifications
4. inspect `profile`, `profile_persistence`, or `get_status` after durable
   changes
5. use `open_profile` only for explicit choice/weak identity;
   `rollback_profile` for retained configuration recovery
6. escalate to framing/parser/cursor/reconnect/line-control/log tools only when
   common path needs them

Shorten/retarget descriptions for at least:

- `list_ports`
- `open`
- `transact`
- `read`
- `write`
- `subscribe`
- `get_status`
- `reconfigure`
- `list_profiles`
- `open_profile`
- `configure`
- `rollback_profile`

Input schemas already explain detailed fields; descriptions should guide tool
choice, not duplicate every nested schema. Preserve critical tagged `ReadFrom`
examples in schema docs even if removed from top-level tool description.

Update:

- README common flow and profile section
- `diagnose_port` prompt: bare open/profile result, `transact` for probes,
  explicit rollback guidance after bad learned settings
- `interactive_terminal` prompt: prefer `transact` for bounded command/response
- `docs/agent-config.md` broken README anchor
- shipped reconnect-profile item in `docs/development/FEATURES.md`
- stale tool-count references, including generated/symlinked contributor docs
- drift tests to assert positive current guidance and absence of `wait_for`/
  removed per-call fields

Do not remove advanced protocol examples or tagged wire-form regression guards.

## Public tool catalog

Eliminate duplicated 26-tool enumeration in tests by exposing one catalog from
`SerialHandler`/server code:

```rust
pub fn tool_catalog() -> Vec<rmcp::model::Tool>
```

It must use the same generated tool attributes served by MCP. Schema tests and
evaluator consume this catalog. Keep exact-count test as guard.

## Local evaluator

Add:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval
```

Allowed optional flags:

```text
--output-dir <path>
--baseline <path>
--write-baseline <path>
```

Default output under `target/agent-interface-eval/`:

- `report.json`
- `report.md`

No timestamps, absolute temp paths, hostnames, payload captures, network, or
user config in deterministic baseline.

`xtask` may depend on root `serial-mcp` library and serde/serde_json. Keep
evaluation code in focused xtask modules with unit tests. Also run:

```bash
cargo test --manifest-path xtask/Cargo.toml
```

### Real catalog metrics

Serialize compact JSON equivalent of actual tools/list result:

```json
{"tools":[...]}
```

Record:

- tool count
- aggregate compact payload bytes
- each tool's compact bytes
- input-schema, output-schema, and description bytes separately where feasible
- top largest tools

Define byte metric exactly; do not count HTTP/SSE headers or pretty-print
whitespace.

### Scenario metrics

Use fixed normalized placeholders (`/dev/ttyACM0`, fixed UUID). For each plan,
serialize compact fixed-ID MCP `tools/call` envelopes and record:

- tool calls
- normalized request bytes
- invalid calls
- retries/fallbacks
- advanced-field occurrences
- stale-data/race risk flags
- completion supported by existing public behavior test reference

At minimum include:

- first console discovery/open
- returning known console
- explicit profile management equivalent
- command/response (`transact` versus write+read)
- line capture
- AT modem
- NDJSON stream
- rollback recovery
- boot-reset prompt capture (current multi-call composition)
- permission/busy/disconnected errors

Current implemented variants use current tool names/shapes. Hypothetical
shorthand/recipe/facade/capture variants must be marked `modeled` and include
their expansion into current calls. Static harness cannot measure model
misunderstanding; state this limitation.

### Fixed decision thresholds

Use these before looking at results:

- automatic profiles justified if returning-device flow saves at least one
  call and 20% request bytes versus explicit management, with same completion
  and no identity regression
- shorthand justified only if >=20% request-byte reduction in at least three
  scenarios and projected catalog growth <=3%
- recipe justified only if >=20% reduction or one repeated advanced object
  removed in at least three scenarios, no extra calls, growth <=2%
- facade justified only if common-task median saves >=1 call and >=30% request
  bytes, 100% modeled completion, catalog growth <=10%
- `capture_boot` justified if it removes arm/reset race or stale-data window and
  reduces composition calls, even if a general facade is not justified
- catalog regression warning: aggregate >=5%; per-tool >=10% or 2 KiB

## Checked-in outputs and decision record

Run evaluator and commit deterministic baseline:

- `docs/development/agent-interface-baseline.json`
- `docs/development/phase-4-agent-interface-evaluation.md`

Decision record must report actual metrics, limitations, dominant friction
(schema size, call shape, setup, orchestration, documentation, or no dominant
source), and explicit yes/no decisions for:

- shorthand now
- initial recipes now
- versioned facade now
- Phase 5 atomic `capture_boot`

Expected conservative result: profiles/transact remain preferred; no recipe or
facade without threshold evidence. `capture_boot` may be justified by race
elimination, not merely schema bytes.

## Expected files

- `src/tools/types.rs`
- `src/tools/port_ops.rs`
- `src/profile_store.rs`
- `src/server.rs`
- `src/tools/mod.rs`
- `src/prompts/diagnose.rs`
- `src/prompts/interactive.rs`
- `tests/serial_pty.rs`
- `tests/http_integration.rs`
- `tests/doc_drift.rs`
- `xtask/Cargo.toml`, `xtask/src/*`
- `README.md`, `AGENTS.md`, `docs/agent-config.md`,
  `docs/development/FEATURES.md`
- baseline and decision record
- this handoff

## Verification

```bash
cargo test --manifest-path xtask/Cargo.toml
cargo run --manifest-path xtask/Cargo.toml -- agent-eval
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
git diff --check
```

Verify rerunning evaluator yields byte-identical baseline/report metrics except
for explicitly excluded presentation paths.

## Commit requirements

Inspect status/diff/log, stage only Phase 4 files, commit conventional message,
return metrics/decisions/tests/hash/deviations. Do not amend, push, merge, open
PR, add attribution, or implement Phase 5/6.
