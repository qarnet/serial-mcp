# Agent-Facing Interface Simplification Plan

## Goal

Make common serial workflows obvious, short, and reliable for agents without
removing the advanced framing, replay, reconnect, and diagnostic capabilities
that distinguish `serial-mcp`.

Success is not measured by how often agents use advanced options. Most tasks
should only need port discovery, open, read/write or transact, and close. The
useful measures are fewer tool calls, fewer invalid calls, less stale data,
smaller schemas, and successful recovery when a device behaves unexpectedly.

## Current state and evidence

- The server exposes 25 tools. Common operations and advanced controls appear
  together in one `tools/list` response.
- `OpenArgs` has 17 fields, `TransactArgs` has 11, `ReadResult` has 24, and
  `GetStatusResult` has 26. Most fields are optional or defaulted, but agents
  still see their complete schemas.
- Advanced framing and parser overrides are repeated across `open`, `read`,
  `write`, `subscribe`, and `transact`.
- The connection-default precedence system already allows simple calls after
  setup: explicit call field > call protocol > connection field > connection
  protocol.
- Saved profiles already bind stable device identity to serial and protocol
  defaults. The missing piece is discovery: the normal README and diagnosis
  flow do not teach `open` -> `save_profile` -> `open_profile`.
- `transact` already implements the common command/response workflow, and
  `read(match=...)` already implements pattern waiting. Adding aliases without
  changing tool exposure would increase schema cost and duplicate concepts.
- `read(from="now")` provides an atomic live-edge seek at the start of a read,
  but reset-and-capture still requires multiple calls. `get_status` exposes
  `rx_end_offset` as a manual bookmark.
- `subscribe` is capable, but MCP notification consumption varies by client.
  Synchronous `read` and `transact` are the safer default for agents.
- Current agent-visible drift includes removed `wait_for` references and a
  `diagnose_port` prompt that still supplies removed per-call
  `max_buffered_bytes`.
- README and tool prose describe simple `from: "now"`/`"cursor"` strings, but
  the current `ReadFrom` wire format requires tagged objects such as
  `{"type":"now"}`. Tests use the tagged form. This is a direct source of
  invalid agent calls unless string shorthand is deliberately implemented.
- `flush(target="both")` currently clears OS buffers but, unlike
  `flush(target="input")`, does not clear the RX ring and shared cursor. This
  contradicts its tool description and can preserve stale data.
- The historical `list_ports` schema interoperability problem was fixed by
  commit `d04bac9`; current uint-format schema guards pass. A report from a
  current client still needs its exact error and `tools/list` payload before a
  new fix is justified.
- Profile persistence exists at the OS user-config path (normally
  `~/.config/serial-mcp/profiles.toml`), but production startup currently uses
  `SerialHandler::builder().build()`, whose profile list starts empty.
  `SerialHandler::new()` loads the file but is not used by `main.rs`. HTTP also
  constructs a separate in-memory profile list per MCP session. Profile state
  therefore needs a shared process-level store before it can be a reliable
  learned-session feature.

## Design principles

1. **Optimize successful common paths, not advanced-feature adoption.** Dormant
   advanced capability is acceptable when ordinary work succeeds quickly.
2. **Teach existing composition before adding aliases.** `transact` is already
   `command`; `read(match=...)` is already pattern waiting.
3. **Learn device configuration automatically and visibly.** Every open
   connection should have a profile session. High-confidence profile matches
   may load automatically, successful durable setting changes should persist,
   and every result should identify which profile was selected or updated.
4. **Separate device identity, wire protocol, and operation intent.** Saved
   profiles identify a device; protocol presets define framing/parsing;
   operations define what to do now. Do not merge these into one ambiguous
   preset concept.
5. **Prefer bounded synchronous operations for agents.** Use notifications only
   for genuinely continuous monitoring.
6. **Do not hide race conditions behind convenience names.** A high-level boot
   capture must coordinate mark, reset, and capture inside one server operation.
7. **Measure before breaking compatibility.** Capture schema size, call count,
   invalid-call rate, and task success before redesigning core tool shapes.
8. **Keep advanced escape hatches.** Simplification must not remove absolute
   offsets, explicit framing, parsers, raw encodings, or reconnect controls.
