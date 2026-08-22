//! Shipped protocol-preset parity over the reusable real-PTY fixture.
//!
//! Every public case opens a real slave path through MCP. Peer encoders and
//! mutable device behavior live outside production framing/parser code.

#![cfg(target_os = "linux")]

mod common;

use std::time::Duration;

use anyhow::{Context, Result};
use common::device_fixture::core::Action;
use common::device_fixture::protocol_peers::{
    cobs_encode, cobs_frame, modbus_ascii_frame, modbus_ascii_payload, nmea_sentence, slip_encode,
    AtPeer, DelayedPeer, ModbusAsciiPeer, SilentPeer,
};
use common::device_fixture::{DeviceFixture, DeviceFixtureConfig};
use common::{connect_client, tool_request, TestServer};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(2);
const PEER_RESPONSE_DELAY: Duration = Duration::from_millis(100);

/// Independent registry. Update this table deliberately when product adds or
/// removes a shipped preset; it must not derive names from production enums.
struct ProtocolCoverage {
    preset: &'static str,
    public_case: &'static str,
}

const PROTOCOL_COVERAGE: [ProtocolCoverage; 7] = [
    ProtocolCoverage {
        preset: "at_command",
        public_case: "at_command_connection_default_drives_stateful_transact_and_parser_quirk",
    },
    ProtocolCoverage {
        preset: "slip",
        public_case: "slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery",
    },
    ProtocolCoverage {
        preset: "json_lines",
        public_case: "json_lines_preset_writes_line_and_preserves_object_only_parser_behavior",
    },
    ProtocolCoverage {
        preset: "cobs",
        public_case: "cobs_preset_uses_independent_zero_byte_vector_for_write_and_read",
    },
    ProtocolCoverage {
        preset: "ndjson",
        public_case: "ndjson_preset_parses_records_and_skips_blank_whitespace_lines",
    },
    ProtocolCoverage {
        preset: "nmea0183",
        public_case: "nmea0183_preset_parses_valid_independently_checksummed_sentence",
    },
    ProtocolCoverage {
        preset: "modbus_ascii",
        public_case: "modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation",
    },
];

const EXPECTED_SHIPPED_PRESETS: [&str; 7] = [
    "at_command",
    "slip",
    "json_lines",
    "cobs",
    "ndjson",
    "nmea0183",
    "modbus_ascii",
];

#[test]
fn shipped_protocol_preset_coverage_registry_is_exact_and_unique() {
    let registered: Vec<_> = PROTOCOL_COVERAGE
        .iter()
        .map(|coverage| coverage.preset)
        .collect();
    assert_eq!(registered, EXPECTED_SHIPPED_PRESETS);
    assert_eq!(
        registered
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        EXPECTED_SHIPPED_PRESETS.len(),
        "protocol coverage registry contains a duplicate"
    );
    assert!(
        PROTOCOL_COVERAGE
            .iter()
            .all(|coverage| !coverage.public_case.is_empty()),
        "every preset must name its fixture-backed public case"
    );
}

fn emitted(action: &Action) -> &[u8] {
    match action {
        Action::Emit(bytes) => bytes,
        other => panic!("expected emitted peer bytes, got {other:?}"),
    }
}

#[test]
fn at_peer_has_stateful_urc_error_and_no_response_paths() {
    let mut peer = AtPeer::default();
    assert_eq!(
        peer.handle_command(b"ATE1"),
        vec![Action::Emit(b"OK\r\n".to_vec())]
    );
    let first = peer.handle_command(b"AT+CSQ");
    assert_eq!(emitted(&first[0]), b"AT+CSQ\r\n");
    assert_eq!(emitted(&first[1]), b"+CEREG: 0\r\n");
    assert_eq!(emitted(&first[2]), b"+CSQ: 1,99\r\n");
    assert_eq!(emitted(&first[3]), b"OK\r\n");
    let second = peer.handle_command(b"AT+CSQ");
    assert_eq!(emitted(&second[1]), b"+CEREG: 1\r\n");
    assert_eq!(emitted(&second[2]), b"+CSQ: 2,99\r\n");
    assert_eq!(
        emitted(peer.handle_command(b"AT+CME").last().expect("CME response")),
        b"+CME ERROR: 10\r\n"
    );
    assert_eq!(peer.handle_command(b"AT+NORESPONSE").len(), 1);
    assert_eq!(
        emitted(peer.handle_command(b"BAD").last().expect("error response")),
        b"ERROR\r\n"
    );
}

