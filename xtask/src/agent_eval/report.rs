//! Report rendering for `xtask agent-eval`: `report.json` (the committed
//! baseline format) and `report.md` (the human-readable decision record).
//!
//! Determinism: no timestamps, hostnames, absolute paths, network, or user
//! configuration appear in either file; reruns are byte-identical.

use anyhow::Result;

use super::EvalReport;

/// Render the machine-readable baseline (`report.json`).
pub fn render_json(report: &EvalReport) -> Result<String> {
    let text = serde_json::to_string_pretty(report)?;
    Ok(text)
}

/// Render the human-readable decision record (`report.md`).
pub fn render_md(report: &EvalReport) -> Result<String> {
    let mut out = String::new();
    out.push_str("# Agent-Interface Evaluation Report\n\n");
    out.push_str("Deterministic local measurement: no network access, no user profiles, ");
    out.push_str("no hardware, no timestamps. Rerunning `xtask agent-eval` reproduces this ");
    out.push_str("report byte-for-byte (except explicitly excluded presentation paths).\n\n");

    // Catalog
    out.push_str("## Tool catalog (`tools/list` payload)\n\n");
    out.push_str(&format!(
        "- tool count: **{}**\n",
        report.catalog.tool_count
    ));
    out.push_str(&format!(
        "- aggregate compact payload: **{} bytes** (whole `{{\"tools\":[...]}}` result)\n",
        report.catalog.aggregate_bytes
    ));
    out.push_str("\nByte metric: compact `serde_json` serialization of the tool list and of ");
    out.push_str("each schema — no HTTP/SSE headers, no pretty-print whitespace.\n\n");
    out.push_str("| tool | total | description | input schema | output schema |\n");
    out.push_str("|---|---|---|---|---|\n");
    for tool in &report.catalog.per_tool_bytes {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            tool.name,
            tool.total_bytes,
            tool.description_bytes,
            tool.input_schema_bytes,
            tool.output_schema_bytes
        ));
    }
    out.push_str("\nLargest tools:\n\n");
    for tool in &report.catalog.top_largest {
        out.push_str(&format!("- `{}`: {} bytes\n", tool.name, tool.total_bytes));
    }

    // Scenarios
    out.push_str("\n## Scenario metrics\n\n");
    out.push_str("Fixed normalized placeholders (`/dev/ttyACM0`, fixed UUID). Request bytes = ");
    out.push_str("compact JSON of fixed-ID MCP `tools/call` envelopes.\n\n");
    out.push_str("| scenario | calls | bytes | invalid | retries | advanced fields | stale/race | completion reference |\n");
    out.push_str("|---|---|---|---|---|---|---|---|\n");
    for s in &report.scenarios {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | `{}` |\n",
            s.id,
            s.tool_calls,
            s.request_bytes,
            s.invalid_calls,
            s.retries,
            s.advanced_fields,
            if s.stale_race { "yes" } else { "no" },
            s.completion_ref,
        ));
    }
    out.push_str("\nModeled (hypothetical, NOT implemented) variants and their expansion into current calls:\n\n");
    out.push_str(
        "| scenario | kind | calls | bytes | expansion calls | expansion bytes | note |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|\n");
    for s in &report.scenarios {
        if let Some(m) = &s.modeled {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} |\n",
                s.id,
                m.kind,
                m.tool_calls,
                m.request_bytes,
                m.expansion_calls,
                m.expansion_bytes,
                m.note,
            ));
        }
    }

    // Comparisons + decisions
    out.push_str("\n## Comparisons\n\n");
    out.push_str(&format!(
        "- automatic profile reuse vs explicit management: {} calls vs {} (savings {}), {} bytes vs {} ({:+.1}%)\n",
        report.metrics.automatic_profiles.alternative_calls,
        report.metrics.automatic_profiles.current_calls,
        report.metrics.automatic_profiles.call_savings,
        report.metrics.automatic_profiles.alternative_bytes,
        report.metrics.automatic_profiles.current_bytes,
        report.metrics.automatic_profiles.byte_reduction_pct,
    ));
    out.push_str(&format!(
        "- `transact` vs `write`+`read`: {} calls vs {} (savings {}), {} bytes vs {} ({:+.1}%)\n",
        report.metrics.transact_vs_write_read.alternative_calls,
        report.metrics.transact_vs_write_read.current_calls,
        report.metrics.transact_vs_write_read.call_savings,
        report.metrics.transact_vs_write_read.alternative_bytes,
        report.metrics.transact_vs_write_read.current_bytes,
        report.metrics.transact_vs_write_read.byte_reduction_pct,
    ));
    for c in &report.metrics.shorthand_comparisons {
        out.push_str(&format!(
            "- shorthand ({}) vs current: {} calls vs {} (savings {}), {} bytes vs {} ({:+.1}%)\n",
            c.scenario,
            c.alternative_calls,
            c.current_calls,
            c.call_savings,
            c.alternative_bytes,
            c.current_bytes,
            c.byte_reduction_pct,
        ));
    }
    for c in &report.metrics.recipe_comparisons {
        out.push_str(&format!(
            "- recipe ({}) vs current: {} calls vs {} (savings {}), {} bytes vs {} ({:+.1}%), repeated advanced objects removed: {}\n",
            c.scenario, c.alternative_calls, c.current_calls, c.call_savings,
            c.alternative_bytes, c.current_bytes, c.byte_reduction_pct, c.reduced_advanced_objects,
        ));
    }
    for c in &report.metrics.facade_comparisons {
        out.push_str(&format!(
            "- facade ({}) vs current: {} calls vs {} (savings {}), {} bytes vs {} ({:+.1}%)\n",
            c.scenario,
            c.alternative_calls,
            c.current_calls,
            c.call_savings,
            c.alternative_bytes,
            c.current_bytes,
            c.byte_reduction_pct,
        ));
    }
    out.push_str(&format!(
        "- capture_boot vs current composition: {} calls vs {} (savings {}), {} bytes vs {} ({:+.1}%), stale/race window: {}\n",
        report.metrics.capture_boot.alternative_calls,
        report.metrics.capture_boot.current_calls,
        report.metrics.capture_boot.call_savings,
        report.metrics.capture_boot.alternative_bytes,
        report.metrics.capture_boot.current_bytes,
        report.metrics.capture_boot.byte_reduction_pct,
        if report.metrics.boot_stale_race { "yes" } else { "no" },
    ));

    out.push_str("\n## Decisions (fixed thresholds, evaluated after measurement)\n\n");
    let d = &report.decisions;
    out.push_str(&format!(
        "- automatic profiles: **{}** — {}\n",
        d.automatic_profiles.decision, d.automatic_profiles.reason
    ));
    out.push_str(&format!(
        "- shorthand now: **{}** — {}\n",
        d.shorthand_now.decision, d.shorthand_now.reason
    ));
    out.push_str(&format!(
        "- initial recipes now: **{}** — {}\n",
        d.initial_recipes_now.decision, d.initial_recipes_now.reason
    ));
    out.push_str(&format!(
        "- versioned facade now: **{}** — {}\n",
        d.versioned_facade_now.decision, d.versioned_facade_now.reason
    ));
    out.push_str(&format!(
        "- atomic `capture_boot`: **{}** — {}\n",
        d.phase5_capture_boot.decision, d.phase5_capture_boot.reason
    ));

    out.push_str(&format!(
        "\n## Dominant friction\n\n**{}** — chosen by the fixed rule: schema size if the \
         aggregate `tools/list` payload is >= 64 KiB; else call shape if the median common-task \
         call count is >= 3; else setup if first-connect needs >= 4 calls; else orchestration if \
         any scenario retries. Documentation friction is not measurable by a static harness.\n",
        report.dominant_friction
    ));

    out.push_str(&format!(
        "\n## Catalog regression\n\nStatus: **{}**",
        report.regression.status
    ));
    if let Some(pct) = report.regression.aggregate_growth_pct {
        out.push_str(&format!(" (aggregate growth {pct:.1}%, warning at >=5%)"));
    }
    out.push('\n');
    if report.regression.per_tool_regressions.is_empty() {
        out.push_str("No per-tool regressions (warning at >=10% or +2 KiB per tool).\n");
    } else {
        out.push_str("Per-tool regressions:\n\n");
        for r in &report.regression.per_tool_regressions {
            out.push_str(&format!(
                "- `{}`: {} -> {} bytes ({:+.1}%, {:+}) \n",
                r.name, r.baseline_bytes, r.current_bytes, r.growth_pct, r.growth_bytes
            ));
        }
    }

    out.push_str("\n## Limitations\n\n");
    out.push_str("- A static harness cannot measure model misunderstanding, invalid-call rates ");
    out.push_str("from real agents, or how descriptions steer tool choice.\n");
    out.push_str("- `invalid calls`/`retries` are plan-level facts for the fixed scenarios, not ");
    out.push_str("measured agent behavior.\n");
    out.push_str("- Modeled candidates are hypothetical shapes with explicit expansions into ");
    out.push_str("current calls; they are NOT implemented and their projected catalog growth is ");
    out.push_str("reported as 0% (no new tools) — oneOf-branch growth inside existing schemas is ");
    out.push_str("not modeled.\n");
    out.push_str("- Request bytes exclude transport framing (HTTP/SSE headers) and result ");
    out.push_str("payloads; only request envelopes and the `tools/list` payload are measured.\n");

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_eval::catalog::catalog_metrics;
    use crate::agent_eval::decisions;
    use crate::agent_eval::scenarios;
    use crate::agent_eval::EvalReport;

    fn sample_report() -> EvalReport {
        let catalog = catalog_metrics();
        let scenario_metrics: Vec<_> = scenarios::scenarios()
            .iter()
            .map(scenarios::scenario_metrics)
            .collect();
        let metrics = decisions::aggregate(&scenario_metrics);
        let decisions = decisions::evaluate(&metrics);
        let dominant_friction = decisions::dominant_friction(&metrics, &catalog, &scenario_metrics);
        EvalReport {
            schema: "serial-mcp-agent-interface-eval/v1",
            catalog,
            scenarios: scenario_metrics,
            metrics,
            decisions,
            dominant_friction,
            regression: decisions::CatalogRegression::no_baseline(),
        }
    }

    #[test]
    fn report_json_is_deterministic() {
        let a = render_json(&sample_report()).unwrap();
        let b = render_json(&sample_report()).unwrap();
        assert_eq!(a, b, "report.json must be byte-identical across runs");
        let parsed: serde_json::Value = serde_json::from_str(&a).unwrap();
        // Determinism contract: no timestamps or machine-specific strings.
        let text = a.clone();
        for banned in ["timestamp", "hostname", "/tmp/", "/home/", "epoch"] {
            assert!(!text.contains(banned), "report must not contain {banned}");
        }
        assert_eq!(parsed["catalog"]["tool_count"], serde_json::json!(25));
    }

    #[test]
    fn report_md_covers_decisions_and_limitations() {
        let md = render_md(&sample_report()).unwrap();
        for needle in [
            "tool count",
            "aggregate compact payload",
            "Scenario metrics",
            "Decisions",
            "shorthand now",
            "initial recipes now",
            "versioned facade now",
            "capture_boot",
            "Dominant friction",
            "Catalog regression",
            "Limitations",
        ] {
            assert!(md.contains(needle), "report.md must contain {needle}");
        }
    }
}