9. **Test behavior, not implementation shape.** Acceptance tests should drive
   public MCP operations across realistic lifecycle boundaries and assert
   externally observable results. Internal field values, constructor wiring,
   helper-call counts, and private data-structure layout are not substitutes
   for proving persistence, selection, isolation, recovery, and failure paths.

## Interface model

Use four layers rather than a growing flat set of convenience aliases.

### Layer 1: common operations

The common path remains:

1. `list_ports`
2. `open` or `open_profile`
3. `read`, `write`, or `transact`
4. `close`

Common calls should require few fields and return concise primary results.
Advanced options remain optional and should be described as overrides, not as
the expected starting point.

### Layer 2: learned device configuration

Profiles provide persistent identity and defaults:

1. `open` resolves an explicit profile when supplied.
2. Otherwise it selects the most recently used high-confidence profile for the
   same physical device.
3. With no safe match, it starts a generated profile session from a recipe or
   ordinary defaults.
4. Durable session changes update the bound profile automatically.
5. Later opens reuse the last profile used on that device unless explicitly
   overridden.

Automatic behavior must remain observable. `open`, `get_status`, mutation
results, and `list_ports` should expose active profile name, selection source,
match confidence, dirty/persisted state, and candidate ambiguity. Weak or
ambiguous identity must not silently select settings from another physical
device.

### Layer 3: generic connection recipes

Connection recipes provide identity-free starting configurations for common
device classes such as a 115200 line console or an AT-command modem. A recipe
may supply serial defaults, an existing protocol preset, and bounded operation
defaults. Explicit call fields continue to override recipe values.

Recipes are not saved profiles and must not contain a device selector. They are
also not new wire-protocol definitions: framing and parser behavior should
continue to come from the existing protocol presets.

### Layer 4: advanced controls

Explicit framing, parser, ring-offset, subscription, reconnect, line-control,
and diagnostic tools remain available for uncommon tasks. Documentation and
tool descriptions should identify them as escalation paths.

## Ideas to pursue

### 1. Make profiles discoverable from port discovery

Extend each listed port with zero or more matching profile names, or return a
parallel match map in `ListPortsResult`. A normal `list_ports` call then tells
the agent both what exists and whether the server already knows how to open it.

Recommended behavior:

- No match: `open` starts a generated profile session using a recipe or normal
  defaults and records the strongest available device identity.
- One high-confidence match: `open` applies it automatically and reports the
  selected profile.
- Multiple profiles for one high-confidence device identity: apply the most
  recently used profile unless caller names another one.
- Weak or ambiguous device identity: do not apply another device's settings.
  Start a new session profile and return candidate information so the agent can
  merge or select explicitly.
- VID/PID alone is not high-confidence identity when multiple identical devices
  may exist. Prefer serial number plus VID/PID/interface; define and test the
  complete confidence rules before implementation.

Likely files:

- `src/tools/port_ops.rs`
- `src/tools/types.rs`
- `src/profiles.rs`
- `src/server.rs`
- `tests/http_integration.rs`
- profile unit tests in `src/profiles.rs`

Verification:

- no profile, unique match, multiple matches, and disconnected-profile cases
- generated `list_ports` output validates against its output schema
- existing raw `PortInfo` identity remains unchanged unless a deliberate wire
  format change is accepted

Design decision before implementation: prefer a parallel `profile_matches`
field on `ListPortsResult` over putting configuration knowledge into the
OS-level `PortInfo` type. This keeps `PortInfo` a pure device-enumeration model.

### 2. Build a shared learned-profile store

Replace the handler-local `Vec<Profile>` plus path with a process-level
`ProfileStore` shared by every stdio/HTTP handler and connection. It should own:

- startup loading and schema-version migration
- profile lookup and confidence-ranked device matching
- active profile-session bindings by connection ID
- revisioned read-modify-write updates
- atomic file replacement and multi-process file locking
- last-used and update metadata
- generated profile naming
- optional bounded revision history for rollback

The current atomic temp-file rename prevents torn files but does not prevent
lost updates when two writers load the same old file and then replace it.
Learned write-through profiles require one in-process mutation lock plus an
advisory lock for multiple server processes.

Likely files:

