---
id: PB-040
title: "Org-mode product backlog and documentation lifecycle"
status: Review
assignee: []
created_date: '2026-09-04 17:00'
updated_date: '2026-09-04 17:00'
labels:
  - 'size:L'
  - 'area:documentation'
dependencies: []
priority: p1
type: docs
ordinal: 1400
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
### Problem

The Markdown backlog table mixes workflow state, priority, and work type
in one Status column; `docs/development/` mixes policies, references, reports,
plans, and decisions; completed/abandoned history was being deleted instead
of retained.

### Desired outcome

One Org file per product item with controlled states and metadata; a
purpose-named docs taxonomy; Neovim Org agenda as the human view and OpenCode
skills as the agent interface over the same Git files; implementation agents
stop at REVIEW with human-only DONE acceptance.

### Scope

- Docs taxonomy: guides/ reference/ product/ design/ adr/ reports/
  maintenance/.
- Backlog migration with deterministic IDs (document order).
- Product contract at docs/product/README.md.
- OpenCode skills: add/refine/implement product-backlog-item.
- Neovim Orgmode agenda setup (lazy.nvim plugin).
- documentation-hygiene skill updated for the new marker path.

### Non-goals

- Backlog parser, Kanban app, database, dashboards, generated indexes.
- CI enforcement, DONE automation, GitHub sync.
- Any production serial-mcp behavior change.

### Technical context

Design doc: docs/design/PB-040-org-backlog-migration.md (this migration's
executable plan).

### Open questions

None.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [x] #1 Generic docs/development/ grouping replaced by purpose-specific categories.
- [x] #2 User docs under guides/; normative policy/matrix under reference/.
- [x] #3 Product policy under product/; designs under design/; ADRs kept under adr/; reports and baselines under reports/; maintenance procedures and the hygiene marker under maintenance/.
- [x] #4 documentation-hygiene skill and marker agree after the restructure.
- [x] #5 docs/BACKLOG.md replaced by one Org file per item, divided into active/ completed/ dropped/.
- [x] #6 Every item has a unique stable PB-NNN ID; filename, headline, and ID property agree.
- [x] #7 Controlled metadata vocabulary documented.
- [x] #8 Entries migrated without inventing missing product requirements.
- [x] #9 Near-term/Later/Wish/Infrastructure normalized into independent priority/type/status fields.
- [x] #10 Design docs link to their PB items.
- [x] #11 Completed and dropped history retained.
- [x] #12 Neovim Orgmode installed via lazy.nvim (a normal plugin).
- [x] #13 Neovim provides a useful active-backlog agenda, a prominent REVIEW view, and filtering by state/priority/type/area/size.
- [x] #14 OpenCode add-product-backlog-item skill documents and follows the schema; allocates IDs across lifecycle dirs; defaults incomplete requests to BACKLOG without fabricating.
- [x] #15 AGENTS.md states implementation agents stop at REVIEW; no agent autonomously marks DONE; only explicit human acceptance causes REVIEW -> DONE.
- [x] #16 Active documentation references use the new paths; historical changelog content untouched.
- [x] #17 No second backlog source of truth remains.
<!-- AC:END -->

## Implementation Plan

<!-- SECTION:PLAN:BEGIN -->
Phases 1-6 per the design document; phase order: taxonomy, migration,
skills, nvim, verification, commits.
<!-- SECTION:PLAN:END -->

## Implementation Notes

<!-- SECTION:NOTES:BEGIN -->Org iteration (docs/product/backlog Org files, Neovim Orgmode agenda,
three Org-writing skills) was completed and rejected at human review: the
agenda did not prove useful in practice. Pivot decision 2026-09-04: keep
the storage model (one file per item, stable PB-NNN IDs, retained
history, product contract, human-only Done), replace the format and the
tooling with Backlog.md.

Backlog.md verified against upstream source before committing:
statuses list with terminal-is-last rule, task_prefix PB, zero-padded
3-digit IDs, configurable types/priorities, complete-vs-archive
semantics (Done -> completed/ via task complete; dropped -> archive/
via task archive with rationale in Final Summary).

Migration: 40 Org items converted 1:1 to Backlog.md task files with
identical PB-NNN IDs (39 tasks/ + 1 archived drop). SIZE and AREA became
labels (size:S/M/L, area:<x>); DEPENDS_ON became native dependencies.
backlog doctor: no duplicate IDs or cycles; task list renders statuses,
priorities, types, AC counts; board renders the six columns.

Tooling: backlog-md pinned as flake input, exposed via devShell only
(nix develop -c backlog). Upstream nixpkgs NOT followed from ours
deliberately: qemu-user 11.1.0 (our newer unstable) breaks upstream's
AVX2 installCheck; their pinned nixos-unstable (qemu 11.0.1) passes.
Neovim orgmode spec reverted (orgmode pin removed from lazy-lock).
OpenCode skills rewritten to drive the CLI. Docs taxonomy, product
contract, and AGENTS.md boundaries from the Org iteration carry over
unchanged in substance.

