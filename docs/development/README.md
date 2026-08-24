# Development notes

Index of active development documentation. Remove consumed implementation plans
and phase handoffs after their work completes. Git history preserves that
history. Durable behavior lives in `AGENTS.md`, the changelog, and the focused
documents below.

| Document | Purpose |
|---|---|
| [FEATURES.md](FEATURES.md) | Active roadmap and technical debt. Shipped items are in [CHANGELOG.md](../../CHANGELOG.md) and [AGENTS.md](../../AGENTS.md), not here. |
| [mcp-version-compatibility-policy.md](mcp-version-compatibility-policy.md) | MCP compatibility contract: supported versions, permanent `2025-11-25` retention, admission checklist, proof layers, and the one-command compatibility gate. |
| [protocol-matrix.md](protocol-matrix.md) | Support and status matrix for protocols with a cited spec in `resources/`. |
| [agent-interface-baseline.json](agent-interface-baseline.json) | Committed historical evaluator baseline with 26 tools and 258964 bytes. `xtask agent-eval --baseline` compares the live catalog with it. |
| [agent-interface-evaluation.md](agent-interface-evaluation.md) | Current evaluator report with the 25-tool catalog, interface decisions, thresholds, and limitations. |
| [safe-continuous-capture-design.md](safe-continuous-capture-design.md) | Future continuous disk capture design. It is not implemented. Wait for concrete task evidence. The shipped bounded `export_log` store is documented in [persistent-capture.md](../persistent-capture.md) and `AGENTS.md`. |
| [windows-serial-e2e-investigation.md](windows-serial-e2e-investigation.md) | Windows serial E2E decision record. Deferred because GitHub-hosted runners do not install privileged virtual-port drivers. A pre-provisioned signed-driver runner or an approved design is required. |

## Reproduce the evaluation

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```