- `src/profiles.rs`, likely split into model and store modules
- `src/server.rs`
- `src/main.rs`
- `src/serial.rs` or a profile-session registry kept above the serial layer
- profile and HTTP multi-session integration tests

Production startup must load persisted profiles through the same builder path
used by `main.rs`. HTTP handlers must share one `Arc<ProfileStore>` rather than
constructing separate empty profile vectors.

Storage policy:

- Keep OS user config as the default because device knowledge should follow the
  user across repositories.
- Add `--profiles-path <path>` for deliberate project-specific or portable
  stores.
- Do not silently use the current working directory when the OS config
  directory is unavailable. Fail clearly or require an explicit path.
- Consider a project-local overlay later, but do not automatically write USB
  identity into a repository dotfile that may be committed.

### 3. Attach a profile session to every open connection

An active profile session should record:

- selected profile ID/name
- selection source: explicit, automatic match, or generated
- match confidence and matched identity fields
- base revision and current revision
- dirty/persisted state and last persistence error
- settings changed during this connection

Durable mutations update the profile after the hardware/session mutation
succeeds:

- `reconfigure`: baud, data bits, stop bits, parity, flow control
- connection-mode `configure`: framing/parser/protocol, reconnect, read and
  subscription defaults
- other future connection-default setters

Transient operations must not change the learned profile:

- DTR/RTS pulses and BREAK
- read cursor movement, flush, and subscriptions
- per-call encoding, match, framing, parser, or protocol overrides
- individual write/transact payloads

Snapshot again on clean close as a safety net. Each mutation result should say
whether the profile update persisted. If hardware mutation succeeds but disk
persistence fails, report partial success explicitly rather than pretending the
hardware change was rolled back.

Immediate persistence can capture an experimental wrong baud because serial
ports open and reconfigure successfully even when device communication is
garbled. Keep bounded profile revisions from the first release of automatic
learning. A later “last known good” promotion policy may use a successful
expected-pattern match or valid parsed frame, but should not be inferred from
mere receipt of arbitrary bytes.

### 4. Teach the automatic “learn once” workflow

Update normal documentation and prompts so agents know that opening creates or
selects a profile session, durable changes are learned, and explicit profile
selection overrides the automatic last-used choice. Keep `save_profile` as a
manual naming/snapshot/clone operation rather than the only persistence path.

Likely files:

- `README.md`
- `src/prompts/diagnose.rs`
- `src/server.rs`
- `docs/agent-config.md`
- `docs/protocols.md`

Verification:

- prompt tests assert current tool names and argument fields
- documentation drift tests cover removed tool names and removed per-call
  fields
- example flow demonstrates automatic generated profile, learned reconfigure,
  close, and later automatic reuse

### 5. Default the ordinary serial case

Consider making `open.baud_rate` default to 115200. Data bits, stop bits,
parity, and flow control already default to 8-N-1 and none. This would make the
minimum raw open call `open(port=...)` without adding an `open_115200` alias.

Likely files:

- `src/tools/types.rs`
- `src/tools/port_ops.rs`
- schema and integration tests
- README examples

Verification:

- omitted baud resolves to 115200
- explicit baud continues to win
- generated input schema no longer requires `baud_rate`

This is additive at the JSON-call level but changes behavior for calls that are
currently rejected. Record it in the changelog.

### 6. Define a narrow connection-recipe contract

Keep three concepts distinct:

- saved profile = which physical device and its known configuration
- connection recipe = generic starting behavior for an unknown device class
- protocol preset = framing, parser, and checksum behavior

Potential initial recipes:

- `console`: 115200/8-N-1, no flow control, line-oriented receive defaults
- `raw`: 115200/8-N-1 with no framing or parser
- `at_modem`: existing `at_command` protocol preset plus conservative command
  timeouts
- `ndjson_stream`: existing `ndjson` protocol preset

Do not encode a fixed baud rate into recipes for protocols that commonly vary
unless the baud remains directly overridable. Do not duplicate every existing
protocol preset as a recipe; add a recipe only when it contributes serial or
operation defaults beyond framing/parsing.

Possible call shape:

```text
open(port="/dev/ttyACM0", recipe={"type":"console"})
```

Likely files:

