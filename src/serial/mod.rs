//! Serial port discovery, configuration, and a session-less connection manager.
//!
//! Public surface:
//! - [`PortInfo::list_available`] enumerates serial ports on the host.
//! - [`SerialConnection::open`] opens a single configured port.
//! - [`ConnectionManager`] holds a set of open connections indexed by id and
//!   rejects double-opens of the same port. Its connection-opening boundary
//!   ([`ConnectionOpener`]) is injectable so alternate backends can drive
//!   the full surface without an OS serial port.
//!
//! The implementation is split into focused submodules: configuration types
//! and defaults (`config`), OS port enumeration (`port_info`), the connection
//! and its I/O backend (`connection`), the multi-connection registry
//! (`manager`), and in-memory test backends (`test_support`). Public types are
//! re-exported at `crate::serial::*`.

mod config;
mod connection;
mod manager;
mod port_info;
pub mod test_support;

pub(crate) use config::{data_bits_to_str, flow_control_to_str, parity_to_str, stop_bits_to_str};
pub use config::{
    is_fatal_disconnect, ActiveProfileBinding, ConnectionConfig, ConnectionState, ConnectionStatus,
    ConnectionSummary, DataBits, FlowControl, FlushTarget, Parity, ReconnectPolicy, StopBits,
    MAX_BAUD_RATE,
};
pub use connection::{SerialConnection, SerialIo};
pub use manager::{ConnectionManager, ConnectionOpener};
pub use port_info::{PortInfo, PortProvider, PortTransport, SystemPortProvider};

// =============================================================================
// JSON Schema unsigned-integer regression guards.
//
// schemars (1.x) emits a `"format": "uintN"` keyword for unsigned integer
// types: `uint8`, `uint16`, `uint32`, `uint64`, `uint`. None of these are part
// of the JSON Schema spec. Validators (jsonschema, AJV, …) log a warning like
//     unknown format "uint16" ignored in schema at path "#/properties/vid"
// and then drop the constraint. Functionally harmless but noisy, and a sign
// that a field was added without the required `#[schemars(schema_with = ...)]`
// override.
//
// Every time a new struct with a `uN`/`Option<uN>` field is added and derives
// `JsonSchema`, it MUST annotate that field with `crate::schema_helpers::uint_schema`
// (for `uN`) or `crate::schema_helpers::option_uint_schema` (for `Option<uN>`).
// The previous guard (`tool_schemas_have_no_nonstandard_uint_formats` in
// `src/tools/mod.rs`) only checked the tool `outputSchema` strings and only
// asserted on `uint`/`uint32`/`uint64`. It missed:
//   1. `u8`/`u16` formats (this regression's `PortInfo.vid`/`pid`/`interface`).
//   2. Types that appear in resource/prompt schemas but are also reachable via
//      tool outputs (e.g. `PortInfo` is in `ListPortsResult` AND
//      `ConnectionStatus.port_info`).
// These tests close both gaps:
//   - They enumerate every known public `JsonSchema`-deriving struct and
//     reject *any* `uint*` format keyword.
//   - They also keep the tool-level string scan, now covering uint8/uint16
//     (see `src/tools/mod.rs`).
// If you add a new public type that derives `JsonSchema` and has unsigned
// integer fields, ADD IT to the `check_schema!` list below. The compile-time
// cost is tiny; the cost of shipping noisy schemas to every MCP client is not.
// =============================================================================
#[cfg(test)]
mod schema {
    use schemars::schema_for;
    use serde_json::{self, Value};

    use crate::framing::{
        Frame, ParsedFrame, RxFramingConfig, RxFramingMode, TxFramingConfig, TxFramingMode,
    };
    use crate::profiles::{
        Profile, ProfileMetadata, ProfilePersistenceResult, ProfileRevision, ProfileSelector,
        ProfileSessionResult,
    };
    use crate::serial::{ConnectionStatus, PortInfo, PortTransport};
    use crate::tools::types::{
        CaptureBootArgs, CaptureBootReset, CaptureBootResult, ClearLogResult, CloseResult,
        ComputeChecksumResult, ConfigureResult, DeleteProfileResult, ExportLogResult, FlushResult,
        GetLogResult, GetStatusResult, ListConnectionsResult, ListPortsResult, ListProfilesResult,
        OpenResult, PortProfileMatch, ProfileMatchCandidate, ReadResult, ReconfigureResult,
        ReconnectResult, RollbackProfileArgs, RollbackProfileResult, SaveProfileResult,
        SendBreakResult, SetDtrRtsResult, SetFlowControlResult, TransactResult, WriteResult,
    };

