pub mod control_ops;
pub mod helpers;
pub mod io_ops;
pub mod port_ops;
pub mod rx_consume;
pub mod stream_ops;
pub mod types;
pub mod utility_ops;

#[cfg(test)]
mod tests {
    use schemars::schema_for;
    use serde_json;

    use crate::server::SerialHandler;
    use crate::tools::types::OpenArgs;

    /// Regression guard: every MCP tool must carry `outputSchema` and `title`,
    /// and every MCP tool `outputSchema` must be free of the non-standard
    /// `uint*` format keywords that schemars 1.x emits for unsigned integer
    /// fields.
    ///
    /// DO NOT DELETE — see the header of `serial::schema` (src/serial.rs) and
    /// `src/schema_helpers.rs` for the full rationale. History: b12b09fd,
    /// bc37a0b0, and the PortInfo regression this test originally missed
    /// because it only checked `uint`/`uint32`/`uint64` and not `uint8`/
    /// `uint16`. The `uint8`/`uint16` cases are now covered here, and the
    /// per-type coverage lives in `serial::schema`.
    ///
    /// Keep this list in sync with the `#[tool]` methods in `src/server.rs`.
    /// The list below is exhaustive (25 tools); a missing tool would skip its
    /// `outputSchema`/`title` check and any uint-format scan.
    fn all_tool_attrs() -> Vec<(&'static str, rmcp::model::Tool)> {
        vec![
            ("list_ports", SerialHandler::list_ports_tool_attr()),
            (
                "list_connections",
                SerialHandler::list_connections_tool_attr(),
            ),
            ("open", SerialHandler::open_tool_attr()),
            ("close", SerialHandler::close_tool_attr()),
            ("write", SerialHandler::write_tool_attr()),
            ("transact", SerialHandler::transact_tool_attr()),
            ("read", SerialHandler::read_tool_attr()),
            ("flush", SerialHandler::flush_tool_attr()),
            ("set_dtr_rts", SerialHandler::set_dtr_rts_tool_attr()),
            (
                "set_flow_control",
                SerialHandler::set_flow_control_tool_attr(),
            ),
            ("send_break", SerialHandler::send_break_tool_attr()),
            ("subscribe", SerialHandler::subscribe_tool_attr()),
            ("unsubscribe", SerialHandler::unsubscribe_tool_attr()),
            ("get_status", SerialHandler::get_status_tool_attr()),
            ("reconfigure", SerialHandler::reconfigure_tool_attr()),
            ("list_profiles", SerialHandler::list_profiles_tool_attr()),
            ("open_profile", SerialHandler::open_profile_tool_attr()),
            ("save_profile", SerialHandler::save_profile_tool_attr()),
            ("delete_profile", SerialHandler::delete_profile_tool_attr()),
            ("configure", SerialHandler::configure_tool_attr()),
            ("get_log", SerialHandler::get_log_tool_attr()),
            ("clear_log", SerialHandler::clear_log_tool_attr()),
            ("export_log", SerialHandler::export_log_tool_attr()),
            ("reconnect", SerialHandler::reconnect_tool_attr()),
            (
                "compute_checksum",
                SerialHandler::compute_checksum_tool_attr(),
            ),
        ]
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

    #[test]
    fn tool_schemas_have_no_nonstandard_uint_formats() {
        for tool in all_tool_attrs() {
            let schema_str = serde_json::to_string(&tool).unwrap();
            for bad_format in ["uint", "uint8", "uint16", "uint32", "uint64"] {
                assert!(
                    !schema_str.contains(&format!("\"format\":\"{bad_format}\"")),
                    "schema for {} contains non-standard '{bad_format}' format.\n\
                     Fix: annotate each uN/Option<uN> field with \
                     `#[schemars(schema_with = \"crate::schema_helpers::uint_schema\")]` \
                     (or `option_uint_schema` for Option<uN>). \
                     See src/schema_helpers.rs.",
                    tool.0
                );
            }
        }
    }

    #[test]
    fn open_args_schema_has_minimum_zero_for_baud_rate() {
        let schema = schema_for!(OpenArgs);
        let json = serde_json::to_value(&schema).unwrap();
        let props = json.get("properties").unwrap();
        let baud = props.get("baud_rate").unwrap();
        // baud_rate is optional (Phase 3A): anyOf [null, integer min 0].
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
        // Phase 3A: omitted baud/default-bearing fields must be valid calls
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
            "poll_interval_ms",
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

    #[test]
    fn connections_resource_schema_has_no_uint_format() {
        use crate::resources::types::ConnectionsResource;
        let schema = schema_for!(ConnectionsResource);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.contains("\"format\":\"uint\""));
    }

    /// Phase 1 regression guard: after renaming `framing` → `rx_framing` and
    /// adding `tx_framing`, the write/read/subscribe input schemas must expose
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

        let schema = schema_for!(crate::tools::types::SubscribeArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"rx_framing\""),
            "SubscribeArgs schema must contain rx_framing"
        );
        assert!(
            !json.contains("\"framing\""),
            "SubscribeArgs schema must NOT contain bare 'framing'"
        );
    }

    /// Phase 4a: after relocating `parser` from `rx_framing` to sibling
    /// `rx_parser`, verify `rx_parser` appears in ReadArgs and SubscribeArgs
    /// schemas.
    #[test]
    fn rx_parser_present_in_schemas() {
        let schema = schema_for!(crate::tools::types::ReadArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"rx_parser\""),
            "ReadArgs must contain rx_parser"
        );

        let schema = schema_for!(crate::tools::types::SubscribeArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"rx_parser\""),
            "SubscribeArgs must contain rx_parser"
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

    /// Phase 4b: after adding the `protocol` field, verify it appears in
    /// WriteArgs, ReadArgs, and SubscribeArgs schemas.
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

        let schema = schema_for!(crate::tools::types::SubscribeArgs);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(
            json.contains("\"protocol\""),
            "SubscribeArgs must contain protocol"
        );
    }
}
