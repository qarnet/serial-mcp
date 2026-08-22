//! Independent native-row registry for executable differential batches.

use std::collections::BTreeSet;

use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};

pub use super::model::DifferentialBatch;
use super::model::DifferentialCase;

/// Exact source names from the two native oracle suites. This list remains
/// independent of native source/prose parsing so removed or renamed rows fail
/// validation rather than silently disappearing.
pub const NATIVE_CASES: [&str; 49] = [
    "native_ping_roundtrip",
    "native_pending_read_then_write_ping_roundtrip",
    "native_split_writes_preserve_command_order",
    "native_framing_reports_single_split_command",
    "native_trace_reports_exact_split_byte_sequence",
    "native_read_match_on_spam_complete",
    "native_read_buffer_budget_stops_under_flood",
    "native_bootloader_touch_exits_42",
    "native_list_ports_after_open",
    "native_list_ports_includes_identity_fields",
    "native_flush_after_write",
    "native_get_status_after_write_increments_tx_counter",
    "native_reconfigure_baud_rate_persists",
    "native_ack_command_provides_pre_execution_ack",
    "native_txbuf_status_reports_pending",
    "native_flush_input_clears_host_rx",
    "native_flush_during_arm_cmd_delay",
    "native_flush_output_after_full_delivery_is_safe",
    "native_partial_line_buffered_then_completed",
    "native_read_regex_matches_pong",
    "native_read_glob_matches_pong_line",
    "native_auto_reconnect_preserves_connection",
    "native_read_line_framing_splits_lines",
    "native_read_json_parser_decodes_jsonout",
    "native_read_at_parser_parses_pong",
    "native_read_framing_max_frames_stops",
    "native_read_framing_plus_match_combined",
    "native_open_protocol_default_drives_write_and_read",
    "native_explicit_rx_framing_beats_connection_default",
    "native_read_slip_decodes_frame",
    "native_read_slip_malformed_escape_returns_partial_result",
    "native_read_delimiter_framing_decodes",
    "native_read_length_prefixed_framing_decodes",
    "native_read_start_end_framing_decodes",
    "native_write_tx_framing_modes_observed_via_trace",
    "native_read_explicit_line_endings_split_correctly",
    "native_read_slip_recovers_after_error_on_next_call",
    "native_read_cobs_preset_decodes_frame",
    "native_read_ndjson_preset_decodes_json_frames",
    "native_read_ndjson_preset_skips_empty_lines",
    "native_read_nmea0183_preset_decodes_parsed_frame",
    "native_read_modbus_ascii_preset_decodes_parsed_frame",
    "native_capture_boot_arm_only_captures_post_arm_command_output",
    "native_named_connection_appears_in_list_connections",
    "native_set_flow_control_updates_summary_and_result",
    "native_close_while_read_active_returns_normal_result",
    "native_reopen_same_port_after_close_works",
    "native_reopen_then_match_finds_fresh_output",
    "native_open_with_flow_control_persists_in_summary",
];

pub const RETIRED_NATIVE_CASES: [&str; 3] = [
    "native_list_ports_after_open",
    "native_flush_after_write",
    "native_reopen_same_port_after_close_works",
];

const LIST_PORTS_RETIRED_PROOFS: &[&str] = &[
    "call_tool_list_ports_returns_structured_result",
    "ports_resource_includes_profile_match_map",
    "list_ports_preview_empty_store_reports_none_parallel_and_pure_ports",
];
const FLUSH_RETIRED_PROOFS: &[&str] = &["output_flush_after_full_delivery_preserves_later_traffic"];
const REOPEN_RETIRED_PROOFS: &[&str] =
    &["reopen_same_path_returns_distinct_id_and_only_fresh_generation"];
const PENDING_READ_BASELINE_PROOFS: &[&str] =
    &["pending_read_receives_later_output_after_readiness_proven_hold"];
const REGEX_GLOB_BASELINE_PROOFS: &[&str] = &["regex_and_glob_matchers_find_complete_peer_line"];
const MAX_FRAMES_BASELINE_PROOFS: &[&str] = &["max_frames_stops_after_exact_limit"];
const FRAMING_MATCH_BASELINE_PROOFS: &[&str] =
    &["framing_plus_match_returns_matching_frame_and_index"];