#[test]
fn slip_oracle_encodes_escapes_and_keeps_fault_vector_independent() {
    assert_eq!(
        slip_encode(&[0xC0, 0xDB]).expect("SLIP encode"),
        [0xC0, 0xDB, 0xDC, 0xDB, 0xDD, 0xC0]
    );
    let malformed_then_valid = [0xC0, 0xDB, 0x41, 0xC0, 0xC0, 0x01, 0xC0];
    assert_eq!(&malformed_then_valid[..4], &[0xC0, 0xDB, 0x41, 0xC0]);
    assert_eq!(&malformed_then_valid[4..], &[0xC0, 0x01, 0xC0]);
}

#[test]
fn cobs_oracle_covers_zero_max_block_fragmentation_and_fault() {
    assert_eq!(cobs_encode(&[0, 1, 0]), [1, 2, 1, 1, 0]);
    let max_block = cobs_encode(&vec![1; 254]);
    assert_eq!(max_block[0], 0xFF);
    assert_eq!(max_block.last(), Some(&0));

    let mut dest = [0u8; 16];
    let mut decoder = cobs::CobsDecoder::new(&mut dest);
    assert!(decoder.push(&[2]).expect("first COBS fragment").is_none());
    assert!(decoder.push(&[1]).expect("second COBS fragment").is_none());
    let report = decoder
        .push(&[0])
        .expect("terminator fragment")
        .expect("fragmented COBS frame");
    assert_eq!(report.frame_size(), 1);

    let error = cobs::decode(&[4, 1, 0], &mut dest).expect_err("truncated COBS frame");
    assert!(matches!(error, cobs::DecodeError::InvalidFrame { .. }));
}

#[test]
fn json_ndjson_oracle_keeps_values_and_malformed_recovery_distinct() {
    let records = [
        serde_json::json!({"sensor":"temp","seq":1}),
        serde_json::json!([1, 2, 3]),
        serde_json::json!(42),
    ];
    let mut stream = Vec::new();
    for record in records {
        serde_json::to_writer(&mut stream, &record).expect("serialize JSON record");
        stream.push(b'\n');
    }
    stream.extend_from_slice(b"   \n{bad}\n{\"sensor\":\"temp\",\"seq\":2}\n");
    let lines: Vec<&[u8]> = stream.split(|byte| *byte == b'\n').collect();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(lines[0]).expect("first JSON")["seq"],
        1
    );
    assert!(serde_json::from_slice::<serde_json::Value>(lines[4]).is_err());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(lines[5]).expect("recovered JSON")["seq"],
        2
    );
}

#[test]
fn nmea_oracle_covers_standard_ais_proprietary_bad_and_missing_checksums() {
    assert_eq!(
        nmea_sentence(b'$', "GPGGA,123519,4807.038,N,01131.000,E,1,08", true),
        b"$GPGGA,123519,4807.038,N,01131.000,E,1,08*77\r\n"
    );
    let ais = nmea_sentence(b'!', "AIVDM,1,1,,A,15Muq?P000o;T@<E>4p@0?vN0<0u,0", true);
    assert!(ais.starts_with(b"!AIVDM,"));
    let proprietary = nmea_sentence(b'$', "PTEST,1,2", true);
    assert!(proprietary.starts_with(b"$PTEST,"));
    let missing = nmea_sentence(b'$', "GPHDT,123.4,T", false);
    assert!(!missing.contains(&b'*'));
    let mut bad = nmea_sentence(b'$', "GPHDT,123.4,T", true);
    let checksum_index = bad
        .iter()
        .position(|byte| *byte == b'*')
        .expect("NMEA checksum marker")
        + 1;
    bad[checksum_index] = if bad[checksum_index] == b'0' {
        b'1'
    } else {
        b'0'
    };
    assert_ne!(bad, nmea_sentence(b'$', "GPHDT,123.4,T", true));
}

