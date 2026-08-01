//! Tool argument and response types for serial MCP tools.
//!
//! These structs define the JSON schema for tool requests and responses.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::framing::ParsedFrame;
use crate::serial::{ConnectionSummary, FlowControl, FlushTarget, PortInfo};

// ---- Argument structs ------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpenArgs {
    pub port: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Baud rate. Omitted values resolve to the selected profile's default,
    /// else 115200.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub baud_rate: Option<u32>,
    /// Data bits. Omitted values resolve to the selected profile's default,
    /// else 8.
    #[serde(default)]
    pub data_bits: Option<String>,
    /// Stop bits. Omitted values resolve to the selected profile's default,
    /// else 1.
    #[serde(default)]
    pub stop_bits: Option<String>,
    /// Parity. Omitted values resolve to the selected profile's default,
    /// else none.
    #[serde(default)]
    pub parity: Option<String>,
    /// Flow control. Omitted values resolve to the selected profile's
    /// default, else none.
    #[serde(default)]
    pub flow_control: Option<String>,
    /// Log buffer capacity in events. 0 disables logging. Default: 1024.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub log_capacity: Option<usize>,
    /// Whether logging is enabled. Default: true (ignored when capacity is 0).
    #[serde(default)]
    pub log_enabled: Option<bool>,
    /// Reconnect policy for this connection. Default: disabled.
    #[serde(default)]
    pub reconnect_policy: Option<crate::serial::ReconnectPolicy>,
    /// Default TX framing applied when subsequent `write` calls omit `tx_framing`.
    #[serde(default)]
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    /// Default RX framing applied when subsequent `read`/`subscribe` omit `rx_framing`.
    #[serde(default)]
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    /// Default RX parser applied when subsequent `read`/`subscribe` omit `rx_parser`.
    #[serde(default)]
    pub rx_parser: Option<crate::framing::ParserConfig>,
    /// Default protocol preset. Expands to fill framing/parser gaps.
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
    /// Per-connection RX ring buffer size in bytes. The ring retains
    /// this much RX history between reads/subscribes. Default 256 KiB
    /// (~23s of 115200-baud traffic). Open-time only; reopen to resize.
    /// Validated against the buffer budget pool and a 16 MiB ceiling.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub rx_buffer_size: Option<usize>,
    /// Default max buffered bytes for `read` on this connection.
    /// Default 32768 (32 KiB).
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub max_buffered_bytes: Option<usize>,
    /// Default poll interval for `subscribe` in milliseconds.
    /// Default 200.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub poll_interval_ms: Option<u64>,
    /// Automatic profile-session mode. Default `auto`: bare open reuses the
    /// most recently used high-confidence profile or creates a durable
    /// generated profile for a new high-confidence device; weak or ambiguous
    /// identity gets a transient session. `none` disables automatic
    /// selection/creation for deliberate troubleshooting.
    #[serde(default)]
    pub profile_mode: Option<crate::profiles::ProfileMode>,
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
    /// Optional TX framing configuration. When present, the payload is framed
    /// before being sent (e.g. line terminator appended, delimiter appended,
    /// length prefix prepended, or start/end markers wrapped).
    #[serde(default)]
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    /// Optional protocol preset. When set, fills in default `tx_framing`
    /// for the named protocol. Explicit `tx_framing` overrides the
    /// preset's component.
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ReadArgs {
    pub connection_id: String,
    /// Where to start reading from. `{"type":"cursor"}` (default) — shared
    /// read cursor, `{"type":"now"}` — live edge (skip buffered backlog),
    /// `{"type":"buffer_start"}` — replay everything retained in the ring,
    /// or `{"type":"offset","offset":N}` — absolute stream offset from a
    /// prior result's `from_offset`/`next_offset`.
    #[serde(default)]
    pub from: Option<ReadFrom>,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_timeout_ms_schema")]
    pub timeout_ms: Option<u64>,
    /// Silence timeout in milliseconds. When set, the read stops if no new data
    /// arrives within this window. The timer starts immediately and resets on each
    /// received byte. Omitted or `null` means disabled. `0` is invalid.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_positive_timeout_ms_schema")]
    pub no_new_rx_timeout_ms: Option<u64>,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// Optional match configuration. When present, the read accumulates bytes
    /// until the pattern is found (or another stop condition triggers). The
    /// result includes `matched` and `match_index` fields.
    #[serde(default)]
    pub r#match: Option<crate::match_config::MatchRequest>,
    /// Optional RX frame decoder configuration. When present, the byte stream is
    /// split into structured frames. The result includes `frames` in addition
    /// to the raw `data` field. Can be combined with `match`.
    #[serde(default)]
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    /// Optional RX parser configuration. When present, each decoded frame's
    /// content is interpreted (AT commands, JSON lines, shell prompts). Sibling
    /// to `rx_framing`; the parser operates on frames produced by `rx_framing`.
    #[serde(default)]
    pub rx_parser: Option<crate::framing::ParserConfig>,
    /// Optional protocol preset. When set, fills in default `rx_framing`
    /// and `rx_parser` for the named protocol. Explicit fields override
    /// the preset's corresponding component.
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
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

/// Where to start reading from, shared by `read` and `subscribe`.
///
/// Wire format: `{"type": "now"}`, `{"type": "cursor"}`,
/// `{"type": "buffer_start"}`, or `{"type": "offset", "offset": N}`.
/// Each tool resolves `None` to its own default: `read` defaults to
/// `Cursor` (advances the shared cursor), `subscribe` defaults to `Now`
/// (live edge, does not move the shared cursor).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum ReadFrom {
    /// Start at the live edge — only new data after the call.
    #[serde(rename = "now")]
    Now,
    /// Start at the shared read cursor — replay what `read` hasn't consumed.
    #[serde(rename = "cursor")]
    Cursor,
    /// Replay everything retained in the ring, then go live.
    #[serde(rename = "buffer_start")]
    BufferStart,
    /// Start at an absolute stream offset.
    #[serde(rename = "offset")]
    Offset {
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        offset: u64,
    },
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeArgs {
    pub connection_id: String,
    /// Where to start reading from. `{"type":"now"}` (default) — live edge,
    /// `{"type":"cursor"}` — shared read cursor, `{"type":"buffer_start"}` —
    /// oldest retained byte, or `{"type":"offset","offset":N}` — absolute
    /// stream offset. Replayed history flows through the same framing/match
    /// pipeline as live data.
    #[serde(default)]
    pub from: Option<ReadFrom>,
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
    /// Optional match configuration. When present, the stream detects the
    /// first match and emits a final stop notification with `matched=true`
    /// and `match_index`, then terminates.
    #[serde(default)]
    pub r#match: Option<crate::match_config::MatchRequest>,
    /// Optional RX frame decoder configuration. When present, the stream emits
    /// one notification per decoded frame (instead of per raw chunk). Can
    /// be combined with `match`.
    #[serde(default)]
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    /// Optional RX parser configuration. When present, each decoded frame's
    /// content is interpreted (AT commands, JSON lines, shell prompts). Sibling
    /// to `rx_framing`; the parser operates on frames produced by `rx_framing`.
    #[serde(default)]
    pub rx_parser: Option<crate::framing::ParserConfig>,
    /// Optional protocol preset. When set, fills in default `rx_framing`
    /// and `rx_parser` for the named protocol. Explicit fields override
    /// the preset's corresponding component.
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
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
    /// Log buffer capacity in events. Omitted values use the profile
    /// default (1024). 0 disables logging.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub log_capacity: Option<usize>,
    /// Whether logging is enabled. Omitted values use the profile default.
    #[serde(default)]
    pub log_enabled: Option<bool>,
    /// Per-connection RX ring buffer size in bytes. Omitted values use the
    /// profile default (256 KiB). Open-time only; reopen to resize.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub rx_buffer_size: Option<usize>,
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
    /// Active profile-session binding: how this connection was bound to a
    /// profile (automatic/explicit/generated/transient/disabled), the
    /// profile name, confidence, dirty state, and any persistence error.
    pub profile: Option<crate::profiles::ProfileSessionResult>,
    /// Write-through persistence outcome for a dirty selected-profile
    /// overlay (open override learning). `null` when the open had nothing
    /// to persist (clean/generated/transient/disabled sessions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_persistence: Option<crate::profiles::ProfilePersistenceResult>,
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
    /// Active profile-session binding captured before clean close (with
    /// any dirty/stale state after the close snapshot/retry).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::profiles::ProfileSessionResult>,
    /// Close-snapshot persistence outcome. Always present on a successful
    /// close: a connection without a durable binding reports
    /// `state = "transient"`; dirty/differing persistent bindings are
    /// retried and report `persisted`/`not_needed`/`failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_persistence: Option<crate::profiles::ProfilePersistenceResult>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct WriteResult {
    pub connection_id: String,
    pub name: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_written: usize,
    /// Decoded payload length before framing (always ≤ `bytes_written`).
    /// When `tx_framing` is not used, `decoded_bytes == bytes_written`.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub decoded_bytes: usize,
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
    pub parsed: Option<crate::framing::ParsedFrame>,
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
    /// `peer_disconnected`, `budget_exhausted`, `max_frames`, `framing_error`.
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
    /// that contained the match. `null` when no match, or rx_framing not used,
    /// or match found in raw stream (no rx_framing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub match_frame_index: Option<usize>,
    /// Decoded frames, present when the `rx_framing` option was used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frames: Option<Vec<FrameResult>>,
    /// Number of frames dropped due to decode errors (checksum mismatches
    /// with `validate: true`) and encoding failures. When a checksum-mismatched
    /// frame is dropped by the decoder, it does NOT appear in `frames` and is
    /// counted here instead.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub frames_dropped: usize,
    /// Framing/decode error message. Set when `stop_reason` is
    /// `framing_error` (a stream-fatal SLIP malformed escape or COBS
    /// invalid code); `null` for all other stop reasons. Parity with
    /// subscribe's final-notification `error` field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Absolute stream offset where this read's data starts (clamped to
    /// ring start_offset if the cursor had fallen behind). `null` only
    /// when the read produced no data and no cursor was consumed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub from_offset: Option<u64>,
    /// Absolute stream offset of the cursor after this read (where the next
    /// read starts). Equal to `from_offset + bytes_returned` for a consuming
    /// read. To re-read the same bytes non-destructively, pass the same
    /// `from` on the next read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub next_offset: Option<u64>,
    /// Bytes lost to ring wrap since the cursor's original position. Non-zero
    /// means the cursor had fallen behind `start_offset` and the read
    /// started at `start_offset` instead. Always 0 for a healthy read.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_lost: u64,
    /// Unread bytes remaining in the ring after this read (between
    /// `next_offset` and `end_offset`). 0 when the read drained to the live
    /// edge.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub buffered_remaining: u64,
    /// Absolute stream offset of the oldest byte retained in the ring at result
    /// time. Use with `from: {"type":"offset","offset":start_offset}` to replay
    /// from the oldest retained byte.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub start_offset: u64,
    /// Absolute stream offset of the newest byte retained in the ring at result
    /// time (the live edge). Equals the cursor position `from: {"type":"now"}`
    /// would resolve to.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub end_offset: u64,
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
    /// Active profile-session binding after write-through learning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::profiles::ProfileSessionResult>,
    /// Write-through persistence outcome for the flow-control change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_persistence: Option<crate::profiles::ProfilePersistenceResult>,
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

/// Per-chunk notification emitted by `subscribe` while streaming. Sent
/// as the `data` field of a `notifications/message` event with logger
/// `"serial:<connection_id>"`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribeChunkNotification {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_read: usize,
    pub encoding: String,
    pub data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub bytes_lost: Option<u64>,
}

/// Per-frame notification emitted by `subscribe` when framing is active.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribeFrameNotification {
    pub connection_id: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub frame_index: usize,
    pub frame_type: String,
    pub encoding: String,
    pub data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<ParsedFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched: Option<bool>,
}

