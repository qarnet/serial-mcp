# Code Cleanup Phase 3 Handoff

## Goal

Reduce repeated profile-binding literals, make profile-preview branches read as
domain decisions, and remove one obsolete search alias. Preserve all profile
selection, persistence, ordering, and wire behavior.

Implement Phase 3 only. Commit completed work before returning.

## In scope

- `src/tools/port_ops.rs`
- `src/tools/helpers.rs`
- `src/match_config.rs`
- Focused pure tests in `src/tools/port_ops.rs` only if needed
- Existing profile/public-boundary tests
- This handoff document

## Out of scope

- No changes to profile structs, TOML schema, revisions, history, selectors,
  ranking, persistence operations, open sequencing, or public result fields.
- No changes to `ProfileStore`, `learning`, `SerialConnection`, or MCP schemas.
- No generic many-argument `ActiveProfileBinding` constructor.
- Do not move factories onto public profile/serial types.
- Do not change candidate display sorting or timestamp-only winner selection.
- Do not touch subscription/read-loop/test-harness cleanup.
- Do not update dependencies or package version.

## Grounding and required behavior

### Binding construction

`attach_session_binding` in `src/tools/port_ops.rs` repeats full
`ActiveProfileBinding` literals for:

- Disabled session
- Transient session
- Automatic selected profile, mark-used success/failure
- Explicit selected profile, mark-used success/failure
- Generated profile success/failure

Preserve:

1. Disabled: empty name, `Disabled`, caller confidence, nonpersistent, clean,
   no candidates/error.
2. Transient: empty name, `Transient`, caller confidence, nonpersistent, clean,
   supplied candidates, no error unless generated creation failed.
3. Automatic selected: `Automatic`, confidence always `High`, persistent,
   supplied dirty state; mark-used success takes returned metadata; failure
   keeps original profile metadata and stores error.
4. Explicit selected: `Explicit`, confidence from matched live port, otherwise
   same mark-used success/failure rules.
5. Generated success: `Generated`, `High`, persistent, generated metadata,
   revision, clean.
6. Generated failure: transient high-confidence open success, empty name,
   nonpersistent, clean, persistence error retained.
7. Every branch still calls `conn.set_active_profile_binding(Some(binding))`
   once after construction.

### Profile preview

`compute_profile_matches` must preserve:

- One high-fingerprint count map for the whole live port snapshot.
- Weak identities never auto-select; only explicitly matching non-empty
  selectors appear, sorted by profile name.
- Duplicate live high fingerprints produce `Duplicate` with no candidates.
- No eligible high profile produces `None` with no candidates.
- Eligible candidate display order: newest `last_used_at_ms` first, then name.
- Selection decision uses timestamps only. Name never breaks equal top rank.
- One candidate selects. Multiple candidates select only when top timestamp is
  strictly different from second timestamp; equal top rank is `Ambiguous`.

### Search alias

`src/tools/helpers.rs` re-exports `crate::util::find_subsequence` as legacy
`find_subslice`. Only `src/match_config.rs` imports it. Remove alias and import
the canonical utility directly.

## Exact implementation shape

### 1. Private binding factories

Add concise private functions in `src/tools/port_ops.rs` near
`attach_session_binding`:

```rust
fn disabled_binding(confidence: IdentityConfidence) -> ActiveProfileBinding

fn transient_binding(
    confidence: IdentityConfidence,
    candidates: Vec<String>,
    persistence_error: Option<String>,
) -> ActiveProfileBinding

fn persistent_binding(
    profile: &Profile,
    source: ProfileSelectionSource,
    confidence: IdentityConfidence,
    dirty: bool,
    persistence_error: Option<String>,
) -> ActiveProfileBinding
```

Use exact type paths/imports fitting current file. `persistent_binding` derives
name, generated flag, and revision from supplied profile; it sets persistent,
stale, and candidates exactly as current literals do.

Add one async helper for selected/explicit mark-used handling:

```rust
async fn mark_used_binding(
    store: &ProfileStore,
    profile: Profile,
    source: ProfileSelectionSource,
    confidence: IdentityConfidence,
    dirty: bool,
) -> ActiveProfileBinding
```

On success, call `persistent_binding` with returned profile and no error. On
failure, call it with original profile and error. `mark_used` must execute
exactly once.

Use factories from all `attach_session_binding` branches. Keep generated
selector/default creation and policy decisions in the main match.

### 2. Profile preview helpers

Extract only cohesive pure decisions:

- `weak_identity_profile_match(port, confidence, profiles)` builds/sorts weak
  candidates and returns `None` or `Ineligible`.
- `ranked_profile_match(port, confidence, eligible)` builds display candidates
  and returns `Selected` or `Ambiguous` using current timestamp rule.

For duplicate and no-eligible branches, direct short literals are acceptable;
do not add generic constructors merely to hide five fields.

Main `compute_profile_matches` should visibly read:

1. Determine confidence/high identity.
2. Delegate weak identity.
3. Determine duplicate and eligible profiles.
4. Return duplicate/no-match or delegate ranked decision.

Do not combine the duplicate and eligibility computations if doing so changes
which selectors are evaluated or candidate ownership.

### 3. Canonical search name

- Delete legacy re-export and its comment from `src/tools/helpers.rs`.
- Change `src/match_config.rs` import and call to
  `crate::util::find_subsequence`.
- No compatibility concern: alias is `pub(crate)` and has one internal caller.

### 4. Comments

Replace chronology-only `Phase 3`/`Phase 4` wording in the touched
`compute_profile_matches` doc comment with present behavior. Keep details about
one fresh snapshot, high identity, duplicate handling, timestamp-only ranking,
and weak identity because those are non-obvious policy.

Do not sweep unrelated files for phase comments yet.

## Tests

Existing integration coverage is authoritative for profile behavior. Run full
library and relevant integration suites. Add pure unit tests only if extracted
preview helpers expose an untested boundary; do not test helper call counts or
factory field assignment independently when public results already cover it.

## Verification

```bash
cargo fmt --all -- --check
cargo test --lib --locked
cargo test --test serial_pty --locked -- --test-threads=1
cargo test --test http_integration --locked -- --test-threads=1
cargo test --test allowlist --locked -- --test-threads=1
cargo clippy --all-targets --locked -- -D warnings
```

## Acceptance criteria

- Automatic and explicit mark-used branches share one implementation.
- Repeated persistent/transient/disabled field literals are centralized in
  domain-named private factories.
- Profile-preview policy reads in smaller cohesive branches without changing
  ordering or outcomes.
- Legacy `find_subslice` alias is gone.
- No public behavior/schema change.
- Requested checks pass with no warnings.
- Diff contains no unrelated cleanup.
- Working tree clean after commit.

## Commit and recap

Before returning:

1. Inspect status, diff, and recent log.
2. Stage only intended source files and this handoff.
3. Commit with suggested message:

   `refactor: simplify profile binding flow`

4. Do not push, merge, open PR, amend, force-push, or add attribution.
5. Return files, behavior preserved, tests/results, commit hash/message,
   deviations, blockers, and follow-up concerns.
