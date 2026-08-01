# Post-0.9 Refinement — Phase 2 Handoff

## Role and delivery constraint

Implement Phase 2 on existing branch `refactor/post-0.9-refinement`. Follow
`docs/development/post-0.9-refinement-plan.md` and repository `AGENTS.md`.
Do not create or push a branch or PR. Commit completed Phase 2 work before
returning. Do not amend prior commits or add attribution footers.

## Goal

Make vendored config-schema validation mandatory, offline, and fail-closed on
all normal Cargo and Nix runs. Validate Claude Code, Codex, and opencode every
time. Preserve scheduled latest-upstream validation as explicitly networked.

## In scope

- Vendor `https://models.dev/model-schema.json` unchanged as
  `schemas/models-dev-model.schema.json`.
- Add `schemas/README.md` with source URI, retrieval date, SHA-256, exact update
  procedure, and rule that upstream blobs remain unedited.
- Make every local schema and instance file required; missing or malformed JSON
  must fail with path-specific errors.
- Compile local schemas with a no-network retriever and register vendored
  models.dev schema under its original URI.
- Keep four references in `schemas/opencode.schema.json` unchanged.
- Prove missing-file, unresolved-resource, registered-resource, and all-three
  real-fixture behavior.
- Include all schemas in Nix source filtering.
- Update existing schema-refresh script so documented refresh behavior includes
  models.dev resource.
- Remove completed models.dev debt from FEATURES and update concise repository
  truth in AGENTS.md.

## Out of scope

- No MCP tool/schema changes.
- No edits inside any downloaded upstream schema blob.
- No conversion of scheduled latest-upstream test into an offline test.
- No release/doc-drift work from Phase 3.
- No dependency or package-version changes.

## Grounding evidence

- `tests/config_schema_validation.rs::load_json_file` currently returns
  `Option<Value>` and prints `skipping: file not found`; both schema and instance
  callers silently continue.
- `schemas/opencode.schema.json` contains exactly four refs to
  `https://models.dev/model-schema.json#/$defs/Model`; no other vendored schema
  contains an HTTP `$ref`.
- Official models.dev document declares draft 2020-12, `$id`
  `https://models.dev/model-schema.json`, and `$defs.Model`.
- Locked `jsonschema 0.26.2` exposes `Resource::from_contents`,
  `ValidationOptions::with_resource`, `ValidationOptions::with_retriever`, and
  public `Retrieve`/`Uri` APIs. Registered resources resolve before retriever
  fallback.
- `flake.nix` currently admits `/schemas` except
  `opencode.schema.json`; comments explicitly describe current silent skip.
- `scripts/update-config-schemas.sh` already refreshes three top-level schemas
  and runs focused validation.

## Exact implementation decisions

### Vendored resource and provenance

Fetch exact bytes from `https://models.dev/model-schema.json` into
`schemas/models-dev-model.schema.json`. Do not pretty-print, normalize, or edit
it. Compute SHA-256 from committed bytes. Record source URL, UTC retrieval date,
checksum, and refresh/check commands in `schemas/README.md`.

Extend `scripts/update-config-schemas.sh` to fetch this fourth resource with the
same fail-fast curl policy used for existing schemas. Keep the focused test at
the end. README checksum update may remain an explicit manual step, but script
must print or compute the new checksum so it cannot be forgotten silently.

### Required file loading

Replace optional loading with a result-returning required loader. Error text
must distinguish read failure from JSON parse failure and include full path.
Tests should exercise nonexistent schema and instance paths through production
helpers, not copied filesystem logic. Top-level tests may panic with the
returned message for normal Rust test output, but there must be no `None`,
`continue`, `return`, `eprintln!("skipping...")`, or equivalent skip path.

Anchor fixture paths at `env!("CARGO_MANIFEST_DIR")` so tests do not depend on
process current directory.

### Hermetic local compilation

Implement a small `Retrieve` type for local validation that always returns a
descriptive error for unknown external resources. Configure local compilation
with that retriever. Load the vendored models.dev document using the same
required loader, convert it with `Resource::from_contents`, and register it with
`with_resource("https://models.dev/model-schema.json", ...)`.

Do not rewrite opencode refs to file paths or custom URIs. Do not permit HTTP
fallback in the non-ignored test. Claude and Codex should compile through the
same local helper; registering the unused models resource for all three cases is
acceptable and keeps one path.

Keep ignored latest-upstream validation networked. It may use default
`jsonschema` retrieval for nested external refs, but its local instance file is
still mandatory.

### Behavior tests

Add focused tests in `tests/config_schema_validation.rs` proving:

1. nonexistent required schema path returns explicit path-bearing error;
2. nonexistent required instance path returns explicit path-bearing error;
3. malformed JSON returns explicit path-bearing parse error;
4. compiling opencode without registering models.dev fails and names unresolved
   URI/resource;
5. compiling opencode with registered vendored resource succeeds;
6. `example_configs_match_vendored_schemas` always iterates exactly three cases
   and all validate offline.

Use temporary paths/directories where useful. Avoid tests of private field or
helper-call identity; assert returned failures and public validation outcomes.

### Nix and docs

Change source filter to include complete `/schemas` tree. Remove stale exception
comment and all language about silent schema skipping. Keep comments concise and
accurate: config-schema fixtures are mandatory.

Remove completed `Vendor the models.dev model schema...` section from
`docs/development/FEATURES.md`. Update `AGENTS.md` test-map/Nix guidance to state
that all three vendored examples validate hermetically and missing fixtures
fail. Do not edit unrelated roadmap entries.

## Required verification

Run:

```bash
cargo fmt --all -- --check
cargo test --locked --test config_schema_validation
cargo test --locked --test config_schema_validation -- --ignored
nix flake check --accept-flake-config --print-build-logs
git diff --check
```

The ignored test is networked; if upstream availability alone prevents it,
record exact failure but do not weaken it. All non-networked checks must pass.

Also inspect:

```bash
git status --short
git diff -- tests/config_schema_validation.rs schemas scripts/update-config-schemas.sh flake.nix AGENTS.md docs/development
git log --oneline -10
```

Confirm no upstream schema blob was reformatted, package version remains 0.9.0,
tool count remains 27, and no network-dependent code path exists in local
validation.

## Commit and recap

Stage only Phase 2 files and this handoff. Commit with:

```text
test: make config schema validation hermetic
```

Return files changed, behavior changed, exact source/checksum provenance, every
command/result, commit hash/message, blockers, deviations, and suggested Phase
3 follow-up. Do not push, merge, open a PR, or amend.
