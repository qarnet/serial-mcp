//! Deterministic agent-interface evaluation (Phase 4).
//!
//! `xtask agent-eval` measures the real `tools/list` catalog (via
//! `serial_mcp::server::tool_catalog`) plus representative fixed call
//! scenarios, applies the fixed decision thresholds from the Phase 4
//! handoff, and writes `report.json` + `report.md` under
//! `target/agent-interface-eval/` (or `--output-dir`).
//!
//! Determinism contract: no timestamps, hostnames, absolute temp paths,
//! network access, user profiles, or payload captures appear in the
//! output. Rerunning yields byte-identical reports.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub mod catalog;
pub mod decisions;
pub mod report;
pub mod scenarios;

/// Default output directory for the evaluation report.
pub const DEFAULT_OUTPUT_DIR: &str = "target/agent-interface-eval";

/// Fixed normalized connection id used in scenario envelopes (never a real
/// runtime UUID, so reruns are byte-identical).
pub const FIXED_CONNECTION_ID: &str = "9f1e3c2a-b3d4-4a5b-9c2d-1e2f3a4b5c6d";

/// Fixed port placeholder used in scenario envelopes.
pub const FIXED_PORT: &str = "/dev/ttyACM0";

/// Fixed JSON-RPC id used in scenario envelopes.
pub const FIXED_ENVELOPE_ID: &str = "1";

/// Agent-eval CLI options.
#[derive(Debug, Default, Clone)]
pub struct EvalOptions {
    pub output_dir: Option<PathBuf>,
    pub baseline: Option<PathBuf>,
    pub write_baseline: Option<PathBuf>,
}

/// The complete deterministic evaluation result.
#[derive(Debug, Clone, serde::Serialize)]
pub struct EvalReport {
    pub schema: &'static str,
    pub catalog: catalog::CatalogMetrics,
    pub scenarios: Vec<scenarios::ScenarioMetrics>,
    pub metrics: decisions::AggregateMetrics,
    pub decisions: decisions::Decisions,
    pub dominant_friction: String,
    pub regression: decisions::CatalogRegression,
}

/// The subset of a previous `report.json` needed for the catalog
/// regression comparison.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BaselineFile {
    pub catalog: catalog::CatalogMetrics,
}

/// Run the evaluation and write the report files.
pub fn run(options: &EvalOptions) -> Result<()> {
    let catalog_metrics = catalog::catalog_metrics();
    let scenarios = scenarios::scenarios();
    let scenario_metrics: Vec<scenarios::ScenarioMetrics> =
        scenarios.iter().map(scenarios::scenario_metrics).collect();
    let metrics = decisions::aggregate(&scenario_metrics);
    let decisions = decisions::evaluate(&metrics);
    let dominant_friction =
        decisions::dominant_friction(&metrics, &catalog_metrics, &scenario_metrics);
    let regression = match &options.baseline {
        Some(path) => {
            let baseline_text = std::fs::read_to_string(path)
                .with_context(|| format!("read baseline {}", path.display()))?;
            let baseline: BaselineFile = serde_json::from_str(&baseline_text)
                .with_context(|| format!("parse baseline {}", path.display()))?;
            decisions::regression_vs_baseline(&baseline.catalog, &catalog_metrics)
        }
        None => decisions::CatalogRegression::no_baseline(),
    };

    let report = EvalReport {
        schema: "serial-mcp-agent-interface-eval/v1",
        catalog: catalog_metrics,
        scenarios: scenario_metrics,
        metrics,
        decisions,
        dominant_friction,
        regression,
    };

    let output_dir = options
        .output_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from(DEFAULT_OUTPUT_DIR));
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("create output dir {}", output_dir.display()))?;

    let json_text = report::render_json(&report)?;
    let md_text = report::render_md(&report)?;
    let json_path = output_dir.join("report.json");
    let md_path = output_dir.join("report.md");
    std::fs::write(&json_path, &json_text)
        .with_context(|| format!("write {}", json_path.display()))?;
    std::fs::write(&md_path, &md_text).with_context(|| format!("write {}", md_path.display()))?;

    if let Some(write_baseline) = &options.write_baseline {
        std::fs::write(write_baseline, &json_text)
            .with_context(|| format!("write baseline {}", write_baseline.display()))?;
    }

    eprintln!("xtask agent-eval: wrote {}", json_path.display());
    eprintln!("xtask agent-eval: wrote {}", md_path.display());
    eprintln!(
        "xtask agent-eval: {} tools, aggregate catalog {} bytes",
        report.catalog.tool_count, report.catalog.aggregate_bytes
    );
    Ok(())
}