/// Per-chunk error notification emitted by `subscribe` when the chunk
/// cannot be encoded in the requested encoding.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribeEncodingErrorNotification {
    pub connection_id: String,
    pub encoding_error: bool,
    pub encoding: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_dropped: usize,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub bytes_lost: Option<u64>,
}

/// Per-frame partial-flush notification emitted by `subscribe` at stop
/// time when a partial frame remains in the decoder.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribePartialFrameNotification {
    pub connection_id: String,
    pub partial: bool,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub frame_index: usize,
    pub frame_type: String,
    pub encoding: String,
    pub data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parsed: Option<ParsedFrame>,
}

/// Final stop notification emitted by `subscribe` when the stream ends.
#[derive(Debug, Serialize, JsonSchema)]
pub struct SubscribeStopNotification {
    pub connection_id: String,
    pub stop_reason: String,
    pub truncated: bool,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_observed: usize,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_returned: usize,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub elapsed_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub no_new_rx_timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub from_offset: Option<u64>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub next_offset: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub bytes_lost: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub match_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub match_frame_index: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub frames_emitted: usize,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub frames_dropped: usize,
    /// Ring start offset (new in 0.8.0 — matches ReadResult's start_offset).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub start_offset: u64,
    /// Ring end offset (new in 0.8.0 — matches ReadResult's end_offset).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub end_offset: u64,
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
    /// RX ring buffer size in bytes.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_buffer_size: usize,
    /// Oldest retained byte stream offset (ring start).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_start_offset: u64,
    /// Total bytes appended since open (ring end, monotonic).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_end_offset: u64,
    /// Shared read cursor position (where the next read starts).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_cursor: u64,
    /// Unread bytes between cursor and end_offset.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_buffered_unread: u64,
    /// Lifetime total of bytes lost to ring wrap.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_bytes_wrapped_total: u64,
    /// Active profile-session binding. `null` for connections inserted
    /// directly by low-level tests.
    pub profile: Option<crate::profiles::ProfileSessionResult>,
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
    /// Active profile-session binding after write-through learning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::profiles::ProfileSessionResult>,
    /// Write-through persistence outcome for the reconfigure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_persistence: Option<crate::profiles::ProfilePersistenceResult>,
}