- `src/tools/types.rs`
- `src/tools/port_ops.rs`
- a focused recipe module rather than more branching in `port_ops`
- `src/precedence.rs` or a new open-configuration resolver
- `src/profiles.rs` if saved profiles may reference recipes
- schema, precedence, HTTP, and native_sim tests

Precedence must be designed explicitly before implementation. A likely order
is explicit open field > saved profile field > selected recipe field > built-in
default. Protocol expansion remains a separate field-level resolution step.

Verification:

- every recipe expands to documented concrete settings
- explicit fields override recipe values independently
- saved-profile behavior remains deterministic if recipes are referenced
- recipe schemas remain substantially smaller than spelling out their expanded
  settings

### 7. Add one narrow boot-capture operation

Do not add a general scripting engine. If acceptance workflows justify a new
tool, add one bounded `capture_boot` operation that performs unique server-side
coordination:

1. snapshot the RX live edge
2. optionally perform a configured DTR/RTS reset pulse
3. capture only bytes after the snapshot
4. stop on pattern, silence, timeout, or size cap
5. return concise data plus stop reason and offsets

This addresses a real race that aliases around `read` cannot solve. An
external-reset mode may arm capture without toggling lines, but its UX must be
tested with real MCP clients because the call remains pending while the user or
external system resets the device.

Likely files:

- `src/server.rs`
- `src/tools/types.rs`
- a focused tool module or shared composition in `src/tools/io_ops.rs`
- `src/rx_session.rs`
- `src/tools/control_ops.rs`
- `tests/http_integration.rs`
- `tests/native_sim_validation/unix.rs`

Verification:

- stale pre-mark data never appears
- bytes emitted immediately after reset are retained
- pattern, silence, timeout, cancellation, ring wrap, and disconnect behavior
- no interference with shared read cursor unless explicitly documented

### 8. Make tool descriptions act as a decision tree

Shorten descriptions while making selection clearer:

- `read`: use for buffered data or waiting for unsolicited data/patterns
- `transact`: use for command/response; prefer over separate write/read
- `subscribe`: use only for ongoing live notifications
- `get_status`: use for diagnostics and offsets, not routine reads
- framing/parser fields: omit unless profile/protocol defaults do not fit

Avoid restating complete nested schema semantics in every tool description.
Keep detailed behavior in `docs/protocols.md` and Rust field documentation.

Likely files:

- `src/server.rs`
- `README.md`
- `docs/protocols.md`
- tool schema tests

Verification:

- snapshot serialized `tools/list` byte size before and after
- retain required safety and destructive-operation descriptions
- run representative agent tasks against both descriptions

### 9. Build an agent ergonomics evaluation suite

Before large API changes, create a small repeatable task set using loopback and
native_sim:

- discover and open a 115200 console
- wait for a boot prompt without stale output
- issue a command and capture its response
- reconnect after simulated disconnect
- use a saved profile on a returning device
- capture line, NDJSON, AT, SLIP, and COBS traffic
- diagnose permission, busy-port, and unplug errors

Record:

- tool calls per task
- invalid tool calls
- retries and fallback use
- stale-data failures
- task completion
- `tools/list` serialized bytes/tokens
- whether advanced fields were needed

Likely files:

- new tests under `tests/` or a non-CI evaluation harness under `xtask/`
- `docs/development/` evaluation notes

Do not add remote telemetry. If local usage statistics are later implemented,
keep them opt-in and local as already described in `FEATURES.md`.

### 10. Keep all capabilities visible

A startup common/full toolset mode is rejected. Users who perform a quick
default installation should still discover the advanced feature suite. Hiding
tools would reduce schema context, but it could also leave users unaware that
framing, replay, reconnect, profiles, line control, and diagnostics exist.

Simplify through call shapes, descriptions, profile discovery, and recipes
while keeping advanced tools listed. If aggregate schema size remains a proven
problem, prefer a versioned facade or future MCP discovery mechanism that does
not make capabilities installation-dependent.

### 11. Explore a versioned simple facade without hiding advanced tools

If evidence shows schemas themselves remain the main problem, a future major
version could make the familiar names concise and move advanced shapes behind
explicit tools:

