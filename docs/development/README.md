# Development Notes

Index of active development documentation. Retained migration plans, research,
and batch handoffs are historical records where they describe removed
`native_sim`/NCS paths; durable behavior lives in `AGENTS.md`, the changelog,
and the focused documents below.

| Doc | What it is |
|---|---|
| [FEATURES.md](FEATURES.md) | Active roadmap + tech debt. Shipped items live in [CHANGELOG.md](../../CHANGELOG.md) and [AGENTS.md](../../AGENTS.md), not here. |
| [mcp-version-compatibility-policy.md](mcp-version-compatibility-policy.md) | Durable MCP protocol compatibility contract: supported versions, permanent `2025-11-25` retention, admission checklist, proof layers, and the one-command compatibility gate. |
| [protocol-matrix.md](protocol-matrix.md) | Support/status matrix for every protocol with a cited spec in `resources/`. |
| [agent-interface-baseline.json](agent-interface-baseline.json) | Committed historical evaluator baseline (26 tools, 258964 bytes), kept so `xtask agent-eval --baseline` can diff the live catalog against it. |
| [agent-interface-evaluation.md](agent-interface-evaluation.md) | Consolidated, current evaluator report: 25-tool catalog, accepted/rejected interface decisions with thresholds, limitations. |
| [storage-hygiene-plan.md](storage-hygiene-plan.md) | Phase 1–3 storage-hygiene work implemented; only the profile-size experiment remains planned. |
| [native-sim-replacement-research-plan.md](native-sim-replacement-research-plan.md) | Historical research plan and evidence gates for replacing native_sim/NCS test dependencies with the lightweight serial-device fixture and stateful peers. |
| [native-sim-virtual-serial-candidate-survey.md](native-sim-virtual-serial-candidate-survey.md) | Historical Stage 1 source-linked survey and provisional scorecard for Rust PTY, native utility, scripting, driver, and full-emulator boundary candidates. |
| [native-sim-replacement-research-progress.md](native-sim-replacement-research-progress.md) | Historical resumable research checkpoint with traceability/coupling findings, candidate and protocol conclusions, prototype failures/results, and Phase F source-removal record. |
| [native-sim-test-traceability.md](native-sim-test-traceability.md) | Historical test-level disposition for all 49 native cases plus retained replacement-proof mapping and source-removal record. |
| [native-sim-protocol-peer-worksheets.md](native-sim-protocol-peer-worksheets.md) | Historical per-preset/framing/parser simulator contract, state model, independent oracle, vectors, fragmentation, fault/recovery cases, and coverage-drift metadata. |
| [native-sim-boundary-prototype-results.md](native-sim-boundary-prototype-results.md) | Historical reproducible Linux 100-run comparison of direct nix, rustix, and Python PTYs, including the resolved public peer-disconnect blocker. |
| [native-sim-emulator-core-research.md](native-sim-emulator-core-research.md) | Historical Stage 3 observable state model, layered fixture architecture, hybrid clock, explicit queue/fault/ownership contract, and disposable core prototypes. |
| [native-sim-replacement-recommendation.md](native-sim-replacement-recommendation.md) | Historical combined recommendation, dependency/architecture choice, differential strategy, review-sized phases, deletion set, and verification gates. |
| [safe-continuous-capture-design.md](safe-continuous-capture-design.md) | Design for continuous disk capture — **NOT implemented**; recommendation is to wait for concrete task evidence. The shipped foundation (bounded `export_log` store) is documented in [persistent-capture.md](../persistent-capture.md) and AGENTS.md. |
| [windows-serial-e2e-investigation.md](windows-serial-e2e-investigation.md) | Windows serial E2E decision record: deferred — no privileged virtual-port driver install on GitHub-hosted runners; needs a pre-provisioned signed-driver runner or an approved design. |

Reproduce the evaluation:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```