/// Summary of a single profile returned by `list_profiles`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ProfileSummary {
    pub name: String,
    pub selector: crate::profiles::ProfileSelector,
    pub defaults: crate::profiles::ProfileDefaults,
    /// Bookkeeping metadata (generated flag, revision, timestamps, usage).
    pub metadata: crate::profiles::ProfileMetadata,
    /// Bounded history of prior selector/defaults snapshots (for future
    /// rollback). Empty for profiles that were never overwritten.
    pub revisions: Vec<crate::profiles::ProfileRevision>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListProfilesResult {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub count: usize,
    pub profiles: Vec<ProfileSummary>,
}

// ---- Default helpers -------------------------------------------------------

pub fn default_encoding() -> String {
    "utf8".into()
}
pub fn default_flush_target() -> FlushTarget {
    FlushTarget::Both
}
pub fn default_break_duration_ms() -> u64 {
    250
}

// ---- Profile management tools ----------------------------------------------

/// Configure connection defaults. Two modes:
/// - `profile` mode: write defaults to a named profile in the profiles TOML.
///   Applies to future `open_profile` calls. Does NOT touch live connections.
/// - `connection` mode: mutate defaults on a live connection (the four
///   framing defaults + reconnect_policy + max_buffered_bytes +
///   poll_interval_ms). Does NOT persist to disk. `rx_buffer_size`,
///   serial-line params, `log_capacity`, and `log_enabled` only apply via
///   profile + reopen (LogBuffer has no live setter for capacity/enabled).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigureArgs {
    /// Profile name to write (profile mode), or connection name to mutate
    /// (connection mode). Exactly one of `profile` or `connection_id` must be set.
    #[serde(default)]
    pub profile: Option<String>,
    #[serde(default)]
    pub connection_id: Option<String>,
    /// If true (profile mode), replace an existing profile with the same name.
    /// If false (default), return an error when the name already exists.
    #[serde(default)]
    pub overwrite: bool,
    /// Defaults to apply. All fields optional — omit a field to leave it
    /// unchanged on the profile / connection.
    #[serde(default)]
    pub defaults: crate::profiles::ProfileDefaults,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ConfigureResult {
    /// Which mode was applied: "profile" or "connection".
    pub mode: String,
    /// The effective defaults after applying the change.
    pub defaults: crate::profiles::ProfileDefaults,
    /// For profile mode: true if newly created, false if overwritten.
    /// For connection mode: always null.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<bool>,
    /// Active profile-session binding after connection-mode write-through
    /// learning. `null` in profile mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<crate::profiles::ProfileSessionResult>,
    /// Write-through persistence outcome for connection mode. `null` in
    /// profile mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_persistence: Option<crate::profiles::ProfilePersistenceResult>,
}

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

