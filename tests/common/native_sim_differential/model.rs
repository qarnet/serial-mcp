//! Typed, narrow normalization for differential public MCP outcomes.

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context, Result};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const LOGICAL_CONNECTION: &str = "$CONNECTION";
pub const LOGICAL_ENDPOINT: &str = "$ENDPOINT";

/// One executable differential scenario across isolated migration batches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DifferentialCase {
    #[serde(rename = "native_ping_roundtrip")]
    PingRoundtrip,
    #[serde(rename = "native_pending_read_then_write_ping_roundtrip")]
    PendingReadThenWritePingRoundtrip,
    #[serde(rename = "native_split_writes_preserve_command_order")]
    SplitWritesPreserveCommandOrder,
    #[serde(rename = "native_framing_reports_single_split_command")]
    FramingDiagnosticSplitPing,
    #[serde(rename = "native_trace_reports_exact_split_byte_sequence")]
    TraceDiagnosticSplitPing,
    #[serde(rename = "native_get_status_after_write_increments_tx_counter")]
    GetStatusAfterWriteIncrementsTxCounter,
    #[serde(rename = "native_reconfigure_baud_rate_persists")]
    ReconfigureBaudRatePersists,
    #[serde(rename = "native_named_connection_appears_in_list_connections")]
    NamedConnectionAppearsInListConnections,
    #[serde(rename = "native_set_flow_control_updates_summary_and_result")]
    SetFlowControlUpdatesSummaryAndResult,
    #[serde(rename = "native_open_with_flow_control_persists_in_summary")]
    OpenWithFlowControlPersistsInSummary,
    #[serde(rename = "native_read_regex_matches_pong")]
    RegexMatchesPong,
    #[serde(rename = "native_read_glob_matches_pong_line")]
    GlobMatchesPongLine,
    #[serde(rename = "native_read_line_framing_splits_lines")]
    LineFramingSplitsLines,
    #[serde(rename = "native_read_framing_max_frames_stops")]
    FramingMaxFramesStops,
    #[serde(rename = "native_read_framing_plus_match_combined")]
    FramingPlusMatchCombined,
    #[serde(rename = "native_explicit_rx_framing_beats_connection_default")]
    ExplicitRxFramingBeatsConnectionDefault,
    #[serde(rename = "native_read_delimiter_framing_decodes")]
    DelimiterFramingDecodes,
    #[serde(rename = "native_read_length_prefixed_framing_decodes")]
    LengthPrefixedFramingDecodes,
    #[serde(rename = "native_read_start_end_framing_decodes")]
    StartEndFramingDecodes,
    #[serde(rename = "native_write_tx_framing_modes_observed_via_trace")]
    TxFramingModesObservedViaTrace,
    #[serde(rename = "native_read_explicit_line_endings_split_correctly")]
    ExplicitLineEndingsSplitCorrectly,
    #[serde(rename = "native_read_match_on_spam_complete")]
    FloodMatcherSpamComplete,
    #[serde(rename = "native_read_buffer_budget_stops_under_flood")]
    FloodBufferBudget,
    #[serde(rename = "native_partial_line_buffered_then_completed")]
    PartialLineThenCompletePing,
    #[serde(rename = "native_ack_command_provides_pre_execution_ack")]
    AckStateMachine,
    #[serde(rename = "native_flush_output_after_full_delivery_is_safe")]
    OutputFlushAfterDelivery,
    #[serde(rename = "native_read_slip_decodes_frame")]
    SlipHappyPath,
    #[serde(rename = "native_read_slip_malformed_escape_returns_partial_result")]
    SlipMalformedEscape,
    #[serde(rename = "native_read_slip_recovers_after_error_on_next_call")]
    SlipRecoveryAfterMalformed,
    #[serde(rename = "native_read_cobs_preset_decodes_frame")]
    CobsPresetDecode,
    #[serde(rename = "native_read_at_parser_parses_pong")]
    AtParserPong,
    #[serde(rename = "native_open_protocol_default_drives_write_and_read")]
    AtProtocolDefaultPong,
    #[serde(rename = "native_read_json_parser_decodes_jsonout")]
    JsonParserJsonout,
    #[serde(rename = "native_read_ndjson_preset_decodes_json_frames")]
    NdjsonPresetJsonFrames,
    #[serde(rename = "native_read_ndjson_preset_skips_empty_lines")]
    NdjsonPresetSkipsEmptyLines,
}

/// Explicit executable-batch membership. Pending and retired registry rows do
/// not have a batch because they do not execute differential scenarios yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifferentialBatch {
    CommandLifecycle,
    GenericMatchingFraming,
    RawGenericFraming,
    FloodBuffer,
    CommandDiagnostics,
    AckState,
    OutputFlush,
    SlipHappy,
    SlipMalformed,
    SlipRecovery,
    CobsPreset,
    AtParser,
    AtProtocolDefault,
    JsonParser,
    NdjsonPreset,
}

