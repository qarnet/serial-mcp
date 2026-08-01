# Development Notes

Index of the active development documentation. Historical phase handoffs and
per-phase evaluation reports were removed in the 2026-08 agent-ergonomics
cleanup — the consolidated evaluation below is the single current record.

| Doc | What it is |
|---|---|
| [FEATURES.md](FEATURES.md) | Active roadmap + tech debt. Shipped items live in [CHANGELOG.md](../../CHANGELOG.md) and [AGENTS.md](../../AGENTS.md), not here. |
| [post-0.9-refinement-plan.md](post-0.9-refinement-plan.md) | Active one-branch/one-PR plan for schema integrity, RX parity, internal refinement, and the 0.9.1 release. |
| [protocol-matrix.md](protocol-matrix.md) | Support/status matrix for every protocol with a cited spec in `resources/`. |
| [agent-interface-baseline.json](agent-interface-baseline.json) | Committed Phase 4 evaluator baseline (26 tools, 258964 bytes) — **historical**, kept so `xtask agent-eval --baseline` can diff the live catalog against it. |
| [agent-interface-evaluation.md](agent-interface-evaluation.md) | Consolidated, current evaluator report: 27-tool catalog, accepted/rejected interface decisions with thresholds, limitations. |
| [safe-continuous-capture-design.md](safe-continuous-capture-design.md) | Design for continuous disk capture — **NOT implemented**; recommendation is to wait for concrete task evidence. The shipped foundation (bounded `export_log` store) is documented in the README and AGENTS.md. |

Reproduce the evaluation:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```
