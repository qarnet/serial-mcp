//! Shared public-MCP scenarios and deterministic batch-isolated reports.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use anyhow::{ensure, Context, Result};
use rmcp::model::CallToolResult;
use rmcp::service::{RoleClient, RunningService};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::backend::{
    spam_stream, BackendKind, DifferentialEndpoint, BOOT_BANNER, JSONOUT_RESPONSE,
    NDJSON_PRESET_JSON_FRAMES_RESPONSE, NDJSON_PRESET_SKIPS_EMPTY_LINES_RESPONSE, PONG_RESPONSE,
};
use super::model::{
    normalize_batch_one_raw_read, normalize_connection_summary, normalize_flush, normalize_open,
    normalize_positioned_read, normalize_read, normalize_reconfigure, normalize_set_flow_control,
    normalize_status, normalize_write, status_delta, ConnectionSummaryObservation,
    DifferentialCase, FlushObservation, FrameObservation, NormalizationContext, Observation,
    OpenObservation, ParsedFrameObservation, PeerWireDirection, PeerWireMode, PeerWireObservation,
    ReadObservation, ReadPositionObservation, ReconfigureObservation, ScenarioOutcome,
    SetFlowControlObservation, StatusDeltaObservation, WriteObservation, LOGICAL_CONNECTION,
    LOGICAL_ENDPOINT,
};
use super::registry::{self, DifferentialBatch, RegistryCounts};
use crate::common::device_fixture::core::Action;
use crate::common::{connect_2026_07_28_client, tool_request, TestServer, VersionedClientHandler};

pub const REPORT_SCHEMA_ID: &str = "serial-mcp.native-sim-differential.command-lifecycle-batch.v1";
pub const GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.generic-matching-framing-batch.v1";
pub const RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.raw-generic-framing-batch.v1";
pub const RAW_GENERIC_FRAMING_REPORT_FILENAME: &str = "raw-generic-framing-batch.json";
pub const FLOOD_BUFFER_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.flood-buffer-batch.v1";
pub const FLOOD_BUFFER_REPORT_FILENAME: &str = "flood-buffer-batch.json";
pub const COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.command-diagnostics-batch.v1";
pub const COMMAND_DIAGNOSTICS_REPORT_FILENAME: &str = "command-diagnostics-batch.json";
pub const ACK_STATE_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.ack-state-batch.v1";
pub const ACK_STATE_REPORT_FILENAME: &str = "ack-state-batch.json";
pub const OUTPUT_FLUSH_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.output-flush-batch.v1";
pub const OUTPUT_FLUSH_REPORT_FILENAME: &str = "output-flush-batch.json";
pub const SLIP_HAPPY_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.slip-happy-batch.v1";
pub const SLIP_HAPPY_REPORT_FILENAME: &str = "slip-happy-batch.json";
pub const SLIP_MALFORMED_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.slip-malformed-batch.v1";
pub const SLIP_MALFORMED_REPORT_FILENAME: &str = "slip-malformed-batch.json";
pub const SLIP_RECOVERY_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.slip-recovery-batch.v1";
pub const SLIP_RECOVERY_REPORT_FILENAME: &str = "slip-recovery-batch.json";
pub const COBS_PRESET_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.cobs-preset-batch.v1";
pub const COBS_PRESET_REPORT_FILENAME: &str = "cobs-preset-batch.json";
pub const AT_PARSER_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.at-parser-batch.v1";
pub const AT_PARSER_REPORT_FILENAME: &str = "at-parser-batch.json";
pub const AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.at-protocol-default-batch.v1";
pub const AT_PROTOCOL_DEFAULT_REPORT_FILENAME: &str = "at-protocol-default-batch.json";
pub const JSON_PARSER_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.json-parser-batch.v1";
pub const JSON_PARSER_REPORT_FILENAME: &str = "json-parser-batch.json";
pub const NDJSON_PRESET_REPORT_SCHEMA_ID: &str =
    "serial-mcp.native-sim-differential.ndjson-preset-batch.v1";
pub const NDJSON_PRESET_REPORT_FILENAME: &str = "ndjson-preset-batch.json";

const TOOL_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const READ_TIMEOUT_MS: u64 = 3_000;
const NO_NEW_RX_TIMEOUT_MS: u64 = 100;
const INHERITED_BASELINE_DELAY: Duration = Duration::from_millis(100);
const RAW_STIMULUS_DELAY: Duration = Duration::from_millis(1000);
const MAX_FRAMES_WRITE_SEPARATION: Duration = Duration::from_millis(100);
const CONNECTION_NAME: &str = "differential-named-uart";
const FRAMING_DIAGNOSTIC_LINE: &[u8] = b"LINE len=4 data=\"ping\"\r\n";
const SLIP_HAPPY_SENDRAW_COMMAND: &str = "sendraw hex C0706F6E67C0\r\n";
const SLIP_HAPPY_WIRE: &[u8] = &[0xc0, b'p', b'o', b'n', b'g', 0xc0];
const SLIP_MALFORMED_SENDRAW_COMMAND: &str = "sendraw hex C0DB41C0\r\n";
const SLIP_MALFORMED_WIRE: &[u8] = &[0xc0, 0xdb, 0x41, 0xc0];
const COBS_PRESET_SENDRAW_COMMAND: &str = "sendraw hex 0005706F6E6700\r\n";
const COBS_PRESET_WIRE: &[u8] = &[0x00, 0x05, b'p', b'o', b'n', b'g', 0x00];
const NDJSON_PRESET_JSON_FRAMES_SENDRAW_COMMAND: &str =
    "sendraw hex 7B2261223A317D0A0A7B2262223A327D0A\r\n";
const NDJSON_PRESET_SKIPS_EMPTY_LINES_SENDRAW_COMMAND: &str =
    "sendraw hex 7B2261223A317D0A0A0A7B2262223A327D0A2020200A7B2263223A337D0A\r\n";

#[derive(Debug)]
struct AckStep {
    command: &'static [u8],
    marker: &'static str,
    payload: &'static [u8],
    write_bytes: usize,
    match_index: usize,
    position: ReadPositionObservation,
}

static ACK_STEPS: &[AckStep] = &[
    AckStep {
        command: b"ack on\r\n",
        marker: "ack on",
        payload: b"ack on\r\n",
        write_bytes: 8,
        match_index: 0,
        position: ReadPositionObservation {
            from_offset: 32,
            next_offset: 40,
            bytes_lost: 0,
            buffered_remaining: 0,
            start_offset: 0,
            end_offset: 40,
        },
    },
    AckStep {
        command: b"ping\r\n",
        marker: "pong",
        payload: b"ack 0\r\npong\r\n",
        write_bytes: 6,
        match_index: 7,
        position: ReadPositionObservation {
            from_offset: 40,
            next_offset: 53,
            bytes_lost: 0,
            buffered_remaining: 0,
            start_offset: 0,
            end_offset: 53,
        },
    },
    AckStep {
        command: b"ping\r\n",
        marker: "pong",
        payload: b"ack 1\r\npong\r\n",
        write_bytes: 6,
        match_index: 7,
        position: ReadPositionObservation {
            from_offset: 53,
            next_offset: 66,
            bytes_lost: 0,
            buffered_remaining: 0,
            start_offset: 0,
            end_offset: 66,
        },
    },
    AckStep {
        command: b"ack off\r\n",
        marker: "ack off",
        payload: b"ack 2\r\nack off\r\n",
        write_bytes: 9,
        match_index: 7,
        position: ReadPositionObservation {
            from_offset: 66,
            next_offset: 82,
            bytes_lost: 0,
            buffered_remaining: 0,
            start_offset: 0,
            end_offset: 82,
        },
    },
    AckStep {
        command: b"ping\r\n",
        marker: "pong",
        payload: b"pong\r\n",
        write_bytes: 6,
        match_index: 0,
        position: ReadPositionObservation {
            from_offset: 82,
            next_offset: 88,
            bytes_lost: 0,
            buffered_remaining: 0,
            start_offset: 0,
            end_offset: 88,
        },
    },
];

type ModernClient = RunningService<RoleClient, VersionedClientHandler>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentialReport {
    pub schema_id: String,
    pub registry: RegistryCounts,
    pub paired_outcomes: Vec<PairedScenarioOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairedScenarioOutcome {
    pub case: DifferentialCase,
    pub native: ScenarioOutcome,
    pub fixture: ScenarioOutcome,
}

/// Run Batch 1's fixed command/lifecycle cases only.
pub async fn run_command_lifecycle_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::CommandLifecycle).await
}

/// Run Batch 2's fixed generic matching/framing cases only.
pub async fn run_generic_matching_framing_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::GenericMatchingFraming).await
}

/// Run Batch 3's fixed raw generic framing cases only.
pub async fn run_raw_generic_framing_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::RawGenericFraming).await
}

/// Run Batch 4's fixed flood/buffer cases only.
pub async fn run_flood_buffer_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::FloodBuffer).await
}

/// Run Batch 5's fixed command-diagnostic cases only.
pub async fn run_command_diagnostics_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::CommandDiagnostics).await
}

/// Run Batch 6's fixed ACK state-machine case only.
pub async fn run_ack_state_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::AckState).await
}

/// Run Batch 7's one output-flush-after-delivery case only.
pub async fn run_output_flush_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::OutputFlush).await
}

/// Run Batch 8's one direct SLIP happy-path case only.
pub async fn run_slip_happy_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::SlipHappy).await
}

/// Run Batch 9's one malformed SLIP baseline-and-stronger case only.
pub async fn run_slip_malformed_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::SlipMalformed).await
}

/// Run Batch 10's one direct malformed-then-recovery SLIP case only.
pub async fn run_slip_recovery_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::SlipRecovery).await
}

/// Run Batch 11's one direct COBS-preset case only.
pub async fn run_cobs_preset_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::CobsPreset).await
}

/// Run Batch 12's one direct AT-parser case only.
pub async fn run_at_parser_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::AtParser).await
}

/// Run Batch 13's one direct AT protocol-default case only.
pub async fn run_at_protocol_default_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::AtProtocolDefault).await
}

/// Run Batch 14's one direct JSON-parser case only.
pub async fn run_json_parser_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::JsonParser).await
}

/// Run Batch 15's two direct NDJSON-preset cases only.
pub async fn run_ndjson_preset_batch() -> Result<DifferentialReport> {
    run_batch(DifferentialBatch::NdjsonPreset).await
}

/// Run one explicit batch through the same scenario function for each real PTY
/// backend, then produce a typed, dynamic-free report value.
async fn run_batch(batch: DifferentialBatch) -> Result<DifferentialReport> {
    let registry = registry::validate_current_registry()?;
    let mut paired_outcomes = Vec::new();
    for case in registry::executable_cases(batch)? {
        let native = run_case(case, BackendKind::Native)
            .await
            .with_context(|| format!("run native differential case {}", case.id()))?;
        let fixture = run_case(case, BackendKind::Fixture)
            .await
            .with_context(|| format!("run fixture differential case {}", case.id()))?;
        if native.outcome != fixture.outcome {
            let native_json = serde_json::to_string_pretty(&native.outcome)
                .context("serialize mismatched native normalized outcome")?;
            let fixture_json = serde_json::to_string_pretty(&fixture.outcome)
                .context("serialize mismatched fixture normalized outcome")?;
            let native_raw = serde_json::to_string_pretty(&native.raw_observations)
                .context("serialize mismatched native raw public observations")?;
            let fixture_raw = serde_json::to_string_pretty(&fixture.raw_observations)
                .context("serialize mismatched fixture raw public observations")?;
            anyhow::bail!(
                "differential mismatch for {}\nnative normalized outcome:\n{native_json}\nfixture normalized outcome:\n{fixture_json}\nnative raw public structured results:\n{native_raw}\nfixture raw public structured results:\n{fixture_raw}",
                case.id()
            );
        }
        paired_outcomes.push(PairedScenarioOutcome {
            case,
            native: native.outcome,
            fixture: fixture.outcome,
        });
    }
    let report = DifferentialReport {
        schema_id: report_schema_id(batch).to_owned(),
        registry,
        paired_outcomes,
    };
    validate_report(&report)?;
    Ok(report)
}

/// Serialize and write canonical pretty JSON below the workspace target tree.
pub fn write_report(report: &DifferentialReport) -> Result<PathBuf> {
    validate_report(report)?;
    let batch = batch_for_report_schema(&report.schema_id)?;
    let path = crate::common::workspace_root()
        .join("target")
        .join("native-sim-differential")
        .join(report_filename(batch));
    let parent = path
        .parent()
        .context("differential report path had no parent directory")?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("create differential report directory {}", parent.display()))?;
    let bytes = serialize_report(report)?;
    std::fs::write(&path, bytes)
        .with_context(|| format!("write differential report {}", path.display()))?;
    Ok(path)
}

fn report_schema_id(batch: DifferentialBatch) -> &'static str {
    match batch {
        DifferentialBatch::CommandLifecycle => REPORT_SCHEMA_ID,
        DifferentialBatch::GenericMatchingFraming => GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID,
        DifferentialBatch::RawGenericFraming => RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID,
        DifferentialBatch::FloodBuffer => FLOOD_BUFFER_REPORT_SCHEMA_ID,
        DifferentialBatch::CommandDiagnostics => COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID,
        DifferentialBatch::AckState => ACK_STATE_REPORT_SCHEMA_ID,
        DifferentialBatch::OutputFlush => OUTPUT_FLUSH_REPORT_SCHEMA_ID,
        DifferentialBatch::SlipHappy => SLIP_HAPPY_REPORT_SCHEMA_ID,
        DifferentialBatch::SlipMalformed => SLIP_MALFORMED_REPORT_SCHEMA_ID,
        DifferentialBatch::SlipRecovery => SLIP_RECOVERY_REPORT_SCHEMA_ID,
        DifferentialBatch::CobsPreset => COBS_PRESET_REPORT_SCHEMA_ID,
        DifferentialBatch::AtParser => AT_PARSER_REPORT_SCHEMA_ID,
        DifferentialBatch::AtProtocolDefault => AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID,
        DifferentialBatch::JsonParser => JSON_PARSER_REPORT_SCHEMA_ID,
        DifferentialBatch::NdjsonPreset => NDJSON_PRESET_REPORT_SCHEMA_ID,
    }
}