impl DifferentialCase {
    pub const BATCH_ONE: [Self; 8] = [
        Self::PingRoundtrip,
        Self::PendingReadThenWritePingRoundtrip,
        Self::SplitWritesPreserveCommandOrder,
        Self::GetStatusAfterWriteIncrementsTxCounter,
        Self::ReconfigureBaudRatePersists,
        Self::NamedConnectionAppearsInListConnections,
        Self::SetFlowControlUpdatesSummaryAndResult,
        Self::OpenWithFlowControlPersistsInSummary,
    ];

    pub const BATCH_TWO: [Self; 6] = [
        Self::RegexMatchesPong,
        Self::GlobMatchesPongLine,
        Self::LineFramingSplitsLines,
        Self::FramingMaxFramesStops,
        Self::FramingPlusMatchCombined,
        Self::ExplicitRxFramingBeatsConnectionDefault,
    ];

    pub const BATCH_THREE: [Self; 5] = [
        Self::DelimiterFramingDecodes,
        Self::LengthPrefixedFramingDecodes,
        Self::StartEndFramingDecodes,
        Self::TxFramingModesObservedViaTrace,
        Self::ExplicitLineEndingsSplitCorrectly,
    ];

    pub const BATCH_FOUR: [Self; 2] = [Self::FloodMatcherSpamComplete, Self::FloodBufferBudget];

    pub const BATCH_FIVE: [Self; 3] = [
        Self::FramingDiagnosticSplitPing,
        Self::TraceDiagnosticSplitPing,
        Self::PartialLineThenCompletePing,
    ];

    pub const BATCH_SIX: [Self; 1] = [Self::AckStateMachine];

    pub const BATCH_SEVEN: [Self; 1] = [Self::OutputFlushAfterDelivery];

    pub const BATCH_EIGHT: [Self; 1] = [Self::SlipHappyPath];

    pub const BATCH_NINE: [Self; 1] = [Self::SlipMalformedEscape];

    pub const BATCH_TEN: [Self; 1] = [Self::SlipRecoveryAfterMalformed];

    pub const BATCH_ELEVEN: [Self; 1] = [Self::CobsPresetDecode];

    pub const BATCH_TWELVE: [Self; 1] = [Self::AtParserPong];

    pub const BATCH_THIRTEEN: [Self; 1] = [Self::AtProtocolDefaultPong];

    pub const BATCH_FOURTEEN: [Self; 1] = [Self::JsonParserJsonout];

    pub const BATCH_FIFTEEN: [Self; 2] = [
        Self::NdjsonPresetJsonFrames,
        Self::NdjsonPresetSkipsEmptyLines,
    ];

