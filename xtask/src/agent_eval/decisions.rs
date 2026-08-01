//! Fixed decision thresholds for the agent-interface evaluation (from the
//! Phase 4 handoff). All thresholds are constants fixed BEFORE looking at
//! results; the evaluator only computes whether they are met.

use serde::Serialize;

use super::catalog::CatalogMetrics;
use super::scenarios::ScenarioMetrics;

/// One current-vs-alternative comparison.
#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub scenario: String,
    pub label: String,
    pub current_calls: usize,
    pub alternative_calls: usize,
    pub current_bytes: usize,
    pub alternative_bytes: usize,
    /// `current_calls - alternative_calls` (positive = the alternative saves calls).
    pub call_savings: i64,
    /// `(current - alternative) / current * 100`. Positive = the alternative
    /// is smaller. Negative = the alternative is larger.
    pub byte_reduction_pct: f64,
    /// Recipe metric: repeated advanced objects removed by the alternative.
    pub reduced_advanced_objects: usize,
    pub completion_ref: String,
    pub invalid_calls: usize,
}

impl Comparison {
    fn new(
        current: &ScenarioMetrics,
        alternative: &ScenarioMetrics,
        reduced_advanced_objects: usize,
    ) -> Self {
        let call_savings = current.tool_calls as i64 - alternative.tool_calls as i64;
        let byte_reduction_pct = if current.request_bytes == 0 {
            0.0
        } else {
            (current.request_bytes as f64 - alternative.request_bytes as f64)
                / current.request_bytes as f64
                * 100.0
        };
        Comparison {
            scenario: current.id.clone(),
            label: current.label.clone(),
            current_calls: current.tool_calls,
            alternative_calls: alternative.tool_calls,
            current_bytes: current.request_bytes,
            alternative_bytes: alternative.request_bytes,
            call_savings,
            byte_reduction_pct,
            reduced_advanced_objects,
            completion_ref: alternative.completion_ref.clone(),
            invalid_calls: alternative.invalid_calls,
        }
    }
}

/// Aggregated metrics feeding the decisions.
#[derive(Debug, Clone, Serialize)]
pub struct AggregateMetrics {
    pub automatic_profiles: Comparison,
    pub transact_vs_write_read: Comparison,
    pub shorthand_comparisons: Vec<Comparison>,
    pub recipe_comparisons: Vec<Comparison>,
    pub facade_comparisons: Vec<Comparison>,
    pub capture_boot: Comparison,
    pub boot_stale_race: bool,
    pub common_median_calls: f64,
    pub common_median_facade_call_savings: f64,
    pub common_median_facade_byte_reduction_pct: f64,
    pub total_scenario_request_bytes: usize,
}

