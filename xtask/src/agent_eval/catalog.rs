//! Catalog metrics measure the exact `tools/list` payload served by the MCP
//! server in compact JSON bytes. They exclude HTTP/SSE headers and pretty-print
//! whitespace. The catalog comes from `serial_mcp::server::tool_catalog()`, so
//! measurements match the router output.

use serde::{Deserialize, Serialize};

/// Byte metrics use these fixed definitions:
///
/// - Per-tool bytes: compact `serde_json::to_string(tool)` length.
/// - Input schema bytes: compact JSON length of `tool.input_schema`.
/// - Output schema bytes: compact JSON length of `tool.output_schema`, 0 when
///   absent (all tools carry one today).
/// - Description bytes: UTF-8 length of `tool.description`, 0 when absent.
/// - Aggregate bytes: compact JSON length of `{"tools":[...]}`, the serialized
///   `tools/list` result body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogMetrics {
    pub tool_count: usize,
    /// Compact byte length of the whole `{"tools":[...]}` result.
    pub aggregate_bytes: usize,
    /// Compact byte length of each tool's JSON, keyed by tool name.
    pub per_tool_bytes: Vec<ToolBytes>,
    /// Up to five largest tools by per-tool byte count, in descending order.
    pub top_largest: Vec<ToolBytes>,
}

/// Byte counts for one tool in the catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolBytes {
    /// Tool name.
    pub name: String,
    /// Compact JSON byte count for the tool.
    pub total_bytes: usize,
    /// UTF-8 byte count for the tool description.
    pub description_bytes: usize,
    /// Compact JSON byte count for the input schema.
    pub input_schema_bytes: usize,
    /// Compact JSON byte count for the output schema.
    pub output_schema_bytes: usize,
}

/// Measure the live catalog.
///
/// `Tool` serialization and schemars output are deterministic for a given
/// build.
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
        assert_eq!(metrics.tool_count, 25);
        assert_eq!(metrics.per_tool_bytes.len(), 25);
        // Aggregate size includes envelope framing.
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
