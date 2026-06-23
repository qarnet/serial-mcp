pub mod control_ops;
pub mod helpers;
pub mod io_ops;
pub mod port_ops;
pub mod rx_consume;
pub mod stream_ops;
pub mod types;

#[cfg(test)]
mod tests {
    use schemars::schema_for;
    use serde_json;

    use crate::server::SerialHandler;
    use crate::tools::types::OpenArgs;

    #[test]
    fn verify_all_tool_schemas() {
        let tools = vec![
            ("list_ports", SerialHandler::list_ports_tool_attr()),
            (
                "list_connections",
                SerialHandler::list_connections_tool_attr(),
            ),
            ("open", SerialHandler::open_tool_attr()),
            ("close", SerialHandler::close_tool_attr()),
            ("write", SerialHandler::write_tool_attr()),
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
        ];

        for (name, tool) in tools {
            assert!(
                tool.output_schema.is_some(),
                "{name} must have outputSchema"
            );
            assert!(tool.title.is_some(), "{name} must have title");
        }
    }

    #[test]
    fn tool_schemas_have_no_nonstandard_uint_formats() {
        let tools = vec![
            SerialHandler::list_ports_tool_attr(),
            SerialHandler::list_connections_tool_attr(),
            SerialHandler::open_tool_attr(),
            SerialHandler::close_tool_attr(),
            SerialHandler::write_tool_attr(),
            SerialHandler::read_tool_attr(),
            SerialHandler::flush_tool_attr(),
            SerialHandler::set_dtr_rts_tool_attr(),
            SerialHandler::set_flow_control_tool_attr(),
            SerialHandler::send_break_tool_attr(),
            SerialHandler::subscribe_tool_attr(),
            SerialHandler::unsubscribe_tool_attr(),
            SerialHandler::get_status_tool_attr(),
            SerialHandler::reconfigure_tool_attr(),
            SerialHandler::list_profiles_tool_attr(),
            SerialHandler::open_profile_tool_attr(),
        ];

        for tool in tools {
            let schema_str = serde_json::to_string(&tool).unwrap();
            assert!(
                !schema_str.contains("\"format\":\"uint\""),
                "schema for {} contains non-standard 'uint' format",
                tool.name
            );
            assert!(
                !schema_str.contains("\"format\":\"uint32\""),
                "schema for {} contains non-standard 'uint32' format",
                tool.name
            );
            assert!(
                !schema_str.contains("\"format\":\"uint64\""),
                "schema for {} contains non-standard 'uint64' format",
                tool.name
            );
        }
    }

    #[test]
    fn open_args_schema_has_minimum_zero_for_baud_rate() {
        let schema = schema_for!(OpenArgs);
        let json = serde_json::to_value(&schema).unwrap();
        let props = json.get("properties").unwrap();
        let baud = props.get("baud_rate").unwrap();
        assert_eq!(baud.get("minimum"), Some(&serde_json::json!(0)));
    }

    #[test]
    fn connections_resource_schema_has_no_uint_format() {
        use crate::resources::types::ConnectionsResource;
        let schema = schema_for!(ConnectionsResource);
        let json = serde_json::to_string(&schema).unwrap();
        assert!(!json.contains("\"format\":\"uint\""));
    }
}