/// Build the aggregate metrics from measured scenarios.
pub fn aggregate(scenarios: &[ScenarioMetrics]) -> AggregateMetrics {
    let by_id = |id: &str| -> &ScenarioMetrics {
        scenarios
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("scenario {id} missing"))
    };
    // Materialize each modeled variant as a ScenarioMetrics (owned) so the
    // comparisons below can borrow stable values.
    let modeled_metrics: Vec<(String, ScenarioMetrics)> = scenarios
        .iter()
        .filter_map(|s| {
            s.modeled.as_ref().map(|m| {
                let id = format!("{}::modeled", s.id);
                let modeled = ScenarioMetrics {
                    id: id.clone(),
                    label: m.label.clone(),
                    tool_calls: m.tool_calls,
                    request_bytes: m.request_bytes,
                    invalid_calls: 0,
                    retries: 0,
                    advanced_fields: 0,
                    stale_race: false,
                    completion_ref: "modeled".to_string(),
                    common: false,
                    modeled: None,
                };
                (id, modeled)
            })
        })
        .collect();
    let modeled_of = |id: &str| -> &ScenarioMetrics {
        modeled_metrics
            .iter()
            .find(|(mid, _)| mid == &format!("{id}::modeled"))
            .map(|(_, m)| m)
            .unwrap_or_else(|| panic!("scenario {id} has no modeled variant"))
    };

    let automatic = Comparison::new(
        by_id("explicit_profile_management"),
        by_id("returning_known_console_automatic"),
        0,
    );
    let transact = Comparison::new(
        by_id("command_response_write_read"),
        by_id("command_response_transact"),
        0,
    );

    let shorthand_comparisons = ["command_response_transact", "at_modem"]
        .iter()
        .map(|id| Comparison::new(by_id(id), modeled_of(id), 0))
        .collect();

    // Recipes: at_modem_recipe and ndjson_stream (modeled recipe variants)
    // both replace one repeated protocol-preset object on `open`.
    let recipe_comparisons = ["at_modem_recipe", "ndjson_stream"]
        .iter()
        .map(|id| Comparison::new(by_id(id), modeled_of(id), 1))
        .collect();

    let facade_comparisons: Vec<Comparison> = ["command_response_facade"]
        .iter()
        .map(|id| Comparison::new(by_id(id), modeled_of(id), 0))
        .collect();

    // capture_boot (Phase 5, implemented) vs the pre-Phase-5 manual
    // composition: one atomic call removes the arm/reset race of the
    // 5-call open+read+set_dtr_rts×2+read sequence.
    let capture_boot = Comparison::new(
        by_id("boot_reset_manual_composition"),
        by_id("boot_reset_prompt_capture"),
        0,
    );
    let boot_stale_race = by_id("boot_reset_manual_composition").stale_race;

    // Common-task medians (facade decision).
    let common: Vec<&ScenarioMetrics> = scenarios.iter().filter(|s| s.common).collect();
    let mut common_calls: Vec<f64> = common.iter().map(|s| s.tool_calls as f64).collect();
    common_calls.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let common_median_calls = median(&common_calls);

    let mut facade_savings: Vec<f64> = facade_comparisons
        .iter()
        .map(|c| c.call_savings as f64)
        .collect();
    facade_savings.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let common_median_facade_call_savings = median(&facade_savings);
    let mut facade_bytes: Vec<f64> = facade_comparisons
        .iter()
        .map(|c| c.byte_reduction_pct)
        .collect();
    facade_bytes.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let common_median_facade_byte_reduction_pct = median(&facade_bytes);

    let total_scenario_request_bytes: usize = scenarios.iter().map(|s| s.request_bytes).sum();

    AggregateMetrics {
        automatic_profiles: automatic,
        transact_vs_write_read: transact,
        shorthand_comparisons,
        recipe_comparisons,
        facade_comparisons,
        capture_boot,
        boot_stale_race,
        common_median_calls,
        common_median_facade_call_savings,
        common_median_facade_byte_reduction_pct,
        total_scenario_request_bytes,
    }
}

fn median(sorted: &[f64]) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

/// One yes/no decision with its reason.
#[derive(Debug, Clone, Serialize)]
pub struct Decision {
    pub decision: &'static str,
    pub thresholds_met: bool,
    pub reason: String,
}

/// The four Phase 4 decisions plus the automatic-profiles justification.
#[derive(Debug, Clone, Serialize)]
pub struct Decisions {
    pub automatic_profiles: Decision,
    pub shorthand_now: Decision,
    pub initial_recipes_now: Decision,
    pub versioned_facade_now: Decision,
    pub phase5_capture_boot: Decision,
}

/// Projected catalog growth for hypothetical variants. Measured catalog
/// growth is 0% for all modeled candidates (none add tools in this phase);
/// oneOf-branch growth inside existing schemas is not modeled, which the
/// report states as a limitation.
const MODELED_CATALOG_GROWTH_PCT: f64 = 0.0;