fn report_filename(batch: DifferentialBatch) -> &'static str {
    match batch {
        DifferentialBatch::CommandLifecycle => "command-lifecycle-batch.json",
        DifferentialBatch::GenericMatchingFraming => "generic-matching-framing-batch.json",
        DifferentialBatch::RawGenericFraming => RAW_GENERIC_FRAMING_REPORT_FILENAME,
        DifferentialBatch::FloodBuffer => FLOOD_BUFFER_REPORT_FILENAME,
        DifferentialBatch::CommandDiagnostics => COMMAND_DIAGNOSTICS_REPORT_FILENAME,
        DifferentialBatch::AckState => ACK_STATE_REPORT_FILENAME,
        DifferentialBatch::OutputFlush => OUTPUT_FLUSH_REPORT_FILENAME,
        DifferentialBatch::SlipHappy => SLIP_HAPPY_REPORT_FILENAME,
        DifferentialBatch::SlipMalformed => SLIP_MALFORMED_REPORT_FILENAME,
        DifferentialBatch::SlipRecovery => SLIP_RECOVERY_REPORT_FILENAME,
        DifferentialBatch::CobsPreset => COBS_PRESET_REPORT_FILENAME,
        DifferentialBatch::AtParser => AT_PARSER_REPORT_FILENAME,
        DifferentialBatch::AtProtocolDefault => AT_PROTOCOL_DEFAULT_REPORT_FILENAME,
        DifferentialBatch::JsonParser => JSON_PARSER_REPORT_FILENAME,
        DifferentialBatch::NdjsonPreset => NDJSON_PRESET_REPORT_FILENAME,
    }
}

fn batch_for_report_schema(schema_id: &str) -> Result<DifferentialBatch> {
    match schema_id {
        REPORT_SCHEMA_ID => Ok(DifferentialBatch::CommandLifecycle),
        GENERIC_MATCHING_FRAMING_REPORT_SCHEMA_ID => Ok(DifferentialBatch::GenericMatchingFraming),
        RAW_GENERIC_FRAMING_REPORT_SCHEMA_ID => Ok(DifferentialBatch::RawGenericFraming),
        FLOOD_BUFFER_REPORT_SCHEMA_ID => Ok(DifferentialBatch::FloodBuffer),
        COMMAND_DIAGNOSTICS_REPORT_SCHEMA_ID => Ok(DifferentialBatch::CommandDiagnostics),
        ACK_STATE_REPORT_SCHEMA_ID => Ok(DifferentialBatch::AckState),
        OUTPUT_FLUSH_REPORT_SCHEMA_ID => Ok(DifferentialBatch::OutputFlush),
        SLIP_HAPPY_REPORT_SCHEMA_ID => Ok(DifferentialBatch::SlipHappy),
        SLIP_MALFORMED_REPORT_SCHEMA_ID => Ok(DifferentialBatch::SlipMalformed),
        SLIP_RECOVERY_REPORT_SCHEMA_ID => Ok(DifferentialBatch::SlipRecovery),
        COBS_PRESET_REPORT_SCHEMA_ID => Ok(DifferentialBatch::CobsPreset),
        AT_PARSER_REPORT_SCHEMA_ID => Ok(DifferentialBatch::AtParser),
        AT_PROTOCOL_DEFAULT_REPORT_SCHEMA_ID => Ok(DifferentialBatch::AtProtocolDefault),
        JSON_PARSER_REPORT_SCHEMA_ID => Ok(DifferentialBatch::JsonParser),
        NDJSON_PRESET_REPORT_SCHEMA_ID => Ok(DifferentialBatch::NdjsonPreset),
        _ => anyhow::bail!("unknown differential report schema_id {schema_id:?}"),
    }
}

/// Canonical pretty JSON bytes used both by report writing and deterministic
/// serialization tests.
pub fn serialize_report(report: &DifferentialReport) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(report).context("serialize differential report")?;
    bytes.push(b'\n');
    Ok(bytes)
}

