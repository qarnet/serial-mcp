# Phase 6 Agent-Interface Evaluation Note — Safe Persistent Capture

Deterministic local measurement, same harness as Phase 4/5: no network, no
user profiles, no hardware, no timestamps. Run with:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```

The Phase 4 baseline (`docs/development/agent-interface-baseline.json`) and
its decision record are historical and were NOT rewritten. This note records
the Phase 6 export schema/catalog delta against that baseline.

## Catalog delta

- tool count: **27** (unchanged — Phase 6 adds no tool)
- aggregate compact `tools/list` payload: **258964 → 285738 bytes (+10.3%)**
  — of which `capture_boot` (+25338, Phase 5) accounts for nearly all
  growth; Phase 6 adds **+1435 bytes (0.5%)**
- Phase 6 per-tool delta (only `export_log` changed):
  - `export_log`: **754 → 2189 bytes (+1435, +190%)** — deliberate schema
    growth, not bloat: the description now states the disabled-by-default
    store, the `--capture-dir` requirement, the portable filename-only
    `path` contract, no-overwrite/symlink policy, and quotas; `path`
    documents "canonical absolute final file"; the result gained three
    uint-schema-annotated fields (`bytes_written`, `files_used`,
    `total_bytes_used`). Input schema still requires only
    `connection_id` + `path`.
  - all other 26 tools byte-identical to the Phase 4 baseline
- **Phase 6 review polish** (follow-up commit): per-file quota rejection
  moved before the process-local mutex, cause-neutral persist error text,
  and the post-commit durability contract — a Unix root-directory sync
  failure after `persist_noclobber` is now reported as an additive
  optional `durability_warning` on an otherwise-successful export instead
  of a false "no final file" error. Final delta: `export_log` **754 →
  2736 bytes** (description 931, input schema 542, output schema 1111);
  aggregate **286285 bytes** (+10.5% vs the Phase 4 baseline, of which
  `capture_boot` accounts for 25338). `durability_warning` is
  `skip_serializing_if`-omitted on success, so the committed-wire result
  shape is unchanged unless a warning actually occurs.
- the evaluator's per-tool regression rule flags `export_log` (>= some
  growth threshold) and reports overall status `warning` — the growth is
  the deliberate Phase 6 scope (public contract hardening), and the
  tool-count guard (`tool_catalog_has_exactly_twenty_seven_tools`) plus the
  uint-format schema guards (`serial::schema::export_log_result_has_no_uint_formats`,
  `tools::mod::tool_schemas_have_no_nonstandard_uint_formats`) pin the shape.

## Scenario delta

- Scenario set unchanged: no scenario exercises `export_log`, so all
  scenario metrics (calls, request bytes, `stale_race`) are identical to
  Phase 5. `boot_reset_prompt_capture` stays one implemented `capture_boot`
  call; `boot_reset_manual_composition` stays the pre-Phase-5 comparison.
- Decisions from fixed thresholds: unchanged (automatic profiles yes,
  shorthand no, recipes yes, facade no, capture_boot yes).

## Behavior proof (public MCP boundary, in-process + spawned binary)

- `tests/http_integration.rs` (13 tests): disabled error ordering (before
  path validation and before connection lookup), JSONL content + exact
  event/byte counts match `get_log`, zero-byte export consumes a file slot,
  traversal/absolute/bad-suffix/overlength/reserved/Windows-stem names all
  fail with no files, existing target byte-identical, symlink target
  rejected with outside target untouched (Unix), concurrent same-name
  exports = exactly one success, per-file quota leaves no file, total quota
  persists across exports AND fresh store instances, file-count quota
  includes prior commits, independent servers sharing a root cannot exceed
  quota (advisory lock), failure leaves connection usable, point-in-time
  snapshot (post-export events never leak in).
- `tests/http_integration.rs::spawned_server_starts_with_capture_dir` +
  `tests/stdio_integration.rs::stdio_server_starts_with_capture_dir`: real
  binary starts in both transports with a valid `--capture-dir`.
- CLI (`tests/stdio_integration.rs`): help documents all four options,
  `--capture-dir --version` is not a version request, quota-without-root /
  relative / missing / file / symlink roots / zero limits / bad quota
  relation all reject startup.
- Unit (`src/capture_store.rs`, `src/log_buffer.rs`): portable filename
  validator table (incl. exact MAX+1 rejection), quota boundaries,
  managed-file scanner classification (symlink rejection, unknown/orphan
  ignoring), no-clobber commit, cross-store advisory-lock concurrency,
  injected post-commit root-sync failure → `durability_warning` (file kept,
  never deleted), JSONL exact-limit and one-byte-over snapshot.

## Decision

- **Safe persistent capture foundation: yes** — implemented exactly as
  handoff-scoped: `CaptureStore` disabled by default, explicit absolute
  `--capture-dir` + quotas, portable flat filename contract, symlink
  policy, bounded point-in-time JSONL snapshot, advisory cross-process
  locking, atomic `persist_noclobber` commits, startup validation, README/
  CHANGELOG/AGENTS.md/docs updates. No continuous capture tool, no version
  bump.
- **Continuous disk capture (future): no (deferred)** — Phase 4/5 evidence
  supports bounded in-memory boot capture only; `docs/development/
  safe-continuous-capture-design.md` specifies the full lifecycle
  (registry, states, private cursor, gaps, stop reasons, rotation, orphan
  policy) with the recommendation "do not implement until concrete task
  evidence".

## Completion references

- `tests/http_integration.rs::export_log_disabled_errors_before_path_write_and_creates_nothing`
- `tests/http_integration.rs::export_log_enabled_writes_valid_jsonl_matching_get_log`
- `tests/http_integration.rs::export_log_concurrent_same_name_yields_exactly_one_success`
- `tests/http_integration.rs::export_log_total_byte_quota_persists_across_exports_and_fresh_stores`
- `tests/http_integration.rs::export_log_independent_servers_sharing_root_cannot_exceed_quota`
- `src/capture_store.rs::concurrent_independent_stores_cannot_exceed_quota`
- `src/log_buffer.rs::jsonl_snapshot_exact_limit_and_one_byte_over`
- `src/capture_store.rs::post_commit_root_sync_failure_is_a_warning_not_a_failed_commit`
