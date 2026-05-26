//! Tool argument and response types for serial MCP tools.
//!
//! These structs define the JSON schema for tool requests and responses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::serial::{FlushTarget, PortInfo};

// ---- Argument structs ------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
pub struct OpenArgs {
    pub port: String,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CloseArgs {
    pub connection_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteArgs {
    pub connection_id: String,
    pub data: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadArgs {
    pub connection_id: String,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub timeout_ms: Option<u64>,
    #[serde(default = "default_max_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub max_bytes: usize,
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct FlushArgs {
    pub connection_id: String,
    #[serde(default = "default_flush_target")]
    pub target: FlushTarget,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetDtrRtsArgs {
    pub connection_id: String,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendBreakArgs {
    pub connection_id: String,
    #[serde(default = "default_break_duration_ms")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub duration_ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubscribeArgs {
    pub connection_id: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    #[serde(default = "default_subscribe_chunk_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub max_chunk_bytes: usize,
    #[serde(default = "default_subscribe_poll_ms")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub poll_interval_ms: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnsubscribeArgs {
    pub connection_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitForArgs {
    pub connection_id: String,
    pub pattern: String,
    #[serde(default = "default_encoding")]
    pub pattern_encoding: String,
    #[serde(default = "default_wait_timeout_ms")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timeout_ms: u64,
    #[serde(default = "default_wait_max_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub max_bytes: usize,
    #[serde(default = "default_encoding")]
    pub response_encoding: String,
}

// ---- Response structs ------------------------------------------------------

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListPortsResult {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub count: usize,
    pub ports: Vec<PortInfo>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct OpenResult {
    pub connection_id: String,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CloseResult {
    pub connection_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WriteResult {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_written: usize,
    pub encoding: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReadResult {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_read: usize,
    pub encoding: String,
    pub data: String,
    pub timed_out: bool,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timeout_ms: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub elapsed_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FlushResult {
    pub connection_id: String,
    pub target: FlushTarget,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SetDtrRtsResult {
    pub connection_id: String,
    pub dtr: bool,
    pub rts: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SendBreakResult {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub duration_ms: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub actual_duration_ms: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribeResult {
    pub connection_id: String,
    pub encoding: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub max_chunk_bytes: usize,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub poll_interval_ms: u64,
    pub replaced_previous: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct UnsubscribeResult {
    pub connection_id: String,
    pub was_active: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct WaitForResult {
    pub connection_id: String,
    pub matched: bool,
    pub timed_out: bool,
    pub data: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_read: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub match_index: Option<usize>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub timeout_ms: u64,
    pub response_encoding: String,
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
pub fn default_max_bytes() -> usize {
    1024
}
pub fn default_flush_target() -> FlushTarget {
    FlushTarget::Both
}
pub fn default_break_duration_ms() -> u64 {
    250
}
pub fn default_wait_timeout_ms() -> u64 {
    2000
}
pub fn default_wait_max_bytes() -> usize {
    4096
}
pub fn default_subscribe_chunk_bytes() -> usize {
    1024
}
pub fn default_subscribe_poll_ms() -> u64 {
    200
}
