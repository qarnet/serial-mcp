# PB-040 — Org-mode product backlog and documentation lifecycle

Status: pivot implemented, PB-040 in Review. Post-review policy evolution
(2026-09-04): the human-only Done gate was relaxed to the PR gate — an
agent may take an item to Done when acceptance criteria are
evidence-backed, gates are green, the Final Summary is filled, and the
Done transition is committed together with the work in one PR titled
with the item ID; the human's PR merge is the official acceptance. This
document is a point-in-time record of the Org iteration and the pivot
decision, not the current policy; docs/product/README.md owns the live
contract. Org iteration rejected at human review; final format Backlog.md (same storage model). History of both iterations lives in the PB-040 item's Implementation Notes. Design for backlog item `PB-040` (this document is the
design the new product system prescribes: substantial cross-cutting work gets a
standalone design; the implementation plan lives inside the backlog item).

This design is transitional. When the work completes, durable information moves
into the product contract (`docs/product/README.md`), this document may be
removed.

---

## Goal

Replace the Markdown backlog table with one Org file per product item, split
the docs tree into purpose-named directories, and wire the human (Neovim Org
agenda) and agent (OpenCode skills) interfaces over the same Git files. The
backlog data store is Git; Neovim and OpenCode are views, not the store.

Derived from the handoff dated 2026-09-04. All handoff sections verified
against the actual environment before this plan was written.

## Handoff issues found during evaluation (resolved)

1. **Neovim plugin install path.** The NixOS flake only provides the nvim
   binary (`programs.neovim.enable = true` in `modules/sw-dev-base.nix`); no
   home-manager, no plugin management. The Neovim config is a separate
   LazyVim repo (`qarnet/neovim-conf`) using lazy.nvim with `lazy-lock.json`.
   Orgmode IS a normal Neovim plugin, so it is installed through lazy.nvim
   like every other plugin in that repo (user decision 2026-09-04: prefer the
   lazy.nvim plugin; nixpkgs path only as fallback). The "Nixpkgs through the
   flake" constraint is relaxed per user instruction.
2. **`docs/development/README.md` has no destination** in the handoff
   taxonomy. It is an index; its rows redistribute. Dissolve it into
   `docs/README.md`.
3. **documentation-hygiene skill vs this restructure.** The skill hardcodes
   the marker path `docs/development/documentation-hygiene.md` in four
   places. User decision: serial-mcp leads best practice, the skill follows.
   Marker moves to `docs/maintenance/documentation-hygiene.md`; the skill is
   updated to match the new layout (repo-aware serial-mcp path, generic
   default kept).
4. **Keep or delete documentation-hygiene vs unslop?** Evaluated: keep both.
   unslop is a style rule set for prose; hygiene is a repository audit process
   (file selection, placement standards, marker protocol, delegation rules).
   Overlap is small; unslop is a tool hygiene uses during rewrites.
5. **Opencode config repo has in-flight user changes** (agents refactor).
   Skill edits and commits must be selective: commit only skill-related
   paths. User decision 2026-09-04: commit everything in the opencode repo,
   all changes valid, push allowed. Selective commit still preferred to keep
   the diff reviewable.
6. **CHANGELOG.md has historical `docs/development/...` references** — never
   rewritten (history stays historically accurate).
7. **Nix flake check and docs moves** — verified the flake source filter
   (flake.nix:42-53) admits only `/schemas`, `/scripts`, and cargo sources;
   docs moves are invisible to `nix flake check`. Per user instruction,
   also evaluated: the flake check contains no tests asserting documentation
   characteristics; `scripts/tests/test_build_registry_manifest.py` asserts
   release-manifest behavior, not doc prose. Nothing to remove. Doc-move
   safety is a property of the source filter, not a dedicated test.
8. **Branch state** — docs branch (`ede074c1`, `caad9a08`) touches only docs;
   PR #78 (`fix/ci-investigation`) touches only `tests/device_fixture.rs`.
   No interaction; this work builds on top of the branch commits.
