//! Disposable Linux-only public-MCP characterization for the native NMEA row.

#![cfg(target_os = "linux")]

mod common;

use std::path::PathBuf;

use anyhow::{ensure, Context, Result};
use common::firmware::NativeSimFirmware;
use common::{connect_2026_07_28_client, tool_request, TestServer, VersionedClientHandler};
use rmcp::model::CallToolResult;
use rmcp::service::{RoleClient, RunningService};
use serde::Serialize;
use serde_json::{json, Map, Value};

const CASE: &str = "native_read_nmea0183_preset_decodes_parsed_frame";
const SCHEMA_ID: &str = "serial-mcp.native-sim-nmea-characterization.v1";
const ARTIFACT_PATH: &str = "target/native-sim-differential/nmea-characterization.json";
const CONNECTION_PLACEHOLDER: &str = "$CONNECTION";
const ENDPOINT_PLACEHOLDER: &str = "$ENDPOINT";
const BAUD_RATE: u32 = 115_200;
const TOOL_TIMEOUT_MS: u64 = 3_000;
const BOOT_BANNER: &str = "serial-mcp test firmware ready\r\n";
const NMEA_BODY: &[u8] = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
const EXPECTED_SENTENCE: &str =
    "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n";
const EXPECTED_SENDRAW_COMMAND: &str =
    "sendraw hex 2447504747412C3132333531392C343830372E3033382C4E2C30313133312E3030302C452C312C30382C302E392C3534352E342C4D2C34362E392C4D2C2C2A34370D0A\r\n";

type ModernClient = RunningService<RoleClient, VersionedClientHandler>;

#[derive(Debug, PartialEq, Eq, Serialize)]
struct PublicOutcome {
    tool: &'static str,
    is_error: Option<bool>,
    structured_content: Value,
}

