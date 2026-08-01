//! Serial port discovery, configuration, and a session-less connection manager.
//!
//! Public surface:
//! - [`PortInfo::list_available`] enumerates serial ports on the host.
//! - [`SerialConnection::open`] opens a single configured port.
//! - [`ConnectionManager`] holds a set of open connections indexed by id and
//!   rejects double-opens of the same port.
//!
//! The implementation is split into focused submodules: configuration types
//! and defaults (`config`), OS port enumeration (`port_info`), the connection
//! and its I/O backend (`connection`), the multi-connection registry
//! (`manager`), and in-memory test backends (`test_support`). Everything
//! formerly public at `crate::serial::*` is re-exported here at the flat
//! original path.

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
pub use manager::ConnectionManager;
pub use port_info::{PortInfo, PortProvider, PortTransport, SystemPortProvider};

// =============================================================================
// JSON Schema regression tests — DO NOT DELETE.
//
// These tests guard against the schemars "non-standard uint format" regression
// that has bitten this crate repeatedly (commits b12b09fd, bc37a0b0, and the
// regression fixed in this commit on `PortInfo`).
//
// Background
// ----------
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
//
// Why this keeps coming back
// --------------------------
// The previous guard (`tool_schemas_have_no_nonstandard_uint_formats` in
// `src/tools/mod.rs`) only checked the tool `outputSchema` strings and only
// asserted on `uint`/`uint32`/`uint64`. It missed:
//   1. `u8`/`u16` formats (this regression's `PortInfo.vid`/`pid`/`interface`).
//   2. Types that appear in resource/prompt schemas but are also reachable via
//      tool outputs (e.g. `PortInfo` is in `ListPortsResult` AND
//      `ConnectionStatus.port_info`).
//
// The tests below close both gaps:
//   - They enumerate every known public `JsonSchema`-deriving struct and
//     reject *any* `uint*` format keyword.
//   - They also keep the tool-level string scan, now covering uint8/uint16
//     (see `src/tools/mod.rs`).
//
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
    use crate::serial::{ConnectionStatus, PortInfo};
    use crate::tools::types::{
        CaptureBootArgs, CaptureBootReset, CaptureBootResult, ClearLogResult, CloseResult,
        ComputeChecksumResult, ConfigureResult, DeleteProfileResult, ExportLogResult, FlushResult,
        GetLogResult, GetStatusResult, ListConnectionsResult, ListPortsResult, ListProfilesResult,
        OpenResult, PortProfileMatch, ProfileMatchCandidate, ReadResult, ReconfigureResult,
        ReconnectResult, RollbackProfileArgs, RollbackProfileResult, SaveProfileResult,
        SendBreakResult, SetDtrRtsResult, SetFlowControlResult, SubscribeChunkNotification,
        SubscribeEncodingErrorNotification, SubscribeFrameNotification,
        SubscribePartialFrameNotification, SubscribeResult, SubscribeStopNotification,
        TransactResult, UnsubscribeResult, WriteResult,
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

    // Profile types (already fixed in b12b09fd; guarded against regressions).
    check_schema!(profile_has_no_uint_formats, Profile);
    check_schema!(profile_selector_has_no_uint_formats, ProfileSelector);
    check_schema!(profile_metadata_has_no_uint_formats, ProfileMetadata);
    check_schema!(profile_revision_has_no_uint_formats, ProfileRevision);
    // Phase 3A session-result shape exposed on open/status/connection
    // summaries (guards the Option<u64> revision field).
    check_schema!(
        profile_session_result_has_no_uint_formats,
        ProfileSessionResult
    );

    // Tool result types reachable by clients.
    // Keep this list in sync with the `#[tool]` methods in `src/server.rs`
    // and with `tool_catalog()` in `src/server.rs`.
    check_schema!(list_ports_result_has_no_uint_formats, ListPortsResult);
    // Phase 4 list_ports profile-match preview types (guards the u64
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
    check_schema!(subscribe_result_has_no_uint_formats, SubscribeResult);
    check_schema!(
        subscribe_chunk_notification_has_no_uint_formats,
        SubscribeChunkNotification
    );
    check_schema!(
        subscribe_frame_notification_has_no_uint_formats,
        SubscribeFrameNotification
    );
    check_schema!(
        subscribe_encoding_error_notification_has_no_uint_formats,
        SubscribeEncodingErrorNotification
    );
    check_schema!(
        subscribe_partial_frame_notification_has_no_uint_formats,
        SubscribePartialFrameNotification
    );
    check_schema!(
        subscribe_stop_notification_has_no_uint_formats,
        SubscribeStopNotification
    );
    check_schema!(unsubscribe_result_has_no_uint_formats, UnsubscribeResult);
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
    // Phase 5 atomic boot capture (guards mark_offset, pre_mark_bytes,
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

    // Profile defaults (Phase 5 framing fields — no unsigned fields, but guard).
    check_schema!(
        profile_defaults_has_no_uint_formats,
        crate::profiles::ProfileDefaults
    );
}
