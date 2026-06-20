//! Tool argument and response types for serial MCP tools.
//!
//! These structs define the JSON schema for tool requests and responses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::serial::{ConnectionSummary, FlowControl, FlushTarget, PortInfo};

// ---- Argument structs ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpenArgs {
    pub port: String,
    #[serde(default)]
    pub name: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: String,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: String,
    #[serde(default = "default_parity")]
    pub parity: String,
    #[serde(default = "default_flow_control")]
    pub flow_control: String,
    /// Log buffer capacity in events. 0 disables logging. Default: 1024.
    #[serde(default = "default_log_capacity")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub log_capacity: usize,
    /// Whether logging is enabled. Default: true (ignored when capacity is 0).
    #[serde(default = "default_true")]
    pub log_enabled: bool,
    /// Reconnect policy for this connection. Default: disabled.
    #[serde(default)]
    pub reconnect_policy: crate::serial::ReconnectPolicy,
}

fn default_log_capacity() -> usize {
    1024
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CloseArgs {
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListConnectionsArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteArgs {
    pub connection_id: String,
    pub data: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadArgs {
    pub connection_id: String,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_timeout_ms_schema")]
    pub timeout_ms: Option<u64>,
    /// Silence timeout in milliseconds. When set, the read stops if no new data
    /// arrives within this window. The timer starts immediately and resets on each
    /// received byte. Omitted or `null` means disabled. `0` is invalid.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_positive_timeout_ms_schema")]
    pub no_new_rx_timeout_ms: Option<u64>,
    /// Maximum bytes to buffer before the read stops. When exceeded, the operation
    /// stops with `max_buffered_bytes` and `truncated` is `true` in the result.
    #[serde(default = "default_max_buffered_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::read_max_buffered_bytes_schema")]
    pub max_buffered_bytes: usize,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// Optional match configuration. When present, the read accumulates bytes
    /// until the pattern is found (or another stop condition triggers). The
    /// result includes `matched` and `match_index` fields.
    #[serde(default)]
    pub r#match: Option<crate::match_config::MatchRequest>,
    /// Optional frame decoder configuration. When present, the byte stream is
    /// split into structured frames. The result includes `frames` in addition
    /// to the raw `data` field. Can be combined with `match`.
    #[serde(default)]
    pub framing: Option<crate::framing::FramingConfig>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FlushArgs {
    pub connection_id: String,
    #[serde(default = "default_flush_target")]
    pub target: FlushTarget,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetDtrRtsArgs {
    pub connection_id: String,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetFlowControlArgs {
    pub connection_id: String,
    pub flow_control: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SendBreakArgs {
    pub connection_id: String,
    #[serde(default = "default_break_duration_ms")]
    #[schemars(schema_with = "crate::schema_helpers::timeout_ms_schema")]
    pub duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeArgs {
    pub connection_id: String,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_timeout_ms_schema")]
    pub timeout_ms: Option<u64>,
    /// Silence timeout in milliseconds. When set, the subscription stops if no
    /// new data arrives within this window. The timer starts immediately and
    /// resets on each received byte. Omitted or `null` means disabled. `0` is
    /// invalid.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_positive_timeout_ms_schema")]
    pub no_new_rx_timeout_ms: Option<u64>,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_subscribe_buffered_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::stream_buffered_bytes_schema")]
    pub max_buffered_bytes: usize,
    #[serde(default = "default_subscribe_poll_ms")]
    #[schemars(schema_with = "crate::schema_helpers::poll_interval_ms_schema")]
    pub poll_interval_ms: u64,
    /// Optional match configuration. When present, the stream detects the
    /// first match and emits a final stop notification with `matched=true`
    /// and `match_index`, then terminates.
    #[serde(default)]
    pub r#match: Option<crate::match_config::MatchRequest>,
    /// Optional frame decoder configuration. When present, the stream emits
    /// one notification per decoded frame (instead of per raw chunk). Can
    /// be combined with `match`.
    #[serde(default)]
    pub framing: Option<crate::framing::FramingConfig>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UnsubscribeArgs {
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetStatusArgs {
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReconfigureArgs {
    pub connection_id: String,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub baud_rate: Option<u32>,
    #[serde(default)]
    pub data_bits: Option<String>,
    #[serde(default)]
    pub stop_bits: Option<String>,
    #[serde(default)]
    pub parity: Option<String>,
    #[serde(default)]
    pub flow_control: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListProfilesArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpenProfileArgs {
    pub profile: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Log buffer capacity in events. 0 disables logging. Default: 1024.
    #[serde(default = "default_log_capacity")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub log_capacity: usize,
    /// Whether logging is enabled. Default: true (ignored when capacity is 0).
    #[serde(default = "default_true")]
    pub log_enabled: bool,
}

// ---- Response structs ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListPortsResult {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub count: usize,
    pub ports: Vec<PortInfo>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpenResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListConnectionsResult {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub count: usize,
    pub connections: Vec<ConnectionSummary>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CloseResult {
    pub connection_id: String,
    pub name: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteResult {
    pub connection_id: String,
    pub name: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_written: usize,
    pub encoding: String,
}

/// A single decoded frame returned in a read result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FrameResult {
    pub data: String,
    pub encoding: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub frame_index: usize,
    pub frame_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed: Option<ParsedFrameResult>,
}

/// Structured field interpretation of a decoded frame.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "parser", rename_all = "snake_case")]
pub enum ParsedFrameResult {
    AtCommand {
        response_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        fields: Vec<String>,
    },
    Json(serde_json::Value),
    ShellPrompt {
        prompt: String,
        prompt_type: String,
    },
    Raw,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadResult {
    pub connection_id: String,
    pub name: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_read: usize,
    pub encoding: String,
    pub data: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timeout_ms: u64,
    /// Configured silence timeout in milliseconds. `null` when not set.
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub no_new_rx_timeout_ms: Option<u64>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub elapsed_ms: u64,
    /// Why the operation stopped. One of: `data_complete`, `timeout`,
    /// `match_found`, `max_buffered_bytes`, `no_new_rx_timeout`,
    /// `connection_closed`, `cancelled`, `read_error`, `channel_closed`,
    /// `peer_disconnected`, `budget_exhausted`.
    pub stop_reason: String,
    /// `true` when `bytes_returned < bytes_observed` because the result
    /// data was capped (e.g. `max_buffered_bytes` limit exceeded observed
    /// data).
    pub truncated: bool,
    /// Total bytes the operation observed from the RX stream before stopping.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_observed: usize,
    /// Bytes actually returned in the result `data` field.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_returned: usize,
    /// Whether the match pattern was found. `false` when no `match` option was
    /// provided. `true` when `match` was provided and the pattern was found
    /// before the operation stopped for another reason.
    #[serde(default)]
    pub matched: bool,
    /// Byte offset within `data` where the matched pattern starts, or `null`
    /// when no match was found or no `match` option was provided.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub match_index: Option<usize>,
    /// When framing is active and a match was found, the index of the frame
    /// that contained the match. `null` when no match, or framing not used,
    /// or match found in raw stream (no framing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub match_frame_index: Option<usize>,
    /// Decoded frames, present when the `framing` option was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<FrameResult>>,
    /// Number of frames dropped due to encoding failures.
    /// Always 0 unless per-frame encoding fails (rare).
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub frames_dropped: usize,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FlushResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub target: FlushTarget,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetDtrRtsResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetFlowControlResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub flow_control: FlowControl,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SendBreakResult {
    pub connection_id: String,
    pub name: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub duration_ms: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub actual_duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub encoding: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub max_buffered_bytes: usize,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub poll_interval_ms: u64,
    pub replaced_previous: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UnsubscribeResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub was_active: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetStatusResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
    pub data_bits: String,
    pub stop_bits: String,
    pub parity: String,
    pub flow_control: String,
    pub is_open: bool,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub tx_bytes: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_bytes: u64,
    /// Last I/O activity as milliseconds since Unix epoch, or null.
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub last_activity_ms: Option<u64>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub read_ops: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub write_ops: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub truncation_count: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub notification_drop_count: u64,
    /// OS-level port identity captured at open time. `null` for connections
    /// without identity data (e.g. loopback tests).
    pub port_info: Option<crate::serial::PortInfo>,
    /// Current connection health state.
    pub state: crate::serial::ConnectionState,
    /// Number of reconnect attempts since last disconnect.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub reconnect_attempts: u64,
    /// Last fatal error message, or null.
    pub last_error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReconfigureResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
    pub data_bits: String,
    pub stop_bits: String,
    pub parity: String,
    pub flow_control: String,
}

/// Summary of a single profile returned by `list_profiles`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProfileSummary {
    pub name: String,
    pub selector: crate::profiles::ProfileSelector,
    pub defaults: crate::profiles::ProfileDefaults,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListProfilesResult {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub count: usize,
    pub profiles: Vec<ProfileSummary>,
}

// ---- Default helpers -------------------------------------------------------

pub fn default_data_bits() -> String {
    "8".into()
}
pub fn default_stop_bits() -> String {
    "1".into()
}
pub fn default_parity() -> String {
    "none".into()
}
pub fn default_flow_control() -> String {
    "none".into()
}
pub fn default_encoding() -> String {
    "utf8".into()
}
pub fn default_max_buffered_bytes() -> usize {
    2048
}
pub fn default_flush_target() -> FlushTarget {
    FlushTarget::Both
}
pub fn default_break_duration_ms() -> u64 {
    250
}
pub fn default_subscribe_buffered_bytes() -> usize {
    2048
}
pub fn default_subscribe_poll_ms() -> u64 {
    200
}

// ---- Profile management tools ----------------------------------------------

/// Save a profile by snapshotting an open connection's identity and config.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SaveProfileArgs {
    pub connection_id: String,
    /// Desired profile name. Must be unique (or overwrite if overwrite=true).
    pub profile_name: String,
    /// If true, replace an existing profile with the same name.
    /// If false (default), return an error when the name already exists.
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SaveProfileResult {
    pub name: String,
    pub selector: crate::profiles::ProfileSelector,
    pub defaults: crate::profiles::ProfileDefaults,
    /// `true` when a new profile was created; `false` when existing was overwritten.
    pub created: bool,
}

/// Delete a profile by name.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeleteProfileArgs {
    pub profile_name: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeleteProfileResult {
    pub profile_name: String,
}

// ---- Log tools -------------------------------------------------------------

/// Arguments for the `get_log` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetLogArgs {
    pub connection_id: String,
    /// Return only events after this timestamp (ms since Unix epoch).
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub since_ms: Option<u64>,
    /// Maximum number of events to return. Default: no limit.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GetLogResult {
    /// Whether logging is enabled for this connection.
    pub log_enabled: bool,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub capacity: usize,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub total_events: usize,
    pub events: Vec<crate::log_buffer::LogEntry>,
}

/// Arguments for the `clear_log` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClearLogArgs {
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClearLogResult {
    pub connection_id: String,
}

/// Arguments for the `export_log` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExportLogArgs {
    pub connection_id: String,
    /// File path to write the JSONL log to.
    pub path: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ExportLogResult {
    pub connection_id: String,
    pub path: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub events_written: usize,
}

// ---- Reconnect tool --------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconnectArgs {
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReconnectResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    pub state: crate::serial::ConnectionState,
}