/// Fixed thresholds (Phase 4 handoff).
const SHORTHAND_MIN_BYTE_REDUCTION_PCT: f64 = 20.0;
const SHORTHAND_MIN_SCENARIOS: usize = 3;
const SHORTHAND_MAX_CATALOG_GROWTH_PCT: f64 = 3.0;
const RECIPE_MIN_BYTE_REDUCTION_PCT: f64 = 20.0;
const RECIPE_MIN_SCENARIOS: usize = 3;
const RECIPE_MAX_CATALOG_GROWTH_PCT: f64 = 2.0;
const FACADE_MIN_CALL_SAVINGS: i64 = 1;
const FACADE_MIN_BYTE_REDUCTION_PCT: f64 = 30.0;
const FACADE_MAX_CATALOG_GROWTH_PCT: f64 = 10.0;
const AUTOMATIC_MIN_CALL_SAVINGS: i64 = 1;
const AUTOMATIC_MIN_BYTE_REDUCTION_PCT: f64 = 20.0;

/// Evaluate all fixed decisions against the measured metrics.
pub fn evaluate(m: &AggregateMetrics) -> Decisions {
    // Shorthand: >=20% request-byte reduction in at least three scenarios
    // with modeled shorthand variants, projected catalog growth <=3%.
    let shorthand_met_count = m
        .shorthand_comparisons
        .iter()
        .filter(|c| c.byte_reduction_pct >= SHORTHAND_MIN_BYTE_REDUCTION_PCT)
        .count();
    let shorthand_thresholds_met = shorthand_met_count >= SHORTHAND_MIN_SCENARIOS
        && MODELED_CATALOG_GROWTH_PCT <= SHORTHAND_MAX_CATALOG_GROWTH_PCT;
    let shorthand_now = Decision {
        decision: if shorthand_thresholds_met {
            "yes"
        } else {
            "no"
        },
        thresholds_met: shorthand_thresholds_met,
        reason: format!(
            "{} of {} shorthand scenarios reach >={:.0}% request-byte reduction \
             (need >={}); projected catalog growth {:.1}% (limit {:.0}%)",
            shorthand_met_count,
            m.shorthand_comparisons.len(),
            SHORTHAND_MIN_BYTE_REDUCTION_PCT,
            SHORTHAND_MIN_SCENARIOS,
            MODELED_CATALOG_GROWTH_PCT,
            SHORTHAND_MAX_CATALOG_GROWTH_PCT,
        ),
    };

    // Recipes: >=20% reduction OR one repeated advanced object removed in at
    // least three scenarios, no extra calls, growth <=2%.
    let recipe_met_count = m
        .recipe_comparisons
        .iter()
        .filter(|c| {
            c.byte_reduction_pct >= RECIPE_MIN_BYTE_REDUCTION_PCT || c.reduced_advanced_objects >= 1
        })
        .filter(|c| c.alternative_calls <= c.current_calls)
        .count();
    let recipe_thresholds_met = recipe_met_count >= RECIPE_MIN_SCENARIOS
        && MODELED_CATALOG_GROWTH_PCT <= RECIPE_MAX_CATALOG_GROWTH_PCT;
    let initial_recipes_now = Decision {
        decision: if recipe_thresholds_met { "yes" } else { "no" },
        thresholds_met: recipe_thresholds_met,
        reason: format!(
            "{} of {} recipe scenarios meet the reduction/advanced-object rule \
             (need >={}) with no extra calls; projected catalog growth {:.1}% (limit {:.0}%)",
            recipe_met_count,
            m.recipe_comparisons.len(),
            RECIPE_MIN_SCENARIOS,
            MODELED_CATALOG_GROWTH_PCT,
            RECIPE_MAX_CATALOG_GROWTH_PCT,
        ),
    };

    // Facade: common-task median saves >=1 call and >=30% request bytes,
    // 100% modeled completion, growth <=10%.
    let facade_all_modeled = m
        .facade_comparisons
        .iter()
        .all(|c| c.invalid_calls == 0 && !c.completion_ref.is_empty());
    let facade_thresholds_met = m.common_median_facade_call_savings
        >= FACADE_MIN_CALL_SAVINGS as f64
        && m.common_median_facade_byte_reduction_pct >= FACADE_MIN_BYTE_REDUCTION_PCT
        && facade_all_modeled
        && MODELED_CATALOG_GROWTH_PCT <= FACADE_MAX_CATALOG_GROWTH_PCT;
    let versioned_facade_now = Decision {
        decision: if facade_thresholds_met { "yes" } else { "no" },
        thresholds_met: facade_thresholds_met,
        reason: format!(
            "common-task median facade call savings {:.1} (need >={}), byte \
             reduction {:.1}% (need >={:.0}%), modeled completion 100%: {}",
            m.common_median_facade_call_savings,
            FACADE_MIN_CALL_SAVINGS,
            m.common_median_facade_byte_reduction_pct,
            FACADE_MIN_BYTE_REDUCTION_PCT,
            if facade_all_modeled { "yes" } else { "no" },
        ),
    };

    // capture_boot: removes the arm/reset race or stale-data window AND
    // reduces composition calls.
    let capture_reduces_calls = m.capture_boot.alternative_calls < m.capture_boot.current_calls;
    let capture_thresholds_met = m.boot_stale_race && capture_reduces_calls;
    let phase5_capture_boot = Decision {
        decision: if capture_thresholds_met { "yes" } else { "no" },
        thresholds_met: capture_thresholds_met,
        reason: format!(
            "boot capture composition has a stale-data/arm-reset race: {}; \
             capture_boot reduces {} calls to {}",
            m.boot_stale_race, m.capture_boot.current_calls, m.capture_boot.alternative_calls,
        ),
    };

    // Automatic profiles (shipped behavior) vs explicit management.
    let auto_thresholds_met = m.automatic_profiles.call_savings >= AUTOMATIC_MIN_CALL_SAVINGS
        && m.automatic_profiles.byte_reduction_pct >= AUTOMATIC_MIN_BYTE_REDUCTION_PCT
        && m.automatic_profiles.invalid_calls == 0;
    let automatic_profiles = Decision {
        decision: if auto_thresholds_met { "yes" } else { "no" },
        thresholds_met: auto_thresholds_met,
        reason: format!(
            "automatic reuse saves {} call(s) (need >={}) and {:.1}% request bytes \
             (need >={:.0}%) vs explicit management; identity rules unchanged",
            m.automatic_profiles.call_savings,
            AUTOMATIC_MIN_CALL_SAVINGS,
            m.automatic_profiles.byte_reduction_pct,
            AUTOMATIC_MIN_BYTE_REDUCTION_PCT,
        ),
    };

    Decisions {
        automatic_profiles,
        shorthand_now,
        initial_recipes_now,
        versioned_facade_now,
        phase5_capture_boot,
    }
}