    pub const ALL: [Self; 35] = [
        Self::PingRoundtrip,
        Self::PendingReadThenWritePingRoundtrip,
        Self::SplitWritesPreserveCommandOrder,
        Self::GetStatusAfterWriteIncrementsTxCounter,
        Self::ReconfigureBaudRatePersists,
        Self::NamedConnectionAppearsInListConnections,
        Self::SetFlowControlUpdatesSummaryAndResult,
        Self::OpenWithFlowControlPersistsInSummary,
        Self::RegexMatchesPong,
        Self::GlobMatchesPongLine,
        Self::LineFramingSplitsLines,
        Self::FramingMaxFramesStops,
        Self::FramingPlusMatchCombined,
        Self::ExplicitRxFramingBeatsConnectionDefault,
        Self::DelimiterFramingDecodes,
        Self::LengthPrefixedFramingDecodes,
        Self::StartEndFramingDecodes,
        Self::TxFramingModesObservedViaTrace,
        Self::ExplicitLineEndingsSplitCorrectly,
        Self::FloodMatcherSpamComplete,
        Self::FloodBufferBudget,
        Self::FramingDiagnosticSplitPing,
        Self::TraceDiagnosticSplitPing,
        Self::PartialLineThenCompletePing,
        Self::AckStateMachine,
        Self::OutputFlushAfterDelivery,
        Self::SlipHappyPath,
        Self::SlipMalformedEscape,
        Self::SlipRecoveryAfterMalformed,
        Self::CobsPresetDecode,
        Self::AtParserPong,
        Self::AtProtocolDefaultPong,
        Self::JsonParserJsonout,
        Self::NdjsonPresetJsonFrames,
        Self::NdjsonPresetSkipsEmptyLines,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::PingRoundtrip => "native_ping_roundtrip",
            Self::PendingReadThenWritePingRoundtrip => {
                "native_pending_read_then_write_ping_roundtrip"
            }
            Self::SplitWritesPreserveCommandOrder => "native_split_writes_preserve_command_order",
            Self::FramingDiagnosticSplitPing => "native_framing_reports_single_split_command",
            Self::TraceDiagnosticSplitPing => "native_trace_reports_exact_split_byte_sequence",
            Self::GetStatusAfterWriteIncrementsTxCounter => {
                "native_get_status_after_write_increments_tx_counter"
            }
            Self::ReconfigureBaudRatePersists => "native_reconfigure_baud_rate_persists",
            Self::NamedConnectionAppearsInListConnections => {
                "native_named_connection_appears_in_list_connections"
            }
            Self::SetFlowControlUpdatesSummaryAndResult => {
                "native_set_flow_control_updates_summary_and_result"
            }
            Self::OpenWithFlowControlPersistsInSummary => {
                "native_open_with_flow_control_persists_in_summary"
            }
            Self::RegexMatchesPong => "native_read_regex_matches_pong",
            Self::GlobMatchesPongLine => "native_read_glob_matches_pong_line",
            Self::LineFramingSplitsLines => "native_read_line_framing_splits_lines",
            Self::FramingMaxFramesStops => "native_read_framing_max_frames_stops",
            Self::FramingPlusMatchCombined => "native_read_framing_plus_match_combined",
            Self::ExplicitRxFramingBeatsConnectionDefault => {
                "native_explicit_rx_framing_beats_connection_default"
            }
            Self::DelimiterFramingDecodes => "native_read_delimiter_framing_decodes",
            Self::LengthPrefixedFramingDecodes => "native_read_length_prefixed_framing_decodes",
            Self::StartEndFramingDecodes => "native_read_start_end_framing_decodes",
            Self::TxFramingModesObservedViaTrace => {
                "native_write_tx_framing_modes_observed_via_trace"
            }
            Self::ExplicitLineEndingsSplitCorrectly => {
                "native_read_explicit_line_endings_split_correctly"
            }
            Self::FloodMatcherSpamComplete => "native_read_match_on_spam_complete",
            Self::FloodBufferBudget => "native_read_buffer_budget_stops_under_flood",
            Self::PartialLineThenCompletePing => "native_partial_line_buffered_then_completed",
            Self::AckStateMachine => "native_ack_command_provides_pre_execution_ack",
            Self::OutputFlushAfterDelivery => "native_flush_output_after_full_delivery_is_safe",
            Self::SlipHappyPath => "native_read_slip_decodes_frame",
            Self::SlipMalformedEscape => "native_read_slip_malformed_escape_returns_partial_result",
            Self::SlipRecoveryAfterMalformed => {
                "native_read_slip_recovers_after_error_on_next_call"
            }
            Self::CobsPresetDecode => "native_read_cobs_preset_decodes_frame",
            Self::AtParserPong => "native_read_at_parser_parses_pong",
            Self::AtProtocolDefaultPong => "native_open_protocol_default_drives_write_and_read",
            Self::JsonParserJsonout => "native_read_json_parser_decodes_jsonout",
            Self::NdjsonPresetJsonFrames => "native_read_ndjson_preset_decodes_json_frames",
            Self::NdjsonPresetSkipsEmptyLines => "native_read_ndjson_preset_skips_empty_lines",
        }
    }

    pub const fn batch(self) -> DifferentialBatch {
        match self {
            Self::PingRoundtrip
            | Self::PendingReadThenWritePingRoundtrip
            | Self::SplitWritesPreserveCommandOrder
            | Self::GetStatusAfterWriteIncrementsTxCounter
            | Self::ReconfigureBaudRatePersists
            | Self::NamedConnectionAppearsInListConnections
            | Self::SetFlowControlUpdatesSummaryAndResult
            | Self::OpenWithFlowControlPersistsInSummary => DifferentialBatch::CommandLifecycle,
            Self::RegexMatchesPong
            | Self::GlobMatchesPongLine
            | Self::LineFramingSplitsLines
            | Self::FramingMaxFramesStops
            | Self::FramingPlusMatchCombined
            | Self::ExplicitRxFramingBeatsConnectionDefault => {
                DifferentialBatch::GenericMatchingFraming
            }
            Self::DelimiterFramingDecodes
            | Self::LengthPrefixedFramingDecodes
            | Self::StartEndFramingDecodes
            | Self::TxFramingModesObservedViaTrace
            | Self::ExplicitLineEndingsSplitCorrectly => DifferentialBatch::RawGenericFraming,
            Self::FloodMatcherSpamComplete | Self::FloodBufferBudget => {
                DifferentialBatch::FloodBuffer
            }
            Self::FramingDiagnosticSplitPing
            | Self::TraceDiagnosticSplitPing
            | Self::PartialLineThenCompletePing => DifferentialBatch::CommandDiagnostics,
            Self::AckStateMachine => DifferentialBatch::AckState,
            Self::OutputFlushAfterDelivery => DifferentialBatch::OutputFlush,
            Self::SlipHappyPath => DifferentialBatch::SlipHappy,
            Self::SlipMalformedEscape => DifferentialBatch::SlipMalformed,
            Self::SlipRecoveryAfterMalformed => DifferentialBatch::SlipRecovery,
            Self::CobsPresetDecode => DifferentialBatch::CobsPreset,
            Self::AtParserPong => DifferentialBatch::AtParser,
            Self::AtProtocolDefaultPong => DifferentialBatch::AtProtocolDefault,
            Self::JsonParserJsonout => DifferentialBatch::JsonParser,
            Self::NdjsonPresetJsonFrames | Self::NdjsonPresetSkipsEmptyLines => {
                DifferentialBatch::NdjsonPreset
            }
        }
    }
}

