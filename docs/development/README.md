# Development Notes

Index of the active development documentation. The post-0.9 refinement
plan and its phase handoffs (Phases 1–8) were consumed and removed when the
branch landed — commits preserve implementation history, and the
consolidated evaluation below is the single current record.

| Doc | What it is |
|---|---|
| [FEATURES.md](FEATURES.md) | Active roadmap + tech debt. Shipped items live in [CHANGELOG.md](../../CHANGELOG.md) and [AGENTS.md](../../AGENTS.md), not here. |
| [code-cleanup-plan.md](code-cleanup-plan.md) | Completed behavior-preserving cleanup plan (implementation done, final gate passed, awaiting delivery review): shared TX transformations, RX readability, profile construction, and cross-platform test-harness consolidation. |
| [protocol-matrix.md](protocol-matrix.md) | Support/status matrix for every protocol with a cited spec in `resources/`. |
| [agent-interface-baseline.json](agent-interface-baseline.json) | Committed Phase 4 evaluator baseline (26 tools, 258964 bytes) — **historical**, kept so `xtask agent-eval --baseline` can diff the live catalog against it. |
| [agent-interface-evaluation.md](agent-interface-evaluation.md) | Consolidated, current evaluator report: 27-tool catalog, accepted/rejected interface decisions with thresholds, limitations. |
| [safe-continuous-capture-design.md](safe-continuous-capture-design.md) | Design for continuous disk capture — **NOT implemented**; recommendation is to wait for concrete task evidence. The shipped foundation (bounded `export_log` store) is documented in the README and AGENTS.md. |
| [windows-serial-e2e-investigation.md](windows-serial-e2e-investigation.md) | Windows serial E2E decision record: deferred — no privileged virtual-port driver install on GitHub-hosted runners; needs a pre-provisioned signed-driver runner or an approved design. |

Reproduce the evaluation:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```
