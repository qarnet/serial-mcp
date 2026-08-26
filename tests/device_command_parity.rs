//! Public-boundary command and lifecycle coverage for Rust PTY device fixture.
//!
//! Every case opens fixture PTY through public MCP tools.

#![cfg(target_os = "linux")]

mod common;

#[cfg(target_os = "linux")]
use std::io::{Read as _, Write as _};
#[cfg(target_os = "linux")]
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result};
use common::device_fixture::core::Action;
use common::device_fixture::{DeviceFixture, DeviceFixtureConfig, DevicePeer, FloodPeer, PingPeer};
use common::{connect_client, tool_request, TestServer};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};
#[cfg(target_os = "linux")]
use tokio::io::{AsyncBufReadExt, BufReader};
#[cfg(target_os = "linux")]
use tokio::process::Command;

const WAIT: Duration = Duration::from_secs(2);
const CONNECTION_NAME: &str = "rust-pty-device";

#[derive(Default)]
struct AckPeer {
    enabled: bool,
    ack_sequence: u64,
    ping_sequence: u64,
}

impl DevicePeer for AckPeer {
    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        match command {
            b"ack on" => {
                self.enabled = true;
                vec![Action::Emit(b"ack on\r\n".to_vec())]
            }
            b"ack off" => {
                self.enabled = false;
                vec![Action::Emit(b"ack off\r\n".to_vec())]
            }
            b"ping" => {
                self.ping_sequence = self.ping_sequence.saturating_add(1);
                let mut actions = Vec::new();
                if self.enabled {
                    actions.push(Action::Emit(
                        format!("ack {}\r\n", self.ack_sequence).into_bytes(),
                    ));
                    self.ack_sequence = self.ack_sequence.saturating_add(1);
                }
                actions.push(Action::Emit(
                    format!("pong seq={}\r\n", self.ping_sequence).into_bytes(),
                ));
                actions
            }
            _ => vec![Action::Emit(b"ERROR\r\n".to_vec())],
        }
    }
}

struct DelayedPingPeer {
    delay: Duration,
}

impl DevicePeer for DelayedPingPeer {
    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        if command == b"ping" {
            vec![
                Action::Delay(self.delay),
                Action::Emit(b"pong delayed\r\n".to_vec()),
            ]
        } else {
            vec![Action::Emit(b"ERROR\r\n".to_vec())]
        }
    }
}

