//! Catalog metrics: the exact `tools/list` payload served by the MCP
//! server, measured in compact JSON bytes (no HTTP/SSE headers, no
//! pretty-print whitespace). The catalog comes from
//! `serial_mcp::server::tool_catalog()`, so the measurement can never
//! drift from what the router serves.

use serde::{Deserialize, Serialize};

/// Byte metric definition (documented, fixed):
///
/// - per-tool bytes: `serde_json::to_string(tool)` length (compact).
/// - input schema bytes: compact JSON of `tool.input_schema`.
/// - output schema bytes: compact JSON of `tool.output_schema`, 0 when
///   absent (all tools carry one today).
/// - description bytes: UTF-8 length of `tool.description`, 0 when absent.
/// - aggregate bytes: compact JSON length of `{"tools":[...]}` — the
///   serialized `tools/list` result body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogMetrics {
    pub tool_count: usize,
    /// Compact bytes of the whole `{"tools":[...]}` result.
    pub aggregate_bytes: usize,
    /// Compact bytes of each tool's JSON, keyed by tool name.
    pub per_tool_bytes: Vec<ToolBytes>,
    /// Top-largest tools by per-tool bytes (name + bytes), descending.
    pub top_largest: Vec<ToolBytes>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolBytes {
    pub name: String,
    pub total_bytes: usize,
    pub description_bytes: usize,
    pub input_schema_bytes: usize,
    pub output_schema_bytes: usize,
}

/// Measure the live catalog. Deterministic: `Tool` serialization and
/// schemars output are fully deterministic for a given build.
pub fn catalog_metrics() -> CatalogMetrics {
    let tools = serial_mcp::server::tool_catalog();
    let mut per_tool: Vec<ToolBytes> = tools
        .iter()
        .map(|tool| {
            let name = tool.name.to_string();
            let total_bytes = serde_json::to_string(tool).expect("tool serializes").len();
            let description_bytes = tool.description.as_deref().map(str::len).unwrap_or(0);
            let input_schema_bytes = serde_json::to_string(tool.input_schema.as_ref())
                .expect("input schema serializes")
                .len();
            let output_schema_bytes = tool
                .output_schema
                .as_ref()
                .map(|s| {
                    serde_json::to_string(s.as_ref())
                        .expect("output schema serializes")
                        .len()
                })
                .unwrap_or(0);
            ToolBytes {
                name,
                total_bytes,
                description_bytes,
                input_schema_bytes,
                output_schema_bytes,
            }
        })
        .collect();
    per_tool.sort_by(|a, b| a.name.cmp(&b.name));

    let aggregate_bytes = serde_json::to_string(&serde_json::json!({ "tools": tools }))
        .expect("tools/list result serializes")
        .len();

    let mut top_largest: Vec<ToolBytes> = per_tool.clone();
    top_largest.sort_by(|a, b| {
        b.total_bytes
            .cmp(&a.total_bytes)
            .then_with(|| a.name.cmp(&b.name))
    });
    top_largest.truncate(5);

    CatalogMetrics {
        tool_count: per_tool.len(),
        aggregate_bytes,
        per_tool_bytes: per_tool,
        top_largest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_deterministic() {
        let a = catalog_metrics();
        let b = catalog_metrics();
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "catalog metrics must be byte-identical across runs"
        );
    }

    #[test]
    fn catalog_matches_served_tool_count() {
        let metrics = catalog_metrics();
        assert_eq!(metrics.tool_count, 27);
        assert_eq!(metrics.per_tool_bytes.len(), 27);
        // The aggregate is larger than the sum of per-tool bytes (the
        // envelope adds keys/array framing).
        let per_tool_sum: usize = metrics.per_tool_bytes.iter().map(|t| t.total_bytes).sum();
        assert!(
            metrics.aggregate_bytes > per_tool_sum,
            "aggregate must include envelope framing"
        );
    }

    #[test]
    fn catalog_breaks_down_schema_and_description_bytes() {
        let metrics = catalog_metrics();
        for tool in &metrics.per_tool_bytes {
            assert!(
                tool.input_schema_bytes > 0,
                "{} must have an input schema",
                tool.name
            );
            assert!(
                tool.output_schema_bytes > 0,
                "{} must have an output schema",
                tool.name
            );
            assert!(
                tool.description_bytes > 0,
                "{} must have a description",
                tool.name
            );
            assert!(
                tool.total_bytes
                    >= tool.description_bytes + tool.input_schema_bytes + tool.output_schema_bytes,
                "{} total must cover its parts",
                tool.name
            );
        }
    }
}