const OPEN_DEFAULT_BASELINE_PROOFS: &[&str] =
    &["call_time_line_framing_beats_connection_delimiter_default"];
const RAW_LENGTH_BASELINE_PROOFS: &[&str] =
    &["delimiter_length_prefixed_and_start_end_decode_exact_payloads"];
const RAW_TX_BASELINE_PROOFS: &[&str] =
    &["tx_framing_modes_produce_exact_independent_wire_vectors"];
const FLOOD_MATCHER_BASELINE_PROOFS: &[&str] =
    &["finite_flood_matcher_reaches_unique_completion_marker"];
const FLOOD_BUFFER_BASELINE_PROOFS: &[&str] =
    &["live_buffer_budget_caps_finite_flood_with_exact_stop_metadata"];
const SPLIT_WRITES_BASELINE_PROOFS: &[&str] =
    &["split_writes_preserve_one_command_and_exact_wire_order"];
const SLIP_MALFORMED_BASELINE_PROOFS: &[&str] =
    &["slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DifferentialStatus {
    Compared(DifferentialCase),
    BaselineAndStronger {
        case: DifferentialCase,
        required_proofs: &'static [&'static str],
    },
    Retired {
        required_proofs: &'static [&'static str],
        reason: &'static str,
    },
    Pending,
}

impl DifferentialStatus {
    fn differential_case(self) -> Option<DifferentialCase> {
        match self {
            Self::Compared(case) | Self::BaselineAndStronger { case, .. } => Some(case),
            Self::Retired { .. } | Self::Pending => None,
        }
    }

    fn required_proofs(self) -> Option<&'static [&'static str]> {
        match self {
            Self::BaselineAndStronger {
                required_proofs, ..
            }
            | Self::Retired {
                required_proofs, ..
            } => Some(required_proofs),
            Self::Compared(_) | Self::Pending => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DifferentialRow {
    pub native_case: &'static str,
    pub batch: Option<DifferentialBatch>,
    pub status: DifferentialStatus,
}

impl DifferentialRow {
    pub const fn compared(
        native_case: &'static str,
        batch: DifferentialBatch,
        case: DifferentialCase,
    ) -> Self {
        Self {
            native_case,
            batch: Some(batch),
            status: DifferentialStatus::Compared(case),
        }
    }

    pub const fn baseline_and_stronger(
        native_case: &'static str,
        batch: DifferentialBatch,
        case: DifferentialCase,
        required_proofs: &'static [&'static str],
    ) -> Self {
        Self {
            native_case,
            batch: Some(batch),
            status: DifferentialStatus::BaselineAndStronger {
                case,
                required_proofs,
            },
        }
    }

    pub const fn retired(
        native_case: &'static str,
        required_proofs: &'static [&'static str],
        reason: &'static str,
    ) -> Self {
        Self {
            native_case,
            batch: None,
            status: DifferentialStatus::Retired {
                required_proofs,
                reason,
            },
        }
    }

    pub const fn pending(native_case: &'static str) -> Self {
        Self {
            native_case,
            batch: None,
            status: DifferentialStatus::Pending,
        }
    }
}

