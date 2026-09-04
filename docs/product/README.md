# Product

Product intent and lifecycle for serial-mcp, managed with
[Backlog.md](https://backlog.md). This file records the project-specific
contract: configuration, field vocabulary, lifecycle semantics, and
ownership boundaries. It is not an index of items; `backlog board` /
`backlog task list` are the views.

- [Configuration](#configuration)
- [Field vocabulary](#field-vocabulary)
- [Lifecycle](#lifecycle)
- [Definitions](#definitions)
- [Section ownership](#section-ownership)
- [Tooling](#tooling)

## Configuration

`backlog.config.yml` at the repository root (the backlog lives at
`docs/product/backlog`, set by `backlog_directory`):

```yaml
statuses: ["Backlog", "Ready", "In Progress", "Blocked", "Review", "Done"]
priorities: ["P0", "P1", "P2", "P3"]
types: ["feature", "bug", "research", "refactor", "tech-debt", "docs"]
task_prefix: "PB"
zero_padded_ids: 3
backlog_directory: "docs/product/backlog"
remote_operations: false
```

Storage layout under `docs/product/backlog/`:

```text
tasks/       # active items (any non-terminal status)
completed/   # terminal Done items (retained product history)
archive/     # dropped items (retained, out of all views)
drafts/      # not used
```

## Field vocabulary

### IDs

`PB-001`, `PB-002`, ... — allocated by `backlog task create` (monotonic
max+1 across visible tasks; `backlog doctor` detects duplicates). IDs are
never reused or renumbered; references use the ID, not the filename.

### Status

```text
Backlog -> Ready -> In Progress -> Review -> Done
                In Progress <-> Blocked
any active status -> archived (dropped)
```

`Done` is the terminal status (last entry of `statuses`).
`backlog task complete <id>` moves a Done task's file to `completed/`.

**Dropped** is not a status. `backlog task archive <id>` moves an active
item to `archive/tasks/` — the archive is our dropped history; the drop
rationale is recorded in the item's Final Summary before archiving.

### Priority

```text
P0  urgent or actively blocking
P1  intended near-term work
P2  useful planned work; normal default
P3  speculative, conditional, or deliberately deferred
```

### Type

```text
feature     new user-visible or product-visible capability
bug         existing behavior is incorrect
research    investigation needed before an implementation decision
refactor    internal structural change, intentionally unchanged behavior
tech-debt   maintenance, build, infrastructure, or accumulated deficiency
docs        documentation is the primary deliverable
```

### Size and area (labels)

Backlog.md has no native fields for these, so they ride as labels:

```text
size:S | size:M | size:L        S localized, M several components, L cross-cutting
area:<subsystem>                mcp runtime platform serial rx tx framing
                                protocol profiles capture schema testing ci
                                release distribution documentation security
```

`size:*` may be absent for an early Backlog item; it must be set before
Ready. An L item normally stays Backlog until refined or split.

### Dependencies

Native frontmatter `dependencies:` list of PB IDs — true sequencing only.
Related-but-independent work is linked in Description text instead.

## Lifecycle

- **Implementation agents may only begin `Ready` work.**
- **An agent may take an item to `Done`** (status edit, then
  `backlog task complete`) through the PR gate:
  1. acceptance criteria checked from real evidence;
  2. repository gates green;
  3. Final Summary filled (outcome, decisions, validation);
  4. the item's Done transition (status + move to `completed/`) committed
     together with the work in one PR whose title starts with the item ID
     (e.g. `PB-006: ...`).
- **The PR merge by the human product owner is the official acceptance.**
  A Done item merged into `main` is officially done. If the PR is rejected
  or changes are requested, move the item back (`tasks/`, status In
  Progress or Review), fix, and re-PR. An agent never merges its own PR.
- **`Review` status** remains available for implementation-complete items
  awaiting direction or bundling, but it is not a required stop — the
  happy path is `In Progress -> Done` inside the PR.
- `Done` does not imply a published release; `CHANGELOG.md` owns release
  history.
- **Dropped**: the human decides; record the rationale in Final Summary,
  then `backlog task archive`. Archived items are retained permanently.
- Status changes go through the `backlog` CLI (or MCP), not hand edits,
  so IDs, filenames, and metadata stay consistent. Content edits
  (Description, plan, notes) may be made in any editor.

## Definitions

**Definition of Ready** — problem understandable; desired outcome
explicit; scope bounded; non-goals stated; acceptance criteria
observable; dependencies known; size set; no unresolved product
question blocks implementation; the item does not require the
implementation agent to invent product behavior.

**Definition of Review** (before entering Review) — acceptance criteria
checked from evidence; repository gates run; Final Summary records the
implementation, validation commands, and outcomes; required user docs /
reference docs / ADR updates included; known limitations stay within
stated non-goals.

**Definition of Done** (agent-reachable, PR-gated) — acceptance criteria
checked from real evidence; repository gates green; Final Summary filled
with validation commands and outcomes; the Done transition committed
together with the work in one PR titled with the item ID. Officially done
when the human product owner merges that PR.

**Blocked** requires a concrete external or technical obstacle recorded
in Implementation Notes. An underspecified item is not blocked; it stays
Backlog.

**Work in progress**: default soft limit is one In Progress item for
this single-owner project unless parallel work is explicitly intended.

## Section ownership

**Product-owned** (implementation agents may not silently change;
refinement may change only when explicitly requested): title, priority,
status, type, Description (Problem / Desired outcome / Scope / Non-goals
parts), acceptance-criteria text.

**Execution-owned**: Implementation Plan, Implementation Notes, Final
Summary, acceptance-criteria checkbox state, validation evidence.

**Shared metadata** (correctable when repository inspection proves the
initial classification wrong; corrections stay visible in task history):
size/area labels, dependencies.

## Tooling

The `backlog` executable is provided by the repository dev shell
(`nix develop`), pinned through the flake input `backlog-md`
(github:MrLesk/Backlog.md). It is intentionally not a global install:
the tool version that understands this backlog lives with the repository.

```bash
nix develop -c backlog board          # terminal Kanban
nix develop -c backlog task list -s Ready --plain
nix develop -c backlog task PB-006 --plain
nix develop -c backlog doctor         # duplicate/cycle validation
nix develop -c backlog browser        # web UI (localhost)
```

The Markdown files are the store; the CLI, board, and web UI are views.
Everything remains greppable:

```bash
rg 'status: Review' docs/product/backlog/tasks
rg "area:tx" docs/product/backlog/tasks
rg 'PB-017' docs/product
```