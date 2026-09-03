# Product

Product intent and lifecycle for serial-mcp. This file is the canonical
contract for backlog item format, allowed values, lifecycle states, and
ownership boundaries. It is not an index of items; the Org agenda is the
derived index.

- [Backlog item format](#backlog-item-format)
- [Allowed values](#allowed-values)
- [Required sections](#required-sections)
- [Section ownership](#section-ownership)
- [Lifecycle](#lifecycle)
- [Definitions](#definitions)
- [Tool-independent access](#tool-independent-access)

## Backlog item format

One Org file per item under `backlog/`:

```text
backlog/
├── TEMPLATE.org
├── active/          # BACKLOG READY IN_PROGRESS BLOCKED REVIEW
├── completed/       # DONE (retained product history)
└── dropped/         # DROPPED (retained with rationale)
```

Status is task metadata in the headline; files move directories only on
`DONE` or `DROPPED`. No per-state subdirectories.

Every item file:

```org
* READY [#1] PB-017 Add TX pacing
  :PROPERTIES:
  :ID:         PB-017
  :TYPE:       feature
  :SIZE:       M
  :AREA:       tx
  :DEPENDS_ON:
  :END:

** Problem
...
```

- Exactly one top-level headline per file.
- Filename: `PB-NNN-short-kebab-case-description.org`.
- Headline: `* <STATE> [#P] PB-NNN <Title>`.
- The `PB-NNN` identity in filename, headline, and `ID` property must agree.

### IDs

- Immutable sequential: `PB-001`, `PB-002`, ... (at least three digits, may
  continue past `PB-999`).
- Never reused, never renumbered, stable across title changes.
- Allocation considers `active/`, `completed/`, and `dropped/`: next ID is
  the largest existing numeric ID plus one. Detect collisions before writing
  or committing.
- References and dependencies use the ID, not the filename.

## Allowed values

### Workflow states

```org
BACKLOG READY IN_PROGRESS BLOCKED REVIEW | DONE DROPPED
```

`DONE` and `DROPPED` (after the separator) are terminal.

### Priorities

Numeric Org priority cookies; every task has one:

```text
[#0]  P0  urgent or actively blocking
[#1]  P1  intended near-term work
[#2]  P2  useful planned work; normal default
[#3]  P3  speculative, conditional, or deliberately deferred
```

### TYPE

```text
feature     new user-visible or product-visible capability
bug         existing behavior is incorrect
research    investigation needed before an implementation decision
refactor    internal structural change, intentionally unchanged product behavior
tech-debt   maintenance, build, infrastructure, or accumulated engineering deficiency
docs        documentation is the primary deliverable
```

### SIZE

```text
S  localized change
M  several related components or files
L  broad, cross-cutting, or architecture-heavy work
```

`SIZE` may be blank for an early `BACKLOG` idea; it must be set before
`READY`. An `L` item normally stays in `BACKLOG` until refined or split into
independently meaningful product outcomes.

### AREA

One primary area per item. Secondary affected areas belong in Technical
context. Initial vocabulary:

```text
mcp runtime platform serial rx tx framing protocol profiles capture
schema testing ci release distribution documentation security
```

New values require updating this contract first, never invented inside an
individual task.

### DEPENDS_ON

Empty, or one or more backlog IDs, space-separated:

```org
:DEPENDS_ON: PB-003 PB-011
```

True sequencing dependencies only. Related-but-independent work is linked in
Technical context instead.

### Omitted fields

`OWNER`, `ASSIGNEE`, `SPRINT`, `STORY_POINTS`, `ESTIMATED_HOURS`,
`PERCENT_COMPLETE`, `DUE_DATE`, `VELOCITY`, `RANK` — process overhead with no
single-owner problem to solve.

## Required sections

```text
Problem
Desired outcome
Scope
Non-goals
Acceptance criteria
Technical context
Open questions
Implementation plan
Implementation notes
Result
```

Early `BACKLOG` items may leave implementation sections empty and keep real
uncertainty under Open questions.

## Section ownership

**Product-owned** (implementation agents may not silently change to match
what was easier to implement; refinement agents may change only when the
user explicitly requested backlog refinement):

headline/title, priority, Problem, Desired outcome, Scope, Non-goals,
acceptance-criteria text.

**Execution-owned** (implementation records its work here):

Implementation plan, Implementation notes, Result, acceptance-criteria
checkbox state, validation evidence.

**Shared metadata** (correctable when repository inspection proves the
initial classification wrong; corrections stay visible in task history):

`SIZE`, `AREA`, `DEPENDS_ON`.

## Lifecycle

```text
BACKLOG
   ▼
READY
   ▼
IN_PROGRESS ─────► BLOCKED
   │                  │
   │                  └────► IN_PROGRESS or READY
   ▼
REVIEW
   ├──── changes requested ────► IN_PROGRESS
   └──── human acceptance ────► DONE

Any active state ── human decision ──► DROPPED
```

- **Implementation agents may only begin `READY` work.**
- **Implementation agents stop at `REVIEW`.** It is their final state.
- **Only the human product owner performs `REVIEW → DONE`**, normally as part
  of or immediately after the accepted merge. `DONE` does not imply a
  published release; `CHANGELOG.md` owns release history.
- On acceptance the file moves `active/ → completed/`. On a drop decision the
  file moves `active/ → dropped/` with the rationale recorded in Result.
- Completed and dropped items are retained permanently. They are product
  history, not clutter.

## Definitions

**Definition of Ready** — problem understandable; desired outcome explicit;
scope bounded; non-goals stated; acceptance criteria observable; dependencies
known; size set; no unresolved product question blocks implementation;
relevant designs/ADRs/specs/source locations referenced; the item does not
require the implementation agent to invent product behavior.

**Definition of Review** (before entering `REVIEW`) — acceptance checkboxes
updated from evidence; tests and repository checks run; Result summarizes
implementation with validation commands and outcomes; required user docs,
reference docs, or ADR updates included; known limitations stay within stated
non-goals.

**Definition of Done** (human-only) — reviewed implementation, acceptance
criteria, and evidence; accepted product behavior; accepted integration.

**BLOCKED** requires a concrete external or technical obstacle recorded in
Implementation notes. An underspecified item is not blocked; it belongs in
`BACKLOG`.

**Work in progress**: default soft limit is one `IN_PROGRESS` item for this
single-owner project unless parallel work is explicitly intended.

## Tool-independent access

Plain text; no tool required to read it:

```bash
find docs/product/backlog/active -name 'PB-*.org'
rg '^\* REVIEW ' docs/product/backlog/active
rg ':AREA:[[:space:]]+protocol' docs/product/backlog/active
rg 'PB-017' docs/product
```

Neovim Org agenda and OpenCode backlog skills are views over these files.
The repository contract in this README is authoritative when a skill and this
document disagree.