/// Exact current status for every native oracle row.
const REGISTRY: &[DifferentialRow] = &[
    DifferentialRow::compared(
        "native_ping_roundtrip",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::PingRoundtrip,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_pending_read_then_write_ping_roundtrip",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::PendingReadThenWritePingRoundtrip,
        PENDING_READ_BASELINE_PROOFS,
    ),
    DifferentialRow::compared(
        "native_split_writes_preserve_command_order",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::SplitWritesPreserveCommandOrder,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_framing_reports_single_split_command",
        DifferentialBatch::CommandDiagnostics,
        DifferentialCase::FramingDiagnosticSplitPing,
        SPLIT_WRITES_BASELINE_PROOFS,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_trace_reports_exact_split_byte_sequence",
        DifferentialBatch::CommandDiagnostics,
        DifferentialCase::TraceDiagnosticSplitPing,
        SPLIT_WRITES_BASELINE_PROOFS,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_match_on_spam_complete",
        DifferentialBatch::FloodBuffer,
        DifferentialCase::FloodMatcherSpamComplete,
        FLOOD_MATCHER_BASELINE_PROOFS,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_buffer_budget_stops_under_flood",
        DifferentialBatch::FloodBuffer,
        DifferentialCase::FloodBufferBudget,
        FLOOD_BUFFER_BASELINE_PROOFS,
    ),
    DifferentialRow::pending("native_bootloader_touch_exits_42"),
    DifferentialRow::retired(
        "native_list_ports_after_open",
        LIST_PORTS_RETIRED_PROOFS,
        "deterministic public list/resource behavior is stronger than ambient enumeration",
    ),
    DifferentialRow::pending("native_list_ports_includes_identity_fields"),
    DifferentialRow::retired(
        "native_flush_after_write",
        FLUSH_RETIRED_PROOFS,
        "queued output may be discarded; fully delivered output retains valid behavior",
    ),
    DifferentialRow::compared(
        "native_get_status_after_write_increments_tx_counter",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::GetStatusAfterWriteIncrementsTxCounter,
    ),
    DifferentialRow::compared(
        "native_reconfigure_baud_rate_persists",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::ReconfigureBaudRatePersists,
    ),
    DifferentialRow::compared(
        "native_ack_command_provides_pre_execution_ack",
        DifferentialBatch::AckState,
        DifferentialCase::AckStateMachine,
    ),
    DifferentialRow::pending("native_txbuf_status_reports_pending"),
    DifferentialRow::pending("native_flush_input_clears_host_rx"),
    DifferentialRow::pending("native_flush_during_arm_cmd_delay"),
    DifferentialRow::compared(
        "native_flush_output_after_full_delivery_is_safe",
        DifferentialBatch::OutputFlush,
        DifferentialCase::OutputFlushAfterDelivery,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_partial_line_buffered_then_completed",
        DifferentialBatch::CommandDiagnostics,
        DifferentialCase::PartialLineThenCompletePing,
        SPLIT_WRITES_BASELINE_PROOFS,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_regex_matches_pong",
        DifferentialBatch::GenericMatchingFraming,
        DifferentialCase::RegexMatchesPong,
        REGEX_GLOB_BASELINE_PROOFS,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_glob_matches_pong_line",
        DifferentialBatch::GenericMatchingFraming,
        DifferentialCase::GlobMatchesPongLine,
        REGEX_GLOB_BASELINE_PROOFS,
    ),
    DifferentialRow::pending("native_auto_reconnect_preserves_connection"),
    DifferentialRow::compared(
        "native_read_line_framing_splits_lines",
        DifferentialBatch::GenericMatchingFraming,
        DifferentialCase::LineFramingSplitsLines,
    ),
    DifferentialRow::compared(
        "native_read_json_parser_decodes_jsonout",
        DifferentialBatch::JsonParser,
        DifferentialCase::JsonParserJsonout,
    ),
    DifferentialRow::compared(
        "native_read_at_parser_parses_pong",
        DifferentialBatch::AtParser,
        DifferentialCase::AtParserPong,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_framing_max_frames_stops",
        DifferentialBatch::GenericMatchingFraming,
        DifferentialCase::FramingMaxFramesStops,
        MAX_FRAMES_BASELINE_PROOFS,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_framing_plus_match_combined",
        DifferentialBatch::GenericMatchingFraming,
        DifferentialCase::FramingPlusMatchCombined,
        FRAMING_MATCH_BASELINE_PROOFS,
    ),
    DifferentialRow::compared(
        "native_open_protocol_default_drives_write_and_read",
        DifferentialBatch::AtProtocolDefault,
        DifferentialCase::AtProtocolDefaultPong,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_explicit_rx_framing_beats_connection_default",
        DifferentialBatch::GenericMatchingFraming,
        DifferentialCase::ExplicitRxFramingBeatsConnectionDefault,
        OPEN_DEFAULT_BASELINE_PROOFS,
    ),
    DifferentialRow::compared(
        "native_read_slip_decodes_frame",
        DifferentialBatch::SlipHappy,
        DifferentialCase::SlipHappyPath,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_slip_malformed_escape_returns_partial_result",
        DifferentialBatch::SlipMalformed,
        DifferentialCase::SlipMalformedEscape,
        SLIP_MALFORMED_BASELINE_PROOFS,
    ),
    DifferentialRow::compared(
        "native_read_delimiter_framing_decodes",
        DifferentialBatch::RawGenericFraming,
        DifferentialCase::DelimiterFramingDecodes,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_read_length_prefixed_framing_decodes",
        DifferentialBatch::RawGenericFraming,
        DifferentialCase::LengthPrefixedFramingDecodes,
        RAW_LENGTH_BASELINE_PROOFS,
    ),
    DifferentialRow::compared(
        "native_read_start_end_framing_decodes",
        DifferentialBatch::RawGenericFraming,
        DifferentialCase::StartEndFramingDecodes,
    ),
    DifferentialRow::baseline_and_stronger(
        "native_write_tx_framing_modes_observed_via_trace",
        DifferentialBatch::RawGenericFraming,
        DifferentialCase::TxFramingModesObservedViaTrace,
        RAW_TX_BASELINE_PROOFS,
    ),
    DifferentialRow::compared(
        "native_read_explicit_line_endings_split_correctly",
        DifferentialBatch::RawGenericFraming,
        DifferentialCase::ExplicitLineEndingsSplitCorrectly,
    ),
    DifferentialRow::compared(
        "native_read_slip_recovers_after_error_on_next_call",
        DifferentialBatch::SlipRecovery,
        DifferentialCase::SlipRecoveryAfterMalformed,
    ),
    DifferentialRow::compared(
        "native_read_cobs_preset_decodes_frame",
        DifferentialBatch::CobsPreset,
        DifferentialCase::CobsPresetDecode,
    ),
    DifferentialRow::compared(
        "native_read_ndjson_preset_decodes_json_frames",
        DifferentialBatch::NdjsonPreset,
        DifferentialCase::NdjsonPresetJsonFrames,
    ),
    DifferentialRow::compared(
        "native_read_ndjson_preset_skips_empty_lines",
        DifferentialBatch::NdjsonPreset,
        DifferentialCase::NdjsonPresetSkipsEmptyLines,
    ),
    DifferentialRow::pending("native_read_nmea0183_preset_decodes_parsed_frame"),
    DifferentialRow::pending("native_read_modbus_ascii_preset_decodes_parsed_frame"),
    DifferentialRow::pending("native_capture_boot_arm_only_captures_post_arm_command_output"),
    DifferentialRow::compared(
        "native_named_connection_appears_in_list_connections",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::NamedConnectionAppearsInListConnections,
    ),
    DifferentialRow::compared(
        "native_set_flow_control_updates_summary_and_result",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::SetFlowControlUpdatesSummaryAndResult,
    ),
    DifferentialRow::pending("native_close_while_read_active_returns_normal_result"),
    DifferentialRow::retired(
        "native_reopen_same_port_after_close_works",
        REOPEN_RETIRED_PROOFS,
        "fresh-generation reopen proof retains the weaker same-path behavior",
    ),
    DifferentialRow::pending("native_reopen_then_match_finds_fresh_output"),
    DifferentialRow::compared(
        "native_open_with_flow_control_persists_in_summary",
        DifferentialBatch::CommandLifecycle,
        DifferentialCase::OpenWithFlowControlPersistsInSummary,
    ),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryCounts {
    pub total_rows: usize,
    pub compared_rows: usize,
    pub baseline_and_stronger_rows: usize,
    pub retired_rows: usize,
    pub pending_rows: usize,
}

pub fn rows() -> &'static [DifferentialRow] {
    REGISTRY
}

