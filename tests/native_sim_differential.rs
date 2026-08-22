//! Linux-only executable native_sim versus Rust PTY differential gate.

#![cfg(target_os = "linux")]

mod common;

use std::collections::BTreeMap;

use anyhow::Result;
use common::native_sim_differential::model::{
    ConnectionSummaryObservation, DifferentialCase, FlushObservation, FrameObservation,
    NormalizedConnectionState, Observation, OpenObservation, ParsedFrameObservation,
    PeerWireDirection, PeerWireMode, PeerWireObservation, ReadObservation, ReadPositionObservation,
    ReconfigureObservation, ScenarioOutcome, SerialSettingsObservation, SetFlowControlObservation,
    StatusDeltaObservation, WriteObservation, LOGICAL_CONNECTION, LOGICAL_ENDPOINT,
};
use common::native_sim_differential::registry::{
    self, DifferentialBatch, DifferentialRow, DifferentialStatus, RegistryCounts,
    RETIRED_NATIVE_CASES,
};
use common::native_sim_differential::scenarios::{
    decode_trace_bytes, run_ack_state_batch, run_at_parser_batch, run_at_protocol_default_batch,
    run_cobs_preset_batch, run_command_diagnostics_batch, run_command_lifecycle_batch,
    run_flood_buffer_batch, run_generic_matching_framing_batch, run_json_parser_batch,
    run_ndjson_preset_batch, run_output_flush_batch, run_raw_generic_framing_batch,
    run_slip_happy_batch, run_slip_malformed_batch, run_slip_recovery_batch, serialize_report,
    write_report, DifferentialReport, PairedScenarioOutcome, ACK_STATE_REPORT_FILENAME,
    ACK_STATE_REPORT_SCHEMA_ID, AT_PARSER_REPORT_FILENAME, AT_PARSER_REPORT_SCHEMA_ID,
    AT_PROTOCOL_DEFAULT_REPORT_FILENAME, AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID,
    COBS_PRESET_REPORT_FILENAME, COBS_PRESET_REPORT_SCHEMA_ID, COMMAND_DIAGNOSTICS_REPORT_FILENAME,
    COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID, FLOOD_BUFFER_REPORT_FILENAME,
    FLOOD_BUFFER_REPORT_SCHEMA_ID, GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
    JSON_PARSER_REPORT_FILENAME, JSON_PARSER_REPORT_SCHEMA_ID, NDJSON_PRESET_REPORT_FILENAME,
    NDJSON_PRESET_REPORT_SCHEMA_ID, OUTPUT_FLUSH_REPORT_FILENAME, OUTPUT_FLUSH_REPORT_SCHEMA_ID,
    RAW_GENERIC_FRAMING_REPORT_FILENAME, RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID, REPORT_SCHEMA_ID,
    SLIP_HAPPY_REPORT_FILENAME, SLIP_HAPPY_REPORT_SCHEMA_ID, SLIP_MALFORMED_REPORT_FILENAME,
    SLIP_MALFORMED_REPORT_SCHEMA_ID, SLIP_RECOVERY_REPORT_FILENAME, SLIP_RECOVERY_REPORT_SCHEMA_ID,
};

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn command_lifecycle_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_command_lifecycle_batch().await?;
    let path = write_report(&report)?;
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn generic_matching_framing_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_generic_matching_framing_batch().await?;
    let path = write_report(&report)?;
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn raw_generic_framing_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_raw_generic_framing_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(RAW_GENERIC_FRAMING_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn flood_buffer_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_flood_buffer_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(FLOOD_BUFFER_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn command_diagnostics_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_command_diagnostics_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(COMMAND_DIAGNOSTICS_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn ack_state_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_ack_state_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(ACK_STATE_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn output_flush_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_output_flush_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(OUTPUT_FLUSH_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn slip_happy_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_slip_happy_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(SLIP_HAPPY_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn slip_malformed_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_slip_malformed_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(SLIP_MALFORMED_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn slip_recovery_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_slip_recovery_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(SLIP_RECOVERY_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn cobs_preset_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_cobs_preset_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(COBS_PRESET_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn at_parser_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_at_parser_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(AT_PARSER_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn at_protocol_default_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_at_protocol_default_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(AT_PROTOCOL_DEFAULT_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn json_parser_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_json_parser_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(JSON_PARSER_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary; migration differential gate"]
async fn ndjson_preset_batch_matches_normalized_public_outcomes() -> Result<()> {
    let report = run_ndjson_preset_batch().await?;
    let path = write_report(&report)?;
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(NDJSON_PRESET_REPORT_FILENAME)
    );
    assert!(
        path.is_file(),
        "differential report was not written: {}",
        path.display()
    );
    Ok(())
}

#[test]
fn registry_has_exact_global_counts_and_isolated_batch_cases() -> Result<()> {
    let counts = registry::validate_current_registry()?;
    assert_eq!(
        counts,
        RegistryCounts {
            total_rows: 49,
            compared_rows: 21,
            baseline_and_stronger_rows: 14,
            retired_rows: 3,
            pending_rows: 11,
        }
    );
    assert_eq!(registry::rows().len(), 49);
    assert_eq!(
        registry::executable_cases(DifferentialBatch::CommandLifecycle)?,
        DifferentialCase::BATCH_ONE.to_vec(),
        "Batch 1 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::GenericMatchingFraming)?,
        DifferentialCase::BATCH_TWO.to_vec(),
        "Batch 2 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::RawGenericFraming)?,
        DifferentialCase::BATCH_THREE.to_vec(),
        "Batch 3 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::FloodBuffer)?,
        DifferentialCase::BATCH_FOUR.to_vec(),
        "Batch 4 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::CommandDiagnostics)?,
        DifferentialCase::BATCH_FIVE.to_vec(),
        "Batch 5 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::AckState)?,
        DifferentialCase::BATCH_SIX.to_vec(),
        "Batch 6 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::OutputFlush)?,
        DifferentialCase::BATCH_SEVEN.to_vec(),
        "Batch 7 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::SlipHappy)?,
        DifferentialCase::BATCH_EIGHT.to_vec(),
        "Batch 8 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::SlipMalformed)?,
        DifferentialCase::BATCH_NINE.to_vec(),
        "Batch 9 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::SlipRecovery)?,
        DifferentialCase::BATCH_TEN.to_vec(),
        "Batch 10 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::CobsPreset)?,
        DifferentialCase::BATCH_ELEVEN.to_vec(),
        "Batch 11 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::AtParser)?,
        DifferentialCase::BATCH_TWELVE.to_vec(),
        "Batch 12 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::AtProtocolDefault)?,
        DifferentialCase::BATCH_THIRTEEN.to_vec(),
        "Batch 13 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::JsonParser)?,
        DifferentialCase::BATCH_FOURTEEN.to_vec(),
        "Batch 14 executable cases drifted"
    );
    assert_eq!(
        registry::executable_cases(DifferentialBatch::NdjsonPreset)?,
        DifferentialCase::BATCH_FIFTEEN.to_vec(),
        "Batch 15 executable cases drifted"
    );
    let expected_all: Vec<_> = DifferentialCase::BATCH_ONE
        .into_iter()
        .chain(DifferentialCase::BATCH_TWO)
        .chain(DifferentialCase::BATCH_THREE)
        .chain(DifferentialCase::BATCH_FOUR)
        .chain(DifferentialCase::BATCH_FIVE)
        .chain(DifferentialCase::BATCH_SIX)
        .chain(DifferentialCase::BATCH_SEVEN)
        .chain(DifferentialCase::BATCH_EIGHT)
        .chain(DifferentialCase::BATCH_NINE)
        .chain(DifferentialCase::BATCH_TEN)
        .chain(DifferentialCase::BATCH_ELEVEN)
        .chain(DifferentialCase::BATCH_TWELVE)
        .chain(DifferentialCase::BATCH_THIRTEEN)
        .chain(DifferentialCase::BATCH_FOURTEEN)
        .chain(DifferentialCase::BATCH_FIFTEEN)
        .collect();
    assert_eq!(DifferentialCase::ALL.len(), 35);
    assert_eq!(DifferentialCase::ALL.to_vec(), expected_all);
    let batch_one_baseline_rows: Vec<_> = registry::rows()
        .iter()
        .filter_map(|row| match row.status {
            DifferentialStatus::BaselineAndStronger {
                case,
                required_proofs,
            } if row.batch == Some(DifferentialBatch::CommandLifecycle) => {
                Some((row.native_case, case, required_proofs))
            }
            DifferentialStatus::Compared(_)
            | DifferentialStatus::BaselineAndStronger { .. }
            | DifferentialStatus::Retired { .. }
            | DifferentialStatus::Pending => None,
        })
        .collect();
    assert_eq!(
        batch_one_baseline_rows.len(),
        1,
        "expected one Batch 1 baseline-and-stronger row"
    );
    let (native_case, case, proofs) = batch_one_baseline_rows[0];
    assert_eq!(native_case, "native_pending_read_then_write_ping_roundtrip");
    assert_eq!(case, DifferentialCase::PendingReadThenWritePingRoundtrip);
    assert_eq!(
        proofs,
        &["pending_read_receives_later_output_after_readiness_proven_hold"]
    );
    let batch_two_baseline_rows: Vec<_> = registry::rows()
        .iter()
        .filter_map(|row| match row.status {
            DifferentialStatus::BaselineAndStronger {
                case,
                required_proofs,
            } if row.batch == Some(DifferentialBatch::GenericMatchingFraming) => {
                Some((row.native_case, case, required_proofs))
            }
            DifferentialStatus::Compared(_)
            | DifferentialStatus::BaselineAndStronger { .. }
            | DifferentialStatus::Retired { .. }
            | DifferentialStatus::Pending => None,
        })
        .collect();
    let regex_glob_proof: &[&str] = &["regex_and_glob_matchers_find_complete_peer_line"];
    let max_frames_proof: &[&str] = &["max_frames_stops_after_exact_limit"];
    let framing_match_proof: &[&str] = &["framing_plus_match_returns_matching_frame_and_index"];
    let open_default_proof: &[&str] =
        &["call_time_line_framing_beats_connection_delimiter_default"];
    assert_eq!(
        batch_two_baseline_rows,
        vec![
            (
                "native_read_regex_matches_pong",
                DifferentialCase::RegexMatchesPong,
                regex_glob_proof,
            ),
            (
                "native_read_glob_matches_pong_line",
                DifferentialCase::GlobMatchesPongLine,
                regex_glob_proof,
            ),
            (
                "native_read_framing_max_frames_stops",
                DifferentialCase::FramingMaxFramesStops,
                max_frames_proof,
            ),
            (
                "native_read_framing_plus_match_combined",
                DifferentialCase::FramingPlusMatchCombined,
                framing_match_proof,
            ),
            (
                "native_explicit_rx_framing_beats_connection_default",
                DifferentialCase::ExplicitRxFramingBeatsConnectionDefault,
                open_default_proof,
            ),
        ],
        "Batch 2 baseline-and-stronger rows/proofs drifted"
    );
    let batch_three_baseline_rows: Vec<_> = registry::rows()
        .iter()
        .filter_map(|row| match row.status {
            DifferentialStatus::BaselineAndStronger {
                case,
                required_proofs,
            } if row.batch == Some(DifferentialBatch::RawGenericFraming) => {
                Some((row.native_case, case, required_proofs))
            }
            DifferentialStatus::Compared(_)
            | DifferentialStatus::BaselineAndStronger { .. }
            | DifferentialStatus::Retired { .. }
            | DifferentialStatus::Pending => None,
        })
        .collect();
    assert_eq!(
        batch_three_baseline_rows,
        vec![
            (
                "native_read_length_prefixed_framing_decodes",
                DifferentialCase::LengthPrefixedFramingDecodes,
                &["delimiter_length_prefixed_and_start_end_decode_exact_payloads"][..],
            ),
            (
                "native_write_tx_framing_modes_observed_via_trace",
                DifferentialCase::TxFramingModesObservedViaTrace,
                &["tx_framing_modes_produce_exact_independent_wire_vectors"][..],
            ),
        ],
        "Batch 3 baseline-and-stronger rows/proofs drifted"
    );
    let batch_four_baseline_rows: Vec<_> = registry::rows()
        .iter()
        .filter_map(|row| match row.status {
            DifferentialStatus::BaselineAndStronger {
                case,
                required_proofs,
            } if row.batch == Some(DifferentialBatch::FloodBuffer) => {
                Some((row.native_case, case, required_proofs))
            }
            DifferentialStatus::Compared(_)
            | DifferentialStatus::BaselineAndStronger { .. }
            | DifferentialStatus::Retired { .. }
            | DifferentialStatus::Pending => None,
        })
        .collect();
    assert_eq!(
        batch_four_baseline_rows,
        vec![
            (
                "native_read_match_on_spam_complete",
                DifferentialCase::FloodMatcherSpamComplete,
                &["finite_flood_matcher_reaches_unique_completion_marker"][..],
            ),
            (
                "native_read_buffer_budget_stops_under_flood",
                DifferentialCase::FloodBufferBudget,
                &["live_buffer_budget_caps_finite_flood_with_exact_stop_metadata"][..],
            ),
        ],
        "Batch 4 baseline-and-stronger rows/proofs drifted"
    );
    let batch_five_baseline_rows: Vec<_> = registry::rows()
        .iter()
        .filter_map(|row| match row.status {
            DifferentialStatus::BaselineAndStronger {
                case,
                required_proofs,
            } if row.batch == Some(DifferentialBatch::CommandDiagnostics) => {
                Some((row.native_case, case, required_proofs))
            }
            DifferentialStatus::Compared(_)
            | DifferentialStatus::BaselineAndStronger { .. }
            | DifferentialStatus::Retired { .. }
            | DifferentialStatus::Pending => None,
        })
        .collect();
    assert_eq!(
        batch_five_baseline_rows,
        vec![
            (
                "native_framing_reports_single_split_command",
                DifferentialCase::FramingDiagnosticSplitPing,
                &["split_writes_preserve_one_command_and_exact_wire_order"][..],
            ),
            (
                "native_trace_reports_exact_split_byte_sequence",
                DifferentialCase::TraceDiagnosticSplitPing,
                &["split_writes_preserve_one_command_and_exact_wire_order"][..],
            ),
            (
                "native_partial_line_buffered_then_completed",
                DifferentialCase::PartialLineThenCompletePing,
                &["split_writes_preserve_one_command_and_exact_wire_order"][..],
            ),
        ],
        "Batch 5 baseline-and-stronger rows/proofs drifted"
    );
    assert_eq!(
        RETIRED_NATIVE_CASES,
        [
            "native_list_ports_after_open",
            "native_flush_after_write",
            "native_reopen_same_port_after_close_works",
        ]
    );
    let batch_seven_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_flush_output_after_full_delivery_is_safe")
        .expect("Batch 7 row must exist");
    assert_eq!(batch_seven_row.batch, Some(DifferentialBatch::OutputFlush));
    assert!(
        matches!(
            batch_seven_row.status,
            DifferentialStatus::Compared(DifferentialCase::OutputFlushAfterDelivery)
        ),
        "Batch 7 row must be direct Compared status without baseline proof: {batch_seven_row:?}"
    );
    let batch_eight_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_read_slip_decodes_frame")
        .expect("Batch 8 row must exist");
    assert_eq!(batch_eight_row.batch, Some(DifferentialBatch::SlipHappy));
    assert!(
        matches!(
            batch_eight_row.status,
            DifferentialStatus::Compared(DifferentialCase::SlipHappyPath)
        ),
        "Batch 8 row must be direct Compared status without baseline proof: {batch_eight_row:?}"
    );
    let batch_nine_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_read_slip_malformed_escape_returns_partial_result")
        .expect("Batch 9 row must exist");
    assert_eq!(batch_nine_row.batch, Some(DifferentialBatch::SlipMalformed));
    assert!(
        matches!(
            batch_nine_row.status,
            DifferentialStatus::BaselineAndStronger {
                case: DifferentialCase::SlipMalformedEscape,
                required_proofs,
            } if required_proofs
                == ["slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery"]
        ),
        "Batch 9 row must bind exact stronger SLIP proof: {batch_nine_row:?}"
    );
    let batch_ten_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_read_slip_recovers_after_error_on_next_call")
        .expect("Batch 10 row must exist");
    assert_eq!(batch_ten_row.batch, Some(DifferentialBatch::SlipRecovery));
    assert!(
        matches!(
            batch_ten_row.status,
            DifferentialStatus::Compared(DifferentialCase::SlipRecoveryAfterMalformed)
        ),
        "Batch 10 row must be direct Compared status without baseline proof: {batch_ten_row:?}"
    );
    let batch_eleven_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_read_cobs_preset_decodes_frame")
        .expect("Batch 11 row must exist");
    assert_eq!(batch_eleven_row.batch, Some(DifferentialBatch::CobsPreset));
    assert!(
        matches!(
            batch_eleven_row.status,
            DifferentialStatus::Compared(DifferentialCase::CobsPresetDecode)
        ),
        "Batch 11 row must be direct Compared status without baseline proof: {batch_eleven_row:?}"
    );
    let batch_twelve_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_read_at_parser_parses_pong")
        .expect("Batch 12 row must exist");
    assert_eq!(batch_twelve_row.batch, Some(DifferentialBatch::AtParser));
    assert!(
        matches!(
            batch_twelve_row.status,
            DifferentialStatus::Compared(DifferentialCase::AtParserPong)
        ),
        "Batch 12 row must be a direct AtParser comparison without baseline proof: {batch_twelve_row:?}"
    );
    let batch_thirteen_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_open_protocol_default_drives_write_and_read")
        .expect("Batch 13 row must exist");
    assert_eq!(
        batch_thirteen_row.batch,
        Some(DifferentialBatch::AtProtocolDefault)
    );
    assert!(
        matches!(
            batch_thirteen_row.status,
            DifferentialStatus::Compared(DifferentialCase::AtProtocolDefaultPong)
        ),
        "Batch 13 row must be a direct AtProtocolDefault comparison without baseline proof: {batch_thirteen_row:?}"
    );
    let batch_fourteen_row = registry::rows()
        .iter()
        .find(|row| row.native_case == "native_read_json_parser_decodes_jsonout")
        .expect("Batch 14 row must exist");
    assert_eq!(
        batch_fourteen_row.batch,
        Some(DifferentialBatch::JsonParser)
    );
    assert!(
        matches!(
            batch_fourteen_row.status,
            DifferentialStatus::Compared(DifferentialCase::JsonParserJsonout)
        ),
        "Batch 14 row must be a direct JsonParser comparison without baseline proof: {batch_fourteen_row:?}"
    );
    let batch_fifteen_rows: Vec<_> = registry::rows()
        .iter()
        .filter_map(|row| match row.status {
            DifferentialStatus::Compared(case)
                if row.batch == Some(DifferentialBatch::NdjsonPreset) =>
            {
                Some((row.native_case, case))
            }
            DifferentialStatus::Compared(_)
            | DifferentialStatus::BaselineAndStronger { .. }
            | DifferentialStatus::Retired { .. }
            | DifferentialStatus::Pending => None,
        })
        .collect();
    assert_eq!(
        batch_fifteen_rows,
        vec![
            (
                "native_read_ndjson_preset_decodes_json_frames",
                DifferentialCase::NdjsonPresetJsonFrames,
            ),
            (
                "native_read_ndjson_preset_skips_empty_lines",
                DifferentialCase::NdjsonPresetSkipsEmptyLines,
            ),
        ],
        "Batch 15 rows/cases drifted"
    );
    Ok(())
}

#[test]
fn registry_rejects_synthetic_duplicate_unknown_missing_case_and_proof_errors() {
    let expected = ["native_one", "native_two"];
    let duplicate = [
        DifferentialRow::pending("native_one"),
        DifferentialRow::pending("native_one"),
    ];
    assert!(registry::validate_registry(&duplicate, &expected, &[], |_| true).is_err());

    let unknown = [
        DifferentialRow::pending("native_one"),
        DifferentialRow::pending("native_unknown"),
    ];
    assert!(registry::validate_registry(&unknown, &expected, &[], |_| true).is_err());

    let missing = [DifferentialRow::pending("native_one")];
    assert!(registry::validate_registry(&missing, &expected, &[], |_| true).is_err());

    let duplicate_case = [
        DifferentialRow::compared(
            "native_one",
            DifferentialBatch::CommandLifecycle,
            DifferentialCase::PingRoundtrip,
        ),
        DifferentialRow::compared(
            "native_two",
            DifferentialBatch::CommandLifecycle,
            DifferentialCase::PingRoundtrip,
        ),
    ];
    assert!(registry::validate_registry(&duplicate_case, &expected, &[], |_| true).is_err());

    let retired = [
        DifferentialRow::retired("native_one", &["required_proof"], "synthetic"),
        DifferentialRow::pending("native_two"),
    ];
    assert!(registry::validate_registry(&retired, &expected, &[], |_| true).is_err());
    assert!(registry::validate_registry(&retired, &expected, &["native_one"], |_| false).is_err());

    let baseline = [
        DifferentialRow::baseline_and_stronger(
            "native_one",
            DifferentialBatch::CommandLifecycle,
            DifferentialCase::PingRoundtrip,
            &["required_proof"],
        ),
        DifferentialRow::pending("native_two"),
    ];
    assert!(registry::validate_registry(&baseline, &expected, &[], |_| false).is_err());
}

#[test]
fn typed_model_keeps_payload_stop_match_write_counter_serial_and_error_mutations_unequal() {
    let expected = sample_outcome();

    let mut payload_changed = expected.clone();
    let Observation::Read(read) = &mut payload_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    read.payload[0] = b'P';
    assert_ne!(
        expected, payload_changed,
        "payload-byte mutation must remain visible"
    );

    let mut stop_changed = expected.clone();
    let Observation::Read(read) = &mut stop_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    read.stop_reason = "timeout".to_owned();
    assert_ne!(
        expected, stop_changed,
        "stop-reason mutation must remain visible"
    );

    let mut match_changed = expected.clone();
    let Observation::Read(read) = &mut match_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    read.matched = false;
    read.match_index = None;
    assert_ne!(
        expected, match_changed,
        "match mutation must remain visible"
    );

    let mut match_frame_changed = expected.clone();
    let Observation::Read(read) = &mut match_frame_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    read.match_frame_index = None;
    assert_ne!(
        expected, match_frame_changed,
        "match-frame-index mutation must remain visible"
    );

    let mut frame_payload_changed = expected.clone();
    let Observation::Read(read) = &mut frame_payload_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    frames[0].payload[0] = b'P';
    assert_ne!(
        expected, frame_payload_changed,
        "frame-payload mutation must remain visible"
    );

    let mut frame_type_changed = expected.clone();
    let Observation::Read(read) = &mut frame_type_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    frames[0].frame_type = "delimiter".to_owned();
    assert_ne!(
        expected, frame_type_changed,
        "frame-type mutation must remain visible"
    );

    let mut frame_index_changed = expected.clone();
    let Observation::Read(read) = &mut frame_index_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    frames[0].frame_index = 1;
    assert_ne!(
        expected, frame_index_changed,
        "frame-index mutation must remain visible"
    );

    let mut parsed_fields_changed = expected.clone();
    let Observation::Read(read) = &mut parsed_fields_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    let Some(ParsedFrameObservation::AtCommand { fields, .. }) = &mut frames[0].parsed else {
        panic!("sample AT parsed frame missing");
    };
    fields[0] = "other".to_owned();
    assert_ne!(
        expected, parsed_fields_changed,
        "parsed AT fields mutation must remain visible"
    );

    let mut frames_dropped_changed = expected.clone();
    let Observation::Read(read) = &mut frames_dropped_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    read.frames_dropped = 1;
    assert_ne!(
        expected, frames_dropped_changed,
        "frames-dropped mutation must remain visible"
    );

    let mut framing_error_changed = expected.clone();
    let Observation::Read(read) = &mut framing_error_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    read.error = Some("malformed frame".to_owned());
    assert_ne!(
        expected, framing_error_changed,
        "framing-error mutation must remain visible"
    );

    let mut position_changed = expected.clone();
    let Observation::Read(read) = &mut position_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    let position = read
        .position
        .as_mut()
        .expect("sample read position missing");
    position.next_offset += 1;
    assert_ne!(
        expected, position_changed,
        "read-position mutation must remain visible"
    );

    let mut write_count_changed = expected.clone();
    let Observation::Write(write) = &mut write_count_changed.observations[1] else {
        panic!("sample write observation missing");
    };
    write.bytes_written += 1;
    assert_ne!(
        expected, write_count_changed,
        "write-byte-count mutation must remain visible"
    );

    let mut counter_changed = expected.clone();
    let Observation::StatusDelta(status) = &mut counter_changed.observations[3] else {
        panic!("sample status observation missing");
    };
    status.tx_bytes += 1;
    assert_ne!(
        expected, counter_changed,
        "counter-delta mutation must remain visible"
    );

    let mut baud_changed = expected.clone();
    let Observation::Reconfigure(reconfigure) = &mut baud_changed.observations[4] else {
        panic!("sample reconfigure observation missing");
    };
    reconfigure.serial.baud_rate = 9_600;
    assert_ne!(expected, baud_changed, "baud mutation must remain visible");

    let mut flow_changed = expected.clone();
    let Observation::SetFlowControl(flow) = &mut flow_changed.observations[6] else {
        panic!("sample flow observation missing");
    };
    flow.flow_control = "hardware".to_owned();
    assert_ne!(
        expected, flow_changed,
        "flow-control mutation must remain visible"
    );

    let mut name_changed = expected.clone();
    let Observation::ConnectionSummary(summary) = &mut name_changed.observations[5] else {
        panic!("sample summary observation missing");
    };
    summary.name = Some("other-name".to_owned());
    assert_ne!(expected, name_changed, "name mutation must remain visible");

    let mut state_changed = expected.clone();
    let Observation::StatusDelta(status) = &mut state_changed.observations[3] else {
        panic!("sample status observation missing");
    };
    status.state = NormalizedConnectionState::Disconnected;
    assert_ne!(
        expected, state_changed,
        "state mutation must remain visible"
    );

    let mut error_state_changed = expected.clone();
    let Observation::Open(open) = &mut error_state_changed.observations[0] else {
        panic!("sample open observation missing");
    };
    open.is_error = Some(false);
    assert_ne!(
        expected, error_state_changed,
        "None and Some(false) is_error states must remain distinct"
    );

    let mut peer_bytes_changed = expected.clone();
    let peer_wire = peer_bytes_changed
        .observations
        .iter_mut()
        .find_map(|observation| match observation {
            Observation::PeerWire(peer_wire) => Some(peer_wire),
            _ => None,
        })
        .expect("sample peer-wire observation missing");
    peer_wire.bytes[0] ^= 1;
    assert_ne!(
        expected, peer_bytes_changed,
        "peer-wire bytes mutation must remain visible"
    );

    let mut peer_mode_changed = expected.clone();
    let peer_wire = peer_mode_changed
        .observations
        .iter_mut()
        .find_map(|observation| match observation {
            Observation::PeerWire(peer_wire) => Some(peer_wire),
            _ => None,
        })
        .expect("sample peer-wire observation missing");
    peer_wire.mode = PeerWireMode::Slip;
    assert_ne!(
        expected, peer_mode_changed,
        "peer-wire mode mutation must remain visible"
    );

    let mut flush_target_changed = expected.clone();
    let flush = flush_target_changed
        .observations
        .iter_mut()
        .find_map(|observation| match observation {
            Observation::Flush(flush) => Some(flush),
            _ => None,
        })
        .expect("sample flush observation missing");
    flush.target = "input".to_owned();
    assert_ne!(
        expected, flush_target_changed,
        "flush-target mutation must remain visible"
    );

    let mut flush_connection_changed = expected.clone();
    let flush = flush_connection_changed
        .observations
        .iter_mut()
        .find_map(|observation| match observation {
            Observation::Flush(flush) => Some(flush),
            _ => None,
        })
        .expect("sample flush observation missing");
    flush.connection_id = "other-connection".to_owned();
    assert_ne!(
        expected, flush_connection_changed,
        "flush-connection mutation must remain visible"
    );

    let mut flush_name_changed = expected.clone();
    let flush = flush_name_changed
        .observations
        .iter_mut()
        .find_map(|observation| match observation {
            Observation::Flush(flush) => Some(flush),
            _ => None,
        })
        .expect("sample flush observation missing");
    flush.name = Some("named".to_owned());
    assert_ne!(
        expected, flush_name_changed,
        "flush-name mutation must remain visible"
    );

    let mut flush_error_changed = expected.clone();
    let flush = flush_error_changed
        .observations
        .iter_mut()
        .find_map(|observation| match observation {
            Observation::Flush(flush) => Some(flush),
            _ => None,
        })
        .expect("sample flush observation missing");
    flush.is_error = Some(false);
    assert_ne!(
        expected, flush_error_changed,
        "flush-error-state mutation must remain visible"
    );
}

#[test]
fn raw_parser_observation_mutations_remain_unequal() {
    let mut raw = sample_outcome();
    let Observation::Read(read) = &mut raw.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    frames[0].parsed = Some(ParsedFrameObservation::Raw);

    let mut no_parser = raw.clone();
    let Observation::Read(read) = &mut no_parser.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    frames[0].parsed = None;
    assert_ne!(
        raw, no_parser,
        "raw parser observation must differ from no parser"
    );

    let mut different_parser = raw.clone();
    let Observation::Read(read) = &mut different_parser.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    frames[0].parsed = Some(ParsedFrameObservation::AtCommand {
        response_type: "data".to_owned(),
        command: None,
        status: None,
        fields: vec!["pong".to_owned()],
    });
    assert_ne!(
        raw, different_parser,
        "raw parser observation must differ from another parser"
    );
}

#[test]
fn json_parser_observation_mutations_remain_unequal() {
    let mut expected = sample_outcome();
    let Observation::Read(read) = &mut expected.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    frames[0].parsed = Some(ParsedFrameObservation::Json {
        fields: BTreeMap::from([
            (
                "sensor".to_owned(),
                serde_json::Value::String("temp".to_owned()),
            ),
            ("value".to_owned(), serde_json::Value::from(25.5)),
        ]),
    });

    let mut key_changed = expected.clone();
    let Observation::Read(read) = &mut key_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    let Some(ParsedFrameObservation::Json { fields }) = &mut frames[0].parsed else {
        panic!("sample JSON parsed frame missing");
    };
    let sensor = fields
        .remove("sensor")
        .expect("sample sensor field missing");
    fields.insert("device".to_owned(), sensor);
    assert_ne!(
        expected, key_changed,
        "retained JSON key mutation must remain visible"
    );

    let mut value_changed = expected.clone();
    let Observation::Read(read) = &mut value_changed.observations[2] else {
        panic!("sample read observation missing");
    };
    let frames = read.frames.as_mut().expect("sample frames missing");
    let Some(ParsedFrameObservation::Json { fields }) = &mut frames[0].parsed else {
        panic!("sample JSON parsed frame missing");
    };
    fields.insert("value".to_owned(), serde_json::Value::from(26.5));
    assert_ne!(
        expected, value_changed,
        "retained JSON value mutation must remain visible"
    );
}

#[test]
fn report_serialization_is_deterministic() -> Result<()> {
    assert_ne!(
        REPORT_SCHEMA_ID, GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        "differential batches must retain separate report schema IDs"
    );
    assert_ne!(REPORT_SCHEMA_ID, RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID);
    assert_ne!(REPORT_SCHEMA_ID, FLOOD_BUFFER_REPORT_SCHEMA_ID);
    assert_ne!(REPORT_SCHEMA_ID, COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID
    );
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        FLOOD_BUFFER_REPORT_SCHEMA_ID
    );
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        FLOOD_BUFFER_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID
    );
    assert_ne!(
        FLOOD_BUFFER_REPORT_SCHEMA_ID,
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID
    );
    assert_ne!(REPORT_SCHEMA_ID, ACK_STATE_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        ACK_STATE_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        ACK_STATE_REPORT_SCHEMA_ID
    );
    assert_ne!(FLOOD_BUFFER_REPORT_SCHEMA_ID, ACK_STATE_REPORT_SCHEMA_ID);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        ACK_STATE_REPORT_SCHEMA_ID
    );
    assert_ne!(REPORT_SCHEMA_ID, OUTPUT_FLUSH_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        OUTPUT_FLUSH_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        OUTPUT_FLUSH_REPORT_SCHEMA_ID
    );
    assert_ne!(FLOOD_BUFFER_REPORT_SCHEMA_ID, OUTPUT_FLUSH_REPORT_SCHEMA_ID);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        OUTPUT_FLUSH_REPORT_SCHEMA_ID
    );
    assert_ne!(ACK_STATE_REPORT_SCHEMA_ID, OUTPUT_FLUSH_REPORT_SCHEMA_ID);
    assert_ne!(REPORT_SCHEMA_ID, SLIP_HAPPY_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        SLIP_HAPPY_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        SLIP_HAPPY_REPORT_SCHEMA_ID
    );
    assert_ne!(FLOOD_BUFFER_REPORT_SCHEMA_ID, SLIP_HAPPY_REPORT_SCHEMA_ID);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        SLIP_HAPPY_REPORT_SCHEMA_ID
    );
    assert_ne!(ACK_STATE_REPORT_SCHEMA_ID, SLIP_HAPPY_REPORT_SCHEMA_ID);
    assert_ne!(OUTPUT_FLUSH_REPORT_SCHEMA_ID, SLIP_HAPPY_REPORT_SCHEMA_ID);
    assert_ne!(REPORT_SCHEMA_ID, SLIP_MALFORMED_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        SLIP_MALFORMED_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        SLIP_MALFORMED_REPORT_SCHEMA_ID
    );
    assert_ne!(
        FLOOD_BUFFER_REPORT_SCHEMA_ID,
        SLIP_MALFORMED_REPORT_SCHEMA_ID
    );
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        SLIP_MALFORMED_REPORT_SCHEMA_ID
    );
    assert_ne!(ACK_STATE_REPORT_SCHEMA_ID, SLIP_MALFORMED_REPORT_SCHEMA_ID);
    assert_ne!(
        OUTPUT_FLUSH_REPORT_SCHEMA_ID,
        SLIP_MALFORMED_REPORT_SCHEMA_ID
    );
    assert_ne!(SLIP_HAPPY_REPORT_SCHEMA_ID, SLIP_MALFORMED_REPORT_SCHEMA_ID);
    assert_ne!(REPORT_SCHEMA_ID, SLIP_RECOVERY_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        SLIP_RECOVERY_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        SLIP_RECOVERY_REPORT_SCHEMA_ID
    );
    assert_ne!(
        FLOOD_BUFFER_REPORT_SCHEMA_ID,
        SLIP_RECOVERY_REPORT_SCHEMA_ID
    );
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        SLIP_RECOVERY_REPORT_SCHEMA_ID
    );
    assert_ne!(ACK_STATE_REPORT_SCHEMA_ID, SLIP_RECOVERY_REPORT_SCHEMA_ID);
    assert_ne!(
        OUTPUT_FLUSH_REPORT_SCHEMA_ID,
        SLIP_RECOVERY_REPORT_SCHEMA_ID
    );
    assert_ne!(SLIP_HAPPY_REPORT_SCHEMA_ID, SLIP_RECOVERY_REPORT_SCHEMA_ID);
    assert_ne!(
        SLIP_MALFORMED_REPORT_SCHEMA_ID,
        SLIP_RECOVERY_REPORT_SCHEMA_ID
    );
    assert_ne!(REPORT_SCHEMA_ID, COBS_PRESET_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        COBS_PRESET_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        COBS_PRESET_REPORT_SCHEMA_ID
    );
    assert_ne!(FLOOD_BUFFER_REPORT_SCHEMA_ID, COBS_PRESET_REPORT_SCHEMA_ID);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        COBS_PRESET_REPORT_SCHEMA_ID
    );
    assert_ne!(ACK_STATE_REPORT_SCHEMA_ID, COBS_PRESET_REPORT_SCHEMA_ID);
    assert_ne!(OUTPUT_FLUSH_REPORT_SCHEMA_ID, COBS_PRESET_REPORT_SCHEMA_ID);
    assert_ne!(SLIP_HAPPY_REPORT_SCHEMA_ID, COBS_PRESET_REPORT_SCHEMA_ID);
    assert_ne!(
        SLIP_MALFORMED_REPORT_SCHEMA_ID,
        COBS_PRESET_REPORT_SCHEMA_ID
    );
    assert_ne!(SLIP_RECOVERY_REPORT_SCHEMA_ID, COBS_PRESET_REPORT_SCHEMA_ID);
    assert_ne!(REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        AT_PARSER_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        AT_PARSER_REPORT_SCHEMA_ID
    );
    assert_ne!(FLOOD_BUFFER_REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        AT_PARSER_REPORT_SCHEMA_ID
    );
    assert_ne!(ACK_STATE_REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(OUTPUT_FLUSH_REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(SLIP_HAPPY_REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(SLIP_MALFORMED_REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(SLIP_RECOVERY_REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(COBS_PRESET_REPORT_SCHEMA_ID, AT_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(REPORT_SCHEMA_ID, AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        FLOOD_BUFFER_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        ACK_STATE_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        OUTPUT_FLUSH_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        SLIP_HAPPY_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        SLIP_MALFORMED_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        SLIP_RECOVERY_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        COBS_PRESET_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(
        AT_PARSER_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID
    );
    assert_ne!(REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        JSON_PARSER_REPORT_SCHEMA_ID
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        JSON_PARSER_REPORT_SCHEMA_ID
    );
    assert_ne!(FLOOD_BUFFER_REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        JSON_PARSER_REPORT_SCHEMA_ID
    );
    assert_ne!(ACK_STATE_REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(OUTPUT_FLUSH_REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(SLIP_HAPPY_REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(
        SLIP_MALFORMED_REPORT_SCHEMA_ID,
        JSON_PARSER_REPORT_SCHEMA_ID
    );
    assert_ne!(SLIP_RECOVERY_REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(COBS_PRESET_REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(AT_PARSER_REPORT_SCHEMA_ID, JSON_PARSER_REPORT_SCHEMA_ID);
    assert_ne!(
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID,
        JSON_PARSER_REPORT_SCHEMA_ID
    );
    for schema_id in [
        REPORT_SCHEMA_ID,
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        FLOOD_BUFFER_REPORT_SCHEMA_ID,
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        ACK_STATE_REPORT_SCHEMA_ID,
        OUTPUT_FLUSH_REPORT_SCHEMA_ID,
        SLIP_HAPPY_REPORT_SCHEMA_ID,
        SLIP_MALFORMED_REPORT_SCHEMA_ID,
        SLIP_RECOVERY_REPORT_SCHEMA_ID,
        COBS_PRESET_REPORT_SCHEMA_ID,
        AT_PARSER_REPORT_SCHEMA_ID,
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID,
        JSON_PARSER_REPORT_SCHEMA_ID,
    ] {
        assert_ne!(
            NDJSON_PRESET_REPORT_SCHEMA_ID, schema_id,
            "Batch 15 report schema must remain isolated"
        );
    }
    assert_ne!(
        "command-lifecycle-batch.json",
        RAW_GENERIC_FRAMING_REPORT_FILENAME
    );
    assert_ne!(
        "generic-matching-framing-batch.json",
        RAW_GENERIC_FRAMING_REPORT_FILENAME
    );
    assert_ne!("command-lifecycle-batch.json", FLOOD_BUFFER_REPORT_FILENAME);
    assert_ne!(
        "generic-matching-framing-batch.json",
        FLOOD_BUFFER_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        FLOOD_BUFFER_REPORT_FILENAME
    );
    assert_ne!(
        "command-lifecycle-batch.json",
        COMMAND_DIAGNOSTICS_REPORT_FILENAME
    );
    assert_ne!(
        "generic-matching-framing-batch.json",
        COMMAND_DIAGNOSTICS_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        COMMAND_DIAGNOSTICS_REPORT_FILENAME
    );
    assert_ne!(
        FLOOD_BUFFER_REPORT_FILENAME,
        COMMAND_DIAGNOSTICS_REPORT_FILENAME
    );
    assert_ne!("command-lifecycle-batch.json", ACK_STATE_REPORT_FILENAME);
    assert_ne!(
        "generic-matching-framing-batch.json",
        ACK_STATE_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        ACK_STATE_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, ACK_STATE_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        ACK_STATE_REPORT_FILENAME
    );
    assert_ne!("command-lifecycle-batch.json", OUTPUT_FLUSH_REPORT_FILENAME);
    assert_ne!(
        "generic-matching-framing-batch.json",
        OUTPUT_FLUSH_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        OUTPUT_FLUSH_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, OUTPUT_FLUSH_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        OUTPUT_FLUSH_REPORT_FILENAME
    );
    assert_ne!(ACK_STATE_REPORT_FILENAME, OUTPUT_FLUSH_REPORT_FILENAME);
    assert_ne!("command-lifecycle-batch.json", SLIP_HAPPY_REPORT_FILENAME);
    assert_ne!(
        "generic-matching-framing-batch.json",
        SLIP_HAPPY_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        SLIP_HAPPY_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, SLIP_HAPPY_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        SLIP_HAPPY_REPORT_FILENAME
    );
    assert_ne!(ACK_STATE_REPORT_FILENAME, SLIP_HAPPY_REPORT_FILENAME);
    assert_ne!(OUTPUT_FLUSH_REPORT_FILENAME, SLIP_HAPPY_REPORT_FILENAME);
    assert_ne!(
        "command-lifecycle-batch.json",
        SLIP_MALFORMED_REPORT_FILENAME
    );
    assert_ne!(
        "generic-matching-framing-batch.json",
        SLIP_MALFORMED_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        SLIP_MALFORMED_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, SLIP_MALFORMED_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        SLIP_MALFORMED_REPORT_FILENAME
    );
    assert_ne!(ACK_STATE_REPORT_FILENAME, SLIP_MALFORMED_REPORT_FILENAME);
    assert_ne!(OUTPUT_FLUSH_REPORT_FILENAME, SLIP_MALFORMED_REPORT_FILENAME);
    assert_ne!(SLIP_HAPPY_REPORT_FILENAME, SLIP_MALFORMED_REPORT_FILENAME);
    assert_ne!(
        "command-lifecycle-batch.json",
        SLIP_RECOVERY_REPORT_FILENAME
    );
    assert_ne!(
        "generic-matching-framing-batch.json",
        SLIP_RECOVERY_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        SLIP_RECOVERY_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, SLIP_RECOVERY_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        SLIP_RECOVERY_REPORT_FILENAME
    );
    assert_ne!(ACK_STATE_REPORT_FILENAME, SLIP_RECOVERY_REPORT_FILENAME);
    assert_ne!(OUTPUT_FLUSH_REPORT_FILENAME, SLIP_RECOVERY_REPORT_FILENAME);
    assert_ne!(SLIP_HAPPY_REPORT_FILENAME, SLIP_RECOVERY_REPORT_FILENAME);
    assert_ne!(
        SLIP_MALFORMED_REPORT_FILENAME,
        SLIP_RECOVERY_REPORT_FILENAME
    );
    assert_ne!("command-lifecycle-batch.json", COBS_PRESET_REPORT_FILENAME);
    assert_ne!(
        "generic-matching-framing-batch.json",
        COBS_PRESET_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        COBS_PRESET_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, COBS_PRESET_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        COBS_PRESET_REPORT_FILENAME
    );
    assert_ne!(ACK_STATE_REPORT_FILENAME, COBS_PRESET_REPORT_FILENAME);
    assert_ne!(OUTPUT_FLUSH_REPORT_FILENAME, COBS_PRESET_REPORT_FILENAME);
    assert_ne!(SLIP_HAPPY_REPORT_FILENAME, COBS_PRESET_REPORT_FILENAME);
    assert_ne!(SLIP_MALFORMED_REPORT_FILENAME, COBS_PRESET_REPORT_FILENAME);
    assert_ne!(SLIP_RECOVERY_REPORT_FILENAME, COBS_PRESET_REPORT_FILENAME);
    assert_ne!("command-lifecycle-batch.json", AT_PARSER_REPORT_FILENAME);
    assert_ne!(
        "generic-matching-framing-batch.json",
        AT_PARSER_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        AT_PARSER_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, AT_PARSER_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        AT_PARSER_REPORT_FILENAME
    );
    assert_ne!(ACK_STATE_REPORT_FILENAME, AT_PARSER_REPORT_FILENAME);
    assert_ne!(OUTPUT_FLUSH_REPORT_FILENAME, AT_PARSER_REPORT_FILENAME);
    assert_ne!(SLIP_HAPPY_REPORT_FILENAME, AT_PARSER_REPORT_FILENAME);
    assert_ne!(SLIP_MALFORMED_REPORT_FILENAME, AT_PARSER_REPORT_FILENAME);
    assert_ne!(SLIP_RECOVERY_REPORT_FILENAME, AT_PARSER_REPORT_FILENAME);
    assert_ne!(COBS_PRESET_REPORT_FILENAME, AT_PARSER_REPORT_FILENAME);
    assert_ne!(
        "command-lifecycle-batch.json",
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        "generic-matching-framing-batch.json",
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        FLOOD_BUFFER_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        ACK_STATE_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        OUTPUT_FLUSH_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        SLIP_HAPPY_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        SLIP_MALFORMED_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        SLIP_RECOVERY_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        COBS_PRESET_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!(
        AT_PARSER_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME
    );
    assert_ne!("command-lifecycle-batch.json", JSON_PARSER_REPORT_FILENAME);
    assert_ne!(
        "generic-matching-framing-batch.json",
        JSON_PARSER_REPORT_FILENAME
    );
    assert_ne!(
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        JSON_PARSER_REPORT_FILENAME
    );
    assert_ne!(FLOOD_BUFFER_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        JSON_PARSER_REPORT_FILENAME
    );
    assert_ne!(ACK_STATE_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(OUTPUT_FLUSH_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(SLIP_HAPPY_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(SLIP_MALFORMED_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(SLIP_RECOVERY_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(COBS_PRESET_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(AT_PARSER_REPORT_FILENAME, JSON_PARSER_REPORT_FILENAME);
    assert_ne!(
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME,
        JSON_PARSER_REPORT_FILENAME
    );
    for filename in [
        "command-lifecycle-batch.json",
        "generic-matching-framing-batch.json",
        RAW_GENERIC_FRAMING_REPORT_FILENAME,
        FLOOD_BUFFER_REPORT_FILENAME,
        COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        ACK_STATE_REPORT_FILENAME,
        OUTPUT_FLUSH_REPORT_FILENAME,
        SLIP_HAPPY_REPORT_FILENAME,
        SLIP_MALFORMED_REPORT_FILENAME,
        SLIP_RECOVERY_REPORT_FILENAME,
        COBS_PRESET_REPORT_FILENAME,
        AT_PARSER_REPORT_FILENAME,
        AT_PROTOCOL_DEFAULT_REPORT_FILENAME,
        JSON_PARSER_REPORT_FILENAME,
    ] {
        assert_ne!(
            NDJSON_PRESET_REPORT_FILENAME, filename,
            "Batch 15 report filename must remain isolated"
        );
    }
    let mut outcome = sample_outcome();
    let Observation::Read(read) = &mut outcome.observations[2] else {
        panic!("sample read observation missing");
    };
    read.position = None;
    let report = DifferentialReport {
        schema_id: REPORT_SCHEMA_ID.to_owned(),
        registry: RegistryCounts {
            total_rows: 49,
            compared_rows: 21,
            baseline_and_stronger_rows: 14,
            retired_rows: 3,
            pending_rows: 11,
        },
        paired_outcomes: vec![PairedScenarioOutcome {
            case: outcome.case,
            native: outcome.clone(),
            fixture: outcome,
        }],
    };
    let first = serialize_report(&report)?;
    let second = serialize_report(&report)?;
    assert_eq!(first, second);
    let text = std::str::from_utf8(&first)?;
    assert!(!text.contains("/dev/pts/") && !text.contains("last_activity_ms"));
    assert!(!text.contains("\"position\""));
    assert!(!text.contains("\"position\": null"));
    Ok(())
}

#[test]
fn strict_trace_decoder_accepts_contiguous_expected_lines() -> Result<()> {
    let data = "RX[0]=0x70\r\nRX[1]=0x69\r\nRX[2]=0x6e\r\n";
    assert_eq!(
        decode_trace_bytes(data, b"pin", 0)?,
        b"pin",
        "trace decoder must retain exact peer bytes"
    );
    Ok(())
}

#[test]
fn strict_trace_decoder_rejects_sequence_byte_and_record_errors() {
    let cases = [
        (
            "malformed sequence",
            "RX[x]=0x70\r\nRX[1]=0x69\r\n",
            b"pi".as_slice(),
        ),
        (
            "sequence gap",
            "RX[0]=0x70\r\nRX[2]=0x69\r\n",
            b"pi".as_slice(),
        ),
        ("wrong byte", "RX[0]=0x71\r\n", b"p".as_slice()),
        (
            "extra nontrace line",
            "RX[0]=0x70\r\nnoise\r\n",
            b"p".as_slice(),
        ),
        (
            "trailing incomplete line",
            "RX[0]=0x70\r\nRX[1]=0x69",
            b"pi".as_slice(),
        ),
    ];
    for (label, data, expected) in cases {
        assert!(
            decode_trace_bytes(data, expected, 0).is_err(),
            "strict trace decoder accepted {label}"
        );
    }
}

fn sample_outcome() -> ScenarioOutcome {
    let serial = SerialSettingsObservation {
        baud_rate: 38_400,
        data_bits: "8".to_owned(),
        stop_bits: "1".to_owned(),
        parity: "none".to_owned(),
        flow_control: "none".to_owned(),
    };
    ScenarioOutcome {
        case: DifferentialCase::PingRoundtrip,
        observations: vec![
            Observation::Open(OpenObservation {
                is_error: None,
                connection_id: LOGICAL_CONNECTION.to_owned(),
                name: Some("differential-named-uart".to_owned()),
                port: LOGICAL_ENDPOINT.to_owned(),
                baud_rate: 115_200,
            }),
            Observation::Write(WriteObservation {
                is_error: None,
                connection_id: LOGICAL_CONNECTION.to_owned(),
                name: Some("differential-named-uart".to_owned()),
                bytes_written: 6,
                decoded_bytes: 6,
                encoding: "utf8".to_owned(),
            }),
            Observation::Read(ReadObservation {
                is_error: None,
                connection_id: LOGICAL_CONNECTION.to_owned(),
                name: Some("differential-named-uart".to_owned()),
                encoding: "utf8".to_owned(),
                payload: b"pong\r\n".to_vec(),
                bytes_read: 6,
                stop_reason: "match_found".to_owned(),
                truncated: false,
                bytes_observed: 6,
                bytes_returned: 6,
                matched: true,
                match_index: Some(0),
                match_frame_index: Some(0),
                frames: Some(vec![FrameObservation {
                    frame_index: 0,
                    frame_type: "line".to_owned(),
                    encoding: "utf8".to_owned(),
                    payload: b"pong".to_vec(),
                    parsed: Some(ParsedFrameObservation::AtCommand {
                        response_type: "data".to_owned(),
                        command: None,
                        status: None,
                        fields: vec!["pong".to_owned()],
                    }),
                }]),
                frames_dropped: 0,
                error: None,
                position: Some(ReadPositionObservation {
                    from_offset: 0,
                    next_offset: 6,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 6,
                }),
            }),
            Observation::StatusDelta(StatusDeltaObservation {
                before_is_error: None,
                after_is_error: None,
                name: Some("differential-named-uart".to_owned()),
                serial: serial.clone(),
                state: NormalizedConnectionState::Open,
                tx_bytes: 6,
                rx_bytes: 6,
                read_ops: 1,
                write_ops: 1,
                truncation_count: 0,
            }),
            Observation::Reconfigure(ReconfigureObservation {
                is_error: None,
                connection_id: LOGICAL_CONNECTION.to_owned(),
                name: Some("differential-named-uart".to_owned()),
                port: LOGICAL_ENDPOINT.to_owned(),
                serial: serial.clone(),
            }),
            Observation::ConnectionSummary(ConnectionSummaryObservation {
                is_error: None,
                connection_id: LOGICAL_CONNECTION.to_owned(),
                name: Some("differential-named-uart".to_owned()),
                port: LOGICAL_ENDPOINT.to_owned(),
                baud_rate: 38_400,
                flow_control: "none".to_owned(),
            }),
            Observation::SetFlowControl(SetFlowControlObservation {
                is_error: None,
                connection_id: LOGICAL_CONNECTION.to_owned(),
                name: Some("differential-named-uart".to_owned()),
                flow_control: "none".to_owned(),
            }),
            Observation::PeerWire(PeerWireObservation {
                direction: PeerWireDirection::HostToPeer,
                mode: PeerWireMode::Delimiter,
                bytes: b"ping|".to_vec(),
            }),
            Observation::Flush(FlushObservation {
                is_error: None,
                connection_id: LOGICAL_CONNECTION.to_owned(),
                name: None,
                target: "output".to_owned(),
            }),
        ],
    }
}
