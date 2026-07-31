# Phase 1 Handoff — Agent Interface Correctness and Drift

## Goal

Fix the confirmed `flush(target="both")` RX-ring behavior bug and remove
agent-facing guidance that describes removed arguments or the wrong `ReadFrom`
wire shape. Prove behavior through public MCP boundaries rather than private
field assertions.

This is Phase 1 of
`docs/development/agent-interface-simplification-plan.md`.

## In scope

1. Make `flush(target="both")` discard retained RX-ring backlog and move the
   shared read cursor to the ring live edge, matching the documented semantics
   and existing `target="input"` behavior.
2. Add a real PTY regression test that proves stale pre-flush bytes are not
   returned after `flush(target="both")`, while post-flush bytes remain readable.
3. Remove stale agent-visible `wait_for` references and replace them with
   `read(match=...)` or `transact`, as appropriate.
4. Remove the deleted per-call `max_buffered_bytes` argument from the rendered
   `diagnose_port` prompt.
5. Correct agent-facing `ReadFrom` examples. Current wire forms are:
   - `{"type":"cursor"}`
   - `{"type":"now"}`
   - `{"type":"buffer_start"}`
   - `{"type":"offset","offset":N}`
6. Add public MCP tests proving prompt/tool descriptions use current call
   shapes.
7. Preserve and commit the already-written
   `docs/development/agent-interface-simplification-plan.md` with this phase.

## Out of scope

- No learned-profile implementation. Shared `ProfileStore` work begins Phase 2.
- No new tools, argument shorthands, protocol presets, or response fields.
- No `list_ports` code change. The historical uint schema issue is already
  fixed by `d04bac9`, current guards pass, and no exact current-client failure
  payload is available.
- No continuous capture, boot-capture, file-write policy, error taxonomy, or
  profile storage changes.
- Do not edit historical changelog entries merely because they use conceptual
  shorthand. Fix current agent instructions, README, generated tool/prompt
  descriptions, and non-historical source comments.
- Do not modify `~/.config/opencode/AGENTS.md`; that global change was already
  made outside this repository.

## Current behavior and grounding

### Flush bug

`src/tools/io_ops.rs:341-412`:

- `FlushTarget::Input` clears OS input, calls `RxRing::clear()`, and moves the
  shared read cursor to `ring.end_offset()`.
- `FlushTarget::Both` flushes TX and clears OS input but never clears the RX
  ring or cursor.
- `src/server.rs:335-338` promises that `target=both` clears both.
- `tests/http_integration.rs::flush_each_target_returns_valid_response` proves
  only that each call returns success; it does not prove buffer behavior.
- `tests/native_sim_validation/unix.rs::native_flush_input_clears_host_rx`
  covers input only and is ignored without firmware.

Implement one shared RX-discard helper/path used by both Input and Both so their
RX semantics cannot drift. Preserve ordering already required for output flush:
for Both, flush queued/OS output first, clear OS input, then clear retained ring
state and clamp the shared cursor. Do not change Output-only behavior.

### ReadFrom documentation mismatch

`src/tools/types.rs:178-203` defines `ReadFrom` with
`#[serde(tag = "type")]`; tests send tagged objects. Current README and tool
descriptions instead advertise strings and an incomplete offset object:

- `README.md:26`
- `src/server.rs` descriptions for `transact`, `read`, and `flush`
- relevant current comments in `src/tools/io_ops.rs` and project `AGENTS.md`

Use exact tagged-object examples in current user/agent-facing text. Do not
implement string shorthand in this phase.

### Prompt drift

- `src/prompts/diagnose.rs:18` emits
  `read(..., max_buffered_bytes=512)`, but that per-call field was removed in
  v0.8.1.
- `src/server.rs:697-699` and `src/prompts/types.rs:23` still name the removed
  `wait_for` tool.
- Non-historical internal comments also mention the removed tool in
  `src/rx_metadata.rs`, `src/buffer_budget.rs`, `src/serial.rs`, and
  `src/stop_controller.rs`. Update these where `wait_for` means the former MCP
  tool. Do not rename the legitimate internal `RxRing::wait_for_data` method.

## Expected public behavior

### Flush

Given an open PTY connection with `OLD` retained in its RX ring:

