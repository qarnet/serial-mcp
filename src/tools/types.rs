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