/// Roll a profile back to a prior retained revision (Phase 3B).
///
/// Restores the snapshot's selector/defaults as a NEW monotonic revision;
/// active connections bound to the profile remain unchanged and become
/// stale. A wrong `expected_revision` (concurrent modification) or an
/// evicted target `revision` is a tool error that leaves the file
/// unchanged.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RollbackProfileArgs {
    pub profile_name: String,
    /// Prior retained revision to restore (see `list_profiles` revisions).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub revision: u64,
    /// Revision the profile currently has (from `list_profiles` metadata).
    /// Guards against rolling back a concurrently modified profile.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub expected_revision: u64,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RollbackProfileResult {
    pub profile_name: String,
    /// The retained revision whose snapshot was restored.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub restored_from_revision: u64,
    /// The revision the profile had before this rollback
    /// (`expected_revision`).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub previous_revision: u64,
    /// The new monotonic revision after the rollback (never moves
    /// backward).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub revision: u64,
    pub selector: crate::profiles::ProfileSelector,
    pub defaults: crate::profiles::ProfileDefaults,
    pub metadata: crate::profiles::ProfileMetadata,
    /// Number of same-process open connections bound to the profile whose
    /// live state was left unchanged (marked stale).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub active_connections_unchanged: usize,
    pub persistence: crate::profiles::ProfilePersistenceResult,
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

