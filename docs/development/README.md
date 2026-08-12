# Development Notes

Index of active development documentation. Consumed implementation plans and
phase handoffs are removed after their work completes; commits preserve that
history, while durable behavior lives in `AGENTS.md`, the changelog, and the
focused documents below.

| Doc | What it is |
|---|---|
| [FEATURES.md](FEATURES.md) | Active roadmap + tech debt. Shipped items live in [CHANGELOG.md](../../CHANGELOG.md) and [AGENTS.md](../../AGENTS.md), not here. |
| [mcp-version-compatibility-policy.md](mcp-version-compatibility-policy.md) | Durable MCP protocol compatibility contract: supported versions, permanent `2025-11-25` retention, admission checklist, proof layers, and the one-command compatibility gate. |
| [protocol-matrix.md](protocol-matrix.md) | Support/status matrix for every protocol with a cited spec in `resources/`. |
| [agent-interface-baseline.json](agent-interface-baseline.json) | Committed historical evaluator baseline (26 tools, 258964 bytes), kept so `xtask agent-eval --baseline` can diff the live catalog against it. |
| [agent-interface-evaluation.md](agent-interface-evaluation.md) | Consolidated, current evaluator report: 25-tool catalog, accepted/rejected interface decisions with thresholds, limitations. |
| [safe-continuous-capture-design.md](safe-continuous-capture-design.md) | Design for continuous disk capture — **NOT implemented**; recommendation is to wait for concrete task evidence. The shipped foundation (bounded `export_log` store) is documented in [persistent-capture.md](../persistent-capture.md) and AGENTS.md. |
| [windows-serial-e2e-investigation.md](windows-serial-e2e-investigation.md) | Windows serial E2E decision record: deferred — no privileged virtual-port driver install on GitHub-hosted runners; needs a pre-provisioned signed-driver runner or an approved design. |

Reproduce the evaluation:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```