/// Normalized public outcome for one semantic scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioOutcome {
    pub case: DifferentialCase,
    pub observations: Vec<Observation>,
}

/// One typed public MCP observation retained by this batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Observation {
    Open(OpenObservation),
    Write(WriteObservation),
    Read(ReadObservation),
    StatusDelta(StatusDeltaObservation),
    Reconfigure(ReconfigureObservation),
    ConnectionSummary(ConnectionSummaryObservation),
    SetFlowControl(SetFlowControlObservation),
    PeerWire(PeerWireObservation),
    Flush(FlushObservation),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenObservation {
    pub is_error: Option<bool>,
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    pub baud_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteObservation {
    pub is_error: Option<bool>,
    pub connection_id: String,
    pub name: Option<String>,
    pub bytes_written: usize,
    pub decoded_bytes: usize,
    pub encoding: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadObservation {
    pub is_error: Option<bool>,
    pub connection_id: String,
    pub name: Option<String>,
    pub encoding: String,
    pub payload: Vec<u8>,
    pub bytes_read: usize,
    pub stop_reason: String,
    pub truncated: bool,
    pub bytes_observed: usize,
    pub bytes_returned: usize,
    pub matched: bool,
    pub match_index: Option<usize>,
    pub match_frame_index: Option<usize>,
    pub frames: Option<Vec<FrameObservation>>,
    pub frames_dropped: usize,
    pub error: Option<String>,
    // Normalized reads retain canonical outcome fields. Caller-supplied request
    // echoes such as `timeout_ms` and `no_new_rx_timeout_ms` are not modeled.
    // Wall-clock `elapsed_ms` remains deliberately excluded following the
    // existing Batch 4 characterization policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<ReadPositionObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadPositionObservation {
    pub from_offset: u64,
    pub next_offset: u64,
    pub bytes_lost: u64,
    pub buffered_remaining: u64,
    pub start_offset: u64,
    pub end_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameObservation {
    pub frame_index: usize,
    pub frame_type: String,
    pub encoding: String,
    pub payload: Vec<u8>,
    pub parsed: Option<ParsedFrameObservation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "parser", rename_all = "snake_case")]
pub enum ParsedFrameObservation {
    Raw,
    AtCommand {
        response_type: String,
        command: Option<String>,
        status: Option<String>,
        fields: Vec<String>,
    },
    Json {
        fields: BTreeMap<String, Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SerialSettingsObservation {
    pub baud_rate: u32,
    pub data_bits: String,
    pub stop_bits: String,
    pub parity: String,
    pub flow_control: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NormalizedConnectionState {
    Open,
    Disconnected,
    Reconnecting,
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusDeltaObservation {
    pub before_is_error: Option<bool>,
    pub after_is_error: Option<bool>,
    pub name: Option<String>,
    pub serial: SerialSettingsObservation,
    pub state: NormalizedConnectionState,
    pub tx_bytes: u64,
    pub rx_bytes: u64,
    pub read_ops: u64,
    pub write_ops: u64,
    pub truncation_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconfigureObservation {
    pub is_error: Option<bool>,
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    pub serial: SerialSettingsObservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSummaryObservation {
    pub is_error: Option<bool>,
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    pub baud_rate: u32,
    pub flow_control: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetFlowControlObservation {
    pub is_error: Option<bool>,
    pub connection_id: String,
    pub name: Option<String>,
    pub flow_control: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlushObservation {
    pub is_error: Option<bool>,
    pub connection_id: String,
    pub name: Option<String>,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerWireObservation {
    pub direction: PeerWireDirection,
    pub mode: PeerWireMode,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerWireDirection {
    HostToPeer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerWireMode {
    Delimiter,
    LengthPrefixed,
    StartEnd,
    Slip,
}

/// Dynamic values validated before being replaced by logical report values.
#[derive(Debug, Clone)]
pub struct NormalizationContext {
    actual_connection_id: String,
    actual_endpoint: String,
}

impl NormalizationContext {
    pub fn actual_connection_id(&self) -> &str {
        &self.actual_connection_id
    }
}

#[derive(Debug)]
pub struct StatusSnapshot {
    is_error: Option<bool>,
    name: Option<String>,
    serial: SerialSettingsObservation,
    state: NormalizedConnectionState,
    tx_bytes: u64,
    rx_bytes: u64,
    read_ops: u64,
    write_ops: u64,
    truncation_count: u64,
    last_activity_ms: Option<u64>,
}

/// Validate and normalize a public `open` result.
pub fn normalize_open(
    value: &Value,
    actual_endpoint: &str,
    is_error: Option<bool>,
) -> Result<(OpenObservation, NormalizationContext)> {
    let object = required_object(value, "open")?;
    let actual_connection_id = required_string(object, "connection_id", "open")?;
    ensure!(
        !actual_connection_id.is_empty(),
        "open result connection_id must be nonempty"
    );
    let port = required_string(object, "port", "open")?;
    ensure!(
        port == actual_endpoint,
        "open result port {port:?} did not match endpoint {actual_endpoint:?}"
    );
    let context = NormalizationContext {
        actual_connection_id,
        actual_endpoint: actual_endpoint.to_owned(),
    };
    Ok((
        OpenObservation {
            is_error,
            connection_id: LOGICAL_CONNECTION.to_owned(),
            name: optional_string(object, "name", "open")?,
            port: LOGICAL_ENDPOINT.to_owned(),
            baud_rate: required_u32(object, "baud_rate", "open")?,
        },
        context,
    ))
}

/// Validate and normalize a public `write` result.
pub fn normalize_write(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<WriteObservation> {
    let object = required_object(value, "write")?;
    validate_connection_reference(object, "write", context)?;
    Ok(WriteObservation {
        is_error,
        connection_id: LOGICAL_CONNECTION.to_owned(),
        name: optional_string(object, "name", "write")?,
        bytes_written: required_usize(object, "bytes_written", "write")?,
        decoded_bytes: required_usize(object, "decoded_bytes", "write")?,
        encoding: required_string(object, "encoding", "write")?,
    })
}

/// Validate and normalize every public `read` result used by differential
/// batches. Payloads retain their effective wire encoding and exact decoded
/// bytes, including every framed payload and supported parsed AT field.
pub fn normalize_read(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<ReadObservation> {
    normalize_read_impl(value, context, is_error, false)
}

/// Validate and normalize a positioned read while retaining its public position
/// and backlog fields. Positioned target reads must carry all six fields.
pub fn normalize_positioned_read(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<ReadObservation> {
    normalize_read_impl(value, context, is_error, true)
}

fn normalize_read_impl(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
    retain_position: bool,
) -> Result<ReadObservation> {
    let object = required_object(value, "read")?;
    validate_connection_reference(object, "read", context)?;
    let (encoding, payload) = decode_payload(object, "read")?;
    let bytes_read = required_usize(object, "bytes_read", "read")?;
    let bytes_observed = required_usize(object, "bytes_observed", "read")?;
    let bytes_returned = required_usize(object, "bytes_returned", "read")?;
    let truncated = required_bool(object, "truncated", "read")?;
    let matched = required_bool(object, "matched", "read")?;
    let match_index = optional_usize(object, "match_index", "read")?;
    let match_frame_index = optional_usize_or_missing(object, "match_frame_index", "read")?;
    let frames = optional_frames(object, "read")?;
    let frames_dropped = required_usize(object, "frames_dropped", "read")?;
    let error = optional_string_or_missing(object, "error", "read")?;
    if matched {
        let index = match_index.context("matched read omitted match_index")?;
        ensure!(
            index <= payload.len(),
            "read match_index {index} exceeded payload length {}",
            payload.len()
        );
    } else {
        ensure!(
            match_index.is_none(),
            "unmatched read unexpectedly carried match_index {match_index:?}"
        );
    }
    let position = retain_position
        .then(|| normalize_read_position(object))
        .transpose()?;
    Ok(ReadObservation {
        is_error,
        connection_id: LOGICAL_CONNECTION.to_owned(),
        name: optional_string(object, "name", "read")?,
        encoding,
        payload,
        bytes_read,
        stop_reason: required_string(object, "stop_reason", "read")?,
        truncated,
        bytes_observed,
        bytes_returned,
        matched,
        match_index,
        match_frame_index,
        frames,
        frames_dropped,
        error,
        position,
    })
}

fn normalize_read_position(object: &Map<String, Value>) -> Result<ReadPositionObservation> {
    let operation = "positioned read";
    Ok(ReadPositionObservation {
        from_offset: required_u64(object, "from_offset", operation)?,
        next_offset: required_u64(object, "next_offset", operation)?,
        bytes_lost: required_u64(object, "bytes_lost", operation)?,
        buffered_remaining: required_u64(object, "buffered_remaining", operation)?,
        start_offset: required_u64(object, "start_offset", operation)?,
        end_offset: required_u64(object, "end_offset", operation)?,
    })
}

/// Batch 1 retains its original raw-read counter invariants. Framed Batch 2
/// rows intentionally do not infer counter relationships from payload length.
pub fn normalize_batch_one_raw_read(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<ReadObservation> {
    let observation = normalize_read(value, context, is_error)?;
    ensure!(
        observation.encoding == "utf8",
        "batch-1 read returned unexpected effective encoding {:?}",
        observation.encoding
    );
    ensure!(
        observation.bytes_returned == observation.payload.len(),
        "read bytes_returned {} did not equal returned payload length {}",
        observation.bytes_returned,
        observation.payload.len()
    );
    ensure!(
        observation.bytes_observed >= observation.bytes_returned,
        "read bytes_observed {} was less than bytes_returned {}",
        observation.bytes_observed,
        observation.bytes_returned
    );
    ensure!(
        observation.bytes_read >= observation.bytes_returned,
        "read bytes_read {} was less than bytes_returned {}",
        observation.bytes_read,
        observation.bytes_returned
    );
    Ok(observation)
}

/// Validate a public status result before computing a scenario-local delta.
pub fn normalize_status(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<StatusSnapshot> {
    let object = required_object(value, "get_status")?;
    validate_connection_reference(object, "get_status", context)?;
    let port = required_string(object, "port", "get_status")?;
    ensure!(
        port == context.actual_endpoint,
        "get_status port {port:?} did not match endpoint {:?}",
        context.actual_endpoint
    );
    Ok(StatusSnapshot {
        is_error,
        name: optional_string(object, "name", "get_status")?,
        serial: serial_settings(object, "get_status")?,
        state: connection_state(object, "state", "get_status")?,
        tx_bytes: required_u64(object, "tx_bytes", "get_status")?,
        rx_bytes: required_u64(object, "rx_bytes", "get_status")?,
        read_ops: required_u64(object, "read_ops", "get_status")?,
        write_ops: required_u64(object, "write_ops", "get_status")?,
        truncation_count: required_u64(object, "truncation_count", "get_status")?,
        last_activity_ms: optional_u64(object, "last_activity_ms", "get_status")?,
    })
}

/// Calculate one normalized status delta and validate timestamp ordering.
pub fn status_delta(
    before: StatusSnapshot,
    after: StatusSnapshot,
) -> Result<StatusDeltaObservation> {
    ensure!(
        before.name == after.name,
        "connection name changed between status snapshots: before={:?}, after={:?}",
        before.name,
        after.name
    );
    let before_activity = before
        .last_activity_ms
        .context("baseline status omitted last_activity_ms after boot synchronization")?;
    let after_activity = after
        .last_activity_ms
        .context("post-operation status omitted last_activity_ms")?;
    ensure!(
        after_activity >= before_activity,
        "last_activity_ms moved backwards from {before_activity} to {after_activity}"
    );
    Ok(StatusDeltaObservation {
        before_is_error: before.is_error,
        after_is_error: after.is_error,
        name: after.name,
        serial: after.serial,
        state: after.state,
        tx_bytes: checked_delta(after.tx_bytes, before.tx_bytes, "tx_bytes")?,
        rx_bytes: checked_delta(after.rx_bytes, before.rx_bytes, "rx_bytes")?,
        read_ops: checked_delta(after.read_ops, before.read_ops, "read_ops")?,
        write_ops: checked_delta(after.write_ops, before.write_ops, "write_ops")?,
        truncation_count: checked_delta(
            after.truncation_count,
            before.truncation_count,
            "truncation_count",
        )?,
    })
}

/// Validate and normalize a public `reconfigure` result.
pub fn normalize_reconfigure(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<ReconfigureObservation> {
    let object = required_object(value, "reconfigure")?;
    validate_connection_reference(object, "reconfigure", context)?;
    let port = required_string(object, "port", "reconfigure")?;
    ensure!(
        port == context.actual_endpoint,
        "reconfigure port {port:?} did not match endpoint {:?}",
        context.actual_endpoint
    );
    Ok(ReconfigureObservation {
        is_error,
        connection_id: LOGICAL_CONNECTION.to_owned(),
        name: optional_string(object, "name", "reconfigure")?,
        port: LOGICAL_ENDPOINT.to_owned(),
        serial: serial_settings(object, "reconfigure")?,
    })
}

/// Validate and normalize one matching public connection summary.
pub fn normalize_connection_summary(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<ConnectionSummaryObservation> {
    let object = required_object(value, "connection summary")?;
    validate_connection_reference(object, "connection summary", context)?;
    let port = required_string(object, "port", "connection summary")?;
    ensure!(
        port == context.actual_endpoint,
        "connection summary port {port:?} did not match endpoint {:?}",
        context.actual_endpoint
    );
    Ok(ConnectionSummaryObservation {
        is_error,
        connection_id: LOGICAL_CONNECTION.to_owned(),
        name: optional_string(object, "name", "connection summary")?,
        port: LOGICAL_ENDPOINT.to_owned(),
        baud_rate: required_u32(object, "baud_rate", "connection summary")?,
        flow_control: required_string(object, "flow_control", "connection summary")?,
    })
}

/// Validate and normalize a public `set_flow_control` result.
pub fn normalize_set_flow_control(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<SetFlowControlObservation> {
    let object = required_object(value, "set_flow_control")?;
    validate_connection_reference(object, "set_flow_control", context)?;
    Ok(SetFlowControlObservation {
        is_error,
        connection_id: LOGICAL_CONNECTION.to_owned(),
        name: optional_string(object, "name", "set_flow_control")?,
        flow_control: required_string(object, "flow_control", "set_flow_control")?,
    })
}

/// Validate and normalize a public output flush result.
pub fn normalize_flush(
    value: &Value,
    context: &NormalizationContext,
    is_error: Option<bool>,
) -> Result<FlushObservation> {
    let object = required_object(value, "flush")?;
    validate_connection_reference(object, "flush", context)?;
    Ok(FlushObservation {
        is_error,
        connection_id: LOGICAL_CONNECTION.to_owned(),
        name: optional_string(object, "name", "flush")?,
        target: required_string(object, "target", "flush")?,
    })
}

fn decode_payload(object: &Map<String, Value>, operation: &str) -> Result<(String, Vec<u8>)> {
    let encoding = required_string(object, "encoding", operation)?;
    let data = required_string(object, "data", operation)?;
    let payload = match encoding.as_str() {
        "utf8" => data.into_bytes(),
        "hex" => {
            let compact = data.trim().replace(' ', "");
            hex::decode(compact)
                .with_context(|| format!("{operation} result data was not valid hex"))?
        }
        "base64" => base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(data.trim()))
            .with_context(|| format!("{operation} result data was not valid base64"))?,
        _ => bail!(
            "{operation} result had unsupported effective encoding {encoding:?}; expected utf8, hex, or base64"
        ),
    };
    Ok((encoding, payload))
}

fn optional_frames(
    object: &Map<String, Value>,
    operation: &str,
) -> Result<Option<Vec<FrameObservation>>> {
    let Some(value) = object.get("frames") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let frames = value
        .as_array()
        .with_context(|| format!("{operation} result field frames must be an array or null"))?;
    frames
        .iter()
        .enumerate()
        .map(|(position, frame)| normalize_frame(frame, operation, position))
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

fn normalize_frame(value: &Value, operation: &str, position: usize) -> Result<FrameObservation> {
    let frame_operation = format!("{operation} frame {position}");
    let object = required_object(value, &frame_operation)?;
    let (encoding, payload) = decode_payload(object, &frame_operation)?;
    Ok(FrameObservation {
        frame_index: required_usize(object, "frame_index", &frame_operation)?,
        frame_type: required_string(object, "frame_type", &frame_operation)?,
        encoding,
        payload,
        parsed: optional_parsed_frame(object, &frame_operation)?,
    })
}

fn optional_parsed_frame(
    object: &Map<String, Value>,
    operation: &str,
) -> Result<Option<ParsedFrameObservation>> {
    let Some(value) = object.get("parsed") else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    let parsed = required_object(value, &format!("{operation} parsed"))?;
    match required_string(parsed, "parser", &format!("{operation} parsed"))?.as_str() {
        "raw" => Ok(Some(ParsedFrameObservation::Raw)),
        "at_command" => Ok(Some(ParsedFrameObservation::AtCommand {
            response_type: required_string(parsed, "response_type", &format!("{operation} parsed"))?,
            command: optional_string_or_missing(parsed, "command", &format!("{operation} parsed"))?,
            status: optional_string_or_missing(parsed, "status", &format!("{operation} parsed"))?,
            fields: required_string_vec(parsed, "fields", &format!("{operation} parsed"))?,
        })),
        "json" => Ok(Some(ParsedFrameObservation::Json {
            fields: parsed
                .iter()
                .filter(|(key, _)| key.as_str() != "parser")
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        })),
        parser => bail!(
            "{operation} parsed frame used unsupported parser {parser:?}; typed differential model supports only `raw`, `at_command`, and `json` parsers"
        ),
    }
}

fn required_object<'a>(value: &'a Value, operation: &str) -> Result<&'a Map<String, Value>> {
    value
        .as_object()
        .with_context(|| format!("{operation} result must be a structured object"))
}

fn required_value<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<&'a Value> {
    object
        .get(field)
        .with_context(|| format!("{operation} result omitted {field}"))
}

fn required_string(object: &Map<String, Value>, field: &str, operation: &str) -> Result<String> {
    required_value(object, field, operation)?
        .as_str()
        .map(str::to_owned)
        .with_context(|| format!("{operation} result field {field} must be a string"))
}

fn optional_string(
    object: &Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<Option<String>> {
    match required_value(object, field, operation)? {
        Value::Null => Ok(None),
        Value::String(value) => Ok(Some(value.clone())),
        _ => bail!("{operation} result field {field} must be a string or null"),
    }
}

fn optional_string_or_missing(
    object: &Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<Option<String>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => bail!("{operation} result field {field} must be a string, null, or omitted"),
    }
}

fn required_bool(object: &Map<String, Value>, field: &str, operation: &str) -> Result<bool> {
    required_value(object, field, operation)?
        .as_bool()
        .with_context(|| format!("{operation} result field {field} must be a boolean"))
}

fn required_u64(object: &Map<String, Value>, field: &str, operation: &str) -> Result<u64> {
    required_value(object, field, operation)?
        .as_u64()
        .with_context(|| format!("{operation} result field {field} must be an unsigned integer"))
}

fn required_u32(object: &Map<String, Value>, field: &str, operation: &str) -> Result<u32> {
    let value = required_u64(object, field, operation)?;
    u32::try_from(value)
        .with_context(|| format!("{operation} result field {field} value {value} exceeds u32"))
}

fn required_usize(object: &Map<String, Value>, field: &str, operation: &str) -> Result<usize> {
    let value = required_u64(object, field, operation)?;
    usize::try_from(value)
        .with_context(|| format!("{operation} result field {field} value {value} exceeds usize"))
}

fn optional_u64(object: &Map<String, Value>, field: &str, operation: &str) -> Result<Option<u64>> {
    match required_value(object, field, operation)? {
        Value::Null => Ok(None),
        value => value.as_u64().map(Some).with_context(|| {
            format!("{operation} result field {field} must be an unsigned integer or null")
        }),
    }
}

fn optional_usize(
    object: &Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<Option<usize>> {
    match optional_u64(object, field, operation)? {
        Some(value) => usize::try_from(value).map(Some).with_context(|| {
            format!("{operation} result field {field} value {value} exceeds usize")
        }),
        None => Ok(None),
    }
}

fn optional_usize_or_missing(
    object: &Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<Option<usize>> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let value = value.as_u64().with_context(|| {
                format!("{operation} result field {field} must be an unsigned integer, null, or omitted")
            })?;
            usize::try_from(value).map(Some).with_context(|| {
                format!("{operation} result field {field} value {value} exceeds usize")
            })
        }
    }
}

fn required_string_vec(
    object: &Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<Vec<String>> {
    required_value(object, field, operation)?
        .as_array()
        .with_context(|| format!("{operation} result field {field} must be an array"))?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value.as_str().map(str::to_owned).with_context(|| {
                format!("{operation} result field {field}[{index}] must be a string")
            })
        })
        .collect()
}

fn validate_connection_reference(
    object: &Map<String, Value>,
    operation: &str,
    context: &NormalizationContext,
) -> Result<()> {
    let connection_id = required_string(object, "connection_id", operation)?;
    ensure!(
        connection_id == context.actual_connection_id,
        "{operation} referenced connection_id {connection_id:?}, expected {:?}",
        context.actual_connection_id
    );
    Ok(())
}

fn serial_settings(
    object: &Map<String, Value>,
    operation: &str,
) -> Result<SerialSettingsObservation> {
    Ok(SerialSettingsObservation {
        baud_rate: required_u32(object, "baud_rate", operation)?,
        data_bits: required_string(object, "data_bits", operation)?,
        stop_bits: required_string(object, "stop_bits", operation)?,
        parity: required_string(object, "parity", operation)?,
        flow_control: required_string(object, "flow_control", operation)?,
    })
}

fn connection_state(
    object: &Map<String, Value>,
    field: &str,
    operation: &str,
) -> Result<NormalizedConnectionState> {
    match required_string(object, field, operation)?.as_str() {
        "open" => Ok(NormalizedConnectionState::Open),
        "disconnected" => Ok(NormalizedConnectionState::Disconnected),
        "reconnecting" => Ok(NormalizedConnectionState::Reconnecting),
        "closed" => Ok(NormalizedConnectionState::Closed),
        state => bail!("{operation} result field {field} has unknown connection state {state:?}"),
    }
}

fn checked_delta(after: u64, before: u64, field: &str) -> Result<u64> {
    after
        .checked_sub(before)
        .with_context(|| format!("status counter {field} decreased from {before} to {after}"))
}