#[test]
fn modbus_ascii_peer_mutates_state_handles_broadcast_and_rejects_bad_lrc() {
    let mut peer = ModbusAsciiPeer::new(1);
    let write = peer
        .handle(b":01060001002ACE\r\n")
        .expect("parse write request")
        .expect("write response");
    assert_eq!(write, b":01060001002ACE\r\n");
    let read = peer
        .handle(b":010300010001FA\r\n")
        .expect("parse read request")
        .expect("read response");
    assert_eq!(read, b":010302002AD0\r\n");
    assert!(peer
        .handle(b":000600020001F7\r\n")
        .expect("broadcast request")
        .is_none());
    assert!(peer.handle(b":01030001000100\r\n").is_err());
    assert!(peer
        .handle(b":010300010001FA\r\n")
        .expect("recovered read")
        .is_some());
}

#[test]
fn generic_and_shell_vectors_define_fragmentation_and_recovery_inputs() {
    let delimiter_chunks: [&[u8]; 3] = [b"one<", b">two<", b">"];
    assert_eq!(delimiter_chunks.concat(), b"one<>two<>");
    assert_eq!(4u32.to_be_bytes(), [0, 0, 0, 4]);
    assert_eq!(4u32.to_le_bytes(), [4, 0, 0, 0]);
    let start_end_chunks: [&[u8]; 3] = [b"noise<<pay", b"load>", b">"];
    assert_eq!(start_end_chunks.concat(), b"noise<<payload>>");
    let shell_chunks: [&[u8]; 3] = [b"result\r\nuser@host:", b"/tmp", b"$ "];
    assert!(shell_chunks.concat().ends_with(b"$ "));
    let raw: Vec<u8> = (0..=255).collect();
    assert_eq!(raw.len(), 256);
}

