//! Generic matcher and framing parity over reusable real-PTY fixture.

#![cfg(unix)]

mod common;

use std::time::Duration;

use anyhow::{Context, Result};
use common::device_fixture::core::Action;
use common::device_fixture::{DeviceFixture, DeviceFixtureConfig, PingPeer};
use common::{connect_client, tool_request, TestServer};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(2);

#[tokio::test]
async fn regex_and_glob_matchers_find_complete_peer_line() -> Result<()> {
    for (mode, pattern) in [("regex", "po.g.*"), ("glob", "po*")] {
        let mut fixture =
            DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
        let server = TestServer::start().await;
        let (client, _) = connect_client(&server).await?;
        let id = open_fixture(&client, &fixture, json!({})).await?;
        fixture
            .run_script(vec![Action::Emit(b"pong seq=1\r\n".to_vec())])
            .await?;
        fixture
            .wait_for(WAIT, |snapshot| snapshot.output_drained == 12)
            .await?;
        let result = read(
            &client,
            &id,
            json!({
                "timeout_ms": 2000,
                "match": {
                    "pattern": pattern,
                    "config": { "mode": mode, "pattern_encoding": "utf8" }
                }
            }),
        )
        .await?;
        assert_eq!(structured(&result)?["matched"], json!(true));
        assert_eq!(structured(&result)?["stop_reason"], json!("match_found"));
        close(&client, &id).await?;
        client.cancel().await.ok();
        fixture.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn line_framing_returns_exact_ordered_peer_frames() -> Result<()> {
    let mut fixture =
        DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;
    fixture
        .run_script(vec![Action::EmitChunks(vec![
            b"pong seq=1\r\n".to_vec(),
            b"info fixture=rust-pty\r\n".to_vec(),
        ])])
        .await?;
    fixture
        .wait_for(WAIT, |snapshot| snapshot.output_drained == 35)
        .await?;
    let result = read(
        &client,
        &id,
        json!({ "timeout_ms": 100, "rx_framing": { "type": "line" } }),
    )
    .await?;
    let frames = frames(&result)?;
    assert_eq!(frames.len(), 2, "unexpected frames: {frames:?}");
    assert_eq!(frames[0]["data"], json!("pong seq=1"));
    assert_eq!(frames[1]["data"], json!("info fixture=rust-pty"));
    assert!(frames.iter().all(|frame| frame["frame_type"] == "line"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn max_frames_stops_after_exact_limit() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;
    fixture
        .run_script(vec![
            Action::Emit(b"one\r\n".to_vec()),
            Action::Delay(Duration::from_millis(20)),
            Action::Emit(b"two\r\n".to_vec()),
            Action::Delay(Duration::from_millis(20)),
            Action::Emit(b"three\r\n".to_vec()),
        ])
        .await?;
    let result = read(
        &client,
        &id,
        json!({
            "timeout_ms": 2000,
            "rx_framing": { "type": "line", "max_frames": 2 }
        }),
    )
    .await?;
    assert_eq!(structured(&result)?["stop_reason"], json!("max_frames"));
    let frames = frames(&result)?;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0]["data"], json!("one"));
    assert_eq!(frames[1]["data"], json!("two"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn framing_plus_match_returns_matching_frame_and_index() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture, json!({})).await?;
    let reader = {
        let peer = client.peer().clone();
        let id = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "from": { "type": "now" },
                    "encoding": "utf8",
                    "timeout_ms": 2000,
                    "rx_framing": { "type": "line" },
                    "match": { "pattern": "pong" }
                }),
            ))
            .await
        })
    };
    fixture
        .run_script(vec![
            Action::Delay(Duration::from_millis(50)),
            Action::Emit(b"noise\r\npong seq=1\r\n".to_vec()),
        ])
        .await?;
    assert!(!reader.is_finished());
    let result = tokio::time::timeout(Duration::from_secs(3), reader)
        .await
        .context("framing+match read timeout")?
        .context("framing+match task join")??;
    assert_success(&result, "framing+match read")?;
    assert_eq!(structured(&result)?["stop_reason"], json!("match_found"));
    assert_eq!(structured(&result)?["matched"], json!(true));
    assert!(structured(&result)?["match_index"].is_number());
    let frames = frames(&result)?;
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[1]["data"], json!("pong seq=1"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn delimiter_length_prefixed_and_start_end_decode_exact_payloads() -> Result<()> {
    let cases = [
        (
            vec![b'|', b'p', b'o', b'n', b'g', b'|'],
            json!({ "type": "delimiter", "delimiter": "|" }),
            "delimiter",
            "pong",
            1usize,
        ),
        (
            vec![4, b'p', b'o', b'n', b'g'],
            json!({ "type": "length_prefixed", "prefix_size": 1, "endianness": "big" }),
            "length_prefixed",
            "pong",
            0usize,
        ),
        (
            b"noise<<pong>>".to_vec(),
            json!({ "type": "start_end", "start": ["<<"], "end": ">>" }),
            "start_end",
            "pong",
            0usize,
        ),
    ];
    for (wire, framing, frame_type, expected, index) in cases {
        let fixture =
            DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
        let server = TestServer::start().await;
        let (client, _) = connect_client(&server).await?;
        let id = open_fixture(&client, &fixture, json!({})).await?;
        fixture.run_script(vec![Action::Emit(wire)]).await?;
        let result = read(
            &client,
            &id,
            json!({ "timeout_ms": 100, "rx_framing": framing }),
        )
        .await?;
        let frames = frames(&result)?;
        assert_eq!(frames[index]["data"], json!(expected));
        assert_eq!(frames[index]["frame_type"], json!(frame_type));
        close(&client, &id).await?;
        client.cancel().await.ok();
        fixture.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn explicit_line_endings_split_with_documented_terminator_semantics() -> Result<()> {
    let cases = [
        ("lf", b"alpha\r\nbeta\n".as_slice(), ["alpha\r", "beta"]),
        ("cr", b"alpha\rbeta\r".as_slice(), ["alpha", "beta"]),
        ("crlf", b"alpha\r\nbeta\r\n".as_slice(), ["alpha", "beta"]),
    ];
    for (ending, wire, expected) in cases {
        let fixture =
            DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
        let server = TestServer::start().await;
        let (client, _) = connect_client(&server).await?;
        let id = open_fixture(&client, &fixture, json!({})).await?;
        fixture
            .run_script(vec![Action::Emit(wire.to_vec())])
            .await?;
        let result = read(
            &client,
            &id,
            json!({
                "timeout_ms": 100,
                "rx_framing": { "type": "line", "ending": ending }
            }),
        )
        .await?;
        let frames = frames(&result)?;
        let actual: Vec<_> = frames
            .iter()
            .map(|frame| frame["data"].as_str().unwrap_or_default())
            .collect();
        assert_eq!(actual, expected, "ending={ending}");
        close(&client, &id).await?;
        client.cancel().await.ok();
        fixture.shutdown().await?;
    }
    Ok(())
}

#[tokio::test]
async fn call_time_line_framing_beats_connection_delimiter_default() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(
        &client,
        &fixture,
        json!({ "rx_framing": { "type": "delimiter", "delimiter": "|" } }),
    )
    .await?;
    fixture
        .run_script(vec![Action::Emit(b"alpha\r\nbeta\r\n".to_vec())])
        .await?;
    let result = read(
        &client,
        &id,
        json!({
            "timeout_ms": 100,
            "rx_framing": { "type": "line", "ending": "auto" }
        }),
    )
    .await?;
    let frames = frames(&result)?;
    assert_eq!(frames.len(), 2);
    assert!(frames.iter().all(|frame| frame["frame_type"] == "line"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn tx_framing_modes_produce_exact_independent_wire_vectors() -> Result<()> {
    let cases = [
        (
            json!({ "type": "delimiter", "delimiter": "|" }),
            b"ping|".to_vec(),
        ),
        (
            json!({ "type": "length_prefixed", "prefix_size": 1, "endianness": "big" }),
            vec![4, b'p', b'i', b'n', b'g'],
        ),
        (
            json!({ "type": "start_end", "start": ["<<"], "end": ">>" }),
            b"<<ping>>".to_vec(),
        ),
        (
            json!({ "type": "slip" }),
            vec![0xC0, b'p', b'i', b'n', b'g', 0xC0],
        ),
    ];
    for (framing, expected) in cases {
        let mut fixture =
            DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
        let server = TestServer::start().await;
        let (client, _) = connect_client(&server).await?;
        let id = open_fixture(&client, &fixture, json!({})).await?;
        let result = call_tool(
            &client,
            "write",
            json!({ "connection_id": id, "data": "ping", "tx_framing": framing }),
        )
        .await?;
        assert_success(&result, "framed write")?;
        let mut actual = Vec::new();
        while actual.len() < expected.len() {
            actual.extend(fixture.next_raw_input(WAIT).await?);
        }
        assert_eq!(actual, expected);
        close(&client, &id).await?;
        client.cancel().await.ok();
        fixture.shutdown().await?;
    }
    Ok(())
}

async fn open_fixture(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    fixture: &DeviceFixture,
    extra: Value,
) -> Result<String> {
    let mut args = json!({
        "port": fixture.port_path().to_string_lossy(),
        "baud_rate": 115200
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

async fn read(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    extra: Value,
) -> Result<CallToolResult> {
    let mut args = json!({ "connection_id": connection_id, "encoding": "utf8" });
    if let (Value::Object(args), Value::Object(extra)) = (&mut args, extra) {
        args.extend(extra);
    }
    let result = call_tool(client, "read", args).await?;
    assert_success(&result, "read")?;
    Ok(result)
}

fn frames(result: &CallToolResult) -> Result<&Vec<Value>> {
    structured(result)?["frames"]
        .as_array()
        .context("read result missing frames array")
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