- concise `open`, advanced `open_configured`
- concise `read`, advanced `read_framed`
- concise `command`, advanced `transact`

Before adding aliases, evaluate shorthand forms inside existing tools:

```json
{
  "from": "now",
  "match": "OK>",
  "protocol": "ndjson",
  "recipe": "console"
}
```

Advanced object forms would remain available for absolute offsets,
regex/glob/context matching, parser validation, and recipe overrides. This may
produce `oneOf` branches in schemas, so compare generated schema size and agent
success against the current tagged-object-only representation.

This is more likely to help models than nesting all advanced fields under an
`advanced` object, because nested fields still occupy `tools/list` context.
Advanced tools must remain visible in the same default installation. Aliases
and duplicate concepts are not justified before an evaluation shows a clear
gain, but a facade is preferred over hiding capabilities when simplification
does justify a larger API change.

## Ideas not recommended now

### A large catalog of default device profiles

Built-in profiles are a poor fit because profiles combine device selectors
with local identity. VID/PID alone is often insufficient, baud rates vary, and
serial numbers are machine/device specific. Shipping examples is useful;
silently matching generic built-ins is risky.

Protocol presets already cover reusable wire formats. Add a distinct concept
such as a “connection recipe” only if repeated evaluation shows that protocol
presets plus 115200/8-N-1 defaults remain insufficient.

### Five new convenience aliases

`open_115200`, `capture_lines`, `command`, `wait_for_pattern`, and
`capture_boot` would add five more schemas. Most duplicate existing behavior.
Default `open`, teach `transact`, teach `read(match=...)`, and reserve a new
tool only for atomic boot-reset capture.

### Common/full tool exposure modes

Do not make advanced capability depend on a startup option. This creates two
different discoverability experiences and makes the quickest installation the
least informative one.

### Opaque or irreversible profile learning

Automatic profile selection and persistence are product goals. What remains
unacceptable is invisible selection, weak-identity cross-device matching,
last-writer-wins corruption, or overwriting the only known configuration with
an experimental value. Surface every binding and persistence result, allow
explicit override/disable, and keep bounded revision history.

### A generic expect/script language

This creates a second automation runtime with large schemas, cancellation and
safety concerns, and extensive testing cost. Keep operations bounded and
purpose-specific unless concrete tasks cannot be solved through `transact` and
boot capture.

### Compact/full fields added to every existing tool immediately

An `output_detail` field adds another decision to common calls, and omitting
currently required response fields changes schema compatibility. Revisit this
for a versioned API after measuring actual response-context cost.

## Behavior-first testing standard

Profile work must be verified primarily through public tool behavior. Tests
should cross the boundaries where real failures occur: MCP requests, multiple
clients, connection close/reopen, process restart, persisted files, device
identity changes, concurrent writes, and I/O failures.

### Required happy paths

- Start server with an isolated profile path, create or learn a profile, stop
  the server, start a fresh server process, and prove the profile is loaded.
- Open a device for the first time, observe a generated profile session,
  reconfigure it, close, reopen with bare `open`, and prove the learned settings
  are applied to the live connection.
- Use one HTTP MCP client to update a profile and a second client to discover
  and use that update without reconnecting the server.
- Maintain two named profiles for one uniquely identified device, use each
  explicitly, then prove bare open selects the most recently used one.
- Apply framing/protocol connection defaults, restart, reopen, and prove actual
  framed or parsed traffic behavior rather than only inspecting stored fields.
- Roll back a learned revision and prove subsequent open and communication use
  the restored settings.

### Required unhappy and edge paths

- Two devices with the same VID/PID but no unique serial identity must not
  silently inherit each other's learned settings.
- Multiple equally ranked profile candidates must produce observable ambiguity
  rather than nondeterministic selection.
- A corrupt profile file must not destroy the last valid revision or lead to
  silent default application.
- Concurrent profile changes from separate HTTP sessions must preserve both
  updates or report a conflict; no lost-update last-writer-wins behavior.
- A profile persistence failure after successful hardware reconfiguration must
  report applied-but-not-persisted state accurately.
- A hardware reconfiguration failure must not update the persisted profile.
- Per-call framing, parser, encoding, match, cursor, DTR/RTS, and BREAK changes
  must not alter learned defaults.
