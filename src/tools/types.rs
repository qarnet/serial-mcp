//! Tool argument and response types for serial MCP tools.
//!
//! These structs define the JSON schema for tool requests and responses.
//! Input integer fields use [`FlexibleU64`], [`FlexibleOptionU64`],
//! [`FlexibleU32`], and [`FlexibleUsize`] wrappers so MCP clients that
//! stringify numbers (e.g. `"5000"` instead of `5000`) still work.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::flex_deserialize::{FlexibleOptionU64, FlexibleU32, FlexibleU64, FlexibleUsize};
use crate::serial::{FlushTarget, PortInfo};

// ---- Argument structs ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpenArgs {
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: FlexibleU32,
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
    pub timeout_ms: FlexibleOptionU64,
    #[serde(default = "default_max_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::read_max_bytes_schema")]
    pub max_bytes: FlexibleUsize,
    #[serde(default = "default_encoding")]
    pub encoding: String,
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
pub struct SendBreakArgs {
    pub connection_id: String,
    #[serde(default = "default_break_duration_ms")]
    #[schemars(schema_with = "crate::schema_helpers::timeout_ms_schema")]
    pub duration_ms: FlexibleU64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeArgs {
    pub connection_id: String,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_timeout_ms_schema")]
    pub timeout_ms: FlexibleOptionU64,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_subscribe_chunk_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::stream_chunk_bytes_schema")]
    pub max_chunk_bytes: FlexibleUsize,
    #[serde(default = "default_subscribe_poll_ms")]
    #[schemars(schema_with = "crate::schema_helpers::poll_interval_ms_schema")]
    pub poll_interval_ms: FlexibleU64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UnsubscribeArgs {
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WaitForArgs {
    pub connection_id: String,
    pub pattern: String,
    #[serde(default = "default_encoding")]
    pub pattern_encoding: String,
    #[serde(default = "default_wait_timeout_ms")]
    #[schemars(schema_with = "crate::schema_helpers::timeout_ms_schema")]
    pub timeout_ms: FlexibleU64,
    #[serde(default = "default_wait_max_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::wait_max_bytes_schema")]
    pub max_bytes: FlexibleUsize,
    #[serde(default = "default_encoding")]
    pub response_encoding: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadLineArgs {
    pub connection_id: String,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_timeout_ms_schema")]
    pub timeout_ms: FlexibleOptionU64,
    #[serde(default = "default_read_line_max_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::wait_max_bytes_schema")]
    pub max_bytes: FlexibleUsize,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

// ---- Response structs ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListPortsResult {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub count: usize,
    pub ports: Vec<PortInfo>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct VersionResult {
    pub name: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpenResult {
    pub connection_id: String,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CloseResult {
    pub connection_id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteResult {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_written: usize,
    pub encoding: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadResult {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_read: usize,
    pub encoding: String,
    pub data: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timeout_ms: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub elapsed_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FlushResult {
    pub connection_id: String,
    pub target: FlushTarget,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SetDtrRtsResult {
    pub connection_id: String,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SendBreakResult {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub duration_ms: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub actual_duration_ms: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResult {
    pub connection_id: String,
    pub encoding: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub max_chunk_bytes: usize,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub poll_interval_ms: u64,
    pub replaced_previous: bool,
    /// Always `null` since PLAN 1b. Subscribe is now always background;
    /// data arrives as notifications, not inline.
    pub data: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub bytes_read: Option<usize>,
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub elapsed_ms: Option<u64>,
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct UnsubscribeResult {
    pub connection_id: String,
    pub was_active: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WaitForResult {
    pub connection_id: String,
    pub matched: bool,
    pub data: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_read: usize,
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub match_index: Option<usize>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timeout_ms: u64,
    pub response_encoding: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadLineResult {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_read: usize,
    pub encoding: String,
    pub line: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timeout_ms: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub elapsed_ms: u64,
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
pub fn default_max_bytes() -> FlexibleUsize {
    FlexibleUsize(1024)
}
pub fn default_flush_target() -> FlushTarget {
    FlushTarget::Both
}
pub fn default_break_duration_ms() -> FlexibleU64 {
    FlexibleU64(250)
}
pub fn default_wait_timeout_ms() -> FlexibleU64 {
    FlexibleU64(2000)
}
pub fn default_wait_max_bytes() -> FlexibleUsize {
    FlexibleUsize(4096)
}
pub fn default_subscribe_chunk_bytes() -> FlexibleUsize {
    FlexibleUsize(1024)
}
pub fn default_subscribe_poll_ms() -> FlexibleU64 {
    FlexibleU64(200)
}
pub fn default_read_line_max_bytes() -> FlexibleUsize {
    FlexibleUsize(4096)
}
