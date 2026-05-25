# Refactoring Plan: Modular Architecture

## Current State

```
src/
├── handler.rs          1,581 lines (GOD FILE)
├── serial.rs            637 lines (config + IO + manager mixed)
├── codec.rs             184 lines (focused, keep as-is)
├── error.rs              27 lines (focused, keep as-is)
├── main.rs               28 lines (entry, keep as-is)
└── bin/http.rs           64 lines (entry, keep as-is)
```

**Problems:**
- handler.rs does everything: tools, prompts, resources, security, completions
- serial.rs mixes config enums with runtime IO + manager
- Tool types (args/responses) scattered in one file
- No clear module boundaries

## Target Architecture (Moderate Split - 6 modules)

```
src/
├── lib.rs                    # Module declarations + re-exports

# Keep as-is:
├── codec.rs                  # Encoding/decoding (focused)
├── error.rs                  # SerialError + Result
├── main.rs                   # stdio binary entry
└── bin/
    └── http.rs              # HTTP binary entry

# New modules:
├── tools/
│   ├── mod.rs               # Re-exports + common helpers
│   ├── types.rs             # All tool argument + response structs
│   ├── port_ops.rs          # list_ports, open, close
│   ├── io_ops.rs            # read, write, flush
│   ├── control_ops.rs       # set_dtr_rts, send_break
│   ├── pattern_ops.rs       # wait_for
│   └── stream_ops.rs        # subscribe, unsubscribe + stream_rx

├── prompts/
│   ├── mod.rs               # Re-exports
│   ├── types.rs             # DiagnosePortArgs, InteractiveTerminalArgs
│   ├── diagnose.rs          # diagnose_port prompt
│   └── interactive.rs       # interactive_terminal prompt

├── resources/
│   ├── mod.rs               # Re-exports + URI constants
│   ├── types.rs             # ConnectionsResource, ResourceUriKind, parse_resource_uri
│   └── handlers.rs          # read_resource, subscribe, unsubscribe logic

├── security.rs              # Allowlist: parse, check, summary
└── server.rs                # ServerHandler impl (slimmed to ~300 lines)
    # Contains: get_info, initialize, list_resources, list_resource_templates,
    #          complete (thin wrappers calling modules)
```

## Step-by-Step Execution Plan

### Phase 1: Extract Tool Types (Low Risk)
1. Create `src/tools/types.rs` with all tool args + response structs
2. Add `pub mod tools` to `src/lib.rs`
3. Import types in `handler.rs` via `use crate::tools::types::*`
4. Remove type definitions from `handler.rs`
5. Move default value helpers to `tools/types.rs`

### Phase 2: Extract Prompt Handling (Low Risk)
1. Create `src/prompts/types.rs` with prompt arg structs
2. Create `src/prompts/diagnose.rs` with diagnose_port logic
3. Create `src/prompts/interactive.rs` with interactive_terminal logic
4. Add `pub mod prompts` to `src/lib.rs`
5. Keep `#[prompt_router]` impl in `handler.rs` but call module functions

### Phase 3: Extract Resources (Medium Risk)
1. Create `src/resources/mod.rs` with URI constants + ResourceUriKind
2. Create `src/resources/types.rs` with ConnectionsResource
3. Move `parse_resource_uri()` and `ConnectionsResource` from `handler.rs`
4. Keep `read_resource` in `server.rs` but use resource helpers

### Phase 4: Extract Security (Low Risk)
1. Create `src/security.rs` with allowlist logic
2. Move `parse_allowlist_env()`, `is_port_allowed()`, `allowlist_summary()`
3. Keep field in `SerialHandler` struct, delegate to module

### Phase 5: Extract Tool Logic (Medium Risk)
1. Create `src/tools/port_ops.rs` with list_ports, open, close
2. Create `src/tools/io_ops.rs` with read, write, flush
3. Create `src/tools/control_ops.rs` with set_dtr_rts, send_break
4. Create `src/tools/pattern_ops.rs` with wait_for
5. Create `src/tools/stream_ops.rs` with subscribe, unsubscribe, stream_rx
6. Keep `#[tool_router]` impl in `handler.rs` but call module functions

### Phase 6: Reorganize Files (High Risk)
1. Rename `handler.rs` → `server.rs`
2. Extract tool helpers (read_bytes, read_until_pattern, etc.) to `tools/helpers.rs`
3. Verify all tests still pass
4. Verify rmcp macros still work

### Phase 7: Test Organization
1. Move unit tests from `handler.rs` to relevant modules
2. Keep integration tests in `tests/` directory
3. Add module-level unit tests for extracted logic

## Key Decisions

1. **Keep `#[tool_handler]` in one file**: rmcp macro requires all `#[tool]` methods
   in a single `impl` block. We'll keep a thin wrapper in `server.rs`.

2. **Use free functions in modules**: Tool logic becomes standalone async functions
   that take `&SerialHandler` or `Arc<ConnectionManager>` as parameter.

3. **Type re-exports**: `tools::types::*` re-exported so existing code doesn't break.

4. **No `#[tool]` macro in modules**: The macro stays in `server.rs`. Modules
   contain pure logic only.

## Estimated Effort

| Phase | Files Changed | Risk | Time |
|-------|--------------|------|------|
| 1 (Tool Types) | 2 new, 1 modified | Low | 30 min |
| 2 (Prompts) | 3 new, 1 modified | Low | 30 min |
| 3 (Resources) | 2 new, 1 modified | Medium | 45 min |
| 4 (Security) | 1 new, 1 modified | Low | 20 min |
| 5 (Tool Logic) | 5 new, 1 modified | Medium | 90 min |
| 6 (Reorganize) | 1 rename, 1 modified | High | 30 min |
| 7 (Tests) | Multiple | Medium | 45 min |

**Total: ~5-6 hours**

## Rollback Strategy

Each phase is independent. After each phase:
1. Run `cargo test`
2. Run `cargo clippy --all-targets -- -D warnings`
3. Commit before starting next phase

If a phase fails, restore from git and try different approach.