9. **Handoff §5 example IDs vs §14 migration order** — resolved: IDs are
   allocated in current BACKLOG.md document order (deterministic migration).
10. **Empty `completed/` dir** — Git cannot track empty directories; use
    `.gitkeep` plus README note (no fake example items).
11. **Explicit-skip classification (user confirmed 2026-09-04):**
    - Remote monitor → `DROPPED` (explicit keep-off-roadmap decision).
    - MRTR elicitation → `BACKLOG` P3 (conditional future feature).
    - SECURITY.md disclosure policy → `BACKLOG` P3 (activation condition).

## Phase 1 — Docs taxonomy restructure

Moves (`git mv`, history-preserving):

| From | To |
|---|---|
| `docs/agent-config.md` | `docs/guides/agent-configuration.md` |
| `docs/device-profiles.md` | `docs/guides/device-profiles.md` |
| `docs/persistent-capture.md` | `docs/guides/persistent-capture.md` |
| `docs/protocols.md` | `docs/guides/protocols.md` |
| `docs/rx-and-reading.md` | `docs/guides/rx-and-reading.md` |
| `docs/development/mcp-version-compatibility-policy.md` | `docs/reference/mcp-version-compatibility-policy.md` |
| `docs/development/protocol-matrix.md` | `docs/reference/protocol-matrix.md` |
| `docs/protocols/references.md` | `docs/reference/protocol-specifications.md` (flattens `docs/protocols/`) |
| `docs/development/plans/stateless-http-runtime-plan.md` | `docs/design/PB-001-server-runtime-ownership.md` |
| `docs/development/plans/safe-continuous-capture-design.md` | `docs/design/PB-025-continuous-capture.md` |
| `docs/development/agent-interface-evaluation.md` | `docs/reports/agent-interface-evaluation.md` |
| `docs/development/agent-interface-baseline.json` | `docs/reports/agent-interface-baseline.json` |
| `docs/development/windows-serial-e2e-investigation.md` | `docs/reports/windows-serial-e2e-investigation.md` |
| `docs/development/documentation-hygiene.md` | `docs/maintenance/documentation-hygiene.md` (byte-identical) |
| `docs/adr/*` | unchanged |

Deletes: `docs/BACKLOG.md`, `docs/development/README.md`,
`docs/development/` entirely, empty `docs/protocols/`.

New files:
- `docs/README.md` — rewritten index (guides/reference/product/design/adr/
  reports/maintenance); no duplicate item list (the agenda is the index).
- `docs/product/README.md` — canonical contract (format, states, priorities,
  ID grammar, vocabulary, sections, ownership, DoR/DoReview/DoDone, archival).
- `docs/product/backlog/TEMPLATE.org`.
- `docs/product/backlog/active/`, `completed/` (`.gitkeep`), `dropped/`.

Active-reference updates (CHANGELOG untouched):
- root `README.md` (~docs section), `AGENTS.md` (lifecycle section rewrite +
  L241 windows investigation, L401 policy doc, L491 evaluator paths, L512
  capture design), `.github/workflows/hardening.yml` L6,
  `src/checksums.rs` L12 (`docs/BACKLOG.md` ref), moved docs' internal
  cross-refs, `docs/adr/README.md` if it references development/.

## Phase 2 — Backlog migration (40 items, deterministic IDs)

Mapping per handoff §14: `In progress`→`IN_PROGRESS` P1; `Near-term`→`BACKLOG`
P1; `Later`→`BACKLOG` P2; `Wish`→`BACKLOG` P3; `Infrastructure`→TYPE
`tech-debt` + independent priority. Status labels are never copied as states.
No invented requirements: one-paragraph ideas get Problem + Desired outcome +
open questions, stay `BACKLOG`, SIZE blank. L items stay BACKLOG until refined.