1. Client calls `flush(connection_id, target="both")`.
2. Device sends `NEW` after flush returns.
3. Client calls ordinary `read`.
4. Returned data contains `NEW` and does not contain `OLD`.
5. Connection remains usable; success must not be explained by disconnect or an
   empty/dead stream.

### Diagnose prompt

Fetching `diagnose_port` over MCP returns instructions using valid current
`read` arguments. Rendered prompt must not contain `max_buffered_bytes` as a
per-call read argument or the removed `wait_for` tool.

### Tool descriptions

Fetching `tools/list` over MCP returns `read`/`transact`/`flush` descriptions
whose explicit examples use the actual tagged `ReadFrom` form. Description and
input schema must no longer disagree.

## Test plan

### Behavior regression: real PTY

Add a focused Linux PTY test in `tests/serial_pty.rs`, named clearly (for
example `pty_flush_both_discards_retained_rx_backlog`). Drive only public MCP
tools and the PTY device side:

1. Use existing `setup()`.
2. Write a unique old marker from device side.
3. Observe through public `get_status` that RX backlog reached the ring (poll
   with a bounded timeout; avoid relying only on a fixed sleep).
4. Call public `flush` with `target="both"`.
5. Write a unique new marker from device side.
6. Call public `read` with a bounded timeout.
7. Assert returned data includes new marker and excludes old marker.

Do not assert private ring fields or helper calls as feature acceptance.

### Public prompt behavior

Extend or add an HTTP integration test near
`get_prompt_diagnose_port_returns_user_message` in
`tests/http_integration.rs`. Fetch the prompt through MCP and assert rendered
content:

- identifies requested port
- uses current `read` flow
- does not contain removed per-call `max_buffered_bytes`
- does not instruct use of removed `wait_for`

### Public tool-description behavior

Add an HTTP integration test that calls `tools/list`, finds the relevant tool
descriptions, and proves explicit `from` examples use tagged objects. Keep
assertions focused on wire-shape correctness, not exact full prose.

### Focused commands

```bash
cargo test --test serial_pty pty_flush_both
cargo test --test http_integration get_prompt_diagnose_port
cargo test --test http_integration read_tool_description
cargo test --test doc_drift
```

Use actual final test names in commands if they differ.

## Files expected to change

- `src/tools/io_ops.rs`
- `src/server.rs`
- `src/prompts/diagnose.rs`
- `src/prompts/types.rs`
- `src/rx_metadata.rs`
- `src/buffer_budget.rs`
- `src/serial.rs`
- `src/stop_controller.rs`
- `README.md`
- `AGENTS.md`
- `tests/serial_pty.rs`
- `tests/http_integration.rs`
- possibly `tests/doc_drift.rs` if a small current-doc guard adds value
- `docs/development/agent-interface-simplification-plan.md` (already present)
- this handoff document

Avoid unrelated formatting or refactoring.

## Repository invariants

- Operational failures normally become MCP tool results with
  `is_error: Some(true)`; malformed request failures remain separate.
- All tools retain `title` and `output_schema`.
- No non-standard unsigned JSON Schema formats.
- `read` and `subscribe` cursor/ring semantics remain unchanged.
- `flush` remains destructive and its annotation stays destructive.
- Production code: no `unwrap`, `expect`, `println!`, `todo!`, or
  `unimplemented!` beyond documented mutex-poison convention.
- Tests must prove public behavior, not constructor wiring, private fields,
  helper-call counts, or `Arc` identity.
- No attribution or co-author footer in commits.

## Full verification

Run in this order:

```bash
cargo fmt --all -- --check
cargo build --all-targets --locked
cargo test --locked
cargo clippy --all-targets --locked -- -D warnings
```

If a command fails, fix scoped failures and rerun it. Report any unrelated
failure with exact output rather than omitting it.

## Commit and return requirements

After implementation and verification:

1. Inspect `git status`, `git diff`, and recent log.
2. Stage only intended repository files.
3. Commit completed Phase 1 work with a concise conventional commit message.
   One or two focused commits are acceptable; do not amend existing commits.
4. Do not push, merge, open a PR, amend, force-push, or add attribution.
5. Return:
   - files changed
   - user-visible behavior changed
   - tests/commands run and results
   - commit hash(es) and message(s)
   - blockers or deviations
   - suggested follow-up