#[derive(Debug, Clone, Copy)]
struct OpenOptions {
    name: Option<&'static str>,
    baud_rate: u32,
    flow_control: Option<&'static str>,
    framing: OpenFraming,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenFraming {
    Standard,
    AtCommandWithExplicitLine,
    AtCommandDefaults,
}

impl OpenOptions {
    const fn standard() -> Self {
        Self {
            name: None,
            baud_rate: 115_200,
            flow_control: None,
            framing: OpenFraming::Standard,
        }
    }
}

fn open_options(case: DifferentialCase) -> OpenOptions {
    match case {
        DifferentialCase::NamedConnectionAppearsInListConnections => OpenOptions {
            name: Some(CONNECTION_NAME),
            ..OpenOptions::standard()
        },
        DifferentialCase::SetFlowControlUpdatesSummaryAndResult
        | DifferentialCase::OpenWithFlowControlPersistsInSummary => OpenOptions {
            flow_control: Some("none"),
            ..OpenOptions::standard()
        },
        DifferentialCase::ExplicitRxFramingBeatsConnectionDefault => OpenOptions {
            framing: OpenFraming::AtCommandWithExplicitLine,
            ..OpenOptions::standard()
        },
        DifferentialCase::AtProtocolDefaultPong => OpenOptions {
            framing: OpenFraming::AtCommandDefaults,
            ..OpenOptions::standard()
        },
        DifferentialCase::PingRoundtrip
        | DifferentialCase::PendingReadThenWritePingRoundtrip
        | DifferentialCase::SplitWritesPreserveCommandOrder
        | DifferentialCase::FramingDiagnosticSplitPing
        | DifferentialCase::TraceDiagnosticSplitPing
        | DifferentialCase::GetStatusAfterWriteIncrementsTxCounter
        | DifferentialCase::ReconfigureBaudRatePersists
        | DifferentialCase::RegexMatchesPong
        | DifferentialCase::GlobMatchesPongLine
        | DifferentialCase::LineFramingSplitsLines
        | DifferentialCase::FramingMaxFramesStops
        | DifferentialCase::FramingPlusMatchCombined
        | DifferentialCase::DelimiterFramingDecodes
        | DifferentialCase::LengthPrefixedFramingDecodes
        | DifferentialCase::StartEndFramingDecodes
        | DifferentialCase::TxFramingModesObservedViaTrace
        | DifferentialCase::ExplicitLineEndingsSplitCorrectly
        | DifferentialCase::FloodMatcherSpamComplete
        | DifferentialCase::FloodBufferBudget
        | DifferentialCase::PartialLineThenCompletePing
        | DifferentialCase::AckStateMachine
        | DifferentialCase::OutputFlushAfterDelivery
        | DifferentialCase::SlipHappyPath
        | DifferentialCase::SlipMalformedEscape
        | DifferentialCase::SlipRecoveryAfterMalformed
        | DifferentialCase::CobsPresetDecode
        | DifferentialCase::AtParserPong
        | DifferentialCase::JsonParserJsonout
        | DifferentialCase::NdjsonPresetJsonFrames
        | DifferentialCase::NdjsonPresetSkipsEmptyLines => OpenOptions::standard(),
    }
}

struct CaseRun {
    outcome: ScenarioOutcome,
    raw_observations: Vec<RawToolObservation>,
}

async fn run_case(case: DifferentialCase, kind: BackendKind) -> Result<CaseRun> {
    let mut endpoint = DifferentialEndpoint::spawn(kind)
        .await
        .with_context(|| format!("spawn {kind:?} endpoint"))?;
    let session = PublicSession::start(&endpoint, open_options(case)).await;
    let (mut session, open) = match session {
        Ok(session) => session,
        Err(error) => {
            return match endpoint.shutdown().await {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(error.context(format!(
                    "differential endpoint cleanup also failed: {cleanup_error}"
                ))),
            };
        }
    };

    let scenario = execute_case(case, &mut session, open, &mut endpoint).await;
    let mut cleanup_errors = session.shutdown().await;
    if let Err(error) = endpoint.shutdown().await {
        cleanup_errors.push(error.context("shutdown differential endpoint"));
    }
    let outcome = combine_result(scenario, cleanup_errors)?;
    Ok(CaseRun {
        outcome,
        raw_observations: session.raw_observations(),
    })
}

fn combine_result(
    scenario: Result<ScenarioOutcome>,
    cleanup_errors: Vec<anyhow::Error>,
) -> Result<ScenarioOutcome> {
    if cleanup_errors.is_empty() {
        return scenario;
    }
    let cleanup_message = cleanup_errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ");
    match scenario {
        Ok(_) => anyhow::bail!(
            "differential cleanup failed after successful scenario: {cleanup_message}"
        ),
        Err(error) => Err(error.context(format!(
            "differential cleanup also failed: {cleanup_message}"
        ))),
    }
}

async fn execute_case(
    case: DifferentialCase,
    session: &mut PublicSession,
    open: OpenObservation,
    endpoint: &mut DifferentialEndpoint,
) -> Result<ScenarioOutcome> {
    let mut observations = vec![Observation::Open(open)];
    let boot = session.sync_boot(case).await?;
    observations.push(Observation::Read(boot));

    match case {
        DifferentialCase::PingRoundtrip => {
            let write = session.write(b"ping\r\n").await?;
            assert_write(&write, b"ping\r\n")?;
            observations.push(Observation::Write(write));
            let pong = session.read_pong(None).await?;
            assert_batch_one_pong(&pong)?;
            observations.push(Observation::Read(pong));
        }
        DifferentialCase::PendingReadThenWritePingRoundtrip => {
            let (write, pong) = session.pending_read_then_write_baseline().await?;
            observations.push(Observation::Write(write));
            observations.push(Observation::Read(pong));
        }
        DifferentialCase::FloodMatcherSpamComplete => {
            let expected = spam_stream(1024);
            let (writes, read) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": 5000,
                        "encoding": "utf8",
                        "match": {
                            "pattern": "Spam complete",
                            "config": {
                                "mode": "literal_substring",
                                "pattern_encoding": "utf8"
                            }
                        }
                    }),
                    &[b"spam 1024 hex\r\n".as_slice()],
                    ReadNormalization::Positioned,
                    None,
                )
                .await?;
            ensure!(
                writes.len() == 1,
                "flood matcher expected one public write, got {writes:?}"
            );
            assert_write(&writes[0], b"spam 1024 hex\r\n")?;
            assert_exact_spam_match_read(&read, &expected)?;
            observations.extend(writes.into_iter().map(Observation::Write));
            observations.push(Observation::Read(read));
        }
        DifferentialCase::FloodBufferBudget => {
            session.configure_max_buffered_bytes().await?;
            let expected = spam_stream(512);
            let (writes, read) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": 3000,
                        "encoding": "utf8"
                    }),
                    &[b"spam 512 hex\r\n".as_slice()],
                    ReadNormalization::Positioned,
                    None,
                )
                .await?;
            ensure!(
                writes.len() == 1,
                "flood buffer expected one public write, got {writes:?}"
            );
            assert_write(&writes[0], b"spam 512 hex\r\n")?;
            assert_exact_flood_buffer_read(&read, &expected[..256])?;
            observations.extend(writes.into_iter().map(Observation::Write));
            observations.push(Observation::Read(read));
        }
        DifferentialCase::SplitWritesPreserveCommandOrder => {
            for fragment in [b"pi".as_slice(), b"n", b"g", b"\r\n"] {
                let write = session.write(fragment).await?;
                assert_write(&write, fragment)?;
                observations.push(Observation::Write(write));
            }
            let pong = session.read_pong(None).await?;
            assert_batch_one_pong(&pong)?;
            observations.push(Observation::Read(pong));
        }
        DifferentialCase::FramingDiagnosticSplitPing => {
            session.framing_setup().await?;
            let (writes, read) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": READ_TIMEOUT_MS,
                        "encoding": "utf8",
                        "match": {
                            "pattern": "pong",
                            "config": {
                                "mode": "literal_substring",
                                "pattern_encoding": "utf8"
                            }
                        }
                    }),
                    &[b"pi".as_slice(), b"ng", b"\r\n"],
                    ReadNormalization::Positioned,
                    None,
                )
                .await?;
            let expected = framing_diagnostic_payload();
            assert_exact_positioned_match_read(
                &read,
                &expected,
                48,
                ReadPositionObservation {
                    from_offset: 44,
                    next_offset: 98,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 98,
                },
                "framing diagnostic split ping",
            )?;
            for (write, payload) in writes.iter().zip([b"pi".as_slice(), b"ng", b"\r\n"]) {
                assert_diagnostic_write(write, payload)?;
            }
            ensure!(
                writes.len() == 3,
                "framing diagnostic split ping expected three writes, got {writes:?}"
            );
            observations.extend(writes.into_iter().map(Observation::Write));
            observations.push(Observation::Read(read));
        }
        DifferentialCase::TraceDiagnosticSplitPing => {
            session.trace_setup().await?;
            let (writes, read) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": READ_TIMEOUT_MS,
                        "encoding": "utf8",
                        "match": {
                            "pattern": "pong",
                            "config": {
                                "mode": "literal_substring",
                                "pattern_encoding": "utf8"
                            }
                        }
                    }),
                    &[b"pi".as_slice(), b"ng", b"\r\n"],
                    ReadNormalization::Positioned,
                    None,
                )
                .await?;
            let expected = trace_diagnostic_payload();
            assert_exact_positioned_match_read(
                &read,
                &expected,
                72,
                ReadPositionObservation {
                    from_offset: 42,
                    next_offset: 120,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 120,
                },
                "trace diagnostic split ping",
            )?;
            for (write, payload) in writes.iter().zip([b"pi".as_slice(), b"ng", b"\r\n"]) {
                assert_diagnostic_write(write, payload)?;
            }
            ensure!(
                writes.len() == 3,
                "trace diagnostic split ping expected three writes, got {writes:?}"
            );
            observations.extend(writes.into_iter().map(Observation::Write));
            observations.push(Observation::Read(read));
        }
        DifferentialCase::PartialLineThenCompletePing => {
            let (writes, read) = session.partial_read_then_complete_ping().await?;
            ensure!(
                writes.len() == 2,
                "partial line scenario expected two writes, got {writes:?}"
            );
            assert_diagnostic_write(&writes[0], b"pi")?;
            assert_diagnostic_write(&writes[1], b"ng\r\n")?;
            assert_exact_positioned_match_read(
                &read,
                PONG_RESPONSE,
                0,
                ReadPositionObservation {
                    from_offset: 32,
                    next_offset: 38,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 38,
                },
                "partial line then complete ping",
            )?;
            observations.extend(writes.into_iter().map(Observation::Write));
            observations.push(Observation::Read(read));
        }
        DifferentialCase::GetStatusAfterWriteIncrementsTxCounter => {
            let before = session.status().await?;
            let write = session.write(b"ping\r\n").await?;
            assert_write(&write, b"ping\r\n")?;
            observations.push(Observation::Write(write));
            let pong = session.read_pong(None).await?;
            assert_batch_one_pong(&pong)?;
            observations.push(Observation::Read(pong));
            let delta = status_delta(before, session.status().await?)?;
            assert_ping_status_delta(&delta)?;
            observations.push(Observation::StatusDelta(delta));
        }
        DifferentialCase::ReconfigureBaudRatePersists => {
            let before = session.status().await?;
            let reconfigure = session.reconfigure(38_400).await?;
            assert_serial_settings(&reconfigure.serial, 38_400, "none")?;
            observations.push(Observation::Reconfigure(reconfigure));
            let delta = status_delta(before, session.status().await?)?;
            ensure!(
                delta.serial.baud_rate == 38_400,
                "post-reconfigure status baud_rate was {}, expected 38400",
                delta.serial.baud_rate
            );
            observations.push(Observation::StatusDelta(delta));
            let write = session.write(b"ping\r\n").await?;
            assert_write(&write, b"ping\r\n")?;
            observations.push(Observation::Write(write));
            let pong = session.read_pong(None).await?;
            assert_batch_one_pong(&pong)?;
            observations.push(Observation::Read(pong));
        }
        DifferentialCase::NamedConnectionAppearsInListConnections => {
            let summary = session.one_connection_summary().await?;
            ensure!(
                summary.name.as_deref() == Some(CONNECTION_NAME),
                "named connection summary had unexpected name {:?}",
                summary.name
            );
            ensure!(
                summary.baud_rate == 115_200 && summary.flow_control == "none",
                "named connection summary settings changed: {summary:?}"
            );
            observations.push(Observation::ConnectionSummary(summary));
        }
        DifferentialCase::SetFlowControlUpdatesSummaryAndResult => {
            let flow = session.set_flow_control("none").await?;
            ensure!(
                flow.flow_control == "none",
                "set_flow_control returned {:?}",
                flow.flow_control
            );
            observations.push(Observation::SetFlowControl(flow));
            let summary = session.one_connection_summary().await?;
            ensure!(
                summary.flow_control == "none",
                "connection summary did not retain live flow control: {summary:?}"
            );
            observations.push(Observation::ConnectionSummary(summary));
        }
        DifferentialCase::OpenWithFlowControlPersistsInSummary => {
            let summary = session.one_connection_summary().await?;
            ensure!(
                summary.flow_control == "none",
                "connection summary did not retain open-time flow control: {summary:?}"
            );
            observations.push(Observation::ConnectionSummary(summary));
        }
        DifferentialCase::AckStateMachine => {
            for step in ACK_STEPS {
                let write = session.write(step.command).await?;
                let read = session.read_positioned_literal(step.marker).await?;
                assert_exact_ack_step(step, &write, &read)?;
                observations.push(Observation::Write(write));
                observations.push(Observation::Read(read));
            }
        }
        DifferentialCase::OutputFlushAfterDelivery => {
            let first_write = session.write(b"ping\r\n").await?;
            assert_exact_anonymous_write(&first_write, b"ping\r\n", "first ping")?;
            observations.push(Observation::Write(first_write));

            let first_read = session.read_positioned_literal("pong").await?;
            assert_exact_positioned_match_read(
                &first_read,
                PONG_RESPONSE,
                0,
                ReadPositionObservation {
                    from_offset: 32,
                    next_offset: 38,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 38,
                },
                "first pong positioned read",
            )?;
            observations.push(Observation::Read(first_read));

            let flush = session.flush_output().await?;
            ensure!(
                flush.is_error == Some(false) && flush.name.is_none() && flush.target == "output",
                "output flush result metadata mismatch: {flush:?}"
            );
            observations.push(Observation::Flush(flush));

            let second_write = session.write(b"ping\r\n").await?;
            assert_exact_anonymous_write(&second_write, b"ping\r\n", "second ping")?;
            observations.push(Observation::Write(second_write));

            let second_read = session.read_positioned_literal("pong").await?;
            assert_exact_positioned_match_read(
                &second_read,
                PONG_RESPONSE,
                0,
                ReadPositionObservation {
                    from_offset: 38,
                    next_offset: 44,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 44,
                },
                "second pong positioned read",
            )?;
            observations.push(Observation::Read(second_read));
        }
        DifferentialCase::SlipHappyPath => {
            session.slip_happy_setup().await?;
            let read = session.read_slip_happy().await?;
            assert_exact_slip_happy_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::SlipMalformedEscape => {
            session.slip_malformed_setup().await?;
            let read = session.read_slip_malformed().await?;
            assert_exact_slip_malformed_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::SlipRecoveryAfterMalformed => {
            session.slip_malformed_setup().await?;
            let malformed = session.read_slip_malformed().await?;
            assert_exact_slip_malformed_read(&malformed)?;
            observations.push(Observation::Read(malformed));

            session.slip_happy_setup().await?;
            let recovery = session.read_slip_recovery().await?;
            assert_exact_slip_recovery_read(&recovery)?;
            observations.push(Observation::Read(recovery));
        }
        DifferentialCase::CobsPresetDecode => {
            session.cobs_preset_setup().await?;
            let read = session.read_cobs_preset().await?;
            assert_exact_cobs_preset_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::AtParserPong => {
            session.at_parser_setup().await?;
            let read = session.read_at_parser().await?;
            assert_exact_at_parser_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::AtProtocolDefaultPong => {
            session.at_protocol_default_setup().await?;
            let read = session.read_at_protocol_default().await?;
            assert_exact_at_protocol_default_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::JsonParserJsonout => {
            session.json_parser_setup().await?;
            let read = session.read_json_parser().await?;
            assert_exact_json_parser_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::NdjsonPresetJsonFrames => {
            session.ndjson_preset_json_frames_setup().await?;
            let read = session.read_ndjson_preset().await?;
            assert_exact_ndjson_json_frames_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::NdjsonPresetSkipsEmptyLines => {
            session.ndjson_preset_skips_empty_lines_setup().await?;
            let read = session.read_ndjson_preset().await?;
            assert_exact_ndjson_skips_empty_lines_read(&read)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::RegexMatchesPong => {
            let (write, pong) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": READ_TIMEOUT_MS,
                        "encoding": "utf8",
                        "match": {
                            "pattern": "po.g",
                            "config": { "mode": "regex", "pattern_encoding": "utf8" }
                        }
                    }),
                    &[b"ping\r\n".as_slice()],
                    ReadNormalization::General,
                    None,
                )
                .await?;
            assert_exact_raw_matched_read(&pong, PONG_RESPONSE, "regex pong")?;
            observations.extend(write.into_iter().map(Observation::Write));
            observations.push(Observation::Read(pong));
        }
        DifferentialCase::GlobMatchesPongLine => {
            let (write, pong) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": READ_TIMEOUT_MS,
                        "encoding": "utf8",
                        "match": {
                            "pattern": "po*",
                            "config": { "mode": "glob", "pattern_encoding": "utf8" }
                        }
                    }),
                    &[b"ping\r\n".as_slice()],
                    ReadNormalization::General,
                    None,
                )
                .await?;
            assert_exact_raw_matched_read(&pong, PONG_RESPONSE, "glob pong")?;
            observations.extend(write.into_iter().map(Observation::Write));
            observations.push(Observation::Read(pong));
        }
        DifferentialCase::LineFramingSplitsLines => {
            let write = session.write(b"write cmd 1 ping\r\n").await?;
            assert_write(&write, b"write cmd 1 ping\r\n")?;
            observations.push(Observation::Write(write));
            let read = session
                .read_general(json!({
                    "connection_id": session.connection_id()?,
                    "timeout_ms": READ_TIMEOUT_MS,
                    "no_new_rx_timeout_ms": NO_NEW_RX_TIMEOUT_MS,
                    "encoding": "utf8",
                    "rx_framing": { "type": "line" }
                }))
                .await?;
            assert_exact_line_frames(
                &read,
                "no_new_rx_timeout",
                vec![
                    expected_line_frame(0, b"ack 1 exec>ping"),
                    expected_line_frame(1, b"pong"),
                ],
                false,
            )?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::FramingMaxFramesStops => {
            let (writes, read) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": READ_TIMEOUT_MS,
                        "encoding": "utf8",
                        "rx_framing": { "type": "line", "max_frames": 2 }
                    }),
                    &[b"ping\r\n".as_slice(), b"ping\r\n", b"ping\r\n"],
                    ReadNormalization::General,
                    Some(MAX_FRAMES_WRITE_SEPARATION),
                )
                .await?;
            assert_exact_line_frames(
                &read,
                "max_frames",
                vec![
                    expected_line_frame(0, b"pong"),
                    expected_line_frame(1, b"pong"),
                ],
                false,
            )?;
            observations.extend(writes.into_iter().map(Observation::Write));
            observations.push(Observation::Read(read));
        }
        DifferentialCase::FramingPlusMatchCombined => {
            let (writes, read) = session
                .pending_read_then_write(
                    json!({
                        "connection_id": session.connection_id()?,
                        "from": { "type": "now" },
                        "timeout_ms": READ_TIMEOUT_MS,
                        "encoding": "utf8",
                        "rx_framing": { "type": "line" },
                        "match": {
                            "pattern": "pong",
                            "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                        }
                    }),
                    &[b"ping\r\n".as_slice()],
                    ReadNormalization::General,
                    None,
                )
                .await?;
            ensure!(
                read.stop_reason == "match_found"
                    && read.matched
                    && read.match_index == Some(0)
                    && read.match_frame_index == Some(0),
                "framing-plus-match result mismatch: {read:?}"
            );
            assert_exact_line_frames(
                &read,
                "match_found",
                vec![expected_line_frame(0, b"pong")],
                true,
            )?;
            observations.extend(writes.into_iter().map(Observation::Write));
            observations.push(Observation::Read(read));
        }
        DifferentialCase::ExplicitRxFramingBeatsConnectionDefault => {
            let write = session.write(b"ping").await?;
            ensure!(
                write.encoding == "utf8" && write.decoded_bytes == 4 && write.bytes_written == 5,
                "AT default write did not append one CR: {write:?}"
            );
            observations.push(Observation::Write(write));
            let read = session
                .read_general(json!({
                    "connection_id": session.connection_id()?,
                    "timeout_ms": READ_TIMEOUT_MS,
                    "encoding": "utf8"
                }))
                .await?;
            let expected = FrameObservation {
                frame_index: 0,
                frame_type: "line".to_owned(),
                encoding: "utf8".to_owned(),
                payload: b"pong\r".to_vec(),
                parsed: Some(ParsedFrameObservation::AtCommand {
                    response_type: "data".to_owned(),
                    command: None,
                    status: None,
                    fields: vec!["pong".to_owned()],
                }),
            };
            assert_exact_line_frames(&read, "max_frames", vec![expected], false)?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::DelimiterFramingDecodes => {
            let wire = b"|pong|";
            let read = session
                .read_raw_rx(
                    endpoint,
                    wire,
                    "hex",
                    json!({
                        "type": "delimiter",
                        "delimiter": "|",
                        "delimiter_encoding": "utf8",
                        "max_frames": 2,
                        "include_terminators": false,
                        "skip_empty": false
                    }),
                )
                .await?;
            assert_exact_raw_framed_read(
                &read,
                "hex",
                wire,
                vec![
                    expected_frame(0, "delimiter", "hex", b""),
                    expected_frame(1, "delimiter", "hex", b"pong"),
                ],
            )?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::LengthPrefixedFramingDecodes => {
            let wire = [4, b'p', b'o', b'n', b'g'];
            let read = session
                .read_raw_rx(
                    endpoint,
                    &wire,
                    "hex",
                    json!({
                        "type": "length_prefixed",
                        "prefix_size": 1,
                        "endianness": "big",
                        "max_frames": 1,
                        "include_terminators": false,
                        "skip_empty": false
                    }),
                )
                .await?;
            assert_exact_raw_framed_read(
                &read,
                "hex",
                &wire,
                vec![expected_frame(0, "length_prefixed", "hex", b"pong")],
            )?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::StartEndFramingDecodes => {
            let wire = b"<<pong>>";
            let read = session
                .read_raw_rx(
                    endpoint,
                    wire,
                    "utf8",
                    json!({
                        "type": "start_end",
                        "start": ["<<"],
                        "end": ">>",
                        "marker_encoding": "utf8",
                        "include_markers": false,
                        "max_frames": 1,
                        "include_terminators": false,
                        "skip_empty": false
                    }),
                )
                .await?;
            assert_exact_raw_framed_read(
                &read,
                "utf8",
                wire,
                vec![expected_frame(0, "start_end", "utf8", b"pong")],
            )?;
            observations.push(Observation::Read(read));
        }
        DifferentialCase::TxFramingModesObservedViaTrace => {
            let modes = [
                (
                    PeerWireMode::Delimiter,
                    json!({
                        "type": "delimiter",
                        "delimiter": "|",
                        "delimiter_encoding": "utf8"
                    }),
                    b"ping|".to_vec(),
                ),
                (
                    PeerWireMode::LengthPrefixed,
                    json!({
                        "type": "length_prefixed",
                        "prefix_size": 1,
                        "endianness": "big"
                    }),
                    vec![4, b'p', b'i', b'n', b'g'],
                ),
                (
                    PeerWireMode::StartEnd,
                    json!({
                        "type": "start_end",
                        "start": ["<<"],
                        "end": ">>",
                        "marker_encoding": "utf8"
                    }),
                    b"<<ping>>".to_vec(),
                ),
                (
                    PeerWireMode::Slip,
                    json!({ "type": "slip" }),
                    vec![0xc0, b'p', b'i', b'n', b'g', 0xc0],
                ),
            ];
            if matches!(endpoint, DifferentialEndpoint::Native(_)) {
                session.trace_setup().await?;
            }
            let secondary = session.connect_secondary_client().await?;
            let operation = async {
                let mut trace_sequence = 0u8;
                for (mode, framing, expected) in modes {
                    let (write, wire) = session
                        .observe_tx_mode(
                            endpoint,
                            &secondary,
                            mode,
                            framing,
                            &expected,
                            trace_sequence,
                        )
                        .await?;
                    observations.push(Observation::Write(write));
                    observations.push(Observation::PeerWire(wire));
                    trace_sequence = trace_sequence.wrapping_add(
                        u8::try_from(expected.len()).context("TX trace sequence overflowed")?,
                    );
                }
                Ok::<(), anyhow::Error>(())
            }
            .await;
            let cleanup =
                cancel_modern_client(secondary, "cancel secondary modern differential MCP client")
                    .await;
            combine_operation_and_cleanup(
                operation,
                cleanup,
                "secondary modern MCP client cleanup",
            )?;
        }
        DifferentialCase::ExplicitLineEndingsSplitCorrectly => {
            let cases = [
                (
                    "lf",
                    b"alpha\r\nbeta\n".as_slice(),
                    [b"alpha\r".as_slice(), b"beta"],
                ),
                (
                    "cr",
                    b"alpha\rbeta\r".as_slice(),
                    [b"alpha".as_slice(), b"beta"],
                ),
                (
                    "crlf",
                    b"alpha\r\nbeta\r\n".as_slice(),
                    [b"alpha".as_slice(), b"beta"],
                ),
            ];
            for (ending, wire, expected_payloads) in cases {
                let read = session
                    .read_raw_rx(
                        endpoint,
                        wire,
                        "utf8",
                        json!({
                            "type": "line",
                            "ending": ending,
                            "max_frames": 2,
                            "include_terminators": false,
                            "skip_empty": false
                        }),
                    )
                    .await?;
                assert_exact_raw_framed_read(
                    &read,
                    "utf8",
                    wire,
                    expected_payloads
                        .into_iter()
                        .enumerate()
                        .map(|(index, payload)| expected_frame(index, "line", "utf8", payload))
                        .collect(),
                )?;
                observations.push(Observation::Read(read));
            }
        }
    }

    Ok(ScenarioOutcome { case, observations })
}

fn assert_write(observation: &WriteObservation, payload: &[u8]) -> Result<()> {
    ensure!(
        observation.encoding == "utf8",
        "write used unexpected encoding {:?}",
        observation.encoding
    );
    ensure!(
        observation.bytes_written == payload.len() && observation.decoded_bytes == payload.len(),
        "write byte counts {:?} did not equal payload length {}",
        observation,
        payload.len()
    );
    Ok(())
}

fn assert_batch_one_boot(observation: &ReadObservation) -> Result<()> {
    assert_batch_one_exact_matched_read(observation, BOOT_BANNER, "boot banner")
}

fn assert_batch_one_pong(observation: &ReadObservation) -> Result<()> {
    assert_batch_one_exact_matched_read(observation, PONG_RESPONSE, "pong")
}

fn assert_exact_raw_matched_read(
    observation: &ReadObservation,
    expected_payload: &[u8],
    label: &str,
) -> Result<()> {
    ensure!(
        observation.encoding == "utf8"
            && observation.payload == expected_payload
            && observation.stop_reason == "match_found"
            && !observation.truncated
            && observation.matched
            && observation.match_index == Some(0)
            && observation.match_frame_index.is_none()
            && observation.frames.is_none()
            && observation.frames_dropped == 0
            && observation.error.is_none(),
        "{label} read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_batch_one_exact_matched_read(
    observation: &ReadObservation,
    expected_payload: &[u8],
    label: &str,
) -> Result<()> {
    assert_exact_raw_matched_read(observation, expected_payload, label)?;
    ensure!(
        observation.bytes_read == expected_payload.len()
            && observation.bytes_observed == expected_payload.len()
            && observation.bytes_returned == expected_payload.len(),
        "{label} read byte metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn framing_diagnostic_payload() -> Vec<u8> {
    let mut payload = Vec::with_capacity(FRAMING_DIAGNOSTIC_LINE.len() * 2 + PONG_RESPONSE.len());
    payload.extend_from_slice(FRAMING_DIAGNOSTIC_LINE);
    payload.extend_from_slice(FRAMING_DIAGNOSTIC_LINE);
    payload.extend_from_slice(PONG_RESPONSE);
    payload
}

fn trace_diagnostic_payload() -> Vec<u8> {
    let mut payload = Vec::with_capacity(78);
    for (sequence, byte) in b"ping\r\n".iter().enumerate() {
        payload.extend_from_slice(format!("RX[{sequence}]=0x{byte:02x}\r\n").as_bytes());
    }
    payload.extend_from_slice(PONG_RESPONSE);
    payload
}

fn assert_diagnostic_write(observation: &WriteObservation, payload: &[u8]) -> Result<()> {
    assert_write(observation, payload)?;
    ensure!(
        observation.is_error == Some(false) && observation.name.is_none(),
        "diagnostic write metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_anonymous_write(
    observation: &WriteObservation,
    payload: &[u8],
    label: &str,
) -> Result<()> {
    assert_write(observation, payload)?;
    ensure!(
        observation.is_error == Some(false)
            && observation.name.is_none()
            && observation.encoding == "utf8"
            && observation.bytes_written == 6
            && observation.decoded_bytes == 6,
        "{label} write metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_positioned_match_read(
    observation: &ReadObservation,
    expected_payload: &[u8],
    match_index: usize,
    position: ReadPositionObservation,
    label: &str,
) -> Result<()> {
    ensure!(
        observation.is_error == Some(false)
            && observation.name.is_none()
            && observation.encoding == "utf8"
            && observation.payload == expected_payload
            && observation.bytes_read == expected_payload.len()
            && observation.bytes_observed == expected_payload.len()
            && observation.bytes_returned == expected_payload.len()
            && observation.stop_reason == "match_found"
            && !observation.truncated
            && observation.matched
            && observation.match_index == Some(match_index)
            && observation.match_frame_index.is_none()
            && observation.frames.is_none()
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position == Some(position),
        "{label} read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_slip_happy_read(observation: &ReadObservation) -> Result<()> {
    let expected_frame = FrameObservation {
        frame_index: 0,
        frame_type: "slip".to_owned(),
        encoding: "hex".to_owned(),
        payload: b"pong".to_vec(),
        parsed: None,
    };
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "hex"
            && observation.payload == SLIP_HAPPY_WIRE
            && observation.bytes_read == 6
            && observation.bytes_observed == 0
            && observation.bytes_returned == 6
            && observation.stop_reason == "max_frames"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames == Some(vec![expected_frame])
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 52,
                    next_offset: 58,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 58,
                }),
        "SLIP happy read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_slip_malformed_read(observation: &ReadObservation) -> Result<()> {
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "hex"
            && observation.payload == SLIP_MALFORMED_WIRE
            && observation.bytes_read == 4
            && observation.bytes_observed == 0
            && observation.bytes_returned == 0
            && observation.stop_reason == "framing_error"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames.is_none()
            && observation.frames_dropped == 0
            && observation.error.as_deref() == Some("SLIP framing error: invalid escape byte 0x41")
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 52,
                    next_offset: 56,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 56,
                }),
        "SLIP malformed read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_slip_recovery_read(observation: &ReadObservation) -> Result<()> {
    let expected_frame = FrameObservation {
        frame_index: 0,
        frame_type: "slip".to_owned(),
        encoding: "hex".to_owned(),
        payload: b"pong".to_vec(),
        parsed: None,
    };
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "hex"
            && observation.payload == SLIP_HAPPY_WIRE
            && observation.bytes_read == 6
            && observation.bytes_observed == 0
            && observation.bytes_returned == 0
            && observation.stop_reason == "timeout"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames == Some(vec![expected_frame])
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 76,
                    next_offset: 82,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 82,
                }),
        "SLIP recovery read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_cobs_preset_read(observation: &ReadObservation) -> Result<()> {
    let expected_frame = FrameObservation {
        frame_index: 0,
        frame_type: "cobs".to_owned(),
        encoding: "hex".to_owned(),
        payload: b"pong".to_vec(),
        parsed: Some(ParsedFrameObservation::Raw),
    };
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "hex"
            && observation.payload == COBS_PRESET_WIRE
            && observation.bytes_read == 7
            && observation.bytes_observed == 0
            && observation.bytes_returned == 0
            && observation.stop_reason == "timeout"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames == Some(vec![expected_frame])
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 52,
                    next_offset: 59,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 59,
                }),
        "COBS preset read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_at_parser_read(observation: &ReadObservation) -> Result<()> {
    let expected_frame = FrameObservation {
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
    };
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "utf8"
            && observation.payload == b"pong\r\n"
            && observation.bytes_read == 6
            && observation.bytes_observed == 0
            && observation.bytes_returned == 0
            && observation.stop_reason == "timeout"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames == Some(vec![expected_frame])
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 52,
                    next_offset: 58,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 58,
                }),
        "AT parser read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_at_protocol_default_read(observation: &ReadObservation) -> Result<()> {
    let expected_frame = FrameObservation {
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
    };
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "utf8"
            && observation.payload == b"pong\r\n"
            && observation.bytes_read == 6
            && observation.bytes_observed == 0
            && observation.bytes_returned == 0
            && observation.stop_reason == "timeout"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames == Some(vec![expected_frame])
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 52,
                    next_offset: 58,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 58,
                }),
        "AT protocol-default read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_json_parser_read(observation: &ReadObservation) -> Result<()> {
    let expected_frames = vec![
        FrameObservation {
            frame_index: 0,
            frame_type: "line".to_owned(),
            encoding: "utf8".to_owned(),
            payload: b"{\"sensor\":\"temp\",\"value\":25.5,\"unit\":\"C\"}".to_vec(),
            parsed: Some(ParsedFrameObservation::Json {
                fields: BTreeMap::from([
                    ("sensor".to_owned(), json!("temp")),
                    ("unit".to_owned(), json!("C")),
                    ("value".to_owned(), json!(25.5)),
                ]),
            }),
        },
        FrameObservation {
            frame_index: 1,
            frame_type: "line".to_owned(),
            encoding: "utf8".to_owned(),
            payload: b"{\"sensor\":\"humidity\",\"value\":60,\"unit\":\"%\"}".to_vec(),
            parsed: Some(ParsedFrameObservation::Json {
                fields: BTreeMap::from([
                    ("sensor".to_owned(), json!("humidity")),
                    ("unit".to_owned(), json!("%")),
                    ("value".to_owned(), json!(60)),
                ]),
            }),
        },
        FrameObservation {
            frame_index: 2,
            frame_type: "line".to_owned(),
            encoding: "utf8".to_owned(),
            payload: b"{\"sensor\":\"pressure\",\"value\":1013.25,\"unit\":\"hPa\"}".to_vec(),
            parsed: Some(ParsedFrameObservation::Json {
                fields: BTreeMap::from([
                    ("sensor".to_owned(), json!("pressure")),
                    ("unit".to_owned(), json!("hPa")),
                    ("value".to_owned(), json!(1013.25)),
                ]),
            }),
        },
    ];
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "utf8"
            && observation.payload == JSONOUT_RESPONSE
            && observation.bytes_read == 140
            && observation.bytes_observed == 0
            && observation.bytes_returned == 0
            && observation.stop_reason == "timeout"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames == Some(expected_frames)
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 52,
                    next_offset: 192,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 192,
                }),
        "JSON parser read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_ndjson_json_frames_read(observation: &ReadObservation) -> Result<()> {
    assert_exact_ndjson_read(
        observation,
        NDJSON_PRESET_JSON_FRAMES_RESPONSE,
        17,
        vec![
            expected_json_frame(
                0,
                b"{\"a\":1}",
                BTreeMap::from([(String::from("a"), json!(1))]),
            ),
            expected_json_frame(
                1,
                b"{\"b\":2}",
                BTreeMap::from([(String::from("b"), json!(2))]),
            ),
        ],
        ReadPositionObservation {
            from_offset: 52,
            next_offset: 69,
            bytes_lost: 0,
            buffered_remaining: 0,
            start_offset: 0,
            end_offset: 69,
        },
    )
}

