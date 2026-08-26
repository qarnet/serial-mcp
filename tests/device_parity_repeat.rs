//! Deterministic 100-iteration public-boundary repeat gate over the real Rust
//! PTY fixture.
//!
//! This remains ignored in the broad default suite so it does not multiply
//! runtime across every platform. Linux x86_64 CI invokes its exact test name.

#![cfg(target_os = "linux")]

mod common;

use std::time::Duration;

use anyhow::{Context, Result};
use common::device_fixture::core::Action;
use common::device_fixture::{DeviceFixture, DeviceFixtureConfig, FixtureExit, PingPeer};
use common::{connect_client, tool_request, TestClientHandler, TestServer};
use rmcp::model::CallToolResult;
use rmcp::service::{RoleClient, RunningService};
use serde_json::{json, Value};

const REPEAT_ITERATIONS: usize = 100;
/// Fixed seed for deterministic iteration data.
const REPEAT_SEED: u64 = 0x5048_4153_455F_4545;
const WAIT: Duration = Duration::from_secs(2);
const TOOL_CALL_TIMEOUT: Duration = Duration::from_secs(3);
const READ_TIMEOUT_MS: u64 = 1_500;
const FLOOD_BOUND: u64 = 256;
const FLOOD_BYTES: usize = 1_024;

type TestClient = RunningService<RoleClient, TestClientHandler>;

#[tokio::test]
#[ignore = "100-iteration public-boundary repeat gate"]
async fn public_boundary_repeat_gate() -> Result<()> {
    for iteration in 0..REPEAT_ITERATIONS {
        if let Err(error) = run_iteration(iteration).await {
            eprintln!(
                "100-iteration public-boundary repeat gate failed: iteration={iteration} seed=0x{REPEAT_SEED:016x}"
            );
            return Err(error.context(format!(
                "100-iteration public-boundary repeat gate iteration {iteration} failed with seed 0x{REPEAT_SEED:016x}"
            )));
        }
    }
    Ok(())
}