| ID | Slug | TYPE | AREA | P | State |
|---|---|---|---|---|---|
| PB-001 | server-runtime-ownership | refactor | runtime | 1 | IN_PROGRESS |
| PB-002 | ci-nix-timing-measurement | tech-debt | ci | 1 | BACKLOG |
| PB-003 | windows-serial-open-gap | research | platform | 1 | BACKLOG |
| PB-004 | usage-statistics | feature | mcp | 1 | BACKLOG |
| PB-005 | declarative-checksums | feature | framing | 1 | BACKLOG |
| PB-006 | tx-pacing | feature | tx | 1 | BACKLOG |
| PB-007 | modbus-tx-auto-lrc | feature | protocol | 1 | BACKLOG |
| PB-008 | config-import-export | feature | profiles | 1 | BACKLOG |
| PB-009 | external-decoder-api | feature | framing | 1 | BACKLOG |
| PB-010 | decoder-integration-hooks | feature | capture | 1 | BACKLOG |
| PB-011 | mcp-tasks-extension | feature | mcp | 2 | BACKLOG |
| PB-012 | positive-cache-ttl | feature | mcp | 2 | BACKLOG |
| PB-013 | http-parameter-headers | feature | mcp | 2 | BACKLOG |
| PB-014 | ring-backpressure | feature | rx | 2 | BACKLOG |
| PB-015 | per-connection-decoder | feature | rx | 2 | BACKLOG |
| PB-016 | per-client-rx-cursors | feature | rx | 2 | BACKLOG |
| PB-017 | baud-rate-auto-detection | feature | serial | 2 | BACKLOG |
| PB-018 | modem-lines-uart-counters | feature | serial | 2 | BACKLOG |
| PB-019 | per-frame-timestamps | feature | rx | 2 | BACKLOG |
| PB-020 | grbl-gcode-preset | feature | protocol | 2 | BACKLOG |
| PB-021 | safety-policies | feature | security | 2 | BACKLOG |
| PB-022 | capture-bookmarks | feature | capture | 2 | BACKLOG |
| PB-023 | expect-script-automation | feature | mcp | 2 | BACKLOG |
| PB-024 | capture-filtering-search | feature | capture | 2 | BACKLOG |
| PB-025 | recording-and-replay | feature | capture | 2 | BACKLOG |
| PB-026 | rs-485-options | feature | serial | 2 | BACKLOG |
| PB-027 | rfc2217-backend | feature | platform | 2 | BACKLOG |
| PB-028 | bridge-mode | feature | platform | 2 | BACKLOG |
| PB-029 | mcpb-distribution | feature | distribution | 2 | BACKLOG |
| PB-030 | earlier-mcp-revisions | feature | mcp | 3 | BACKLOG |
| PB-031 | loopback-virtual-port | feature | platform | 3 | BACKLOG |
| PB-032 | socket-sharing-tee | feature | runtime | 3 | BACKLOG |
| PB-033 | file-transfer-helpers | feature | protocol | 3 | BACKLOG |
| PB-034 | passive-sniffing | feature | platform | 3 | BACKLOG |
| PB-035 | shared-session-tee | feature | runtime | 3 | BACKLOG |
| PB-036 | uint-newtype | tech-debt | schema | 1 | BACKLOG |
| PB-037 | mrtr-elicitation | feature | mcp | 3 | BACKLOG |
| PB-038 | remote-monitor | feature | runtime | 3 | DROPPED (dropped/) |
| PB-039 | security-disclosure-policy | docs | security | 3 | BACKLOG |
| PB-040 | org-backlog-migration | docs | documentation | 1 | IN_PROGRESS |

Dependencies: PB-009 DEPENDS_ON PB-005; PB-020 DEPENDS_ON PB-006. Relations
that are not true sequencing (PB-010↔PB-009, PB-016↔PB-032, PB-035↔PB-032)
go in Technical context, not DEPENDS_ON.