#[tokio::test]
async fn at_command_connection_default_drives_stateful_transact_and_parser_quirk() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(
        DelayedPeer::new(AtPeer::default(), PEER_RESPONSE_DELAY),
        DeviceFixtureConfig::default(),
    )
    .await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(
        &client,
        &fixture,
        json!({ "protocol": { "type": "at_command" } }),
    )
    .await?;

    let echo_enabled = transact(
        &client,
        &id,
        "ATE1",
        json!({ "match": { "pattern": "OK" }, "timeout_ms": 2000 }),
    )
    .await?;
    assert_eq!(
        structured(&echo_enabled)?["write"]["bytes_written"],
        json!(5)
    );
    assert_eq!(
        collect_raw_input(&mut fixture, b"ATE1\r".len()).await?,
        b"ATE1\r"
    );
    assert_eq!(
        transact_read(&echo_enabled)?["stop_reason"],
        json!("match_found")
    );

    let signal = transact(
        &client,
        &id,
        "AT+CSQ",
        json!({ "match": { "pattern": "OK" }, "timeout_ms": 2000 }),
    )
    .await?;
    assert_eq!(
        collect_raw_input(&mut fixture, b"AT+CSQ\r".len()).await?,
        b"AT+CSQ\r"
    );
    let signal_frames = frames(transact_read(&signal)?)?;
    assert_eq!(
        signal_frames.len(),
        4,
        "AT response frames: {signal_frames:?}"
    );
    assert_eq!(signal_frames[0]["data"], json!("AT+CSQ"));
    assert_eq!(signal_frames[1]["parsed"]["parser"], json!("at_command"));
    assert_eq!(signal_frames[1]["parsed"]["command"], json!("CEREG"));
    assert_eq!(signal_frames[1]["parsed"]["fields"], json!(["0"]));
    assert_eq!(signal_frames[2]["parsed"]["command"], json!("CSQ"));
    assert_eq!(signal_frames[2]["parsed"]["fields"], json!(["1", "99"]));
    assert_eq!(signal_frames[3]["parsed"]["response_type"], json!("status"));
    assert_eq!(signal_frames[3]["parsed"]["status"], json!("OK"));

    let cme = transact(
        &client,
        &id,
        "AT+CME",
        json!({ "match": { "pattern": "CME ERROR" }, "timeout_ms": 2000 }),
    )
    .await?;
    let cme_frames = frames(transact_read(&cme)?)?;
    let cme_frame = cme_frames
        .iter()
        .find(|frame| frame["data"] == json!("+CME ERROR: 10"))
        .context("AT CME frame missing")?;
    // Current parser checks generic +COMMAND: response before CME/CMS error.
    // Preserve this public characterization; changing it needs a separate
    // parser-semantics decision.
    assert_eq!(cme_frame["parsed"]["response_type"], json!("response"));
    assert_eq!(cme_frame["parsed"]["command"], json!("CME ERROR"));
    assert_eq!(cme_frame["parsed"]["fields"], json!(["10"]));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn slip_preset_writes_independent_bytes_and_keeps_partial_error_then_recovery() -> Result<()>
{
    let mut fixture = DeviceFixture::spawn(SilentPeer, DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;

    let tx_payload = [0xC0, 0xDB, 0x41];
    let write_result = call_tool(
        &client,
        "write",
        json!({
            "connection_id": id,
            "data": "c0 db 41",
            "encoding": "hex",
            "protocol": { "type": "slip" }
        }),
    )
    .await?;
    assert_success(&write_result, "SLIP write")?;
    let expected_tx = slip_encode(&tx_payload)?;
    assert_eq!(
        collect_raw_input(&mut fixture, expected_tx.len()).await?,
        expected_tx
    );

    let happy_payload = b"SLIP \xC0 \xDB";
    let happy = read_live(
        &client,
        &mut fixture,
        &id,
        json!({
            "protocol": { "type": "slip" },
            "encoding": "hex",
            "match": { "pattern": "53 4c 49 50", "config": { "pattern_encoding": "hex" } },
            "timeout_ms": 2000
        }),
        slip_encode(happy_payload)?,
    )
    .await?;
    let happy_frames = frames(structured(&happy)?)?;
    assert_eq!(happy_frames.len(), 1);
    assert_eq!(happy_frames[0]["frame_type"], json!("slip"));
    assert_eq!(happy_frames[0]["data"], json!("53 4c 49 50 20 c0 20 db"));
    assert_eq!(happy_frames[0]["parsed"]["parser"], json!("raw"));

    let mut malformed = slip_encode(b"OK")?;
    malformed.extend_from_slice(&[0xDB, 0x41]);
    let partial = read_live(
        &client,
        &mut fixture,
        &id,
        json!({ "protocol": { "type": "slip" }, "encoding": "utf8", "timeout_ms": 2000 }),
        malformed,
    )
    .await?;
    let partial_body = structured(&partial)?;
    assert_eq!(partial_body["stop_reason"], json!("framing_error"));
    assert!(
        partial_body["error"]
            .as_str()
            .is_some_and(|message| message.contains("SLIP") && message.contains("0x41")),
        "SLIP error must preserve type and offending byte: {partial_body:?}"
    );
    assert_eq!(partial_body["encoding"], json!("hex"));
    assert_eq!(partial_body["frames_dropped"], json!(0));
    let partial_frames = frames(partial_body)?;
    assert_eq!(partial_frames.len(), 1);
    assert_eq!(partial_frames[0]["data"], json!("OK"));
    assert_eq!(partial_frames[0]["encoding"], json!("utf8"));

    let recovered = read_live(
        &client,
        &mut fixture,
        &id,
        json!({
            "protocol": { "type": "slip" },
            "match": { "pattern": "recovered" },
            "timeout_ms": 2000
        }),
        slip_encode(b"recovered")?,
    )
    .await?;
    assert_eq!(structured(&recovered)?["stop_reason"], json!("match_found"));
    assert_eq!(
        frames(structured(&recovered)?)?[0]["data"],
        json!("recovered")
    );

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn json_lines_preset_writes_line_and_preserves_object_only_parser_behavior() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(SilentPeer, DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;

    let write_result = call_tool(
        &client,
        "write",
        json!({
            "connection_id": id,
            "data": "query",
            "protocol": { "type": "json_lines" }
        }),
    )
    .await?;
    assert_success(&write_result, "JSON Lines write")?;
    assert_eq!(
        collect_raw_input(&mut fixture, b"query\n".len()).await?,
        b"query\n"
    );
    assert_eq!(fixture.next_observed_command(WAIT).await?.command, b"query");

    let result = read_live(
        &client,
        &mut fixture,
        &id,
        json!({
            "protocol": { "type": "json_lines" },
            "match": { "pattern": "[1,2,3]" },
            "timeout_ms": 2000
        }),
        b"{\"sensor\":\"temp\",\"seq\":1}\n[1,2,3]\n".to_vec(),
    )
    .await?;
    let json_frames = frames(structured(&result)?)?;
    assert_eq!(json_frames.len(), 2);
    assert_eq!(json_frames[0]["parsed"]["parser"], json!("json"));
    assert_eq!(json_frames[0]["parsed"]["sensor"], json!("temp"));
    assert_eq!(json_frames[0]["parsed"]["seq"], json!(1));
    // JSON Lines permits arrays, but shipped parser deliberately structures
    // objects only. Keep this behavior characterized rather than changing it.
    assert_eq!(json_frames[1]["data"], json!("[1,2,3]"));
    assert_eq!(json_frames[1]["parsed"]["parser"], json!("raw"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn cobs_preset_uses_independent_zero_byte_vector_for_write_and_read() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(SilentPeer, DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;

    let payload = [0x00, b'A', 0x00];
    let write_result = call_tool(
        &client,
        "write",
        json!({
            "connection_id": id,
            "data": "00 41 00",
            "encoding": "hex",
            "protocol": { "type": "cobs" }
        }),
    )
    .await?;
    assert_success(&write_result, "COBS write")?;
    let expected = cobs_frame(&payload);
    assert_eq!(
        collect_raw_input(&mut fixture, expected.len()).await?,
        expected
    );

    let result = read_live(
        &client,
        &mut fixture,
        &id,
        json!({
            "protocol": { "type": "cobs" },
            "match": { "pattern": "A" },
            "timeout_ms": 2000
        }),
        cobs_frame(&payload),
    )
    .await?;
    let cobs_frames = frames(structured(&result)?)?;
    assert_eq!(cobs_frames.len(), 1);
    assert_eq!(cobs_frames[0]["frame_type"], json!("cobs"));
    assert_eq!(cobs_frames[0]["data"], json!("\u{0}A\u{0}"));
    assert_eq!(cobs_frames[0]["parsed"]["parser"], json!("raw"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn ndjson_preset_parses_records_and_skips_blank_whitespace_lines() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(SilentPeer, DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;

    let result = read_live(
        &client,
        &mut fixture,
        &id,
        json!({
            "protocol": { "type": "ndjson" },
            "match": { "pattern": "\"seq\":2" },
            "timeout_ms": 2000
        }),
        b"{\"sensor\":\"temp\",\"seq\":1}\n\n   \r\n{\"sensor\":\"temp\",\"seq\":2}\n".to_vec(),
    )
    .await?;
    let ndjson_frames = frames(structured(&result)?)?;
    assert_eq!(
        ndjson_frames.len(),
        2,
        "blank NDJSON records must be skipped"
    );
    assert_eq!(ndjson_frames[0]["parsed"]["parser"], json!("json"));
    assert_eq!(ndjson_frames[0]["parsed"]["seq"], json!(1));
    assert_eq!(ndjson_frames[1]["parsed"]["seq"], json!(2));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn nmea0183_preset_parses_valid_independently_checksummed_sentence() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(SilentPeer, DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;

    let sentence = nmea_sentence(
        b'$',
        "GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,",
        true,
    );
    let result = read_live(
        &client,
        &mut fixture,
        &id,
        json!({
            "protocol": { "type": "nmea0183" },
            "match": { "pattern": "GPGGA" },
            "timeout_ms": 2000
        }),
        sentence,
    )
    .await?;
    let nmea_frames = frames(structured(&result)?)?;
    assert_eq!(nmea_frames.len(), 1);
    assert_eq!(nmea_frames[0]["frame_type"], json!("start_end"));
    assert_eq!(nmea_frames[0]["parsed"]["parser"], json!("nmea"));
    assert_eq!(nmea_frames[0]["parsed"]["talker_id"], json!("GP"));
    assert_eq!(nmea_frames[0]["parsed"]["sentence_type"], json!("GGA"));
    assert_eq!(nmea_frames[0]["parsed"]["checksum_valid"], json!(true));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn modbus_ascii_preset_transact_parses_lrc_and_proves_peer_state_mutation() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(
        DelayedPeer::new(ModbusAsciiPeer::new(1), PEER_RESPONSE_DELAY),
        DeviceFixtureConfig::default(),
    )
    .await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;

    let write_pdu = [1, 0x06, 0x00, 0x01, 0x00, 0x2A];
    let write = transact(
        &client,
        &id,
        &modbus_ascii_payload(&write_pdu),
        json!({
            "protocol": { "type": "modbus_ascii" },
            "match": { "pattern": "01060001002A" },
            "timeout_ms": 2000
        }),
    )
    .await?;
    let expected_write = modbus_ascii_frame(&write_pdu);
    assert_eq!(
        collect_raw_input(&mut fixture, expected_write.len()).await?,
        expected_write
    );
    let write_frames = frames(transact_read(&write)?)?;
    assert_eq!(write_frames.len(), 1);
    assert_eq!(write_frames[0]["parsed"]["parser"], json!("modbus_ascii"));
    assert_eq!(write_frames[0]["parsed"]["address"], json!(1));
    assert_eq!(write_frames[0]["parsed"]["function_code"], json!(6));
    assert_eq!(write_frames[0]["parsed"]["checksum_valid"], json!(true));

    let read_pdu = [1, 0x03, 0x00, 0x01, 0x00, 0x01];
    let read = transact(
        &client,
        &id,
        &modbus_ascii_payload(&read_pdu),
        json!({
            "protocol": { "type": "modbus_ascii" },
            "match": { "pattern": "010302002A" },
            "timeout_ms": 2000
        }),
    )
    .await?;
    let read_frames = frames(transact_read(&read)?)?;
    assert_eq!(read_frames.len(), 1);
    assert_eq!(read_frames[0]["data"], json!("010302002AD0"));
    assert_eq!(read_frames[0]["parsed"]["address"], json!(1));
    assert_eq!(read_frames[0]["parsed"]["function_code"], json!(3));
    assert_eq!(read_frames[0]["parsed"]["data"], json!([2, 0, 42]));
    assert_eq!(read_frames[0]["parsed"]["checksum_valid"], json!(true));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

async fn open_fixture(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    fixture: &DeviceFixture,
    extra: Value,
) -> Result<String> {
    let mut args = json!({
        "port": fixture.port_path().to_string_lossy(),
        "baud_rate": 115200,
        "profile_mode": "none"
    });
    if let (Value::Object(args), Value::Object(extra)) = (&mut args, extra) {
        args.extend(extra);
    }
    let result = call_tool(client, "open", args).await?;
    assert_success(&result, "open fixture")?;
    structured(&result)?["connection_id"]
        .as_str()
        .map(str::to_owned)
        .context("open result missing connection_id")
}

async fn transact(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    data: &str,
    extra: Value,
) -> Result<CallToolResult> {
    let mut args = json!({ "connection_id": connection_id, "data": data });
    if let (Value::Object(args), Value::Object(extra)) = (&mut args, extra) {
        args.extend(extra);
    }
    let result = call_tool(client, "transact", args).await?;
    assert_success(&result, "transact")?;
    Ok(result)
}

async fn read_live(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    fixture: &mut DeviceFixture,
    connection_id: &str,
    extra: Value,
    wire: Vec<u8>,
) -> Result<CallToolResult> {
    let mut args = json!({ "connection_id": connection_id, "encoding": "utf8" });
    if let (Value::Object(args), Value::Object(extra)) = (&mut args, extra) {
        args.extend(extra);
    }
    fixture.set_hold(true).await?;
    fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
    let peer = client.peer().clone();
    let reader = tokio::spawn(async move { peer.call_tool(tool_request("read", args)).await });
    tokio::task::yield_now().await;
    anyhow::ensure!(
        !reader.is_finished(),
        "read completed before fixture emitted protocol bytes"
    );

    let pending_before = fixture.snapshot().output_pending;
    let wire_len = wire.len();
    fixture.run_script(vec![Action::Emit(wire)]).await?;
    fixture
        .wait_for(WAIT, |snapshot| {
            snapshot.held && snapshot.output_pending >= pending_before.saturating_add(wire_len)
        })
        .await?;
    anyhow::ensure!(
        !reader.is_finished(),
        "read completed while fixture protocol bytes were held"
    );
    fixture.set_hold(false).await?;
    let result = tokio::time::timeout(WAIT, reader)
        .await
        .context("protocol read timed out")?
        .context("protocol read task join")??;
    assert_success(&result, "read")?;
    Ok(result)
}

async fn collect_raw_input(fixture: &mut DeviceFixture, expected_len: usize) -> Result<Vec<u8>> {
    let mut actual = Vec::with_capacity(expected_len);
    while actual.len() < expected_len {
        actual.extend(fixture.next_raw_input(WAIT).await?);
    }
    anyhow::ensure!(
        actual.len() == expected_len,
        "peer observed {} bytes, expected {expected_len}: {actual:02x?}",
        actual.len()
    );
    Ok(actual)
}

fn transact_read(result: &CallToolResult) -> Result<&Value> {
    structured(result)?
        .get("read")
        .context("transact result missing read half")
}

fn frames(value: &Value) -> Result<&Vec<Value>> {
    value
        .as_object()
        .context("structured result is not an object")?
        .get("frames")
        .context("result missing frames field")?
        .as_array()
        .context("result missing frames array")
}

async fn close(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
) -> Result<()> {
    let result = call_tool(client, "close", json!({ "connection_id": connection_id })).await?;
    assert_success(&result, "close")
}

async fn call_tool(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    name: &'static str,
    args: Value,
) -> Result<CallToolResult> {
    client
        .peer()
        .call_tool(tool_request(name, args))
        .await
        .with_context(|| format!("{name} call failed"))
}

fn assert_success(result: &CallToolResult, operation: &str) -> Result<()> {
    anyhow::ensure!(
        result.is_error != Some(true),
        "{operation} returned tool error: {result:?}"
    );
    Ok(())
}

fn structured(result: &CallToolResult) -> Result<&Value> {
    result
        .structured_content
        .as_ref()
        .context("tool result missing structured content")
}
