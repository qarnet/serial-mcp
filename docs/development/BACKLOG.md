# Product backlog

Planned and in-progress work that is not yet a shipped feature. One line per
entry; each entry links to its design or plan document when one exists.

| Status | Entry | Reference |
| --- | --- | --- |
| In progress | Server-runtime ownership: one `SerialServerRuntime` per server (shared TX queue, reconnect supervisor, deterministic shutdown), cross-process port leases, platform portability | [stateless-http-runtime-plan.md](stateless-http-runtime-plan.md) |
| Planned | CI Nix timing measurement and cache investigation: record cold/warm restore-save and `nix flake check` durations, inspect 8 GiB store-cache retention, before any further Nix tuning | ci-runtime-reduction plan (implemented and removed; measurement scope restated below) |
| Planned | Windows native serial-open gap: decide the `mio-serial` close/reopen contract before claiming external-program exclusion | [stateless-http-runtime-plan.md](stateless-http-runtime-plan.md) (ownership research) |

Entry lifecycle:

- A plan or research document is written and committed while its backlog entry
  is `Planned` or `In progress`, and the backlog references it.
- When the work ships, the entry moves out of the table and the plan document
  is deleted (see `README.md` docs lifecycle); `AGENTS.md`, `CHANGELOG.md`, and
  guides own shipped behavior.
- Abandoned work: delete the entry together with its plan document.

Nix measurement scope (from the removed CI plan, kept verbatim so the follow-up
is self-contained):

1. Record cache restore/save duration and `nix flake check` duration over
   several clean runs and runs with only source changes.
2. Inspect whether the 8 GiB Nix-store cache retains dependency outputs or is
   repeatedly evicted under the repository cache quota.
3. Record cold versus warm time before changing cache keys, store limits, or
   source filtering.