Policy evolution after first human review of the Backlog.md migration
(2026-09-04): the human-only Done gate was relaxed. Agents may take an
item to Done through the PR gate — acceptance criteria checked from
evidence, gates green, Final Summary filled, and the Done transition
committed together with the work in one PR titled with the item ID
(e.g. PB-006: ...). The human's merge of that PR is the official
acceptance; rejection moves the item back to In Progress. Review status
remains for awaiting-direction items but is not a required stop.
AGENTS.md, docs/product/README.md, the implement-product-backlog-item
skill, and the repo-wrapup skill (new backlog PR-gate step) were updated
to the new policy.
Org iteration completed and human-reviewed at REVIEW: the Neovim Org
agenda interface did not prove useful in practice. Product decision
(2026-09-04): pivot the item format from Org to Backlog.md tasks while
keeping the same storage model (one file per item, stable IDs, retained
history, product contract, human-only acceptance).

Backlog.md verified against upstream source before committing:
- Upstream Nix flake exists (github:MrLesk/Backlog.md, bun2nix build,
  x86_64-linux/aarch64-linux/aarch64-darwin, MIT, mainProgram=backlog);
  pin it as a flake input, expose in the devShell -> repo-scoped
  executable, no global install.
- Task file = YAML frontmatter (id, title, status, priority, labels,
  dependencies, created/updated) + Markdown sections (Description,
  Acceptance Criteria checklists, Implementation Plan, Implementation
  Notes, Definition of Done, Final Summary).
- Config knobs (verified in src): `statuses` free-form list but terminal
  status = LAST entry only; `task_prefix` (PB -> PB-001 style IDs);
  `zero_padded_ids`; `types` configurable; `priorities` ordered labels;
  `backlog_directory` chosen at init.
- Lifecycle mapping: terminal Done -> `backlog task complete` moves file
  to completed/; dropped -> `backlog task archive` moves to
  archive/tasks/ (retained, out of views) — archive is our DROPPED
  equivalent, rationale recorded in Final Summary. Single terminal
  status means Done must be last in statuses; Dropped is NOT a status.
- Board columns show statuses in order; `backlog board` terminal Kanban,
  `backlog browser` web UI on localhost.

Pivot plan: statuses [Backlog, Ready, In Progress, Blocked, Review, Done];
priorities [P0, P1, P2, P3] (first sorts highest); types [feature, bug,
research, refactor, tech-debt, docs]; task_prefix PB; zero_padded_ids 3
digit; backlog_directory docs/product/backlog (replacing the Org files in
place); SIZE and AREA become labels (size:S/M/L, area:<x>) since
Backlog.md has no native fields for them; DEPENDS_ON becomes native
`dependencies`. The 40 Org items convert 1:1 to Backlog.md task files
with the same PB-NNN IDs; docs/product/README.md contract is rewritten
for the new format; nvim orgmode spec is reverted; the three OpenCode
backlog skills are rewritten to drive `backlog` CLI commands.
<!-- SECTION:NOTES:END -->

## Final Summary

<!-- SECTION:FINAL_SUMMARY:BEGIN -->Product backlog lives in Backlog.md task files under
docs/product/backlog/ (tasks/ active, completed/ accepted, archive/
dropped) with stable PB-NNN IDs and the docs/product/README.md contract
(st statuses Backlog..Done, priorities P0-P3, six types, size/area
labels, native dependencies). The backlog executable is repo-scoped
through the dev shell via a pinned flake input. Docs tree is
purpose-named (guides/ reference/ product/ design/ adr/ reports/
maintenance/). AGENTS.md pins agent boundaries: begin Ready work only,
stop at Review, human-only Done; dropping = Final Summary rationale +
task archive. Verified: backlog doctor clean over all items; board and
task list render correctly through nix develop -c; link sweep zero
broken; fmt/clippy/build and nix flake check green; hand-written-file
compatibility proven (CLI parses converted files, complete and archive
flows tested).
Org-mode product backlog with 40 migrated items (39 active, 1 dropped
with rationale; completed/ empty, .gitkeep), deterministic IDs in the
old BACKLOG.md document order, controlled vocabulary per contract.
Docs tree purpose-named: guides/ reference/ product/ design/ adr/
reports/ maintenance/; docs/development/ and docs/BACKLOG.md gone.
Product contract at docs/product/README.md; AGENTS.md lifecycle section
points at it and pins the agent boundaries (READY entry, REVIEW stop,
human-only DONE).

Interfaces: nvim-orgmode agenda (overview o / ready r / needs-attention
n, numeric priorities, custom keyword faces, active/ glob only) and
three OpenCode skills (add/refine/implement product-backlog-item) over
the same Git files; documentation-hygiene marker moved to
docs/maintenance/ with the skill made repo-aware.

Validation: validator script 40/40 unique IDs + full grammar; link sweep
zero broken; fmt/clippy/build/nix-flake-check green; headless nvim smoke
proves agenda discovery of all 39 active items and custom-command
filtering. Known limitation: nvim-orgmode 0.7.3 %w+ keyword matcher
cannot express IN_PROGRESS positively (underscore); overview uses
/!-DONE-DROPPED; recorded in the nvim spec comments.
<!-- SECTION:FINAL_SUMMARY:END -->
