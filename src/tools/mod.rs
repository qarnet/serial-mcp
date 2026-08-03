pub mod control_ops;
pub mod helpers;
pub mod io_ops;
pub mod port_ops;
pub mod read_loop;
pub mod result_builders;
pub mod rx_consume;
pub mod rx_validate;
pub mod types;
pub mod utility_ops;

#[cfg(test)]
mod tests {
    use schemars::schema_for;
    use serde_json;
    use serde_json::json;

    use crate::server::tool_catalog;
    use crate::tools::types::OpenArgs;

    /// The exhaustive 25-tool catalog served by MCP (shared with the xtask
    /// `agent-eval` catalog metrics via `crate::server::tool_catalog`). A
    /// missing tool would skip its
    /// `outputSchema`/`title` check and any uint-format scan, so the count
    /// is guarded explicitly.
    #[test]
    fn tool_catalog_has_exactly_twenty_five_tools() {
        let catalog = tool_catalog();
        assert_eq!(
            catalog.len(),
            25,
            "tool catalog must contain exactly 25 tools: {catalog:?}"
        );
        let mut names: Vec<String> = catalog.iter().map(|t| t.name.to_string()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 25, "tool names must be unique");
    }

    /// Regression guard: every MCP tool must carry `outputSchema` and `title`,
    /// and every MCP tool `outputSchema` must be free of the non-standard
    /// `uint*` format keywords that schemars 1.x emits for unsigned integer
    /// fields.
    ///
    /// DO NOT DELETE — see the header of `serial::schema` (src/serial/mod.rs) and
    /// `src/schema_helpers.rs` for the full rationale. History: b12b09fd,
    /// bc37a0b0, and the PortInfo regression this test originally missed
    /// because it only checked `uint`/`uint32`/`uint64` and not `uint8`/
    /// `uint16`. The `uint8`/`uint16` cases are now covered here, and the
    /// per-type coverage lives in `serial::schema`.
    fn all_tool_attrs() -> Vec<(String, rmcp::model::Tool)> {
        tool_catalog()
            .into_iter()
            .map(|tool| (tool.name.to_string(), tool))
            .collect()
    }

    #[test]
    fn verify_all_tool_schemas() {
        for (name, tool) in all_tool_attrs() {
            assert!(
                tool.output_schema.is_some(),
                "{name} must have outputSchema"
            );
            assert!(tool.title.is_some(), "{name} must have title");
        }
    }

    /// Regression guard: the tool catalog (names, descriptions, and generated
    /// input/output schemas) must not carry any removed streaming/logging
    /// surface — `subscribe`/`Subscribe` wording, `poll_interval_ms`,
    /// `notification_drop_count`, `peer_disconnected`, `budget_exhausted`,
    /// `channel_closed`, or `read_error`. The `subscribe`/`unsubscribe`
    /// tools and their schema helpers were removed with MCP logging in the
    /// rmcp 3 server-surface migration; this test keeps generated tool
    /// schemas from regressing into stale wording.
    #[test]
    fn tool_catalog_omits_removed_streaming_surface() {
        let removed = [
            "subscribe",
            "Subscribe",
            "poll_interval_ms",
            "notification_drop_count",
            "peer_disconnected",
            "budget_exhausted",
            "channel_closed",
            "read_error",
        ];
        for (name, tool) in all_tool_attrs() {
            let schema_str = serde_json::to_string(&tool).unwrap();
            for needle in removed {
                assert!(
                    !schema_str.contains(needle),
                    "tool {name} mentions removed surface {needle:?}"
                );
            }
        }
    }

    #[test]
    fn tool_schemas_have_no_nonstandard_uint_formats() {
        for (name, tool) in all_tool_attrs() {
            let schema_str = serde_json::to_string(&tool).unwrap();
            for bad_format in ["uint", "uint8", "uint16", "uint32", "uint64"] {
                assert!(
                    !schema_str.contains(&format!("\"format\":\"{bad_format}\"")),
                    "schema for {name} contains non-standard '{bad_format}' format.\n\
                     Fix: annotate each uN/Option<uN> field with \
                     `#[schemars(schema_with = \"crate::schema_helpers::uint_schema\")]` \
                     (or `option_uint_schema` for Option<uN>). \
                     See src/schema_helpers.rs.",
                );
            }
        }
    }

    #[test]
    fn tool_catalog_names_match_served_route_names() {
        // Every catalog entry must carry a non-empty name and the exact
        // served count (25); duplicate names would make tools/list ambiguous.
        let catalog = tool_catalog();
        let names: Vec<&str> = catalog.iter().map(|t| t.name.as_ref()).collect();
        assert_eq!(names.len(), 25);
        for n in &names {
            assert!(!n.is_empty());
        }
    }

