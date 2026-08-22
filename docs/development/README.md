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
| [native-sim-replacement-research-plan.md](native-sim-replacement-research-plan.md) | Research plan and step-by-step evidence gates for replacing all native_sim/NCS test dependencies with a lightweight serial-device fixture and stateful peers for every shipped protocol. |
| [native-sim-virtual-serial-candidate-survey.md](native-sim-virtual-serial-candidate-survey.md) | Stage 1 source-linked survey and provisional scorecard for Rust PTY, native utility, scripting, driver, and full-emulator boundary candidates; shortlists prototypes without selecting a replacement. |
| [native-sim-replacement-research-progress.md](native-sim-replacement-research-progress.md) | Resumable research checkpoint: current baseline, traceability/coupling findings, candidate and protocol conclusions, prototype failures/results, unresolved decisions, and next work. |
| [native-sim-test-traceability.md](native-sim-test-traceability.md) | Test-level disposition for all 49 native cases plus path-level NCS/native_sim coupling and parity-gated deletion actions. |
| [native-sim-protocol-peer-worksheets.md](native-sim-protocol-peer-worksheets.md) | Per-preset/framing/parser simulator contract, state model, independent oracle, vectors, fragmentation, fault/recovery cases, and future coverage-drift metadata. |
| [native-sim-boundary-prototype-results.md](native-sim-boundary-prototype-results.md) | Reproducible Linux 100-run comparison of direct nix, rustix, and Python PTYs; recommends nix conditionally and records public peer-disconnect blocker. |
| [native-sim-emulator-core-research.md](native-sim-emulator-core-research.md) | Stage 3 observable state model, layered fixture architecture, hybrid clock, explicit queue/fault/ownership contract, and passing disposable core prototypes. |
| [native-sim-replacement-recommendation.md](native-sim-replacement-recommendation.md) | Combined recommendation, exact dependency/architecture choice, blocker, tradeoffs, differential strategy, review-sized phases, rollback points, final deletion set, and verification gates. |
| [safe-continuous-capture-design.md](safe-continuous-capture-design.md) | Design for continuous disk capture — **NOT implemented**; recommendation is to wait for concrete task evidence. The shipped foundation (bounded `export_log` store) is documented in [persistent-capture.md](../persistent-capture.md) and AGENTS.md. |
| [windows-serial-e2e-investigation.md](windows-serial-e2e-investigation.md) | Windows serial E2E decision record: deferred — no privileged virtual-port driver install on GitHub-hosted runners; needs a pre-provisioned signed-driver runner or an approved design. |

Reproduce the evaluation:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```