fn assert_exact_ndjson_skips_empty_lines_read(observation: &ReadObservation) -> Result<()> {
    assert_exact_ndjson_read(
        observation,
        NDJSON_PRESET_SKIPS_EMPTY_LINES_RESPONSE,
        30,
        vec![
            expected_json_frame(
                0,
                b"{\"a\":1}",
                BTreeMap::from([(String::from("a"), json!(1))]),
            ),
            expected_json_frame(
                1,
                b"{\"b\":2}",
                BTreeMap::from([(String::from("b"), json!(2))]),
            ),
            expected_json_frame(
                2,
                b"{\"c\":3}",
                BTreeMap::from([(String::from("c"), json!(3))]),
            ),
        ],
        ReadPositionObservation {
            from_offset: 52,
            next_offset: 82,
            bytes_lost: 0,
            buffered_remaining: 0,
            start_offset: 0,
            end_offset: 82,
        },
    )
}

fn expected_json_frame(
    frame_index: usize,
    payload: &[u8],
    fields: BTreeMap<String, Value>,
) -> FrameObservation {
    FrameObservation {
        frame_index,
        frame_type: "line".to_owned(),
        encoding: "utf8".to_owned(),
        payload: payload.to_vec(),
        parsed: Some(ParsedFrameObservation::Json { fields }),
    }
}