#[tokio::test]
async fn ping_roundtrip_uses_real_path_and_literal_match() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    write(&client, &id, "ping\r\n").await?;
    let result = read_match(&client, &id, "pong seq=1", None).await?;
    assert_eq!(structured(&result)?["matched"], json!(true));
    assert_eq!(structured(&result)?["stop_reason"], json!("match_found"));
    assert!(structured(&result)?["data"]
        .as_str()
        .is_some_and(|data| data.contains("pong seq=1\r\n")));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn pending_read_receives_later_output_after_readiness_proven_hold() -> Result<()> {
    let mut fixture =
        DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    fixture.set_hold(true).await?;
    fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    let reader = {
        let peer = client.peer().clone();
        let id = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "from": { "type": "now" },
                    "timeout_ms": 2000,
                    "match": { "pattern": "pong seq=1" }
                }),
            ))
            .await
        })
    };
    tokio::task::yield_now().await;
    assert!(!reader.is_finished(), "read completed before later write");

    write(&client, &id, "ping\r\n").await?;
    fixture
        .wait_for(WAIT, |snapshot| {
            snapshot.commands_accepted == 1 && snapshot.output_pending > 0 && snapshot.held
        })
        .await?;
    assert!(
        !reader.is_finished(),
        "held peer output reached read before explicit release"
    );
    fixture.set_hold(false).await?;
    let result = tokio::time::timeout(WAIT, reader)
        .await
        .context("pending read timeout")?
        .context("pending read task join")??;
    assert_success(&result, "pending read")?;
    assert_eq!(structured(&result)?["stop_reason"], json!("match_found"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn split_writes_preserve_one_command_and_exact_wire_order() -> Result<()> {
    let mut fixture =
        DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    for fragment in ["pi", "n", "g\r", "\n"] {
        write(&client, &id, fragment).await?;
    }
    let mut raw = Vec::new();
    while raw.len() < b"ping\r\n".len() {
        raw.extend(fixture.next_raw_input(WAIT).await?);
    }
    assert_eq!(raw, b"ping\r\n");
    let observed = fixture.next_observed_command(WAIT).await?;
    assert_eq!(observed.command, b"ping");
    fixture
        .wait_for(WAIT, |snapshot| snapshot.commands_accepted == 1)
        .await?;
    let result = read_match(&client, &id, "pong seq=1", None).await?;
    assert_eq!(structured(&result)?["stop_reason"], json!("match_found"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn named_connection_summary_uses_fixture_stable_path() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let stable_path = fixture.port_path().to_string_lossy().into_owned();
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    let result = call_tool(&client, "list_connections", json!({})).await?;
    assert_success(&result, "list_connections")?;
    let summary = &structured(&result)?["connections"][0];
    assert_eq!(summary["connection_id"], json!(id));
    assert_eq!(summary["name"], json!(CONNECTION_NAME));
    assert_eq!(summary["port"], json!(stable_path));
    assert_eq!(summary["baud_rate"], json!(115200));
    assert_eq!(summary["flow_control"], json!("none"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn reopen_same_path_returns_distinct_id_and_only_fresh_generation() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;

    let first_id = open_fixture(&client, &fixture).await?;
    write(&client, &first_id, "ping\r\n").await?;
    let first = read_match(&client, &first_id, "pong seq=1", None).await?;
    assert_eq!(structured(&first)?["stop_reason"], json!("match_found"));
    close(&client, &first_id).await?;

    let second_id = open_fixture(&client, &fixture).await?;
    assert_ne!(second_id, first_id);
    let pending = {
        let peer = client.peer().clone();
        let second_id = second_id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": second_id,
                    "from": { "type": "now" },
                    "timeout_ms": 2000,
                    "match": { "pattern": "pong seq=2" }
                }),
            ))
            .await
        })
    };
    tokio::task::yield_now().await;
    assert!(
        !pending.is_finished(),
        "stale first pong satisfied fresh read"
    );
    write(&client, &second_id, "ping\r\n").await?;
    let second = tokio::time::timeout(WAIT, pending)
        .await
        .context("fresh reopen read timeout")?
        .context("fresh reopen task join")??;
    assert_success(&second, "fresh reopen read")?;
    assert_eq!(structured(&second)?["stop_reason"], json!("match_found"));
    let data = structured(&second)?["data"]
        .as_str()
        .context("fresh reopen data")?;
    assert!(data.contains("pong seq=2"));
    assert!(!data.contains("pong seq=1"));

    close(&client, &second_id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn status_reports_exact_io_deltas_and_activity() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;
    let before = status(&client, &id).await?;

    write(&client, &id, "ping\r\n").await?;
    let pong = read_match(&client, &id, "pong seq=1", None).await?;
    assert_eq!(structured(&pong)?["stop_reason"], json!("match_found"));
    let after = status(&client, &id).await?;

    assert_eq!(after["is_open"], json!(true));
    assert_eq!(
        after["tx_bytes"].as_u64().context("after tx_bytes")?
            - before["tx_bytes"].as_u64().context("before tx_bytes")?,
        6
    );
    assert_eq!(
        after["rx_bytes"].as_u64().context("after rx_bytes")?
            - before["rx_bytes"].as_u64().context("before rx_bytes")?,
        12
    );
    assert_eq!(
        after["write_ops"].as_u64().context("after write_ops")?
            - before["write_ops"].as_u64().context("before write_ops")?,
        1
    );
    assert!(!after["last_activity_ms"].is_null());

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn reconfigure_updates_status_and_connection_remains_functional() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    let changed = call_tool(
        &client,
        "reconfigure",
        json!({ "connection_id": id, "baud_rate": 38400 }),
    )
    .await?;
    assert_success(&changed, "reconfigure to 38400")?;
    assert_eq!(structured(&changed)?["baud_rate"], json!(38400));
    assert_eq!(status(&client, &id).await?["baud_rate"], json!(38400));
    write(&client, &id, "ping\r\n").await?;
    assert_eq!(
        structured(&read_match(&client, &id, "pong seq=1", None).await?)?["stop_reason"],
        json!("match_found")
    );

    let restored = call_tool(
        &client,
        "reconfigure",
        json!({ "connection_id": id, "baud_rate": 115200 }),
    )
    .await?;
    assert_success(&restored, "restore baud")?;
    assert_eq!(structured(&restored)?["baud_rate"], json!(115200));
    assert_eq!(status(&client, &id).await?["baud_rate"], json!(115200));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn ack_peer_orders_ack_before_response_and_stops_after_disable() -> Result<()> {
    let fixture = DeviceFixture::spawn(AckPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    write(&client, &id, "ack on\r\n").await?;
    read_match(&client, &id, "ack on", None).await?;
    for (sequence, pong_sequence) in [(0, 1), (1, 2)] {
        write(&client, &id, "ping\r\n").await?;
        let response = read_match(&client, &id, &format!("pong seq={pong_sequence}"), None).await?;
        let data = structured(&response)?["data"]
            .as_str()
            .context("ack response data")?;
        let ack = data
            .find(&format!("ack {sequence}\r\n"))
            .context("missing expected ack")?;
        let pong = data
            .find(&format!("pong seq={pong_sequence}\r\n"))
            .context("missing expected pong")?;
        assert!(ack < pong, "ack must precede response: {data:?}");
    }

    write(&client, &id, "ack off\r\n").await?;
    read_match(&client, &id, "ack off", None).await?;
    write(&client, &id, "ping\r\n").await?;
    let response = read_match(&client, &id, "pong seq=3", None).await?;
    let data = structured(&response)?["data"]
        .as_str()
        .context("post-disable response data")?;
    assert!(
        !data.contains("ack "),
        "ack appeared after disable: {data:?}"
    );

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn held_output_reports_nonzero_queue_then_drains_and_recovers() -> Result<()> {
    let config = DeviceFixtureConfig {
        output_capacity: 64,
        output_chunk_size: 4,
        ..DeviceFixtureConfig::default()
    };
    let mut fixture = DeviceFixture::spawn(PingPeer::default(), config).await?;
    fixture.set_hold(true).await?;
    fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;
    write(&client, &id, "ping\r\n").await?;
    let held = fixture
        .wait_for(WAIT, |snapshot| snapshot.output_pending == 12)
        .await?;
    assert_eq!(held.output_drained, 0);
    assert_eq!(held.output_dropped, 0);

    let pending = {
        let peer = client.peer().clone();
        let id = id.clone();
        tokio::spawn(async move {
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": id,
                    "from": { "type": "now" },
                    "timeout_ms": 2000,
                    "match": { "pattern": "pong seq=1" }
                }),
            ))
            .await
        })
    };
    tokio::task::yield_now().await;
    assert!(!pending.is_finished());
    fixture.set_hold(false).await?;
    let result = tokio::time::timeout(WAIT, pending)
        .await
        .context("held queue read timeout")?
        .context("held queue task join")??;
    assert_success(&result, "held queue release read")?;
    let drained = fixture
        .wait_for(WAIT, |snapshot| snapshot.output_pending == 0)
        .await?;
    assert_eq!(drained.output_drained, 12);

    write(&client, &id, "ping\r\n").await?;
    assert_eq!(
        structured(&read_match(&client, &id, "pong seq=2", None).await?)?["stop_reason"],
        json!("match_found")
    );
    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn flush_input_discards_known_old_marker_and_keeps_new_marker() -> Result<()> {
    let mut fixture =
        DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    fixture
        .run_script(vec![Action::Emit(b"OLD-MARKER\r\n".to_vec())])
        .await?;
    fixture
        .wait_for(WAIT, |snapshot| snapshot.output_drained >= 12)
        .await?;
    wait_for_rx_bytes(&client, &id, 12).await?;
    let flush = call_tool(
        &client,
        "flush",
        json!({ "connection_id": id, "target": "input" }),
    )
    .await?;
    assert_success(&flush, "flush input")?;

    fixture
        .run_script(vec![Action::Emit(b"NEW-MARKER\r\n".to_vec())])
        .await?;
    let result = read_match(&client, &id, "NEW-MARKER", None).await?;
    let data = structured(&result)?["data"]
        .as_str()
        .context("post-flush data")?;
    assert!(data.contains("NEW-MARKER"));
    assert!(!data.contains("OLD-MARKER"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn flush_after_command_acceptance_does_not_cancel_delayed_response() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(
        DelayedPingPeer {
            delay: Duration::from_millis(100),
        },
        DeviceFixtureConfig::default(),
    )
    .await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    write(&client, &id, "ping\r\n").await?;
    fixture
        .wait_for(WAIT, |snapshot| snapshot.commands_accepted == 1)
        .await?;
    let flush = call_tool(
        &client,
        "flush",
        json!({ "connection_id": id, "target": "both" }),
    )
    .await?;
    assert_success(&flush, "flush after accepted command")?;
    let response = read_match(&client, &id, "pong delayed", None).await?;
    assert_eq!(structured(&response)?["stop_reason"], json!("match_found"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn output_flush_after_full_delivery_preserves_later_traffic() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    write(&client, &id, "ping\r\n").await?;
    read_match(&client, &id, "pong seq=1", None).await?;
    let flush = call_tool(
        &client,
        "flush",
        json!({ "connection_id": id, "target": "output" }),
    )
    .await?;
    assert_success(&flush, "flush fully delivered output")?;
    write(&client, &id, "ping\r\n").await?;
    let second = read_match(&client, &id, "pong seq=2", None).await?;
    assert_eq!(structured(&second)?["stop_reason"], json!("match_found"));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn finite_flood_matcher_reaches_unique_completion_marker() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(FloodPeer, DeviceFixtureConfig::default()).await?;
    fixture.set_hold(true).await?;
    fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    let reader = start_live_read(&client, &id, "FLOOD-COMPLETE-9e7d");
    write(&client, &id, "flood complete\r\n").await?;
    fixture
        .wait_for(WAIT, |snapshot| {
            snapshot.commands_accepted == 1 && snapshot.output_pending >= 1024
        })
        .await?;
    assert!(
        !reader.is_finished(),
        "held finite flood reached read before explicit release"
    );
    fixture.set_hold(false).await?;
    let result = await_read(reader, "finite flood matcher").await?;
    assert_eq!(structured(&result)?["matched"], json!(true));
    assert_eq!(structured(&result)?["stop_reason"], json!("match_found"));
    assert!(
        structured(&result)?["data"]
            .as_str()
            .is_some_and(|data| data.contains("FLOOD-COMPLETE-9e7d")),
        "completion marker missing from public read result: {:?}",
        structured(&result)?
    );

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn live_buffer_budget_caps_finite_flood_with_exact_stop_metadata() -> Result<()> {
    let mut fixture = DeviceFixture::spawn(FloodPeer, DeviceFixtureConfig::default()).await?;
    fixture.set_hold(true).await?;
    fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    let configured = call_tool(
        &client,
        "configure",
        json!({
            "connection_id": id,
            "defaults": { "max_buffered_bytes": 256 }
        }),
    )
    .await?;
    assert_success(&configured, "configure live buffer budget")?;
    assert_eq!(structured(&configured)?["mode"], json!("connection"));
    assert_eq!(
        structured(&configured)?["defaults"]["max_buffered_bytes"],
        json!(256)
    );

    let reader = start_live_read(&client, &id, "never-present");
    write(&client, &id, "flood budget\r\n").await?;
    fixture
        .wait_for(WAIT, |snapshot| {
            snapshot.commands_accepted == 1 && snapshot.output_pending == 1024
        })
        .await?;
    assert!(
        !reader.is_finished(),
        "held budget flood reached read before explicit release"
    );
    fixture.set_hold(false).await?;
    let result = await_read(reader, "finite flood budget").await?;
    let body = structured(&result)?;
    assert_eq!(body["stop_reason"], json!("max_buffered_bytes"));
    assert_eq!(body["bytes_returned"], json!(256));
    assert!(
        body["bytes_observed"]
            .as_u64()
            .is_some_and(|observed| observed >= 256),
        "read must report observed flood bytes: {body:?}"
    );
    assert!(
        body["bytes_returned"]
            .as_u64()
            .is_some_and(|returned| returned <= 256),
        "read exceeded configured live budget: {body:?}"
    );
    assert_eq!(
        body["truncated"],
        json!(
            body["bytes_returned"].as_u64().expect("bytes_returned")
                < body["bytes_observed"].as_u64().expect("bytes_observed")
        ),
        "truncation indicator must agree with public byte counters: {body:?}"
    );

    let status_after = status(&client, &id).await?;
    assert_eq!(
        status_after["truncation_count"],
        json!(u64::from(body["truncated"].as_bool().expect("truncated"))),
        "connection truncation counter must reflect public result metadata"
    );

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn close_interrupts_readiness_proven_live_read_with_connection_closed() -> Result<()> {
    let mut fixture =
        DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;
    let before_rx = status(&client, &id).await?["rx_bytes"]
        .as_u64()
        .context("initial rx_bytes")?;
    let reader = start_live_read(&client, &id, "never-arrives");
    fixture
        .run_script(vec![Action::Emit(b"READ-READY-MARKER\r\n".to_vec())])
        .await?;
    fixture
        .wait_for(WAIT, |snapshot| snapshot.output_drained >= 19)
        .await?;
    wait_for_rx_bytes(&client, &id, before_rx + 19).await?;
    assert!(
        !reader.is_finished(),
        "unmatched from=now read completed after readiness marker"
    );
    close(&client, &id).await?;
    let result = await_read(reader, "close-owned read").await?;
    assert_success(&result, "close-owned read")?;
    assert_eq!(
        structured(&result)?["stop_reason"],
        json!("connection_closed"),
        "explicit close must own the pending read stop"
    );

    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn flow_control_none_at_open_and_live_set_are_reflected_in_summary() -> Result<()> {
    let fixture = DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture_with_extra(&client, &fixture, json!({ "flow_control": "none" })).await?;

    let initial = call_tool(&client, "list_connections", json!({})).await?;
    assert_success(&initial, "list_connections after open")?;
    assert_eq!(
        structured(&initial)?["connections"][0]["connection_id"],
        json!(id)
    );
    assert_eq!(
        structured(&initial)?["connections"][0]["flow_control"],
        json!("none")
    );

    let changed = call_tool(
        &client,
        "set_flow_control",
        json!({ "connection_id": id, "flow_control": "none" }),
    )
    .await?;
    assert_success(&changed, "set_flow_control none")?;
    assert_eq!(structured(&changed)?["connection_id"], json!(id));
    assert_eq!(structured(&changed)?["flow_control"], json!("none"));

    let summary = call_tool(&client, "list_connections", json!({})).await?;
    assert_success(&summary, "list_connections after set_flow_control")?;
    assert_eq!(
        structured(&summary)?["connections"][0]["connection_id"],
        json!(id)
    );
    assert_eq!(
        structured(&summary)?["connections"][0]["flow_control"],
        json!("none")
    );

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn capture_boot_arm_only_excludes_stale_bytes_and_preserves_shared_cursor() -> Result<()> {
    const POST_ARM_MARKER: &str = "POST-ARM-UNIQUE";

    let mut fixture =
        DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let id = open_fixture(&client, &fixture).await?;

    fixture
        .run_script(vec![Action::Emit(b"STALE-BOOT-MARKER\r\n".to_vec())])
        .await?;
    fixture
        .wait_for(WAIT, |snapshot| snapshot.output_drained >= 19)
        .await?;
    wait_for_rx_bytes(&client, &id, 19).await?;
    let stale = read_match(&client, &id, "STALE-BOOT-MARKER", None).await?;
    let cursor_before_capture = structured(&stale)?["next_offset"]
        .as_u64()
        .context("stale read next_offset")?;

    let before_capture = status(&client, &id).await?;
    let before_capture_read_ops = before_capture["read_ops"]
        .as_u64()
        .context("read_ops before capture")?;
    let (capture_started_tx, capture_started_rx) = tokio::sync::oneshot::channel::<()>();
    let capture = {
        let peer = client.peer().clone();
        let capture_id = id.clone();
        tokio::spawn(async move {
            let call = peer.call_tool(tool_request(
                "capture_boot",
                json!({
                    "connection_id": capture_id,
                    "reset": null,
                    "timeout_ms": 2000,
                    "match": { "pattern": POST_ARM_MARKER }
                }),
            ));
            tokio::pin!(call);
            tokio::select! {
                result = &mut call => result
                    .map_err(|error| anyhow::anyhow!("arm-only capture tool call failed: {error}")),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    capture_started_tx
                        .send(())
                        .map_err(|_| anyhow::anyhow!("capture readiness receiver dropped"))?;
                    call.await
                        .map_err(|error| anyhow::anyhow!("arm-only capture tool call failed: {error}"))
                }
            }
        })
    };
    let (command_start_tx, command_start_rx) = tokio::sync::oneshot::channel::<()>();
    let mut command_fixture = fixture;
    let command = tokio::spawn(async move {
        command_start_rx
            .await
            .map_err(|_| anyhow::anyhow!("post-arm command barrier dropped"))?;
        let accepted_before = command_fixture.snapshot().output_accepted;
        command_fixture
            .run_script(vec![
                Action::Delay(Duration::from_millis(100)),
                Action::Emit(format!("{POST_ARM_MARKER}\r\n").into_bytes()),
            ])
            .await?;
        command_fixture
            .wait_for(WAIT, |snapshot| {
                snapshot.output_accepted >= accepted_before + POST_ARM_MARKER.len() + 2
            })
            .await?;
        Ok::<DeviceFixture, anyhow::Error>(command_fixture)
    });
    tokio::time::timeout(WAIT, capture_started_rx)
        .await
        .context("arm-only capture did not enter bounded wait")?
        .context("arm-only capture readiness sender dropped")?;
    command_start_tx
        .send(())
        .map_err(|_| anyhow::anyhow!("post-arm command task stopped before start"))?;
    let fixture = tokio::time::timeout(WAIT, command)
        .await
        .context("post-arm fixture command timeout")?
        .context("post-arm fixture command task join")??;
    let capture = await_read(capture, "arm-only capture").await?;
    assert_success(&capture, "arm-only capture")?;
    let body = structured(&capture)?;
    assert!(body["reset"].is_null());
    let mark_offset = body["mark_offset"]
        .as_u64()
        .context("capture mark_offset")?;
    assert!(
        mark_offset >= cursor_before_capture,
        "capture mark cannot predate shared cursor: {body:?}"
    );
    assert!(
        body["read"]["from_offset"].is_null()
            || body["read"]["from_offset"] == json!(mark_offset),
        "capture read offset must be absent only after private match shaping, or equal its mark: {body:?}"
    );
    assert_eq!(body["read"]["stop_reason"], json!("match_found"));
    let captured = body["read"]["data"].as_str().context("capture read data")?;
    assert!(captured.contains(POST_ARM_MARKER));
    assert!(!captured.contains("STALE-BOOT-MARKER"));

    let after_capture = status(&client, &id).await?;
    assert_eq!(
        after_capture["read_ops"],
        json!(before_capture_read_ops + 1),
        "capture must complete one read pipeline operation"
    );
    let replay = read_match(
        &client,
        &id,
        POST_ARM_MARKER,
        Some(json!({ "type": "offset", "offset": mark_offset })),
    )
    .await?;
    assert_eq!(structured(&replay)?["stop_reason"], json!("match_found"));
    assert!(structured(&replay)?["data"]
        .as_str()
        .is_some_and(|data| data.contains(POST_ARM_MARKER)));

    let history = call_tool(
        &client,
        "read",
        json!({
            "connection_id": id,
            "from": { "type": "buffer_start" },
            "timeout_ms": 1000
        }),
    )
    .await?;
    assert_success(&history, "capture history replay")?;
    let history_data = structured(&history)?["data"]
        .as_str()
        .context("capture history replay data")?;
    assert!(history_data.contains("STALE-BOOT-MARKER"));
    assert!(history_data.contains(POST_ARM_MARKER));

    close(&client, &id).await?;
    client.cancel().await.ok();
    fixture.shutdown().await?;
    Ok(())
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn touch_write_causes_small_rust_child_peer_to_exit_42() -> Result<()> {
    let (mut child, port) = spawn_touch_exit_peer().await?;
    let server = TestServer::start().await;
    let (client, _) = connect_client(&server).await?;
    let open = call_tool(
        &client,
        "open",
        json!({ "port": port, "name": "touch-exit-peer", "baud_rate": 115200 }),
    )
    .await?;
    assert_success(&open, "open touch exit peer")?;
    let id = structured(&open)?["connection_id"]
        .as_str()
        .context("touch exit peer connection_id")?
        .to_owned();

    let touch = call_tool(
        &client,
        "write",
        json!({ "connection_id": id, "data": "touch\r\n" }),
    )
    .await?;
    assert_success(&touch, "write touch")?;
    assert_eq!(structured(&touch)?["bytes_written"], json!(7));

    let status = tokio::time::timeout(WAIT, child.wait())
        .await
        .context("touch child did not exit")?
        .context("wait for touch child")?;
    assert_eq!(
        status.code(),
        Some(42),
        "public touch write must drive child peer exit(42): {status}"
    );

    client.cancel().await.ok();
    Ok(())
}

/// Child-test entry point for the exit-code parity case. The parent test opens
/// its printed PTY path through MCP, sends `touch\r\n`, and observes this
/// process's real exit status. No fixture-local `FixtureExit::Crashed` stands
/// in for process behavior.
#[cfg(target_os = "linux")]
#[test]
fn touch_exit_peer_child() {
    if std::env::var_os("SERIAL_MCP_TOUCH_EXIT_PEER").is_none() {
        return;
    }

    use nix::pty::{openpty, OpenptyResult};
    use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
    use nix::unistd::ttyname;

    let OpenptyResult { master, slave } = openpty(None, None).expect("open touch peer PTY");
    let mut termios = tcgetattr(&slave).expect("read touch peer termios");
    cfmakeraw(&mut termios);
    tcsetattr(&slave, SetArg::TCSANOW, &termios).expect("set touch peer raw termios");
    let slave_path = ttyname(&slave).expect("resolve touch peer slave path");
    let mut master = std::fs::File::from(master);
    let flags = nix::fcntl::OFlag::from_bits_truncate(
        nix::fcntl::fcntl(&master, nix::fcntl::FcntlArg::F_GETFL)
            .expect("read touch peer master flags"),
    );
    nix::fcntl::fcntl(
        &master,
        nix::fcntl::FcntlArg::F_SETFL(flags & !nix::fcntl::OFlag::O_NONBLOCK),
    )
    .expect("set touch peer master blocking");
    let mut stdout = std::io::stdout().lock();
    writeln!(stdout, "PTY_PATH={}", slave_path.display()).expect("publish touch peer PTY path");
    stdout.flush().expect("flush touch peer PTY path");

    let mut pending = Vec::new();
    let mut buffer = [0u8; 64];
    loop {
        let count = master.read(&mut buffer).expect("read touch peer command");
        if count == 0 {
            std::process::exit(2);
        }
        pending.extend_from_slice(&buffer[..count]);
        if pending.ends_with(b"touch\r\n") {
            std::process::exit(42);
        }
        if pending.len() > 256 {
            std::process::exit(3);
        }
    }
}

async fn open_fixture(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    fixture: &DeviceFixture,
) -> Result<String> {
    open_fixture_with_extra(client, fixture, json!({})).await
}

async fn open_fixture_with_extra(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    fixture: &DeviceFixture,
    extra: Value,
) -> Result<String> {
    let mut args = json!({
        "port": fixture.port_path().to_string_lossy(),
        "name": CONNECTION_NAME,
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

fn start_live_read(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    pattern: &str,
) -> tokio::task::JoinHandle<Result<CallToolResult>> {
    let peer = client.peer().clone();
    let id = connection_id.to_owned();
    let pattern = pattern.to_owned();
    tokio::spawn(async move {
        peer.call_tool(tool_request(
            "read",
            json!({
                "connection_id": id,
                "from": { "type": "now" },
                "timeout_ms": 5000,
                "match": { "pattern": pattern }
            }),
        ))
        .await
        .map_err(|error| anyhow::anyhow!("live read tool call failed: {error}"))
    })
}

async fn await_read(
    reader: tokio::task::JoinHandle<Result<CallToolResult>>,
    operation: &str,
) -> Result<CallToolResult> {
    tokio::time::timeout(WAIT, reader)
        .await
        .with_context(|| format!("{operation} timeout"))?
        .with_context(|| format!("{operation} task join"))?
        .with_context(|| format!("{operation} tool call"))
}

async fn write(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    data: &str,
) -> Result<()> {
    let result = call_tool(
        client,
        "write",
        json!({ "connection_id": connection_id, "data": data }),
    )
    .await?;
    assert_success(&result, "write")
}

async fn read_match(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    pattern: &str,
    from: Option<Value>,
) -> Result<CallToolResult> {
    let mut args = json!({
        "connection_id": connection_id,
        "timeout_ms": 2000,
        "match": { "pattern": pattern }
    });
    if let (Some(from), Value::Object(map)) = (from, &mut args) {
        map.insert("from".to_owned(), from);
    }
    let result = call_tool(client, "read", args).await?;
    assert_success(&result, "read")?;
    Ok(result)
}

async fn close(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
) -> Result<()> {
    let result = call_tool(client, "close", json!({ "connection_id": connection_id })).await?;
    assert_success(&result, "close")
}

async fn status(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
) -> Result<Value> {
    let result = call_tool(
        client,
        "get_status",
        json!({ "connection_id": connection_id }),
    )
    .await?;
    assert_success(&result, "get_status")?;
    result
        .structured_content
        .context("get_status missing structured content")
}

async fn wait_for_rx_bytes(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    minimum: u64,
) -> Result<()> {
    tokio::time::timeout(WAIT, async {
        loop {
            let current = status(client, connection_id).await?["rx_bytes"]
                .as_u64()
                .context("status rx_bytes")?;
            if current >= minimum {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("timed out waiting for RX counter")?
}

#[cfg(target_os = "linux")]
async fn spawn_touch_exit_peer() -> Result<(tokio::process::Child, String)> {
    let executable = std::env::current_exe().context("resolve current test executable")?;
    let mut child = Command::new(executable)
        .args(["--exact", "touch_exit_peer_child", "--nocapture"])
        .env("SERIAL_MCP_TOUCH_EXIT_PEER", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .context("spawn touch exit child peer")?;
    let stdout = child
        .stdout
        .take()
        .context("touch exit child missing stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let port = tokio::time::timeout(WAIT, async {
        loop {
            let line = lines
                .next_line()
                .await
                .context("read touch exit child stdout")?
                .context("touch exit child exited before publishing PTY path")?;
            if let Some(path) = line.split_once("PTY_PATH=").map(|(_, path)| path) {
                return Ok::<String, anyhow::Error>(path.to_owned());
            }
        }
    })
    .await
    .context("timed out waiting for touch exit child PTY path")??;
    Ok((child, port))
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