## Phase 3 — OpenCode skills (global `~/.config/opencode/skills/`)

1. `add-product-backlog-item` — standardized item creation; reads the repo's
   `docs/product/README.md` first (repo policy authoritative over the
   skill); ID allocation across active/completed/dropped (max+1, collision
   check before write); defaults BACKLOG + P2; never fabricates requirements,
   never promotes to READY, never edits history dirs.
2. `refine-product-backlog-item` — idea→spec transition; may edit
   product-owned sections; READY only when Definition of Ready holds.
3. `implement-product-backlog-item` — boundary READY→IN_PROGRESS→DONE (PR-gated);
   fills execution-owned sections; never DONE, never moves to completed/.
4. `audit-product-backlog` + acceptance automation — NOT created (handoff
   §12.4/12.5: later, evidence-driven).
5. `documentation-hygiene` update — marker path repo-aware (serial-mcp:
   `docs/maintenance/documentation-hygiene.md`), required-exception wording,
   placement tree updated to new taxonomy. Kept (see issue 4).
   Opencode repo commit: all changes valid per user; commit skills separately
   from the in-flight agents refactor for reviewability; push allowed.

## Phase 4 — Neovim (repo `qarnet/neovim-conf`)

Orgmode installed via lazy.nvim like every other plugin (user decision;
nixpkgs fallback not needed). New `lua/plugins/orgmode.lua`:

- `org_todo_keywords`: `BACKLOG READY IN_PROGRESS BLOCKED REVIEW | DONE DROPPED`
- numeric priorities: `org_priority_highest = 0`, `org_priority_default = 2`,
  `org_priority_lowest = 3`
- `org_agenda_files`: `~/repos/serial-mcp/docs/product/backlog/active/**/*`
  (completed/dropped excluded from default agenda; TEMPLATE.org outside
  active/ so it never appears)
- treesitter `ensure_installed` += `org`
- custom agenda commands:
  - product overview (all active states; `todo-state-up`, `priority-down`)
  - ready queue (READY; `priority-down`)
  - needs attention (REVIEW + BLOCKED — REVIEW prominent, human action)
  - property search for TYPE/AREA/SIZE via `+TYPE="feature"` style match
    (nvim-orgmode advanced search supports property queries — verified
    against nvim-orgmode configuration docs)
- state changes from the agenda via built-in TODO cycling.

## Phase 5 — Verification

1. Link sweep: `rg -l "docs/development|docs/BACKLOG.md|protocols/references"`
   over tracked files minus CHANGELOG.md = zero hits.
2. One-off Org validator in `/tmp` (NOT committed — handoff §19 forbids an
   in-repo parser): unique IDs, filename=headline=ID agreement, state/
   priority/TYPE/SIZE/AREA vocabulary, required sections, DEPENDS_ON targets
   exist, directory matches state.
3. `cargo fmt/clippy/build --locked` (only a comment changes in src/) +
   `nix flake check --accept-flake-config` (proves docs invisible to crane).
4. Read-through of docs READMEs; handoff §20 checklist item-by-item against
   the PB-040 item.
5. nvim headless smoke: plugin loads, agenda opens on serial-mcp backlog,
   custom commands respond, TODO cycling works.

## Phase 6 — Commits (conventional, no attribution footers)

1. serial-mcp docs branch: one atomic commit `docs: replace Markdown backlog
   with Org-mode product backlog` (restructure + migration + contract land
   together; intermediate states dangle refs). Push. PR after user review.
2. nvim repo: `feat: orgmode product backlog agenda`.
3. opencode repo: `feat: product backlog skills, maintenance marker path`.
   (No nixos-config-flake change needed — plugin comes from lazy.nvim.)

## Non-goals (locked, handoff §19)

No generated index, no CI enforcement of human identity, no DONE automation,
no per-state subdirectories, no second roadmap source, no production
serial-mcp behavior change, no in-repo backlog parser.