fn assert_exact_ndjson_read(
    observation: &ReadObservation,
    payload: &[u8],
    bytes_read: usize,
    frames: Vec<FrameObservation>,
    position: ReadPositionObservation,
) -> Result<()> {
    ensure!(
        observation.is_error == Some(false)
            && observation.connection_id == LOGICAL_CONNECTION
            && observation.name.is_none()
            && observation.encoding == "utf8"
            && observation.payload == payload
            && observation.bytes_read == bytes_read
            && observation.bytes_observed == 0
            && observation.bytes_returned == 0
            && observation.stop_reason == "timeout"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames == Some(frames)
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position == Some(position),
        "NDJSON preset read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_ack_step(
    step: &AckStep,
    write: &WriteObservation,
    read: &ReadObservation,
) -> Result<()> {
    ensure!(
        write.is_error == Some(false)
            && write.name.is_none()
            && write.encoding == "utf8"
            && write.bytes_written == step.write_bytes
            && write.decoded_bytes == step.write_bytes,
        "ACK step write metadata mismatch for command {:?}: {write:?}",
        step.command
    );
    ensure!(
        read.is_error == Some(false)
            && read.name.is_none()
            && read.encoding == "utf8"
            && read.payload == step.payload
            && read.bytes_read == step.payload.len()
            && read.bytes_observed == step.payload.len()
            && read.bytes_returned == step.payload.len()
            && read.stop_reason == "match_found"
            && !read.truncated
            && read.matched
            && read.match_index == Some(step.match_index)
            && read.match_frame_index.is_none()
            && read.frames.is_none()
            && read.frames_dropped == 0
            && read.error.is_none()
            && read.position == Some(step.position.clone())
            && read
                .payload
                .windows(step.marker.len())
                .any(|window| window == step.marker.as_bytes()),
        "ACK step read metadata mismatch for marker {:?}: {read:?}",
        step.marker
    );
    Ok(())
}

fn assert_exact_spam_match_read(
    observation: &ReadObservation,
    expected_payload: &[u8],
) -> Result<()> {
    ensure!(
        expected_payload.len() == 1088,
        "spam 1024 source-derived stream had {} bytes, expected 1088",
        expected_payload.len()
    );
    ensure!(
        observation.is_error != Some(true)
            && observation.encoding == "utf8"
            && observation.payload == expected_payload
            && observation.bytes_read == 1088
            && observation.bytes_observed == 1088
            && observation.bytes_returned == 1088
            && observation.stop_reason == "match_found"
            && !observation.truncated
            && observation.matched
            && observation.match_index == Some(1056)
            && observation.match_frame_index.is_none()
            && observation.frames.is_none()
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 32,
                    next_offset: 1120,
                    bytes_lost: 0,
                    buffered_remaining: 0,
                    start_offset: 0,
                    end_offset: 1120,
                }),
        "spam matcher read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_exact_flood_buffer_read(
    observation: &ReadObservation,
    expected_payload: &[u8],
) -> Result<()> {
    ensure!(
        expected_payload.len() == 256,
        "spam 512 bounded prefix had {} bytes, expected 256",
        expected_payload.len()
    );
    ensure!(
        observation.is_error != Some(true)
            && observation.encoding == "utf8"
            && observation.payload == expected_payload
            && observation.bytes_read == 256
            && observation.bytes_observed == 256
            && observation.bytes_returned == 256
            && observation.stop_reason == "max_buffered_bytes"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames.is_none()
            && observation.frames_dropped == 0
            && observation.error.is_none()
            && observation.position
                == Some(ReadPositionObservation {
                    from_offset: 32,
                    next_offset: 288,
                    bytes_lost: 0,
                    buffered_remaining: 31,
                    start_offset: 0,
                    end_offset: 319,
                }),
        "spam buffer-budget read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn expected_line_frame(frame_index: usize, payload: &[u8]) -> FrameObservation {
    FrameObservation {
        frame_index,
        frame_type: "line".to_owned(),
        encoding: "utf8".to_owned(),
        payload: payload.to_vec(),
        parsed: None,
    }
}

fn expected_frame(
    frame_index: usize,
    frame_type: &str,
    encoding: &str,
    payload: &[u8],
) -> FrameObservation {
    FrameObservation {
        frame_index,
        frame_type: frame_type.to_owned(),
        encoding: encoding.to_owned(),
        payload: payload.to_vec(),
        parsed: None,
    }
}

fn assert_exact_raw_framed_read(
    observation: &ReadObservation,
    encoding: &str,
    wire: &[u8],
    expected_frames: Vec<FrameObservation>,
) -> Result<()> {
    ensure!(
        observation.encoding == encoding
            && observation.payload == wire
            && observation.stop_reason == "max_frames"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames.as_ref() == Some(&expected_frames)
            && observation.frames_dropped == 0
            && observation.error.is_none(),
        "raw framed read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_tx_write(observation: &WriteObservation, expected: &[u8]) -> Result<()> {
    ensure!(
        observation.encoding == "utf8"
            && observation.decoded_bytes == 4
            && observation.bytes_written == expected.len(),
        "TX framing write metadata mismatch: {observation:?}, expected {expected:02x?}"
    );
    Ok(())
}

fn assert_exact_trace_read(
    observation: &ReadObservation,
    expected_wire: &[u8],
    trace_sequence: u8,
) -> Result<()> {
    let mut expected_trace = String::new();
    for (index, byte) in expected_wire.iter().enumerate() {
        let sequence = trace_sequence
            .wrapping_add(u8::try_from(index).context("trace sequence index exceeded u8")?);
        expected_trace.push_str(&format!("RX[{sequence}]=0x{byte:02x}\r\n"));
    }
    ensure!(
        observation.encoding == "utf8"
            && observation.payload == expected_trace.as_bytes()
            && observation.stop_reason == "no_new_rx_timeout"
            && !observation.truncated
            && !observation.matched
            && observation.match_index.is_none()
            && observation.match_frame_index.is_none()
            && observation.frames.is_none()
            && observation.frames_dropped == 0
            && observation.error.is_none(),
        "native trace read metadata mismatch: {observation:?}"
    );
    Ok(())
}

/// Decode and validate native's exact byte-trace text.
pub fn decode_trace_bytes(data: &str, expected: &[u8], first_sequence: u8) -> Result<Vec<u8>> {
    let mut decoded = Vec::with_capacity(expected.len());
    for (index, record) in data.split_inclusive("\r\n").enumerate() {
        ensure!(
            record.ends_with("\r\n"),
            "trace ended with incomplete record at index {index}"
        );
        let line = &record[..record.len() - 2];
        ensure!(
            line.starts_with("RX["),
            "trace record {index} had malformed prefix: {line:?}"
        );
        let (sequence_text, byte_text) = line
            .strip_prefix("RX[")
            .and_then(|rest| rest.split_once("]=0x"))
            .context("trace record had malformed sequence/byte separator")?;
        ensure!(
            !sequence_text.is_empty()
                && (sequence_text == "0" || !sequence_text.starts_with('0'))
                && sequence_text.bytes().all(|byte| byte.is_ascii_digit()),
            "trace record {index} had malformed sequence {sequence_text:?}"
        );
        let sequence = sequence_text
            .parse::<u8>()
            .with_context(|| format!("trace record {index} sequence was out of range"))?;
        ensure!(
            byte_text.len() == 2
                && byte_text.bytes().all(|byte| byte.is_ascii_hexdigit())
                && byte_text.bytes().all(|byte| !byte.is_ascii_uppercase()),
            "trace record {index} had malformed byte {byte_text:?}"
        );
        let byte = u8::from_str_radix(byte_text, 16)
            .with_context(|| format!("trace record {index} byte was not hexadecimal"))?;
        let expected_sequence = first_sequence
            .wrapping_add(u8::try_from(index).context("trace record sequence index exceeded u8")?);
        ensure!(
            sequence == expected_sequence,
            "trace sequence at index {index} was {sequence}, expected {expected_sequence}"
        );
        let expected_byte = expected
            .get(index)
            .with_context(|| format!("trace contained extra record at index {index}"))?;
        ensure!(
            byte == *expected_byte,
            "trace byte at index {index} was 0x{byte:02x}, expected 0x{expected_byte:02x}"
        );
        decoded.push(byte);
    }
    ensure!(
        decoded.len() == expected.len(),
        "trace contained {} records, expected {}",
        decoded.len(),
        expected.len()
    );
    Ok(decoded)
}

async fn collect_fixture_wire(
    fixture: &mut crate::common::device_fixture::DeviceFixture,
    expected_len: usize,
) -> Result<Vec<u8>> {
    let mut actual = Vec::with_capacity(expected_len);
    while actual.len() < expected_len {
        actual.extend(fixture.next_raw_input(CLEANUP_TIMEOUT).await?);
    }
    ensure!(
        actual.len() == expected_len,
        "fixture peer wire contained {} bytes, expected {expected_len}",
        actual.len()
    );
    Ok(actual)
}

fn assert_exact_line_frames(
    observation: &ReadObservation,
    stop_reason: &str,
    expected_frames: Vec<FrameObservation>,
    matched: bool,
) -> Result<()> {
    ensure!(
        observation.encoding == "utf8"
            && observation.stop_reason == stop_reason
            && !observation.truncated
            && observation.matched == matched
            && observation.frames.as_ref() == Some(&expected_frames)
            && observation.frames_dropped == 0
            && observation.error.is_none(),
        "framed line read metadata mismatch: {observation:?}"
    );
    Ok(())
}

fn assert_ping_status_delta(delta: &StatusDeltaObservation) -> Result<()> {
    ensure!(
        delta.tx_bytes == PONG_RESPONSE.len() as u64
            && delta.rx_bytes == PONG_RESPONSE.len() as u64
            && delta.read_ops == 1
            && delta.write_ops == 1
            && delta.truncation_count == 0,
        "ping status counters were not exact/coherent: {delta:?}"
    );
    ensure!(
        matches!(delta.state, super::model::NormalizedConnectionState::Open),
        "ping status state was not open: {:?}",
        delta.state
    );
    assert_serial_settings(&delta.serial, 115_200, "none")
}

fn assert_serial_settings(
    settings: &super::model::SerialSettingsObservation,
    baud_rate: u32,
    flow_control: &str,
) -> Result<()> {
    ensure!(
        settings.baud_rate == baud_rate
            && settings.data_bits == "8"
            && settings.stop_bits == "1"
            && settings.parity == "none"
            && settings.flow_control == flow_control,
        "unexpected serial settings: {settings:?}"
    );
    Ok(())
}

struct PublicSession {
    server: Option<TestServer>,
    client: Option<ModernClient>,
    connection_id: Option<String>,
    normalization: Option<NormalizationContext>,
    raw_observations: StdMutex<Vec<RawToolObservation>>,
}

#[derive(Debug, Clone, Serialize)]
struct RawToolObservation {
    tool: &'static str,
    is_error: Option<bool>,
    structured_content: Option<Value>,
}

#[derive(Debug, Clone, Copy)]
enum ReadNormalization {
    BatchOneRaw,
    General,
    Positioned,
}

impl PublicSession {
    async fn start(
        endpoint: &DifferentialEndpoint,
        options: OpenOptions,
    ) -> Result<(Self, OpenObservation)> {
        let server = TestServer::start().await;
        let client = match tokio::time::timeout(TOOL_TIMEOUT, connect_2026_07_28_client(&server))
            .await
        {
            Ok(Ok((client, _))) => client,
            Ok(Err(error)) => {
                let cleanup = shutdown_server(server).await;
                return match cleanup {
                    Ok(()) => Err(error.context("connect modern differential MCP client")),
                    Err(cleanup_error) => Err(error.context(format!(
                        "connect modern differential MCP client; server cleanup also failed: {cleanup_error}"
                    ))),
                };
            }
            Err(_) => {
                let cleanup = shutdown_server(server).await;
                match cleanup {
                    Ok(()) => anyhow::bail!("timed out connecting modern differential MCP client"),
                    Err(cleanup_error) => anyhow::bail!(
                        "timed out connecting modern differential MCP client; server cleanup also failed: {cleanup_error}"
                    ),
                };
            }
        };
        let mut session = Self {
            server: Some(server),
            client: Some(client),
            connection_id: None,
            normalization: None,
            raw_observations: StdMutex::new(Vec::new()),
        };
        let started = async {
            let endpoint_path = endpoint.port_path()?;
            let mut args = json!({
                "port": endpoint_path,
                "profile_mode": "none",
                "baud_rate": options.baud_rate,
            });
            let args_map = args
                .as_object_mut()
                .context("open arguments must be a JSON object")?;
            if let Some(name) = options.name {
                args_map.insert("name".to_owned(), Value::String(name.to_owned()));
            }
            if let Some(flow_control) = options.flow_control {
                args_map.insert(
                    "flow_control".to_owned(),
                    Value::String(flow_control.to_owned()),
                );
            }
            if options.framing == OpenFraming::AtCommandWithExplicitLine {
                args_map.insert("protocol".to_owned(), json!({ "type": "at_command" }));
                args_map.insert(
                    "rx_framing".to_owned(),
                    json!({ "type": "line", "ending": "lf", "max_frames": 1 }),
                );
            } else if options.framing == OpenFraming::AtCommandDefaults {
                args_map.insert("protocol".to_owned(), json!({ "type": "at_command" }));
            }
            let result = session.call("open", args).await?;
            ensure_success(&result, "open")?;
            let structured = structured(&result, "open")?;
            if let Some(connection_id) = structured.get("connection_id").and_then(Value::as_str) {
                if !connection_id.is_empty() {
                    session.connection_id = Some(connection_id.to_owned());
                }
            }
            let (open, normalization) =
                normalize_open(structured, &endpoint_path, result.is_error)?;
            session.connection_id = Some(normalization.actual_connection_id().to_owned());
            session.normalization = Some(normalization);
            Ok::<OpenObservation, anyhow::Error>(open)
        }
        .await;
        match started {
            Ok(open) => Ok((session, open)),
            Err(error) => {
                let cleanup_errors = session.shutdown().await;
                if cleanup_errors.is_empty() {
                    Err(error)
                } else {
                    let cleanup_message = cleanup_errors
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join("; ");
                    Err(error.context(format!(
                        "differential session cleanup also failed: {cleanup_message}"
                    )))
                }
            }
        }
    }

    async fn call(&self, name: &'static str, args: Value) -> Result<CallToolResult> {
        let result = call_with_client(self.primary_client()?, name, args).await?;
        self.record_raw(name, &result);
        Ok(result)
    }

    fn record_raw(&self, tool: &'static str, result: &CallToolResult) {
        self.raw_observations
            .lock()
            .expect("differential raw observation mutex poisoned")
            .push(RawToolObservation {
                tool,
                is_error: result.is_error,
                structured_content: result.structured_content.clone(),
            });
    }

    fn raw_observations(&self) -> Vec<RawToolObservation> {
        self.raw_observations
            .lock()
            .expect("differential raw observation mutex poisoned")
            .clone()
    }

    fn primary_client(&self) -> Result<&ModernClient> {
        self.client
            .as_ref()
            .context("differential primary client was already shut down")
    }

    async fn connect_secondary_client(&self) -> Result<ModernClient> {
        let server = self
            .server
            .as_ref()
            .context("differential MCP server was already shut down")?;
        match tokio::time::timeout(TOOL_TIMEOUT, connect_2026_07_28_client(server)).await {
            Ok(Ok((client, _))) => Ok(client),
            Ok(Err(error)) => {
                Err(error.context("connect secondary modern differential MCP client"))
            }
            Err(_) => anyhow::bail!(
                "timed out connecting secondary modern differential MCP client after {} ms",
                TOOL_TIMEOUT.as_millis()
            ),
        }
    }

    fn normalization(&self) -> Result<&NormalizationContext> {
        self.normalization
            .as_ref()
            .context("differential session was not opened")
    }

    fn connection_id(&self) -> Result<&str> {
        self.connection_id
            .as_deref()
            .context("differential session has no open connection ID")
    }

    async fn sync_boot(&self, case: DifferentialCase) -> Result<ReadObservation> {
        if case == DifferentialCase::ExplicitRxFramingBeatsConnectionDefault {
            let observation = self
                .read_general(json!({
                    "connection_id": self.connection_id()?,
                    "timeout_ms": READ_TIMEOUT_MS,
                    "encoding": "utf8"
                }))
                .await?;
            let expected = FrameObservation {
                frame_index: 0,
                frame_type: "line".to_owned(),
                encoding: "utf8".to_owned(),
                payload: b"serial-mcp test firmware ready\r".to_vec(),
                parsed: Some(ParsedFrameObservation::AtCommand {
                    response_type: "data".to_owned(),
                    command: None,
                    status: None,
                    fields: vec!["serial-mcp test firmware ready".to_owned()],
                }),
            };
            assert_exact_line_frames(&observation, "max_frames", vec![expected], false)?;
            return Ok(observation);
        }

        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "timeout_ms": READ_TIMEOUT_MS,
                    "encoding": "utf8",
                    "match": {
                        "pattern": String::from_utf8_lossy(BOOT_BANNER),
                        "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                    }
                }),
            )
            .await?;
        ensure_success(&result, "boot synchronization read")?;
        let observation = match case.batch() {
            DifferentialBatch::CommandLifecycle => normalize_batch_one_raw_read(
                structured(&result, "boot synchronization read")?,
                self.normalization()?,
                result.is_error,
            )?,
            DifferentialBatch::GenericMatchingFraming
            | DifferentialBatch::RawGenericFraming
            | DifferentialBatch::FloodBuffer
            | DifferentialBatch::CommandDiagnostics
            | DifferentialBatch::AckState
            | DifferentialBatch::OutputFlush
            | DifferentialBatch::SlipHappy
            | DifferentialBatch::SlipMalformed
            | DifferentialBatch::SlipRecovery
            | DifferentialBatch::CobsPreset
            | DifferentialBatch::AtParser
            | DifferentialBatch::AtProtocolDefault
            | DifferentialBatch::JsonParser
            | DifferentialBatch::NdjsonPreset => normalize_read(
                structured(&result, "boot synchronization read")?,
                self.normalization()?,
                result.is_error,
            )?,
        };
        match case.batch() {
            DifferentialBatch::CommandLifecycle => assert_batch_one_boot(&observation)?,
            DifferentialBatch::GenericMatchingFraming
            | DifferentialBatch::RawGenericFraming
            | DifferentialBatch::FloodBuffer
            | DifferentialBatch::CommandDiagnostics
            | DifferentialBatch::AckState
            | DifferentialBatch::OutputFlush
            | DifferentialBatch::SlipHappy
            | DifferentialBatch::SlipMalformed
            | DifferentialBatch::SlipRecovery
            | DifferentialBatch::CobsPreset
            | DifferentialBatch::AtParser
            | DifferentialBatch::AtProtocolDefault
            | DifferentialBatch::JsonParser
            | DifferentialBatch::NdjsonPreset => {
                assert_exact_raw_matched_read(&observation, BOOT_BANNER, "boot banner")?
            }
        }
        Ok(observation)
    }

    async fn write(&self, payload: &[u8]) -> Result<WriteObservation> {
        self.write_with_client(self.primary_client()?, payload)
            .await
    }

    async fn write_with_client(
        &self,
        client: &ModernClient,
        payload: &[u8],
    ) -> Result<WriteObservation> {
        self.write_with_client_framing(client, payload, None).await
    }

    async fn write_with_client_framing(
        &self,
        client: &ModernClient,
        payload: &[u8],
        tx_framing: Option<Value>,
    ) -> Result<WriteObservation> {
        let payload =
            std::str::from_utf8(payload).context("differential write payload was not UTF-8")?;
        let mut args = json!({
            "connection_id": self.connection_id()?,
            "data": payload,
        });
        if let Some(tx_framing) = tx_framing {
            args.as_object_mut()
                .context("differential write arguments must be an object")?
                .insert("tx_framing".to_owned(), tx_framing);
        }
        let result = call_with_client(client, "write", args).await?;
        self.record_raw("write", &result);
        ensure_success(&result, "write")?;
        normalize_write(
            structured(&result, "write")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn arm_and_write_delayed(
        &self,
        command: &'static str,
        expected_write_bytes: usize,
    ) -> Result<()> {
        let arm = self
            .call(
                "transact",
                json!({
                    "connection_id": self.connection_id()?,
                    "data": "arm_cmd 1000\r\n",
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "match": {
                        "pattern": "arm_cmd delay=1000\r\n",
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8"
                        }
                    }
                }),
            )
            .await?;
        ensure_success(&arm, "delayed command arm")?;
        let arm_read = structured(&arm, "delayed command arm")?
            .get("read")
            .context("delayed command arm transact result omitted read half")?;
        ensure!(
            arm_read.get("encoding").and_then(Value::as_str) == Some("utf8")
                && arm_read.get("data").and_then(Value::as_str) == Some("arm_cmd delay=1000\r\n")
                && arm_read.get("bytes_read").and_then(Value::as_u64) == Some(20)
                && arm_read.get("bytes_observed").and_then(Value::as_u64) == Some(20)
                && arm_read.get("bytes_returned").and_then(Value::as_u64) == Some(20)
                && arm_read.get("stop_reason").and_then(Value::as_str) == Some("match_found")
                && arm_read.get("matched").and_then(Value::as_bool) == Some(true)
                && arm_read.get("match_index").and_then(Value::as_u64) == Some(0)
                && arm_read.get("frames_dropped").and_then(Value::as_u64) == Some(0)
                && arm_read.get("frames").is_none_or(|frames| frames.is_null())
                && arm_read.get("error").is_none_or(|error| error.is_null()),
            "delayed command arm did not produce exact acknowledgement: {arm:?}"
        );

        let write = self
            .call(
                "write",
                json!({
                    "connection_id": self.connection_id()?,
                    "data": command,
                    "encoding": "utf8"
                }),
            )
            .await?;
        ensure_success(&write, "delayed command write")?;
        let observation = normalize_write(
            structured(&write, "delayed command write")?,
            self.normalization()?,
            write.is_error,
        )?;
        ensure!(
            observation.is_error == Some(false)
                && observation.name.is_none()
                && observation.encoding == "utf8"
                && observation.bytes_written == expected_write_bytes
                && observation.decoded_bytes == expected_write_bytes,
            "delayed command write metadata mismatch: {observation:?}"
        );
        Ok(())
    }

    async fn slip_happy_setup(&self) -> Result<()> {
        self.arm_and_write_delayed(SLIP_HAPPY_SENDRAW_COMMAND, 26)
            .await
    }

    async fn slip_malformed_setup(&self) -> Result<()> {
        self.arm_and_write_delayed(SLIP_MALFORMED_SENDRAW_COMMAND, 22)
            .await
    }

    async fn cobs_preset_setup(&self) -> Result<()> {
        self.arm_and_write_delayed(COBS_PRESET_SENDRAW_COMMAND, 28)
            .await
    }

    async fn at_parser_setup(&self) -> Result<()> {
        self.arm_and_write_delayed("ping\r\n", 6).await
    }

    async fn json_parser_setup(&self) -> Result<()> {
        self.arm_and_write_delayed("jsonout\r\n", 9).await
    }

    async fn ndjson_preset_json_frames_setup(&self) -> Result<()> {
        self.arm_and_write_delayed(NDJSON_PRESET_JSON_FRAMES_SENDRAW_COMMAND, 48)
            .await
    }

    async fn ndjson_preset_skips_empty_lines_setup(&self) -> Result<()> {
        self.arm_and_write_delayed(NDJSON_PRESET_SKIPS_EMPTY_LINES_SENDRAW_COMMAND, 74)
            .await
    }

    async fn at_protocol_default_setup(&self) -> Result<()> {
        let arm = self
            .call(
                "transact",
                json!({
                    "connection_id": self.connection_id()?,
                    "data": "arm_cmd 1000",
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "match": {
                        "pattern": "arm_cmd delay=1000",
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8"
                        }
                    }
                }),
            )
            .await?;
        ensure_success(&arm, "AT protocol-default readiness command")?;
        let structured = structured(&arm, "AT protocol-default readiness command")?;
        let write = structured
            .get("write")
            .context("AT protocol-default readiness transact result omitted write half")?;
        ensure!(
            write.get("decoded_bytes").and_then(Value::as_u64) == Some(12)
                && write.get("bytes_written").and_then(Value::as_u64) == Some(13)
                && write.get("encoding").and_then(Value::as_str) == Some("utf8"),
            "AT protocol-default readiness write metadata mismatch: {arm:?}"
        );
        let read = structured
            .get("read")
            .context("AT protocol-default readiness transact result omitted read half")?;
        ensure!(
            read.get("data").and_then(Value::as_str) == Some("arm_cmd delay=1000")
                && read.get("matched").and_then(Value::as_bool) == Some(true)
                && read.get("stop_reason").and_then(Value::as_str) == Some("match_found")
                && read.get("bytes_read").and_then(Value::as_u64) == Some(18)
                && read.get("bytes_observed").and_then(Value::as_u64) == Some(0)
                && read.get("bytes_returned").and_then(Value::as_u64) == Some(20)
                && read.get("match_index").and_then(Value::as_u64) == Some(0)
                && read.get("match_frame_index").and_then(Value::as_u64) == Some(0)
                && read.get("frames_dropped").and_then(Value::as_u64) == Some(0),
            "AT protocol-default readiness read metadata mismatch: {arm:?}"
        );

        let write = self.write(b"ping").await?;
        ensure!(
            write.is_error == Some(false)
                && write.name.is_none()
                && write.encoding == "utf8"
                && write.decoded_bytes == 4
                && write.bytes_written == 5,
            "AT protocol-default ping write metadata mismatch: {write:?}"
        );
        Ok(())
    }

    async fn read_slip_happy(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "timeout_ms": READ_TIMEOUT_MS,
                    "encoding": "hex",
                    "rx_framing": { "type": "slip", "max_frames": 1 }
                }),
            )
            .await?;
        ensure_success(&result, "SLIP happy read")?;
        normalize_positioned_read(
            structured(&result, "SLIP happy read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_slip_malformed(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "timeout_ms": READ_TIMEOUT_MS,
                    "encoding": "utf8",
                    "rx_framing": { "type": "slip" }
                }),
            )
            .await?;
        ensure_success(&result, "SLIP malformed read")?;
        normalize_positioned_read(
            structured(&result, "SLIP malformed read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_slip_recovery(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "encoding": "hex",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "rx_framing": { "type": "slip" }
                }),
            )
            .await?;
        ensure_success(&result, "SLIP recovery read")?;
        normalize_positioned_read(
            structured(&result, "SLIP recovery read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_cobs_preset(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "encoding": "hex",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "protocol": { "type": "cobs" }
                }),
            )
            .await?;
        ensure_success(&result, "COBS preset read")?;
        normalize_positioned_read(
            structured(&result, "COBS preset read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_at_parser(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "rx_framing": { "type": "line" },
                    "rx_parser": { "type": "at_command" }
                }),
            )
            .await?;
        ensure_success(&result, "AT parser read")?;
        normalize_positioned_read(
            structured(&result, "AT parser read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_json_parser(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "rx_framing": { "type": "line" },
                    "rx_parser": { "type": "json_lines" }
                }),
            )
            .await?;
        ensure_success(&result, "JSON parser read")?;
        normalize_positioned_read(
            structured(&result, "JSON parser read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_ndjson_preset(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "protocol": { "type": "ndjson" }
                }),
            )
            .await?;
        ensure_success(&result, "NDJSON preset read")?;
        normalize_positioned_read(
            structured(&result, "NDJSON preset read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_at_protocol_default(&self) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "from": { "type": "now" },
                    "timeout_ms": READ_TIMEOUT_MS,
                    "encoding": "utf8"
                }),
            )
            .await?;
        ensure_success(&result, "AT protocol-default read")?;
        normalize_positioned_read(
            structured(&result, "AT protocol-default read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn arm_delayed_sendraw(&self, wire: &[u8]) -> Result<()> {
        let arm = self
            .call(
                "transact",
                json!({
                    "connection_id": self.connection_id()?,
                    "data": "arm_cmd 1000\r\n",
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "match": {
                        "pattern": "arm_cmd delay=1000\r\n",
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8"
                        }
                    }
                }),
            )
            .await?;
        ensure_success(&arm, "arm_cmd")?;
        let arm_read = structured(&arm, "arm_cmd")?
            .get("read")
            .context("arm_cmd transact result omitted read half")?;
        ensure!(
            arm_read.get("matched").and_then(Value::as_bool) == Some(true)
                && arm_read.get("stop_reason").and_then(Value::as_str) == Some("match_found"),
            "arm_cmd did not produce exact acknowledgement: {arm:?}"
        );

        let command = format!("sendraw hex {}\r\n", compact_hex(wire));
        let write = self
            .call(
                "write",
                json!({
                    "connection_id": self.connection_id()?,
                    "data": command,
                    "encoding": "utf8"
                }),
            )
            .await?;
        ensure_success(&write, "sendraw write")
    }

    async fn trace_setup(&self) -> Result<()> {
        let result = self
            .call(
                "transact",
                json!({
                    "connection_id": self.connection_id()?,
                    "data": "trace on\r\n",
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "match": {
                        "pattern": "trace on\r\n",
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8"
                        }
                    }
                }),
            )
            .await?;
        ensure_success(&result, "trace on")?;
        let read = structured(&result, "trace on")?
            .get("read")
            .context("trace on transact result omitted read half")?;
        ensure!(
            read.get("matched").and_then(Value::as_bool) == Some(true)
                && read.get("stop_reason").and_then(Value::as_str) == Some("match_found"),
            "trace on did not produce exact acknowledgement: {result:?}"
        );
        Ok(())
    }

    async fn framing_setup(&self) -> Result<()> {
        let result = self
            .call(
                "transact",
                json!({
                    "connection_id": self.connection_id()?,
                    "data": "framing on\r\n",
                    "encoding": "utf8",
                    "timeout_ms": READ_TIMEOUT_MS,
                    "match": {
                        "pattern": "framing on\r\n",
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8"
                        }
                    }
                }),
            )
            .await?;
        ensure_success(&result, "framing on")?;
        let read = structured(&result, "framing on")?
            .get("read")
            .context("framing on transact result omitted read result")?;
        ensure!(
            read.get("matched").and_then(Value::as_bool) == Some(true)
                && read.get("stop_reason").and_then(Value::as_str) == Some("match_found"),
            "framing on did not produce exact acknowledgement: {result:?}"
        );
        Ok(())
    }

    async fn read_raw_rx(
        &self,
        endpoint: &mut DifferentialEndpoint,
        wire: &[u8],
        encoding: &'static str,
        framing: Value,
    ) -> Result<ReadObservation> {
        match endpoint {
            DifferentialEndpoint::Native(_) => self.arm_delayed_sendraw(wire).await?,
            DifferentialEndpoint::Fixture(fixture) => {
                let chunks = wire.iter().map(|byte| vec![*byte]).collect();
                fixture
                    .run_script(vec![
                        Action::Delay(RAW_STIMULUS_DELAY),
                        Action::EmitChunks(chunks),
                    ])
                    .await?;
            }
        }
        let mut args = json!({
            "connection_id": self.connection_id()?,
            "from": { "type": "now" },
            "timeout_ms": READ_TIMEOUT_MS,
            "encoding": encoding,
        });
        args.as_object_mut()
            .context("raw RX read arguments must be an object")?
            .insert("rx_framing".to_owned(), framing);
        self.read_general(args).await
    }

    async fn observe_tx_mode(
        &self,
        endpoint: &mut DifferentialEndpoint,
        secondary: &ModernClient,
        mode: PeerWireMode,
        framing: Value,
        expected: &[u8],
        trace_sequence: u8,
    ) -> Result<(WriteObservation, PeerWireObservation)> {
        match endpoint {
            DifferentialEndpoint::Native(_) => {
                self.observe_native_tx_mode(secondary, mode, framing, expected, trace_sequence)
                    .await
            }
            DifferentialEndpoint::Fixture(fixture) => {
                let write = self
                    .write_with_client_framing(secondary, b"ping", Some(framing))
                    .await?;
                assert_tx_write(&write, expected)?;
                let actual = collect_fixture_wire(fixture, expected.len()).await?;
                ensure!(
                    actual == expected,
                    "fixture host-to-peer TX bytes differed: actual={actual:02x?}, expected={expected:02x?}"
                );
                Ok((
                    write,
                    PeerWireObservation {
                        direction: PeerWireDirection::HostToPeer,
                        mode,
                        bytes: actual,
                    },
                ))
            }
        }
    }

    async fn observe_native_tx_mode(
        &self,
        secondary: &ModernClient,
        mode: PeerWireMode,
        framing: Value,
        expected: &[u8],
        trace_sequence: u8,
    ) -> Result<(WriteObservation, PeerWireObservation)> {
        let mut pending = Some(self.start_pending_read(json!({
            "connection_id": self.connection_id()?,
            "from": { "type": "now" },
            "timeout_ms": READ_TIMEOUT_MS,
            "no_new_rx_timeout_ms": NO_NEW_RX_TIMEOUT_MS,
            "encoding": "utf8"
        }))?);
        let operation = async {
            tokio::time::sleep(INHERITED_BASELINE_DELAY).await;
            ensure!(
                !pending
                    .as_ref()
                    .context("trace read task was unexpectedly unavailable")?
                    .is_finished(),
                "trace read completed during 100 ms admission delay"
            );
            let write = self
                .write_with_client_framing(secondary, b"ping", Some(framing))
                .await?;
            assert_tx_write(&write, expected)?;
            let task = pending
                .take()
                .context("trace read task was unexpectedly unavailable")?;
            let read = self
                .await_pending_read(task, ReadNormalization::General)
                .await?;
            assert_exact_trace_read(&read, expected, trace_sequence)?;
            let trace = std::str::from_utf8(&read.payload)
                .context("native trace read returned non-UTF-8 payload")?;
            let bytes = decode_trace_bytes(trace, expected, trace_sequence)?;
            Ok::<(WriteObservation, PeerWireObservation), anyhow::Error>((
                write,
                PeerWireObservation {
                    direction: PeerWireDirection::HostToPeer,
                    mode,
                    bytes,
                },
            ))
        }
        .await;
        let pending_cleanup = match pending.take() {
            Some(task) => abort_and_join_pending_read(task).await,
            None => Ok(()),
        };
        combine_operation_and_cleanup(operation, pending_cleanup, "trace read task cleanup")
    }

    async fn read_pong(&self, from: Option<Value>) -> Result<ReadObservation> {
        self.read_literal(PONG_RESPONSE, from).await
    }

    async fn read_literal(&self, pattern: &[u8], from: Option<Value>) -> Result<ReadObservation> {
        let pattern =
            std::str::from_utf8(pattern).context("batch-1 match pattern was not UTF-8")?;
        let mut args = json!({
            "connection_id": self.connection_id()?,
            "timeout_ms": READ_TIMEOUT_MS,
            "encoding": "utf8",
            "match": {
                "pattern": pattern,
                "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
            }
        });
        if let Some(from) = from {
            args.as_object_mut()
                .context("read arguments must be a JSON object")?
                .insert("from".to_owned(), from);
        }
        let result = self.call("read", args).await?;
        ensure_success(&result, "read")?;
        normalize_batch_one_raw_read(
            structured(&result, "read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_general(&self, args: Value) -> Result<ReadObservation> {
        let result = self.call("read", args).await?;
        ensure_success(&result, "read")?;
        normalize_read(
            structured(&result, "read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn read_positioned_literal(&self, marker: &str) -> Result<ReadObservation> {
        let result = self
            .call(
                "read",
                json!({
                    "connection_id": self.connection_id()?,
                    "timeout_ms": READ_TIMEOUT_MS,
                    "encoding": "utf8",
                    "match": {
                        "pattern": marker,
                        "config": {
                            "mode": "literal_substring",
                            "pattern_encoding": "utf8"
                        }
                    }
                }),
            )
            .await?;
        ensure_success(&result, "positioned literal read")?;
        normalize_positioned_read(
            structured(&result, "positioned literal read")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn flush_output(&self) -> Result<FlushObservation> {
        let result = self
            .call(
                "flush",
                json!({
                    "connection_id": self.connection_id()?,
                    "target": "output"
                }),
            )
            .await?;
        ensure_success(&result, "flush output")?;
        normalize_flush(
            structured(&result, "flush output")?,
            self.normalization()?,
            result.is_error,
        )
    }

    fn start_pending_read(
        &self,
        args: Value,
    ) -> Result<tokio::task::JoinHandle<Result<CallToolResult>>> {
        let client = self
            .client
            .as_ref()
            .context("differential client was already shut down")?;
        let peer = client.peer().clone();
        Ok(tokio::spawn(async move {
            let call = peer.call_tool(tool_request("read", args));
            match tokio::time::timeout(TOOL_TIMEOUT, call).await {
                Ok(result) => result.context("pending read public MCP tool call failed"),
                Err(_) => anyhow::bail!(
                    "pending read public MCP tool call timed out after {} ms",
                    TOOL_TIMEOUT.as_millis()
                ),
            }
        }))
    }

    async fn await_pending_read(
        &self,
        mut pending: tokio::task::JoinHandle<Result<CallToolResult>>,
        normalization: ReadNormalization,
    ) -> Result<ReadObservation> {
        let result = match tokio::time::timeout(TOOL_TIMEOUT, &mut pending).await {
            Ok(result) => result.context("pending read task panicked")??,
            Err(_) => {
                let cleanup = abort_and_join_pending_read(pending).await;
                return match cleanup {
                    Ok(()) => anyhow::bail!(
                        "pending read task timed out after {} ms",
                        TOOL_TIMEOUT.as_millis()
                    ),
                    Err(cleanup_error) => Err(cleanup_error.context(format!(
                        "pending read task timed out after {} ms; pending task cleanup failed",
                        TOOL_TIMEOUT.as_millis()
                    ))),
                };
            }
        };
        self.record_raw("read", &result);
        ensure_success(&result, "pending read")?;
        let observation = match normalization {
            ReadNormalization::BatchOneRaw => normalize_batch_one_raw_read(
                structured(&result, "pending read")?,
                self.normalization()?,
                result.is_error,
            )?,
            ReadNormalization::General => normalize_read(
                structured(&result, "pending read")?,
                self.normalization()?,
                result.is_error,
            )?,
            ReadNormalization::Positioned => normalize_positioned_read(
                structured(&result, "pending read")?,
                self.normalization()?,
                result.is_error,
            )?,
        };
        Ok(observation)
    }

    async fn pending_read_then_write_baseline(
        &self,
    ) -> Result<(WriteObservation, ReadObservation)> {
        let secondary = self.connect_secondary_client().await?;
        let mut pending = None;
        let operation = async {
            pending = Some(self.start_pending_read(json!({
                "connection_id": self.connection_id()?,
                "from": { "type": "now" },
                "timeout_ms": READ_TIMEOUT_MS,
                "encoding": "utf8",
                "match": {
                    "pattern": String::from_utf8_lossy(PONG_RESPONSE),
                    "config": { "mode": "literal_substring", "pattern_encoding": "utf8" }
                }
            }))?);
            // Historical native baseline uses this bounded delay. It is not a
            // readiness proof: required fixture coverage retains the stronger
            // held-output/readiness assertion while this row compares exact
            // final public read/write outcome for both endpoints.
            tokio::time::sleep(INHERITED_BASELINE_DELAY).await;
            let pending_read = pending
                .as_ref()
                .context("pending read join handle was unexpectedly unavailable")?;
            ensure!(
                !pending_read.is_finished(),
                "pending public read completed before later public write"
            );
            let write = self.write_with_client(&secondary, b"ping\r\n").await?;
            assert_write(&write, b"ping\r\n")?;
            let pending_read = pending
                .take()
                .context("pending read join handle was unexpectedly unavailable")?;
            let pong = self
                .await_pending_read(pending_read, ReadNormalization::BatchOneRaw)
                .await?;
            assert_batch_one_pong(&pong)?;
            Ok::<(WriteObservation, ReadObservation), anyhow::Error>((write, pong))
        }
        .await;
        let pending_cleanup = match pending.take() {
            Some(pending) => abort_and_join_pending_read(pending).await,
            None => Ok(()),
        };
        let operation =
            combine_operation_and_cleanup(operation, pending_cleanup, "pending read task cleanup");
        let cleanup =
            cancel_modern_client(secondary, "cancel secondary modern differential MCP client")
                .await;
        combine_operation_and_cleanup(operation, cleanup, "secondary modern MCP client cleanup")
    }

    async fn pending_read_then_write(
        &self,
        read_args: Value,
        payloads: &[&[u8]],
        normalization: ReadNormalization,
        separation: Option<Duration>,
    ) -> Result<(Vec<WriteObservation>, ReadObservation)> {
        ensure!(
            !payloads.is_empty(),
            "differential pending read requires at least one later public write"
        );
        let secondary = self.connect_secondary_client().await?;
        let mut pending = None;
        let operation = async {
            pending = Some(self.start_pending_read(read_args)?);
            tokio::time::sleep(INHERITED_BASELINE_DELAY).await;
            let pending_read = pending
                .as_ref()
                .context("pending read join handle was unexpectedly unavailable")?;
            ensure!(
                !pending_read.is_finished(),
                "pending public read completed before later public write"
            );

            let mut writes = Vec::with_capacity(payloads.len());
            for (index, payload) in payloads.iter().enumerate() {
                let write = self.write_with_client(&secondary, payload).await?;
                assert_write(&write, payload)?;
                writes.push(write);
                if index + 1 < payloads.len() {
                    if let Some(delay) = separation {
                        tokio::time::sleep(delay).await;
                    }
                }
            }

            let pending_read = pending
                .take()
                .context("pending read join handle was unexpectedly unavailable")?;
            let read = self.await_pending_read(pending_read, normalization).await?;
            Ok::<(Vec<WriteObservation>, ReadObservation), anyhow::Error>((writes, read))
        }
        .await;
        let pending_cleanup = match pending.take() {
            Some(pending) => abort_and_join_pending_read(pending).await,
            None => Ok(()),
        };
        let operation =
            combine_operation_and_cleanup(operation, pending_cleanup, "pending read task cleanup");
        let cleanup =
            cancel_modern_client(secondary, "cancel secondary modern differential MCP client")
                .await;
        combine_operation_and_cleanup(operation, cleanup, "secondary modern MCP client cleanup")
    }

    async fn partial_read_then_complete_ping(
        &self,
    ) -> Result<(Vec<WriteObservation>, ReadObservation)> {
        let secondary = self.connect_secondary_client().await?;
        let mut pending = None;
        let operation = async {
            pending = Some(self.start_pending_read(json!({
                "connection_id": self.connection_id()?,
                "from": { "type": "now" },
                "timeout_ms": READ_TIMEOUT_MS,
                "encoding": "utf8",
                "match": {
                    "pattern": "pong",
                    "config": {
                        "mode": "literal_substring",
                        "pattern_encoding": "utf8"
                    }
                }
            }))?);
            tokio::time::sleep(INHERITED_BASELINE_DELAY).await;
            let pending_read = pending
                .as_ref()
                .context("partial-line pending read was unexpectedly unavailable")?;
            ensure!(
                !pending_read.is_finished(),
                "partial-line pending read completed before later public write"
            );

            let first = self.write_with_client(&secondary, b"pi").await?;
            assert_diagnostic_write(&first, b"pi")?;

            tokio::time::sleep(INHERITED_BASELINE_DELAY).await;
            let pending_read = pending
                .as_ref()
                .context("partial-line pending read was unexpectedly unavailable")?;
            ensure!(
                !pending_read.is_finished(),
                "partial-line pending read completed after incomplete pi"
            );

            let second = self.write_with_client(&secondary, b"ng\r\n").await?;
            assert_diagnostic_write(&second, b"ng\r\n")?;
            let pending_read = pending
                .take()
                .context("partial-line pending read was unexpectedly unavailable")?;
            let read = self
                .await_pending_read(pending_read, ReadNormalization::Positioned)
                .await?;
            Ok::<(Vec<WriteObservation>, ReadObservation), anyhow::Error>((
                vec![first, second],
                read,
            ))
        }
        .await;
        let pending_cleanup = match pending.take() {
            Some(pending) => abort_and_join_pending_read(pending).await,
            None => Ok(()),
        };
        let operation =
            combine_operation_and_cleanup(operation, pending_cleanup, "pending read task cleanup");
        let cleanup =
            cancel_modern_client(secondary, "cancel secondary modern differential MCP client")
                .await;
        combine_operation_and_cleanup(operation, cleanup, "secondary modern MCP client cleanup")
    }

    async fn configure_max_buffered_bytes(&self) -> Result<()> {
        let result = self
            .call(
                "configure",
                json!({
                    "connection_id": self.connection_id()?,
                    "defaults": { "max_buffered_bytes": 256 }
                }),
            )
            .await?;
        ensure_success(&result, "configure max_buffered_bytes")?;
        let structured = structured(&result, "configure max_buffered_bytes")?;
        ensure!(
            structured.get("mode") == Some(&Value::String("connection".to_owned()))
                && structured
                    .get("defaults")
                    .and_then(Value::as_object)
                    .and_then(|defaults| defaults.get("max_buffered_bytes"))
                    == Some(&Value::from(256u64)),
            "configure max_buffered_bytes did not return effective connection default: {structured:?}"
        );
        Ok(())
    }

    async fn status(&self) -> Result<super::model::StatusSnapshot> {
        let result = self
            .call(
                "get_status",
                json!({ "connection_id": self.connection_id()? }),
            )
            .await?;
        ensure_success(&result, "get_status")?;
        normalize_status(
            structured(&result, "get_status")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn reconfigure(&self, baud_rate: u32) -> Result<ReconfigureObservation> {
        let result = self
            .call(
                "reconfigure",
                json!({ "connection_id": self.connection_id()?, "baud_rate": baud_rate }),
            )
            .await?;
        ensure_success(&result, "reconfigure")?;
        normalize_reconfigure(
            structured(&result, "reconfigure")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn one_connection_summary(&self) -> Result<ConnectionSummaryObservation> {
        let result = self.call("list_connections", json!({})).await?;
        ensure_success(&result, "list_connections")?;
        let structured = structured(&result, "list_connections")?;
        let count = structured
            .get("count")
            .and_then(Value::as_u64)
            .context("list_connections result omitted unsigned count")?;
        let connections = structured
            .get("connections")
            .and_then(Value::as_array)
            .context("list_connections result omitted connections array")?;
        ensure!(
            count == 1 && connections.len() == 1,
            "named/flow differential scenario expected exactly one connection, got count={count}, entries={}",
            connections.len()
        );
        normalize_connection_summary(&connections[0], self.normalization()?, result.is_error)
    }

    async fn set_flow_control(&self, flow_control: &str) -> Result<SetFlowControlObservation> {
        let result = self
            .call(
                "set_flow_control",
                json!({ "connection_id": self.connection_id()?, "flow_control": flow_control }),
            )
            .await?;
        ensure_success(&result, "set_flow_control")?;
        normalize_set_flow_control(
            structured(&result, "set_flow_control")?,
            self.normalization()?,
            result.is_error,
        )
    }

    async fn shutdown(&mut self) -> Vec<anyhow::Error> {
        let mut errors = Vec::new();
        if let Some(connection_id) = self.connection_id.take() {
            match self
                .call("close", json!({ "connection_id": connection_id }))
                .await
            {
                Ok(result) => {
                    if let Err(error) = ensure_success(&result, "close") {
                        errors.push(error);
                    }
                }
                Err(error) => errors.push(error.context("close differential MCP connection")),
            }
        }
        if let Some(client) = self.client.take() {
            if let Err(error) =
                cancel_modern_client(client, "cancel primary differential MCP client").await
            {
                errors.push(error);
            }
        }
        if let Some(server) = self.server.take() {
            if let Err(error) = shutdown_server(server).await {
                errors.push(error);
            }
        }
        errors
    }
}

async fn call_with_client(
    client: &ModernClient,
    name: &'static str,
    args: Value,
) -> Result<CallToolResult> {
    match tokio::time::timeout(
        TOOL_TIMEOUT,
        client.peer().call_tool(tool_request(name, args)),
    )
    .await
    {
        Ok(result) => result.with_context(|| format!("{name} public MCP tool call failed")),
        Err(_) => anyhow::bail!(
            "{name} public MCP tool call timed out after {} ms",
            TOOL_TIMEOUT.as_millis()
        ),
    }
}

async fn cancel_modern_client(client: ModernClient, operation: &str) -> Result<()> {
    match tokio::time::timeout(CLEANUP_TIMEOUT, client.cancel()).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(anyhow::Error::new(error).context(operation.to_owned())),
        Err(_) => anyhow::bail!(
            "{operation} timed out after {} ms",
            CLEANUP_TIMEOUT.as_millis()
        ),
    }
}

async fn abort_and_join_pending_read(
    pending: tokio::task::JoinHandle<Result<CallToolResult>>,
) -> Result<()> {
    pending.abort();
    match tokio::time::timeout(CLEANUP_TIMEOUT, pending).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) if error.is_cancelled() => Ok(()),
        Ok(Err(error)) => Err(anyhow::Error::new(error).context("join aborted pending read")),
        Err(_) => anyhow::bail!(
            "timed out after {} ms joining aborted pending read",
            CLEANUP_TIMEOUT.as_millis()
        ),
    }
}

fn combine_operation_and_cleanup<T>(
    operation: Result<T>,
    cleanup: Result<()>,
    cleanup_label: &str,
) -> Result<T> {
    match (operation, cleanup) {
        (Ok(value), Ok(())) => Ok(value),
        (Ok(_), Err(cleanup_error)) => Err(cleanup_error.context(cleanup_label.to_owned())),
        (Err(error), Ok(())) => Err(error),
        (Err(error), Err(cleanup_error)) => {
            Err(error.context(format!("{cleanup_label} also failed: {cleanup_error}")))
        }
    }
}

async fn shutdown_server(server: TestServer) -> Result<()> {
    match tokio::time::timeout(CLEANUP_TIMEOUT, server.shutdown_and_join()).await {
        Ok(()) => Ok(()),
        Err(_) => anyhow::bail!(
            "timed out after {} ms joining differential MCP server",
            CLEANUP_TIMEOUT.as_millis()
        ),
    }
}

fn ensure_success(result: &CallToolResult, operation: &str) -> Result<()> {
    ensure!(
        result.is_error != Some(true),
        "{operation} returned MCP tool error: {result:?}"
    );
    Ok(())
}

fn compact_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn structured<'a>(result: &'a CallToolResult, operation: &str) -> Result<&'a Value> {
    result
        .structured_content
        .as_ref()
        .with_context(|| format!("{operation} result omitted structured content"))
}

fn validate_report(report: &DifferentialReport) -> Result<()> {
    let batch = batch_for_report_schema(&report.schema_id)?;
    ensure!(
        report.registry.total_rows == 49
            && report.registry.compared_rows == 21
            && report.registry.baseline_and_stronger_rows == 14
            && report.registry.retired_rows == 3
            && report.registry.pending_rows == 11,
        "differential report registry counts drifted: {:?}",
        report.registry
    );
    let expected_cases = registry::executable_cases(batch)?;
    ensure!(
        report.paired_outcomes.len() == expected_cases.len(),
        "differential report for {batch:?} had {} paired outcomes, expected {}",
        report.paired_outcomes.len(),
        expected_cases.len()
    );
    let mut actual_cases = Vec::with_capacity(report.paired_outcomes.len());
    for paired in &report.paired_outcomes {
        ensure!(
            paired.native.case == paired.case && paired.fixture.case == paired.case,
            "differential report case labels disagreed for {}",
            paired.case.id()
        );
        ensure!(
            paired.native == paired.fixture,
            "differential report stored unequal outcomes for {}",
            paired.case.id()
        );
        actual_cases.push(paired.case);
        validate_logical_outcome(&paired.native)?;
    }
    ensure!(
        actual_cases == expected_cases,
        "differential report for {batch:?} case set/order drifted: actual={actual_cases:?}, expected={expected_cases:?}"
    );
    let serialized = serialize_report_unchecked(report)?;
    ensure!(
        !serialized.contains("/dev/pts/")
            && !serialized.contains("last_activity_ms")
            && !serialized.contains("/tmp/")
            && !contains_uuid_like(&serialized),
        "differential report retained dynamic endpoint, UUID, timestamp, or temporary-path data"
    );
    Ok(())
}

fn validate_logical_outcome(outcome: &ScenarioOutcome) -> Result<()> {
    for observation in &outcome.observations {
        match observation {
            Observation::Open(open) => {
                ensure!(
                    open.connection_id == LOGICAL_CONNECTION && open.port == LOGICAL_ENDPOINT,
                    "open observation retained a dynamic connection ID or endpoint: {open:?}"
                );
            }
            Observation::Write(write) => ensure!(
                write.connection_id == LOGICAL_CONNECTION,
                "write observation retained a dynamic connection ID: {write:?}"
            ),
            Observation::Read(read) => {
                ensure!(
                    read.connection_id == LOGICAL_CONNECTION,
                    "read observation retained a dynamic connection ID: {read:?}"
                );
                if let Some(position) = &read.position {
                    validate_read_position(
                        position,
                        read.bytes_returned,
                        read.bytes_read,
                        read.frames
                            .as_ref()
                            .is_some_and(|frames| !frames.is_empty()),
                        &read.stop_reason,
                    )?;
                }
            }
            Observation::StatusDelta(_) => {}
            Observation::Reconfigure(reconfigure) => ensure!(
                reconfigure.connection_id == LOGICAL_CONNECTION
                    && reconfigure.port == LOGICAL_ENDPOINT,
                "reconfigure observation retained dynamic identifiers: {reconfigure:?}"
            ),
            Observation::ConnectionSummary(summary) => ensure!(
                summary.connection_id == LOGICAL_CONNECTION && summary.port == LOGICAL_ENDPOINT,
                "connection summary retained dynamic identifiers: {summary:?}"
            ),
            Observation::SetFlowControl(flow) => ensure!(
                flow.connection_id == LOGICAL_CONNECTION,
                "set_flow_control observation retained a dynamic connection ID: {flow:?}"
            ),
            Observation::PeerWire(_) => {}
            Observation::Flush(flush) => ensure!(
                flush.connection_id == LOGICAL_CONNECTION,
                "flush observation retained a dynamic connection ID: {flush:?}"
            ),
        }
    }
    Ok(())
}

fn validate_read_position(
    position: &ReadPositionObservation,
    bytes_returned: usize,
    bytes_read: usize,
    decoded_frames_present: bool,
    stop_reason: &str,
) -> Result<()> {
    let bytes_returned = u64::try_from(bytes_returned)
        .context("read bytes_returned exceeded u64 while validating position")?;
    let use_bytes_read =
        stop_reason == "framing_error" || (decoded_frames_present && bytes_returned == 0);
    let (counter_name, expected_next_bytes) = if use_bytes_read {
        (
            "bytes_read",
            u64::try_from(bytes_read)
                .context("read bytes_read exceeded u64 while validating position")?,
        )
    } else {
        ("bytes_returned", bytes_returned)
    };
    let expected_next = position
        .from_offset
        .checked_add(expected_next_bytes)
        .context("read position next_offset arithmetic overflowed")?;
    ensure!(
        position.next_offset == expected_next,
        "read position next_offset {} did not equal from_offset {} + {counter_name} {}",
        position.next_offset,
        position.from_offset,
        expected_next_bytes
    );
    if use_bytes_read {
        ensure!(
            bytes_returned <= expected_next_bytes,
            "read bytes_returned {} exceeded consumed bytes {} from {counter_name}",
            bytes_returned,
            expected_next_bytes
        );
    }
    ensure!(
        position.end_offset >= position.next_offset,
        "read position end_offset {} preceded next_offset {}",
        position.end_offset,
        position.next_offset
    );
    let expected_remaining = position
        .end_offset
        .checked_sub(position.next_offset)
        .context("read position buffered_remaining arithmetic underflowed")?;
    ensure!(
        position.buffered_remaining == expected_remaining,
        "read position buffered_remaining {} did not equal end_offset {} - next_offset {}",
        position.buffered_remaining,
        position.end_offset,
        position.next_offset
    );
    ensure!(
        position.start_offset <= position.from_offset,
        "read position start_offset {} exceeded from_offset {}",
        position.start_offset,
        position.from_offset
    );
    Ok(())
}

fn serialize_report_unchecked(report: &DifferentialReport) -> Result<String> {
    serde_json::to_string(report).context("serialize report dynamic-value guard")
}

fn contains_uuid_like(text: &str) -> bool {
    text.as_bytes().windows(36).any(|candidate| {
        [8, 13, 18, 23]
            .into_iter()
            .all(|index| candidate[index] == b'-')
            && candidate
                .iter()
                .enumerate()
                .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
    })
}