async fn run_iteration(iteration: usize) -> Result<()> {
    let tag = iteration_tag(iteration);
    let mut fixture =
        DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
    let stable_path = fixture.port_path().to_path_buf();
    let server = TestServer::start().await;
    let (client, _) = match connect_client(&server).await {
        Ok(client) => client,
        Err(error) => {
            server.shutdown_and_join().await;
            return Err(error.context("connect repeat MCP client"));
        }
    };
    let mut connection_id = None;

    let lifecycle_result: Result<()> = async {
        let id = open_fixture(&client, &fixture).await?;
        connection_id = Some(id.clone());

        let first_ping = transact_ping(&client, &id, "pong seq=1").await?;
        assert_transact_match(&first_ping, "pong seq=1")?;

        let configured = call_tool(
            &client,
            "configure",
            json!({
                "connection_id": id,
                "defaults": { "max_buffered_bytes": FLOOD_BOUND }
            }),
        )
        .await?;
        assert_success(&configured, "configure finite flood bound")?;
        let configured_body = structured(&configured)?;
        assert_eq!(configured_body["mode"], json!("connection"));
        assert_eq!(
            configured_body["defaults"]["max_buffered_bytes"],
            json!(FLOOD_BOUND)
        );

        let drained_before_flood = fixture.snapshot().output_drained;
        set_hold(&fixture, true).await?;
        fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
        let flood_pattern = format!("repeat-flood-miss-{tag}");
        let flood_reader = start_live_read(&client, &id, flood_pattern);
        tokio::task::yield_now().await;
        anyhow::ensure!(
            !flood_reader.is_finished(),
            "flood read completed before fixture emitted iteration {iteration}"
        );
        run_script(&fixture, vec![Action::Emit(deterministic_flood(iteration))]).await?;
        let held = fixture
            .wait_for(WAIT, |snapshot| {
                snapshot.held && snapshot.output_pending >= FLOOD_BYTES
            })
            .await?;
        assert_eq!(
            held.output_drained, drained_before_flood,
            "held output drained before release at iteration {iteration}: {held:?}"
        );
        anyhow::ensure!(
            !flood_reader.is_finished(),
            "held flood read completed before explicit release at iteration {iteration}"
        );
        set_hold(&fixture, false).await?;
        let flood = await_read(flood_reader, "finite flood read").await?;
        assert_success(&flood, "finite flood read")?;
        assert_flood_stop(structured(&flood)?)?;
        fixture
            .wait_for(WAIT, |snapshot| snapshot.output_pending == 0)
            .await?;

        let flushed = call_tool(
            &client,
            "flush",
            json!({ "connection_id": id, "target": "output" }),
        )
        .await?;
        assert_success(&flushed, "flush after delivered flood")?;
        let post_flush_ping = transact_ping(&client, &id, "pong seq=2").await?;
        assert_transact_match(&post_flush_ping, "pong seq=2")?;

        let before_disconnect_rx = status(&client, &id).await?["rx_bytes"]
            .as_u64()
            .context("status before peer disconnect missing rx_bytes")?;
        let drained_before_readiness = fixture.snapshot().output_drained;
        let disconnect_pattern = format!("repeat-disconnect-miss-{tag}");
        let disconnect_reader = start_live_read(&client, &id, disconnect_pattern);
        tokio::task::yield_now().await;
        anyhow::ensure!(
            !disconnect_reader.is_finished(),
            "disconnect read completed before readiness output at iteration {iteration}"
        );
        let readiness = format!("REPEAT-READINESS-{tag}\r\n");
        run_script(&fixture, vec![Action::Emit(readiness.as_bytes().to_vec())]).await?;
        fixture
            .wait_for(WAIT, |snapshot| {
                snapshot.output_drained >= drained_before_readiness.saturating_add(readiness.len())
            })
            .await?;
        wait_for_rx_bytes(
            &client,
            &id,
            before_disconnect_rx.saturating_add(readiness.len() as u64),
        )
        .await?;
        anyhow::ensure!(
            !disconnect_reader.is_finished(),
            "unmatched readiness output completed disconnect read at iteration {iteration}"
        );
        let disconnected_fixture = tokio::time::timeout(WAIT, fixture.disconnect_peer())
            .await
            .context("timed out disconnecting fixture peer")??;
        assert_eq!(disconnected_fixture.snapshot.exit, FixtureExit::PeerClosed);
        let disconnected = await_read(disconnect_reader, "peer disconnect read").await?;
        assert_success(&disconnected, "peer disconnect read")?;
        assert_eq!(
            structured(&disconnected)?["stop_reason"],
            json!("connection_closed")
        );

        tokio::time::timeout(WAIT, fixture.replace_endpoint(PingPeer::default()))
            .await
            .context("timed out replacing fixture endpoint")??;
        assert_eq!(fixture.port_path(), stable_path.as_path());
        assert_eq!(fixture.snapshot().generation, 2);
        let reconnected = call_tool(&client, "reconnect", json!({ "connection_id": id })).await?;
        assert_success(&reconnected, "reconnect replacement endpoint")?;
        let replacement_ping = transact_ping(&client, &id, "pong seq=1").await?;
        assert_transact_match(&replacement_ping, "pong seq=1")?;

        Ok(())
    }
    .await;

    let close_result = match connection_id.as_deref() {
        Some(id) => close(&client, id).await,
        None => Ok(()),
    };
    let client_result = match tokio::time::timeout(WAIT, client.cancel()).await {
        Ok(result) => result.context("close repeat MCP client"),
        Err(_) => Err(anyhow::anyhow!("timed out closing repeat MCP client")),
    };
    let fixture_result = match tokio::time::timeout(WAIT, fixture.shutdown()).await {
        Ok(result) => result,
        Err(_) => Err(anyhow::anyhow!("timed out shutting down repeat fixture")),
    };
    let server_result = match tokio::time::timeout(WAIT, server.shutdown_and_join()).await {
        Ok(()) => Ok(()),
        Err(_) => Err(anyhow::anyhow!("timed out shutting down repeat server")),
    };

    lifecycle_result?;
    close_result?;
    client_result?;
    server_result?;
    let shutdown = fixture_result?;
    assert_eq!(shutdown.snapshot.exit, FixtureExit::Shutdown);
    anyhow::ensure!(
        !shutdown.task_aborted,
        "fixture task required abort during repeat teardown at iteration {iteration}"
    );
    Ok(())
}