#[derive(Debug, Serialize)]
struct CharacterizationArtifact {
    schema_id: &'static str,
    case: &'static str,
    outcomes: Vec<Vec<PublicOutcome>>,
    omitted_dynamic_fields: [&'static str; 1],
}

#[tokio::test]
#[ignore = "requires native_sim firmware binary"]
async fn records_native_nmea_public_outcomes() -> Result<()> {
    let (sentence, command) = exact_wire_stimulus()?;
    let mut outcomes = Vec::with_capacity(3);

    for endpoint_index in 0..3 {
        let outcome = run_endpoint(endpoint_index, &sentence, &command).await?;
        if let Some(first) = outcomes.first() {
            ensure!(
                first == &outcome,
                "native NMEA characterization endpoint {endpoint_index} differed from first endpoint\nfirst:\n{}\ncurrent:\n{}",
                serde_json::to_string_pretty(first).context("serialize first endpoint evidence")?,
                serde_json::to_string_pretty(&outcome)
                    .context("serialize differing endpoint evidence")?
            );
        }
        outcomes.push(outcome);
    }

    let artifact = CharacterizationArtifact {
        schema_id: SCHEMA_ID,
        case: CASE,
        outcomes,
        omitted_dynamic_fields: ["elapsed_ms"],
    };
    write_artifact(&artifact)
}

async fn run_endpoint(
    endpoint_index: usize,
    sentence: &str,
    command: &str,
) -> Result<Vec<PublicOutcome>> {
    let firmware = NativeSimFirmware::spawn()
        .await
        .with_context(|| format!("spawn native_sim endpoint {endpoint_index}"))?;
    let endpoint = firmware.pty_path().to_owned();
    let server = TestServer::start().await;
    let (client, _) = match connect_2026_07_28_client(&server).await {
        Ok(client) => client,
        Err(error) => {
            server.shutdown_and_join().await;
            let _ = firmware.shutdown_and_join().await;
            return Err(error).context("connect modern NMEA characterization client");
        }
    };

    let run = run_public_sequence(&client, &endpoint, sentence, command).await;
    let client_shutdown = client
        .cancel()
        .await
        .context("close NMEA characterization client");
    server.shutdown_and_join().await;
    let firmware_shutdown = firmware
        .shutdown_and_join()
        .await
        .context("shutdown native_sim NMEA characterization endpoint");

    let outcome = run?;
    client_shutdown?;
    let firmware_shutdown = firmware_shutdown?;
    ensure!(
        !firmware_shutdown.stdout_drain_aborted,
        "native_sim endpoint {endpoint_index} stdout drain required abort"
    );
    Ok(outcome)
}

async fn run_public_sequence(
    client: &ModernClient,
    endpoint: &str,
    sentence: &str,
    command: &str,
) -> Result<Vec<PublicOutcome>> {
    let open = call_tool(
        client,
        "open",
        json!({
            "port": endpoint,
            "name": null,
            "baud_rate": BAUD_RATE,
            "profile_mode": "none"
        }),
    )
    .await?;
    assert_success(&open, "open")?;
    let open_structured = structured(&open, "open")?;
    let connection_id = open_structured
        .get("connection_id")
        .and_then(Value::as_str)
        .context("open result missing connection_id")?
        .to_owned();
    ensure!(
        !connection_id.is_empty(),
        "open result returned empty connection_id"
    );
    ensure!(
        open_structured.get("name") == Some(&Value::Null),
        "anonymous open returned unexpected name: {open_structured:?}"
    );
    ensure!(
        open_structured.get("port").and_then(Value::as_str) == Some(endpoint),
        "open result endpoint mismatch: {open_structured:?}"
    );
    ensure!(
        open_structured.get("baud_rate").and_then(Value::as_u64) == Some(BAUD_RATE as u64),
        "open result baud mismatch: {open_structured:?}"
    );

    let boot = call_tool(
        client,
        "read",
        json!({
            "connection_id": connection_id,
            "encoding": "utf8",
            "timeout_ms": TOOL_TIMEOUT_MS,
            "match": {
                "pattern": BOOT_BANNER,
                "config": {
                    "mode": "literal_substring",
                    "pattern_encoding": "utf8"
                }
            }
        }),
    )
    .await?;
    assert_success(&boot, "boot-banner read")?;
    assert_raw_read(&boot, BOOT_BANNER, "boot-banner read")?;

    let arm = call_tool(
        client,
        "transact",
        json!({
            "connection_id": connection_id,
            "data": "arm_cmd 1000\r\n",
            "encoding": "utf8",
            "timeout_ms": TOOL_TIMEOUT_MS,
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
    assert_success(&arm, "native arm barrier")?;
    assert_arm_barrier(&arm)?;

    let write = call_tool(
        client,
        "write",
        json!({
            "connection_id": connection_id,
            "data": command,
            "encoding": "utf8"
        }),
    )
    .await?;
    assert_success(&write, "NMEA sendraw write")?;
    assert_sendraw_write(&write, command.len())?;

    let target = call_tool(
        client,
        "read",
        json!({
            "connection_id": connection_id,
            "from": { "type": "now" },
            "encoding": "utf8",
            "timeout_ms": TOOL_TIMEOUT_MS,
            "protocol": { "type": "nmea0183" }
        }),
    )
    .await?;
    assert_success(&target, "NMEA target read")?;
    assert_nmea_target(&target, sentence)?;

    let close = call_tool(client, "close", json!({ "connection_id": connection_id })).await?;
    assert_success(&close, "close")?;

    Ok(vec![
        normalize_outcome("open", &open, &connection_id, endpoint)?,
        normalize_outcome("read", &boot, &connection_id, endpoint)?,
        normalize_outcome("read", &target, &connection_id, endpoint)?,
    ])
}

async fn call_tool(
    client: &ModernClient,
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
    ensure!(
        result.is_error == Some(false),
        "{operation} returned unexpected tool result: {result:?}"
    );
    Ok(())
}

fn structured<'a>(result: &'a CallToolResult, operation: &str) -> Result<&'a Value> {
    result
        .structured_content
        .as_ref()
        .with_context(|| format!("{operation} result missing structured content"))
}

fn assert_raw_read(result: &CallToolResult, expected: &str, operation: &str) -> Result<()> {
    let structured = structured(result, operation)?;
    ensure!(
        structured.get("encoding").and_then(Value::as_str) == Some("utf8")
            && structured.get("data").and_then(Value::as_str) == Some(expected)
            && structured.get("matched").and_then(Value::as_bool) == Some(true)
            && structured.get("error").is_none_or(Value::is_null),
        "{operation} did not return complete expected UTF-8 data: {structured:?}"
    );
    Ok(())
}

fn assert_arm_barrier(result: &CallToolResult) -> Result<()> {
    let structured = structured(result, "arm transact")?;
    let read = structured
        .get("read")
        .context("arm transact result missing read half")?;
    ensure!(
        read.get("encoding").and_then(Value::as_str) == Some("utf8")
            && read.get("data").and_then(Value::as_str) == Some("arm_cmd delay=1000\r\n")
            && read.get("matched").and_then(Value::as_bool) == Some(true)
            && read.get("stop_reason").and_then(Value::as_str) == Some("match_found")
            && read.get("error").is_none_or(Value::is_null),
        "arm barrier did not return exact acknowledgement: {structured:?}"
    );
    Ok(())
}

fn assert_sendraw_write(result: &CallToolResult, expected_bytes: usize) -> Result<()> {
    let structured = structured(result, "sendraw write")?;
    ensure!(
        structured.get("encoding").and_then(Value::as_str) == Some("utf8")
            && structured.get("bytes_written").and_then(Value::as_u64)
                == Some(expected_bytes as u64)
            && structured.get("decoded_bytes").and_then(Value::as_u64)
                == Some(expected_bytes as u64),
        "sendraw write metadata mismatch: {structured:?}"
    );
    Ok(())
}

fn assert_nmea_target(result: &CallToolResult, sentence: &str) -> Result<()> {
    let structured = structured(result, "NMEA target read")?;
    ensure!(
        structured.get("encoding").and_then(Value::as_str) == Some("utf8")
            && structured.get("data").and_then(Value::as_str) == Some(sentence),
        "NMEA target did not return complete UTF-8 sentence: {structured:?}"
    );

    let frames = structured
        .get("frames")
        .and_then(Value::as_array)
        .context("NMEA target omitted frames array")?;
    ensure!(
        frames.len() == 1,
        "NMEA target returned unexpected frame count: {frames:?}"
    );
    let frame = &frames[0];
    let expected_frame_data = format!(
        "{}*47",
        std::str::from_utf8(NMEA_BODY).context("NMEA body was not UTF-8")?
    );
    ensure!(
        frame.get("frame_index").and_then(Value::as_u64) == Some(0)
            && frame.get("frame_type").and_then(Value::as_str) == Some("start_end")
            && frame.get("encoding").and_then(Value::as_str) == Some("utf8")
            && frame.get("data").and_then(Value::as_str) == Some(expected_frame_data.as_str()),
        "NMEA target frame shape mismatch: {frame:?}"
    );

    let parsed = frame
        .get("parsed")
        .and_then(Value::as_object)
        .context("NMEA target frame omitted parsed object")?;
    ensure!(
        parsed.get("parser").and_then(Value::as_str) == Some("nmea")
            && parsed.get("talker_id").and_then(Value::as_str) == Some("GP")
            && parsed.get("sentence_type").and_then(Value::as_str) == Some("GGA")
            && parsed.get("checksum_valid").and_then(Value::as_bool) == Some(true)
            && parsed.get("fields")
                == Some(&json!([
                    "123519",
                    "4807.038",
                    "N",
                    "01131.000",
                    "E",
                    "1",
                    "08",
                    "0.9",
                    "545.4",
                    "M",
                    "46.9",
                    "M",
                    "",
                    ""
                ])),
        "NMEA target parsed frame mismatch: {parsed:?}"
    );
    ensure!(
        structured.get("frames_dropped").and_then(Value::as_u64) == Some(0)
            && structured.get("error").is_none_or(Value::is_null),
        "NMEA target reported dropped frame or error: {structured:?}"
    );
    Ok(())
}

fn normalize_outcome(
    tool: &'static str,
    result: &CallToolResult,
    connection_id: &str,
    endpoint: &str,
) -> Result<PublicOutcome> {
    let structured_content =
        normalize_value(structured(result, tool)?.clone(), connection_id, endpoint)?;
    Ok(PublicOutcome {
        tool,
        is_error: result.is_error,
        structured_content,
    })
}

fn normalize_value(value: Value, connection_id: &str, endpoint: &str) -> Result<Value> {
    match value {
        Value::Array(values) => values
            .into_iter()
            .map(|value| normalize_value(value, connection_id, endpoint))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        Value::Object(object) => {
            let mut normalized = Map::new();
            for (key, value) in object {
                if key == "elapsed_ms" {
                    continue;
                }
                let value = match key.as_str() {
                    "connection_id" => {
                        let actual = value
                            .as_str()
                            .with_context(|| "connection_id was not a string")?;
                        ensure!(
                            actual == connection_id,
                            "unexpected connection_id in public result: {actual:?}"
                        );
                        Value::String(CONNECTION_PLACEHOLDER.to_owned())
                    }
                    "port" | "endpoint" => {
                        let actual = value
                            .as_str()
                            .with_context(|| format!("{key} was not a string"))?;
                        ensure!(
                            actual == endpoint,
                            "unexpected endpoint in public result field {key}: {actual:?}"
                        );
                        Value::String(ENDPOINT_PLACEHOLDER.to_owned())
                    }
                    _ => normalize_value(value, connection_id, endpoint)?,
                };
                normalized.insert(key, value);
            }
            Ok(Value::Object(normalized))
        }
        value => Ok(value),
    }
}

fn exact_wire_stimulus() -> Result<(String, String)> {
    let checksum = xor_checksum(NMEA_BODY);
    ensure!(
        checksum == 0x47,
        "source-derived NMEA checksum changed: {checksum:02X}"
    );
    let body = std::str::from_utf8(NMEA_BODY).context("NMEA body was not UTF-8")?;
    let sentence = format!("${body}*{checksum:02X}\r\n");
    ensure!(
        NMEA_BODY.len() == 61,
        "unexpected NMEA body length: {}",
        NMEA_BODY.len()
    );
    ensure!(
        sentence.len() == 67,
        "unexpected NMEA sentence length: {}",
        sentence.len()
    );
    ensure!(
        sentence == EXPECTED_SENTENCE,
        "NMEA sentence wire mismatch: {sentence:?}"
    );

    let command = format!("sendraw hex {}\r\n", uppercase_hex(sentence.as_bytes()));
    ensure!(
        command.len() == 148,
        "unexpected sendraw command length: {}",
        command.len()
    );
    ensure!(
        command == EXPECTED_SENDRAW_COMMAND,
        "sendraw command wire mismatch: {command:?}"
    );
    Ok((sentence, command))
}

fn xor_checksum(bytes: &[u8]) -> u8 {
    bytes.iter().fold(0u8, |checksum, byte| checksum ^ byte)
}

fn uppercase_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02X}")).collect()
}

fn write_artifact(artifact: &CharacterizationArtifact) -> Result<()> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(ARTIFACT_PATH);
    let parent = path
        .parent()
        .context("NMEA characterization artifact path had no parent")?;
    std::fs::create_dir_all(parent).with_context(|| {
        format!(
            "create NMEA characterization directory {}",
            parent.display()
        )
    })?;
    let mut bytes = serde_json::to_vec_pretty(artifact).context("serialize NMEA artifact")?;
    bytes.push(b'\n');
    std::fs::write(&path, bytes)
        .with_context(|| format!("write NMEA characterization artifact {}", path.display()))?;
    Ok(())
}