/// Dominant friction source, chosen by a fixed deterministic rule over the
/// measured metrics:
///
/// 1. `schema size` — aggregate `tools/list` payload >= 64 KiB;
/// 2. `call shape` — median common-task call count >= 3;
/// 3. `setup` — the first-connect scenario needs >= 4 calls;
/// 4. `orchestration` — any scenario requires retries/fallbacks;
/// 5. otherwise `no dominant source`.
///
/// Documentation friction is not measurable by a static harness and is
/// reported as a limitation instead.
pub fn dominant_friction(
    m: &AggregateMetrics,
    catalog: &CatalogMetrics,
    scenarios: &[ScenarioMetrics],
) -> String {
    let schema_flag = catalog.aggregate_bytes >= 64 * 1024;
    let call_shape_flag = m.common_median_calls >= 3.0;
    let setup_flag = scenarios
        .iter()
        .find(|s| s.id == "first_console_discovery_open")
        .map(|s| s.tool_calls >= 4)
        .unwrap_or(false);
    let orchestration_flag = scenarios.iter().any(|s| s.retries > 0);
    if schema_flag {
        "schema size".to_string()
    } else if call_shape_flag {
        "call shape".to_string()
    } else if setup_flag {
        "setup".to_string()
    } else if orchestration_flag {
        "orchestration".to_string()
    } else {
        "no dominant source".to_string()
    }
}