fn iteration_tag(iteration: usize) -> String {
    let value = REPEAT_SEED ^ (iteration as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    format!("{value:016x}")
}

fn deterministic_flood(iteration: usize) -> Vec<u8> {
    let tag = iteration_tag(iteration);
    let mut flood = Vec::with_capacity(FLOOD_BYTES);
    while flood.len() < FLOOD_BYTES {
        flood.extend_from_slice(tag.as_bytes());
        flood.push(b'|');
    }
    flood.truncate(FLOOD_BYTES);
    flood
}

async fn set_hold(fixture: &DeviceFixture, held: bool) -> Result<()> {
    tokio::time::timeout(WAIT, fixture.set_hold(held))
        .await
        .context("timed out setting fixture hold")??;
    Ok(())
}

async fn run_script(fixture: &DeviceFixture, actions: Vec<Action>) -> Result<()> {
    tokio::time::timeout(WAIT, fixture.run_script(actions))
        .await
        .context("timed out scheduling fixture script")??;
    Ok(())
}

async fn open_fixture(client: &TestClient, fixture: &DeviceFixture) -> Result<String> {
    let result = call_tool(
        client,
        "open",
        json!({
            "port": fixture.port_path().to_string_lossy(),
            "baud_rate": 115200,
            "profile_mode": "none"
        }),
    )
    .await?;
    assert_success(&result, "open fixture")?;
    structured(&result)?["connection_id"]
        .as_str()
        .map(str::to_owned)
        .context("open result missing connection_id")
}

async fn transact_ping(
    client: &TestClient,
    connection_id: &str,
    expected: &str,
) -> Result<CallToolResult> {
    let result = call_tool(
        client,
        "transact",
        json!({
            "connection_id": connection_id,
            "data": "ping\r\n",
            "timeout_ms": READ_TIMEOUT_MS,
            "match": { "pattern": expected }
        }),
    )
    .await?;
    assert_success(&result, "ping transact")?;
    Ok(result)
}

fn start_live_read(
    client: &TestClient,
    connection_id: &str,
    pattern: String,
) -> tokio::task::JoinHandle<Result<CallToolResult>> {
    let peer = client.peer().clone();
    let connection_id = connection_id.to_owned();
    tokio::spawn(async move {
        let result = tokio::time::timeout(
            TOOL_CALL_TIMEOUT,
            peer.call_tool(tool_request(
                "read",
                json!({
                    "connection_id": connection_id,
                    "from": { "type": "now" },
                    "timeout_ms": READ_TIMEOUT_MS,
                    "match": { "pattern": pattern }
                }),
            )),
        )
        .await
        .context("live read tool call timed out")?
        .context("live read tool call failed")?;
        assert_success(&result, "live read")?;
        Ok(result)
    })
}

async fn await_read(
    reader: tokio::task::JoinHandle<Result<CallToolResult>>,
    operation: &str,
) -> Result<CallToolResult> {
    tokio::time::timeout(TOOL_CALL_TIMEOUT, reader)
        .await
        .with_context(|| format!("{operation} timed out"))?
        .with_context(|| format!("{operation} task join failed"))?
        .with_context(|| format!("{operation} tool call failed"))
}

async fn status(client: &TestClient, connection_id: &str) -> Result<Value> {
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

async fn wait_for_rx_bytes(client: &TestClient, connection_id: &str, minimum: u64) -> Result<()> {
    tokio::time::timeout(WAIT, async {
        loop {
            let rx_bytes = status(client, connection_id).await?["rx_bytes"]
                .as_u64()
                .context("get_status missing rx_bytes")?;
            if rx_bytes >= minimum {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("timed out waiting for fixture bytes to reach serial-mcp")?
}

async fn close(client: &TestClient, connection_id: &str) -> Result<()> {
    let result = call_tool(client, "close", json!({ "connection_id": connection_id })).await?;
    assert_success(&result, "close repeat connection")
}

async fn call_tool(client: &TestClient, name: &'static str, args: Value) -> Result<CallToolResult> {
    tokio::time::timeout(
        TOOL_CALL_TIMEOUT,
        client.peer().call_tool(tool_request(name, args)),
    )
    .await
    .with_context(|| format!("{name} tool call timed out"))?
    .with_context(|| format!("{name} tool call failed"))
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

fn assert_transact_match(result: &CallToolResult, expected: &str) -> Result<()> {
    let read = structured(result)?
        .get("read")
        .context("transact result missing read half")?;
    assert_eq!(read["matched"], json!(true));
    assert_eq!(read["stop_reason"], json!("match_found"));
    anyhow::ensure!(
        read["data"]
            .as_str()
            .is_some_and(|data| data.contains(expected)),
        "transact result missing expected payload {expected:?}: {read:?}"
    );
    Ok(())
}

fn assert_flood_stop(body: &Value) -> Result<()> {
    assert_eq!(body["stop_reason"], json!("max_buffered_bytes"));
    assert_eq!(body["bytes_returned"], json!(FLOOD_BOUND));
    let observed = body["bytes_observed"]
        .as_u64()
        .context("flood result missing bytes_observed")?;
    let returned = body["bytes_returned"]
        .as_u64()
        .context("flood result missing bytes_returned")?;
    anyhow::ensure!(
        observed >= FLOOD_BOUND,
        "flood result observed too few bytes: {body:?}"
    );
    anyhow::ensure!(
        returned <= FLOOD_BOUND,
        "flood result exceeded configured bound: {body:?}"
    );
    assert_eq!(body["truncated"], json!(returned < observed));
    Ok(())
}