/// Cases that execute in one explicitly selected batch. Baseline rows execute
/// alongside their cited stronger fixture proof; Batch 1 never silently gains
/// later rows.
pub fn executable_cases(batch: DifferentialBatch) -> Result<Vec<DifferentialCase>> {
    let _ = validate_current_registry()?;
    Ok(REGISTRY
        .iter()
        .filter_map(|row| match row.status {
            DifferentialStatus::Compared(case)
            | DifferentialStatus::BaselineAndStronger { case, .. }
                if row.batch == Some(batch) =>
            {
                Some(case)
            }
            DifferentialStatus::Retired { .. } | DifferentialStatus::Pending => None,
            DifferentialStatus::Compared(_) | DifferentialStatus::BaselineAndStronger { .. } => {
                None
            }
        })
        .collect())
}

/// Validate arbitrary registry input. Unit tests use synthetic rows to prove
/// duplicate, unknown, missing, retired-set, case-ID, and proof failures.
pub fn validate_registry(
    rows: &[DifferentialRow],
    expected_native_cases: &[&str],
    expected_retired_cases: &[&str],
    proof_exists: impl Fn(&str) -> bool,
) -> Result<RegistryCounts> {
    let expected: BTreeSet<_> = expected_native_cases.iter().copied().collect();
    ensure!(
        expected.len() == expected_native_cases.len(),
        "expected native case list contains duplicate names"
    );
    let expected_retired: BTreeSet<_> = expected_retired_cases.iter().copied().collect();
    ensure!(
        expected_retired.len() == expected_retired_cases.len(),
        "expected retired case list contains duplicate names"
    );

    let mut seen_native = BTreeSet::new();
    let mut seen_case_ids = BTreeSet::new();
    let mut retired = BTreeSet::new();
    let mut counts = RegistryCounts {
        total_rows: rows.len(),
        compared_rows: 0,
        baseline_and_stronger_rows: 0,
        retired_rows: 0,
        pending_rows: 0,
    };

    for row in rows {
        ensure!(
            expected.contains(row.native_case),
            "differential registry contains unknown native case {:?}",
            row.native_case
        );
        ensure!(
            seen_native.insert(row.native_case),
            "differential registry contains duplicate native case {:?}",
            row.native_case
        );
        if let Some(case) = row.status.differential_case() {
            let batch = row.batch.with_context(|| {
                format!(
                    "differential registry executable row {:?} omitted explicit batch membership",
                    row.native_case
                )
            })?;
            ensure!(
                batch == case.batch(),
                "differential registry row {:?} assigned {:?}, but case {:?} belongs to {:?}",
                row.native_case,
                batch,
                case,
                case.batch()
            );
            ensure!(
                seen_case_ids.insert(case.id()),
                "differential registry contains duplicate case ID {:?}",
                case.id()
            );
        } else {
            ensure!(
                row.batch.is_none(),
                "differential registry non-executable row {:?} unexpectedly has batch membership {:?}",
                row.native_case,
                row.batch
            );
        }
        if let Some(required_proofs) = row.status.required_proofs() {
            ensure!(
                !required_proofs.is_empty(),
                "differential registry row {:?} requires at least one source proof",
                row.native_case
            );
            for proof in required_proofs {
                ensure!(
                    proof_exists(proof),
                    "differential registry row {:?} cites missing source proof {:?}",
                    row.native_case,
                    proof
                );
            }
        }
        match row.status {
            DifferentialStatus::Compared(_) => counts.compared_rows += 1,
            DifferentialStatus::BaselineAndStronger { .. } => {
                counts.baseline_and_stronger_rows += 1;
            }
            DifferentialStatus::Retired { .. } => {
                counts.retired_rows += 1;
                retired.insert(row.native_case);
            }
            DifferentialStatus::Pending => counts.pending_rows += 1,
        }
    }

    if seen_native != expected {
        let missing: Vec<_> = expected.difference(&seen_native).copied().collect();
        let unexpected: Vec<_> = seen_native.difference(&expected).copied().collect();
        bail!(
            "differential registry native case set mismatch; missing={missing:?}, unexpected={unexpected:?}"
        );
    }
    ensure!(
        retired == expected_retired,
        "differential registry retired set mismatch; actual={retired:?}, expected={expected_retired:?}"
    );
    Ok(counts)
}

pub fn validate_current_registry() -> Result<RegistryCounts> {
    let counts = validate_registry(
        REGISTRY,
        &NATIVE_CASES,
        &RETIRED_NATIVE_CASES,
        source_proof_exists,
    )?;
    ensure!(
        counts.total_rows == 49
            && counts.compared_rows == 21
            && counts.baseline_and_stronger_rows == 14
            && counts.retired_rows == 3
            && counts.pending_rows == 11,
        "differential registry counts drifted: {counts:?}"
    );
    Ok(counts)
}

fn source_proof_exists(identifier: &str) -> bool {
    [
        include_str!("../../device_command_parity.rs"),
        include_str!("../../device_framing_parity.rs"),
        include_str!("../../device_protocol_parity.rs"),
        include_str!("../../http_integration.rs"),
        include_str!("../../serial_pty.rs"),
    ]
    .into_iter()
    .any(|source| source.contains(&format!("fn {identifier}")))
}