/// Catalog regression warning vs a previous baseline.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogRegression {
    pub status: String,
    pub aggregate_growth_pct: Option<f64>,
    pub per_tool_regressions: Vec<PerToolRegression>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PerToolRegression {
    pub name: String,
    pub baseline_bytes: usize,
    pub current_bytes: usize,
    pub growth_pct: f64,
    pub growth_bytes: i64,
}

impl CatalogRegression {
    pub fn no_baseline() -> Self {
        CatalogRegression {
            status: "no_baseline".to_string(),
            aggregate_growth_pct: None,
            per_tool_regressions: Vec::new(),
        }
    }
}

/// Fixed regression thresholds: aggregate >=5%; per-tool >=10% or +2 KiB.
const REGRESSION_AGGREGATE_PCT: f64 = 5.0;
const REGRESSION_PER_TOOL_PCT: f64 = 10.0;
const REGRESSION_PER_TOOL_BYTES: i64 = 2048;

pub fn regression_vs_baseline(
    baseline: &CatalogMetrics,
    current: &CatalogMetrics,
) -> CatalogRegression {
    let aggregate_growth_pct = if baseline.aggregate_bytes == 0 {
        0.0
    } else {
        (current.aggregate_bytes as f64 - baseline.aggregate_bytes as f64)
            / baseline.aggregate_bytes as f64
            * 100.0
    };
    let mut per_tool_regressions = Vec::new();
    for tool in &current.per_tool_bytes {
        if let Some(b) = baseline.per_tool_bytes.iter().find(|t| t.name == tool.name) {
            let growth_pct = if b.total_bytes == 0 {
                0.0
            } else {
                (tool.total_bytes as f64 - b.total_bytes as f64) / b.total_bytes as f64 * 100.0
            };
            let growth_bytes = tool.total_bytes as i64 - b.total_bytes as i64;
            if growth_pct >= REGRESSION_PER_TOOL_PCT || growth_bytes >= REGRESSION_PER_TOOL_BYTES {
                per_tool_regressions.push(PerToolRegression {
                    name: tool.name.clone(),
                    baseline_bytes: b.total_bytes,
                    current_bytes: tool.total_bytes,
                    growth_pct,
                    growth_bytes,
                });
            }
        }
    }
    let status =
        if aggregate_growth_pct >= REGRESSION_AGGREGATE_PCT || !per_tool_regressions.is_empty() {
            "warning".to_string()
        } else {
            "ok".to_string()
        };
    CatalogRegression {
        status,
        aggregate_growth_pct: Some(aggregate_growth_pct),
        per_tool_regressions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_scenario(id: &str, calls: usize, bytes: usize) -> ScenarioMetrics {
        ScenarioMetrics {
            id: id.to_string(),
            label: id.to_string(),
            tool_calls: calls,
            request_bytes: bytes,
            invalid_calls: 0,
            retries: 0,
            advanced_fields: 0,
            stale_race: false,
            completion_ref: "tests/example::test".to_string(),
            common: true,
            modeled: None,
        }
    }

    /// A shorthand comparison that comfortably beats the 20% threshold.
    #[test]
    fn shorthand_decision_requires_three_scenarios() {
        let s = fake_scenario("x", 2, 1000);
        let mut scenario = s.clone();
        scenario.modeled = Some(crate::agent_eval::scenarios::ModeledMetrics {
            kind: "shorthand".into(),
            label: "m".into(),
            tool_calls: 2,
            request_bytes: 500,
            expansion_calls: 2,
            expansion_bytes: 1000,
            note: "".into(),
        });
        let m = AggregateMetrics {
            automatic_profiles: Comparison::new(
                &fake_scenario("a", 3, 300),
                &fake_scenario("b", 2, 240),
                0,
            ),
            transact_vs_write_read: Comparison::new(
                &fake_scenario("a", 3, 300),
                &fake_scenario("b", 2, 240),
                0,
            ),
            shorthand_comparisons: vec![Comparison::new(&scenario, &fake_scenario("m", 2, 500), 0)],
            recipe_comparisons: Vec::new(),
            facade_comparisons: Vec::new(),
            capture_boot: Comparison::new(
                &fake_scenario("boot", 5, 1000),
                &fake_scenario("cb", 1, 200),
                0,
            ),
            boot_stale_race: true,
            common_median_calls: 2.0,
            common_median_facade_call_savings: 0.0,
            common_median_facade_byte_reduction_pct: 10.0,
            total_scenario_request_bytes: 0,
        };
        let d = evaluate(&m);
        assert_eq!(
            d.shorthand_now.decision, "no",
            "1 of 3 scenarios must not justify shorthand"
        );
        assert_eq!(
            d.phase5_capture_boot.decision, "yes",
            "race removal + call reduction justifies capture_boot"
        );
    }

    #[test]
    fn facade_needs_call_savings() {
        // A 1:1 alias saves zero calls -> never justified regardless of bytes.
        let mut m_facade = fake_scenario("f", 2, 1000);
        m_facade.modeled = Some(crate::agent_eval::scenarios::ModeledMetrics {
            kind: "facade".into(),
            label: "m".into(),
            tool_calls: 2,
            request_bytes: 300,
            expansion_calls: 2,
            expansion_bytes: 1000,
            note: "".into(),
        });
        let comp = Comparison::new(&m_facade, &fake_scenario("m", 2, 300), 0);
        let m = AggregateMetrics {
            automatic_profiles: Comparison::new(
                &fake_scenario("a", 3, 300),
                &fake_scenario("b", 2, 240),
                0,
            ),
            transact_vs_write_read: Comparison::new(
                &fake_scenario("a", 3, 300),
                &fake_scenario("b", 2, 240),
                0,
            ),
            shorthand_comparisons: Vec::new(),
            recipe_comparisons: Vec::new(),
            facade_comparisons: vec![comp],
            capture_boot: Comparison::new(
                &fake_scenario("boot", 5, 1000),
                &fake_scenario("cb", 1, 200),
                0,
            ),
            boot_stale_race: false,
            common_median_calls: 2.0,
            common_median_facade_call_savings: 0.0,
            common_median_facade_byte_reduction_pct: 70.0,
            total_scenario_request_bytes: 0,
        };
        let d = evaluate(&m);
        assert_eq!(
            d.versioned_facade_now.decision, "no",
            "0 call savings must fail the facade threshold"
        );
    }

    #[test]
    fn regression_warning_flags_aggregate_and_per_tool_growth() {
        let baseline = CatalogMetrics {
            tool_count: 26,
            aggregate_bytes: 100_000,
            per_tool_bytes: vec![crate::agent_eval::catalog::ToolBytes {
                name: "big".into(),
                total_bytes: 10_000,
                description_bytes: 100,
                input_schema_bytes: 9000,
                output_schema_bytes: 900,
            }],
            top_largest: Vec::new(),
        };
        let grown = CatalogMetrics {
            tool_count: 26,
            aggregate_bytes: 106_000, // +6% -> warning
            per_tool_bytes: vec![crate::agent_eval::catalog::ToolBytes {
                name: "big".into(),
                total_bytes: 11_000, // +10%
                description_bytes: 100,
                input_schema_bytes: 9900,
                output_schema_bytes: 1000,
            }],
            top_largest: Vec::new(),
        };
        let r = regression_vs_baseline(&baseline, &grown);
        assert_eq!(r.status, "warning");
        assert_eq!(r.per_tool_regressions.len(), 1);
    }
}