/// Algorithm for `compute_checksum`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    /// NMEA-0183 XOR checksum (single byte, XOR of all bytes).
    Xor,
    /// Modbus ASCII LRC (single byte, two's complement of byte sum).
    Lrc,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ComputeChecksumArgs {
    /// Input data encoded as `encoding`. Decoded before checksumming.
    pub data: String,
    /// Encoding of `data`: "utf8", "hex", or "base64". Default "utf8".
    #[serde(default = "default_encoding")]
    pub encoding: String,
    pub algorithm: ChecksumAlgorithm,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ComputeChecksumResult {
    pub algorithm: String,
    /// Checksum value as a 2-char uppercase hex string (e.g. "5A").
    pub checksum_hex: String,
    /// Raw checksum byte as an integer (0-255).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub checksum: u8,
    /// Number of bytes checksummed.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub byte_count: usize,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TransactArgs {
    pub connection_id: String,
    pub data: String,
    #[serde(default = "default_encoding")]
    pub encoding: String,
    /// Where the read half starts. `{"type":"now"}` (default) — live edge,
    /// skip pre-write buffered backlog; `{"type":"cursor"}` — shared read
    /// cursor; `{"type":"buffer_start"}` — replay everything retained; or
    /// `{"type":"offset","offset":N}`.
    #[serde(default)]
    pub from: Option<ReadFrom>,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_timeout_ms_schema")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    #[schemars(schema_with = "crate::schema_helpers::option_positive_timeout_ms_schema")]
    pub no_new_rx_timeout_ms: Option<u64>,
    #[serde(default)]
    pub r#match: Option<crate::match_config::MatchRequest>,
    /// Optional TX framing for the write half.
    #[serde(default)]
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    /// Optional RX framing for the read half.
    #[serde(default)]
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    /// Optional RX parser for the read half.
    #[serde(default)]
    pub rx_parser: Option<crate::framing::ParserConfig>,
    /// Optional protocol preset (applies to both write and read halves).
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TransactResult {
    pub connection_id: String,
    pub name: Option<String>,
    pub write: WriteResult,
    pub read: ReadResult,
}