    #[test]
    fn open_args_schema_has_minimum_zero_for_baud_rate() {
        let schema = schema_for!(OpenArgs);
        let json = serde_json::to_value(&schema).unwrap();
        let props = json.get("properties").unwrap();
        let baud = props.get("baud_rate").unwrap();
        // baud_rate is optional: anyOf [null, integer min 0].
        let inner = &baud["anyOf"][1];
        assert_eq!(inner.get("minimum"), Some(&serde_json::json!(0)));
        let required = json
            .get("required")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            required,
            vec![serde_json::json!("port")],
            "only `port` must be required on open"
        );
    }

    #[test]
    fn open_args_schema_no_longer_requires_default_bearing_fields() {
        // Omitted baud/default-bearing fields must be valid calls
        // (they resolve to profile defaults / built-ins).
        let schema = schema_for!(OpenArgs);
        let json = serde_json::to_value(&schema).unwrap();
        let required = json
            .get("required")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        for field in [
            "baud_rate",
            "data_bits",
            "stop_bits",
            "parity",
            "flow_control",
            "log_capacity",
            "log_enabled",
            "reconnect_policy",
            "rx_buffer_size",
            "max_buffered_bytes",
        ] {
            assert!(
                !required.contains(&serde_json::json!(field)),
                "open schema must not require {field}: {required:?}"
            );
        }
        // The profile_mode field must be present.
        let props = json.get("properties").unwrap();
        assert!(
            props.get("profile_mode").is_some(),
            "open schema must expose profile_mode"
        );
    }

    #[test]
    fn open_profile_args_schema_makes_overrides_optional() {
        let schema = schema_for!(crate::tools::types::OpenProfileArgs);
        let json = serde_json::to_value(&schema).unwrap();
        let required = json
            .get("required")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(
            required,
            vec![serde_json::json!("profile")],
            "only `profile` must be required on open_profile: {required:?}"
        );
    }

    /// Review gate: the optional override fields must genuinely
    /// accept null (and omission) against the GENERATED schema — not merely
    /// be absent from the `required` list. Validates public schema behavior
    /// via the jsonschema validator, like the tool schema guards do.
    #[test]
    fn open_and_open_profile_schemas_accept_null_overrides() {
        use jsonschema::validator_for;

        let open_schema = serde_json::to_value(schema_for!(OpenArgs)).unwrap();
        let open_validator = validator_for(&open_schema).unwrap();
        let open_instances = [
            json!({ "port": "/dev/ttyACM0" }),
            json!({
                "port": "/dev/ttyACM0",
                "baud_rate": null,
                "data_bits": null,
                "stop_bits": null,
                "parity": null,
                "flow_control": null,
                "log_capacity": null,
                "log_enabled": null,
                "reconnect_policy": null,
                "rx_buffer_size": null,
                "max_buffered_bytes": null,
                "profile_mode": null,
            }),
        ];
        for instance in &open_instances {
            let errors: Vec<String> = open_validator
                .iter_errors(instance)
                .map(|e| e.to_string())
                .collect();
            assert!(
                errors.is_empty(),
                "open schema must accept {instance}: {errors:?}"
            );
        }

        let profile_schema =
            serde_json::to_value(schema_for!(crate::tools::types::OpenProfileArgs)).unwrap();
        let profile_validator = validator_for(&profile_schema).unwrap();
        let profile_instances = [
            json!({ "profile": "dev" }),
            json!({
                "profile": "dev",
                "name": null,
                "log_capacity": null,
                "log_enabled": null,
                "rx_buffer_size": null,
            }),
            json!({
                "profile": "dev",
                "name": "renamed",
                "log_capacity": 512,
                "log_enabled": false,
                "rx_buffer_size": 4096,
            }),
        ];
        for instance in &profile_instances {
            let errors: Vec<String> = profile_validator
                .iter_errors(instance)
                .map(|e| e.to_string())
                .collect();
            assert!(
                errors.is_empty(),
                "open_profile schema must accept {instance}: {errors:?}"
            );
        }
    }

    #[test]
    fn connections_resource_schema_has_no_uint_format() {
        use crate::resources::types::ConnectionsResource;
        let schema = schema_for!(ConnectionsResource);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.contains("\"format\":\"uint\""));
    }

    /// Regression guard: after renaming `framing` → `rx_framing` and
    /// adding `tx_framing`, the write/read input schemas must expose
    /// `rx_framing` / `tx_framing` and NOT expose the old `framing` field.
    #[test]
    fn framing_fields_renamed_in_tool_schemas() {
        let schema = schema_for!(crate::tools::types::WriteArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"tx_framing\""),
            "WriteArgs schema must contain tx_framing"
        );
        assert!(
            !json.contains("\"framing\""),
            "WriteArgs schema must NOT contain bare 'framing'"
        );

        let schema = schema_for!(crate::tools::types::ReadArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"rx_framing\""),
            "ReadArgs schema must contain rx_framing"
        );
        assert!(
            !json.contains("\"framing\""),
            "ReadArgs schema must NOT contain bare 'framing'"
        );
    }

    /// After relocating `parser` from `rx_framing` to sibling
    /// `rx_parser`, verify `rx_parser` appears in ReadArgs schema.
    #[test]
    fn rx_parser_present_in_schemas() {
        let schema = schema_for!(crate::tools::types::ReadArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"rx_parser\""),
            "ReadArgs must contain rx_parser"
        );

        // Verify rx_framing sub-schema no longer exposes a "parser" property.
        // The `rx_framing` field value is a ref, so check the RxFramingConfig
        // schema directly.
        let schema = schema_for!(crate::framing::RxFramingConfig);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            !json.contains("\"parser\""),
            "RxFramingConfig must NOT contain parser property"
        );
    }

    /// After adding the `protocol` field, verify it appears in
    /// WriteArgs and ReadArgs schemas.
    #[test]
    fn protocol_field_present_in_schemas() {
        let schema = schema_for!(crate::tools::types::WriteArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"protocol\""),
            "WriteArgs must contain protocol"
        );

        let schema = schema_for!(crate::tools::types::ReadArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"protocol\""),
            "ReadArgs must contain protocol"
        );
    }
}