- Unplug/replug and changed OS port paths must retain identity only when the
  stable selector still matches.
- Profile schema migration must preserve prior settings and remain restartable;
  unsupported future versions must fail clearly rather than being rewritten.

### Appropriate lower-level tests

Focused unit and property tests remain useful for pure behavior with clear
inputs and outputs, such as identity confidence ranking, selector matching,
profile merge precedence, revision conflict detection, TOML migration, and
generated-name normalization. These tests should avoid asserting private field
layout or duplicating implementation branches line by line.

### Tests that do not count as feature acceptance

- checking that a constructor copied a default constant into a field
- checking that two builders contain equal private values
- checking that an `Arc` points to a particular allocation
- checking only that a tool returned success without observing its effect
- checking only serialized profile contents when live reopen behavior is the
  feature under test
- mocking the profile store so completely that disk, restart, locking, or
  multi-client behavior is bypassed

Every phase handoff should state the user-observable behavior being proved,
then name the smallest public-boundary test that would fail before the fix and
pass afterward.

## Phased plan

### Phase 1 — Correctness and agent-visible drift

Scope:

- Make `flush(target="both")` clear the RX ring and cursor like input flush.
- Remove stale `wait_for` references from agent-visible descriptions.
- Remove stale per-call `max_buffered_bytes` from `diagnose_port`.
- Make all `from` examples use the actual tagged-object wire format until and
  unless string shorthand support is implemented.
- Add prompt/document drift guards for current tool names and call fields.
- Reproduce `list_ports` only if the reporting client's exact schema error is
  available; retain current schema guards regardless.

Files:

- `src/tools/io_ops.rs`
- `src/server.rs`
- `src/prompts/types.rs`
- `src/prompts/diagnose.rs`
- relevant internal comments
- `tests/http_integration.rs`
- `tests/native_sim_validation/unix.rs`
- `tests/doc_drift.rs`

Acceptance:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

Also verify a semantic test proves `flush(target="both")` removes retained RX
backlog, not merely that the call succeeds.

### Phase 2 — Learned-profile storage foundation

Scope:

- Replace handler-local profile vectors with one shared `ProfileStore`.
- Load persisted profiles on the production builder path used by stdio and
  HTTP.
- Share one store across all HTTP MCP sessions.
- Add schema version, revision metadata, last-used metadata, and generated vs
  user-named profile metadata.
- Serialize in-process mutations and add multi-process file locking.
- Add `--profiles-path`; retain OS user config as default and remove silent cwd
  fallback.
- Add bounded profile revision history or equivalent rollback support before
  automatic write-through learning.

Files:

- `src/profiles.rs` and likely new profile-store module(s)
- `src/server.rs`
- `src/main.rs`
- `tests/http_integration.rs`
- `tests/stdio_integration.rs`
- profile unit tests

Acceptance:

- production stdio restart reloads a persisted profile
- separate HTTP MCP sessions observe the same profile updates
- concurrent updates do not lose unrelated profiles
- failed/torn writes preserve the previous valid file
- explicit custom path works and missing OS config path fails clearly
- full Rust gate from Phase 1

### Phase 3 — Automatic profile sessions and learning

Scope:

- Add confidence-ranked device fingerprints and last-used profile selection.
- Bind every open connection to an explicit, automatically selected, or
  generated profile session.
- Apply profile defaults on bare `open` before recipes/built-in defaults;
  explicit open overrides win.
- Persist successful durable changes from `reconfigure` and connection-mode
  `configure` through the bound profile session.
- Snapshot on close as a safety net.
- Add active-profile metadata to open, status, mutation, port-discovery, and
  profile-list results.
- Keep transient line control, per-call framing, cursors, and payloads out of
  learned defaults.
- Retain `save_profile` for explicit naming, cloning, and snapshots.

Files:

- profile store/model modules
- `src/tools/port_ops.rs`
- `src/tools/types.rs`
- `src/server.rs`
- `src/serial.rs` or a separate connection-to-profile-session registry
- `README.md`
- `src/prompts/diagnose.rs`
- HTTP, PTY, schema, restart, and concurrency tests

Acceptance:

- first open creates a generated profile session
- later bare open of the same high-confidence device reuses its most recently
  used profile