    /// Walk a JSON Schema `Value` and collect every `"format"` whose value
    /// starts with `uint`. Returns the dotted path of each offending node so
    /// failures point at the exact field.
    fn collect_nonstandard_uint_formats(schema: &Value) -> Vec<String> {
        let mut offenders = Vec::new();
        let mut stack: Vec<(String, &Value)> = Vec::new();
        stack.push((String::new(), schema));

        while let Some((path, value)) = stack.pop() {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(fmt)) = map.get("format") {
                        if fmt.starts_with("uint") {
                            offenders.push(format!("{path} (format={fmt})"));
                        }
                    }
                    for (k, v) in map {
                        let next = if path.is_empty() {
                            k.clone()
                        } else {
                            format!("{path}.{k}")
                        };
                        stack.push((next, v));
                    }
                }
                Value::Array(arr) => {
                    for (i, v) in arr.iter().enumerate() {
                        stack.push((format!("{path}[{i}]"), v));
                    }
                }
                _ => {}
            }
        }

        offenders
    }

    /// Generate a focused test for one type that asserts its JSON Schema
    /// contains no non-standard `uint*` format keywords.
    macro_rules! check_schema {
        ($name:ident, $ty:ty) => {
            #[test]
            fn $name() {
                let schema = schema_for!($ty);
                let json = serde_json::to_value(&schema).expect("schema serializes");
                let offenders = collect_nonstandard_uint_formats(&json);
                assert!(
                    offenders.is_empty(),
                    "{} schema emits non-standard JSON Schema uint formats: {offenders:?}.\n\
                         Fix: annotate each uN/Option<uN> field on the type with\n\
                         `#[schemars(schema_with = \"crate::schema_helpers::uint_schema\")]`\n\
                         (or `option_uint_schema` for Option<uN>). See\n\
                         src/schema_helpers.rs and the header comment of this test\n\
                         module (serial::schema) for the full rationale.",
                    stringify!($ty),
                );
            }
        };
    }

    // Core identity + status types (the source of this regression).
    check_schema!(port_info_has_no_uint_formats, PortInfo);
    check_schema!(connection_status_has_no_uint_formats, ConnectionStatus);

    /// Regression guard for the `PortInfo` `required` bug: schemars 1.2.2
    /// does not recognise `Option<T>` through `#[schemars(schema_with = ...)]`,
    /// so `vid`/`pid`/`interface` used to land in the schema's `required`
    /// array even though non-USB ports (e.g. `/dev/ttyS0`) omit those fields
    /// during serialization (`skip_serializing_if`). Strict MCP clients then
    /// rejected `list_ports` output with
    /// `Invalid structured content: 'vid' is a required property`.
    ///
    /// The fix keeps the `schema_with` override (it suppresses the
    /// non-standard `uint16` format) and adds `#[serde(default)]`, which
    /// makes schemars treat the field as optional without emitting any
    /// `"default"` key (the default is `None`, so `skip_serializing_if`
    /// skips it). Serialized output is unchanged.
    #[test]
    fn port_info_optional_usb_fields_not_required() {
        let schema = schema_for!(PortInfo);
        let json = serde_json::to_value(&schema).expect("schema serializes");
        let required: Vec<&str> = json
            .pointer("/required")
            .and_then(Value::as_array)
            .expect("PortInfo schema must have a required list")
            .iter()
            .filter_map(Value::as_str)
            .collect();

        // Optional USB-identity fields: must stay OUT of `required`.
        for field in ["vid", "pid", "interface"] {
            assert!(
                !required.contains(&field),
                "PortInfo schema lists optional USB field `{field}` in \
                 required={required:?}; non-USB ports omit it during \
                 serialization and MCP clients reject the payload. Keep \
                 `#[serde(skip_serializing_if = \"Option::is_none\")]` and \
                 add `default` (see src/port_info.rs)."
            );
            // Still declared in `properties`, just optional.
            assert!(
                json.pointer(&format!("/properties/{field}")).is_some(),
                "PortInfo schema is missing the `{field}` property"
            );
        }

        // Non-optional identity fields must stay required.
        for field in ["name", "display_name", "description", "transport"] {
            assert!(
                required.contains(&field),
                "PortInfo schema must keep `{field}` in required, got \
                 required={required:?}"
            );
        }

        // Serialized output is unchanged: `None` USB-identity fields are still
        // omitted, so `skip_serializing_if` behaviour is intact even with
        // `default` present.
        let non_usb = PortInfo {
            name: "/dev/ttyS0".into(),
            display_name: "ttyS0".into(),
            description: "Serial Port".into(),
            hardware_id: None,
            transport: PortTransport::Unknown,
            vid: None,
            pid: None,
            serial_number: None,
            manufacturer: None,
            product: None,
            interface: None,
        };
        let serialized = serde_json::to_value(&non_usb).expect("PortInfo serializes");
        let obj = serialized
            .as_object()
            .expect("serialized PortInfo must be a JSON object");
        for field in ["vid", "pid", "interface"] {
            assert!(
                !obj.contains_key(field),
                "serialized non-USB PortInfo must omit `{field}`, got keys: {:?}",
                obj.keys().collect::<Vec<_>>()
            );
        }

        // The `default` evaluates to `None` (skipped), so the schema must not
        // advertise a `"default"` key on the property — locks the claim made
        // in the doc comment above.
        for field in ["vid", "pid", "interface"] {
            let prop = json
                .pointer(&format!("/properties/{field}"))
                .expect("PortInfo schema must declare the `{field}` property");
            assert!(
                prop.get("default").is_none(),
                "PortInfo schema property `{field}` must not carry a \
                 \"default\" key, got: {prop}"
            );
        }
    }

    /// Same schemars `required` bug as `PortInfo` vid/pid/interface, on the
    /// prompt arguments type: `DiagnosePortArgs.baud_rate` is
    /// `Option<u32>` with `schema_with` + `skip_serializing_if`, so before
    /// the fix it landed in the schema's `required` array while callers
    /// omit it during serialization. Strict MCP clients reject prompt
    /// arguments on the same validation path as tool output.
    #[test]
    fn diagnose_port_args_baud_rate_not_required() {
        let schema = schema_for!(crate::prompts::types::DiagnosePortArgs);
        let json = serde_json::to_value(&schema).expect("schema serializes");
        let required: Vec<&str> = json
            .pointer("/required")
            .and_then(Value::as_array)
            .expect("DiagnosePortArgs schema must have a required list")
            .iter()
            .filter_map(Value::as_str)
            .collect();

        // Optional field: must stay OUT of `required`.
        assert!(
            !required.contains(&"baud_rate"),
            "DiagnosePortArgs schema lists optional field `baud_rate` in \
             required={required:?}; callers omit it during serialization \
             and strict MCP clients reject the arguments. Keep \
             `#[serde(skip_serializing_if = \"Option::is_none\")]` and add \
             `default` (see src/prompts/types.rs)."
        );
        // Still declared in `properties`, just optional.
        assert!(
            json.pointer("/properties/baud_rate").is_some(),
            "DiagnosePortArgs schema is missing the `baud_rate` property"
        );
        // No `"default"` key leaks into the schema (the default is `None`,
        // so `skip_serializing_if` omits it from serialized output).
        let prop = json
            .pointer("/properties/baud_rate")
            .expect("DiagnosePortArgs schema must declare `baud_rate`");
        assert!(
            prop.get("default").is_none(),
            "DiagnosePortArgs schema property `baud_rate` must not carry a \
             \"default\" key, got: {prop}"
        );

        // Serialized output unchanged: `None` is still omitted.
        let args = crate::prompts::types::DiagnosePortArgs {
            port: "/dev/ttyUSB0".into(),
            baud_rate: None,
        };
        let obj = serde_json::to_value(&args)
            .expect("DiagnosePortArgs serializes")
            .as_object()
            .expect("serialized DiagnosePortArgs must be a JSON object")
            .clone();
        assert!(
            !obj.contains_key("baud_rate"),
            "serialized DiagnosePortArgs must omit `baud_rate`, got keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }

    // Profile types (already fixed in b12b09fd; guarded against regressions).
    check_schema!(profile_has_no_uint_formats, Profile);
    check_schema!(profile_selector_has_no_uint_formats, ProfileSelector);
    check_schema!(profile_metadata_has_no_uint_formats, ProfileMetadata);
    check_schema!(profile_revision_has_no_uint_formats, ProfileRevision);
    // Session-result shape exposed on open/status/connection
    // summaries (guards the Option<u64> revision field).
    check_schema!(
        profile_session_result_has_no_uint_formats,
        ProfileSessionResult
    );

    // Tool result types reachable by clients.
    // Keep this list in sync with the `#[tool]` methods in `src/server.rs`
    // and with `tool_catalog()` in `src/server.rs`.
    check_schema!(list_ports_result_has_no_uint_formats, ListPortsResult);
    // `list_ports` profile-match preview types (guards the u64
    // revision / Option<u64> last_used_at_ms fields).
    check_schema!(
        profile_match_candidate_has_no_uint_formats,
        ProfileMatchCandidate
    );
    check_schema!(port_profile_match_has_no_uint_formats, PortProfileMatch);
    check_schema!(
        list_connections_result_has_no_uint_formats,
        ListConnectionsResult
    );
    check_schema!(open_result_has_no_uint_formats, OpenResult);
    check_schema!(close_result_has_no_uint_formats, CloseResult);
    check_schema!(write_result_has_no_uint_formats, WriteResult);
    check_schema!(read_result_has_no_uint_formats, ReadResult);
    check_schema!(flush_result_has_no_uint_formats, FlushResult);
    check_schema!(set_dtr_rts_result_has_no_uint_formats, SetDtrRtsResult);
    check_schema!(
        set_flow_control_result_has_no_uint_formats,
        SetFlowControlResult
    );
    check_schema!(send_break_result_has_no_uint_formats, SendBreakResult);
    check_schema!(get_status_result_has_no_uint_formats, GetStatusResult);
    check_schema!(reconfigure_result_has_no_uint_formats, ReconfigureResult);
    check_schema!(list_profiles_result_has_no_uint_formats, ListProfilesResult);
    check_schema!(save_profile_result_has_no_uint_formats, SaveProfileResult);
    check_schema!(
        delete_profile_result_has_no_uint_formats,
        DeleteProfileResult
    );
    check_schema!(get_log_result_has_no_uint_formats, GetLogResult);
    check_schema!(clear_log_result_has_no_uint_formats, ClearLogResult);
    check_schema!(export_log_result_has_no_uint_formats, ExportLogResult);
    check_schema!(reconnect_result_has_no_uint_formats, ReconnectResult);
    check_schema!(configure_result_has_no_uint_formats, ConfigureResult);
    check_schema!(
        compute_checksum_result_has_no_uint_formats,
        ComputeChecksumResult
    );
    check_schema!(transact_result_has_no_uint_formats, TransactResult);
    // Atomic boot capture (guards mark_offset, pre_mark_bytes,
    // and the nested ReadResult's uint fields).
    check_schema!(capture_boot_args_has_no_uint_formats, CaptureBootArgs);
    check_schema!(capture_boot_reset_has_no_uint_formats, CaptureBootReset);
    check_schema!(capture_boot_result_has_no_uint_formats, CaptureBootResult);
    check_schema!(
        rollback_profile_args_has_no_uint_formats,
        RollbackProfileArgs
    );
    check_schema!(
        rollback_profile_result_has_no_uint_formats,
        RollbackProfileResult
    );
    check_schema!(
        profile_persistence_result_has_no_uint_formats,
        ProfilePersistenceResult
    );

    // Framing config types (checked for uint format regressions on fields like
    // prefix_size, max_frames, and cobs delimiter).
    check_schema!(tx_framing_config_has_no_uint_formats, TxFramingConfig);
    check_schema!(tx_framing_mode_has_no_uint_formats, TxFramingMode);
    check_schema!(rx_framing_config_has_no_uint_formats, RxFramingConfig);
    check_schema!(rx_framing_mode_has_no_uint_formats, RxFramingMode);

    // Parsed frame enum (guards uint fields on ParsedFrame::ModbusAscii
    // and any future variants that carry unsigned integers).
    check_schema!(parsed_frame_has_no_uint_formats, ParsedFrame);

    // Decoded frame (guards the Vec<u8> data field + the usize index field).
    check_schema!(frame_has_no_uint_formats, Frame);

    // Profile defaults (framing fields — no unsigned fields, but guard).
    check_schema!(
        profile_defaults_has_no_uint_formats,
        crate::profiles::ProfileDefaults
    );
}
