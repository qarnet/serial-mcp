# Development Notes

Index of durable development documentation: policies, references, and the
active plans folder. Planned and in-progress work is indexed in the
[product backlog](../BACKLOG.md) instead of this file.

| Doc | What it is |
|---|---|
| [mcp-version-compatibility-policy.md](mcp-version-compatibility-policy.md) | Durable MCP protocol compatibility contract: supported versions, permanent `2025-11-25` retention, admission checklist, proof layers, and the one-command compatibility gate. |
| [protocol-matrix.md](protocol-matrix.md) | Support/status matrix for every protocol with a cited spec in `resources/`. |
| [agent-interface-baseline.json](agent-interface-baseline.json) | Committed historical evaluator baseline (26 tools, 258964 bytes), kept so `xtask agent-eval --baseline` can diff the live catalog against it. |
| [agent-interface-evaluation.md](agent-interface-evaluation.md) | Consolidated, current evaluator report: 25-tool catalog, accepted/rejected interface decisions with thresholds, limitations. |
| [windows-serial-e2e-investigation.md](windows-serial-e2e-investigation.md) | Windows serial E2E decision record: deferred — no privileged virtual-port driver install on GitHub-hosted runners; needs a pre-provisioned signed-driver runner or an approved design. |
| [documentation-hygiene.md](documentation-hygiene.md) | Marker managed by the OpenCode `documentation-hygiene` skill; records the last full-repository audit. |

## Plans

[plans/](plans/) holds working design documents for backlog entries —
transient by design, deleted when their work ships or is abandoned. Do not
keep implemented plans there; that folder is an index of active work, not an
archive.

The [product backlog](../BACKLOG.md) documents the entry lifecycle.

Reproduce the evaluation:

```bash
cargo run --manifest-path xtask/Cargo.toml -- agent-eval \
  --baseline docs/development/agent-interface-baseline.json
```