- explicit profile or explicit fields override automatic choice
- identical VID/PID devices without stable unique identity do not inherit each
  other's settings
- baud and connection-default changes persist and survive process restart
- persistence failure is reported as partial success without misrepresenting
  hardware state
- prior profile revision can restore an accidentally learned setting
- per-call read/write overrides never mutate the profile
- full Rust gate plus relevant native_sim/PTY tests

### Phase 4 — Profile-led simple setup and ergonomics measurement

Scope:

- Add profile-match information to `list_ports` without contaminating the
  OS-level `PortInfo` model.
- Teach automatic selection, generated sessions, learned mutations, explicit
  override, and rollback.
- Consider defaulting omitted baud to 115200 for devices with no profile.
- Specify connection-recipe boundaries and precedence.
- Rewrite server instructions and common tool descriptions as a decision tree.
- Add representative task scenarios and record baseline metrics.
- Snapshot per-tool and aggregate `tools/list` sizes.
- Compare automatic profile sessions against explicit profile management.
- Compare concise facade calls and selected connection recipes against direct
  advanced calls.
- Decide whether a versioned facade and initial recipes are justified.

Files:

- `xtask/` or a focused evaluation harness under `tests/`
- `docs/development/` results
- `README.md`, prompts, protocol/profile documentation, and drift tests

Acceptance:

- repeatable local command produces comparable task metrics
- no remote telemetry or user data collection
- decision record states whether schema size, call shape, documentation,
  initial setup, or orchestration is the dominant failure source

### Phase 5 — Atomic boot capture, if evaluation supports it

Scope:

- Add bounded `capture_boot` composition.
- Reuse existing ring, matcher, stop controller, and line-control logic.
- Keep initial result in memory; no arbitrary file path.

Acceptance:

- native_sim proves no stale pre-mark data and no missed immediate boot bytes
- cancellation, disconnect, timeout, pattern, and silence behavior match
  existing RX stop vocabulary
- full Rust gate plus native_sim suites

### Phase 6 — Safe persistent capture, separately designed

Before any continuous capture-to-file feature, replace unrestricted path writes
with a configured export directory, containment checks, symlink policy, file
size quotas, and lifecycle controls. Apply the same policy to `export_log` and
future raw capture.

This is a security and resource-management design, not part of basic interface
simplification.

## Explicit non-scope

- No removal of advanced framing or parser support.
- No weak-identity cross-device profile application.
- No invisible profile selection or persistence result.
- No automatic persistence of transient line-control state, read cursors,
  payloads, or per-call overrides.
- No generic scripting language.
- No protocol expansion solely for interface simplification.
- No remote telemetry.
- No breaking response compaction before measurement and versioning review.
- No continuous file capture until path and quota policy exists.

## Open decisions

1. What exact identity fields and confidence thresholds permit automatic
   profile reuse, especially for devices without USB serial numbers?
2. Should generated profile sessions persist immediately on open, or only after
   the first durable mutation or successful communication signal?
3. How many prior revisions should each learned profile retain, and what tool
   or argument restores one?
4. Should `list_ports` return a parallel profile-match map, or should a new
   higher-level discovery result replace it in a future major API?
5. Is defaulting omitted baud to 115200 acceptable, or should baud remain an
   explicit choice to avoid false confidence on unknown devices?
6. Which real agent/client produced the `list_ports` schema error, and was it
   using a build containing `d04bac9`?
7. Should boot capture own a DTR/RTS pulse recipe, or only arm capture and let
   an external actor perform reset?
8. Which clients fail to consume subscription notifications reliably enough to
   include in the evaluation matrix?
9. Which concrete settings belong in a connection recipe beyond selecting an
   existing protocol preset?
10. What improvement threshold would justify a versioned simple facade while
   all advanced tools remain visible?

## Rollout order

1. Ship correctness and drift fixes.
2. Fix and harden shared persistent profile storage.
3. Add automatic profile sessions, selection, learning, and rollback.
4. Improve discovery, teaching, recipes, and simple call shapes.
5. Measure representative agent workflows.
6. Add only high-level operations that remove demonstrated races.
7. Revisit versioned facades with evidence while keeping capabilities visible.
