//! Durable real-PTY fixture behavior and public MCP boundary proofs.

#![cfg(target_os = "linux")]

mod common;

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use common::device_fixture::core::{
    Action, InputAssembler, OutputQueue, QueuePolicy, ScriptLimits,
};
use common::device_fixture::{DeviceFixture, DeviceFixtureConfig, FixtureExit, PingPeer};
use common::{connect_client, tool_request, TestServer};
use rmcp::model::CallToolResult;
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(2);

static FIXTURE_TEST_SERIALIZER: Mutex<()> = Mutex::new(());

fn run_fixture_test(future: impl Future<Output = Result<()>>) -> Result<()> {
    let _guard = FIXTURE_TEST_SERIALIZER
        .lock()
        .expect("fixture test serializer mutex poisoned");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("build fixture test runtime")?;
    runtime.block_on(future)
}

#[test]
fn input_assembler_preserves_fragmented_and_batched_commands() {
    let mut input = InputAssembler::new(64).expect("input assembler");
    assert!(input.push(b"pi").expect("first fragment").is_empty());
    assert_eq!(input.pending_len(), 2);
    assert_eq!(
        input
            .push(b"ng\rinfo\r\nping\n")
            .expect("remaining commands"),
        [b"ping".to_vec(), b"info".to_vec(), b"ping".to_vec()]
    );
    assert_eq!(input.pending_len(), 0);
}

#[test]
fn input_and_script_limits_reject_one_byte_over_without_partial_acceptance() {
    let mut input = InputAssembler::new(4).expect("input assembler");
    assert!(input.push(b"1234").expect("exact input limit").is_empty());
    let error = input.push(b"5").expect_err("input one byte over");
    assert!(error.to_string().contains("limit is 4"));
    assert_eq!(input.pending_len(), 4);

    let limits = ScriptLimits {
        max_actions: 2,
        max_supplied_bytes: 4,
    };
    limits
        .validate(&[
            Action::Emit(b"12".to_vec()),
            Action::Malformed(b"34".to_vec()),
        ])
        .expect("exact script limits");
    assert!(limits
        .validate(&[
            Action::Emit(b"12".to_vec()),
            Action::Delay(Duration::ZERO),
            Action::Close,
        ])
        .expect_err("action one over")
        .to_string()
        .contains("3 actions"));
    assert!(limits
        .validate(&[Action::Emit(b"12345".to_vec())])
        .expect_err("bytes one over")
        .to_string()
        .contains("supplies 5 bytes"));
}

#[test]
fn output_queue_exact_boundary_hold_drop_block_and_drain_are_observable() {
    let mut drop_new = OutputQueue::new(4, 2, QueuePolicy::DropNew).expect("drop queue");
    assert_eq!(drop_new.enqueue(b"1234").accepted, 4);
    let over = drop_new.enqueue(b"5");
    assert_eq!((over.accepted, over.dropped, over.blocked), (0, 1, 0));
    drop_new.set_held(true);
    assert!(drop_new.next_chunk().is_empty());
    drop_new.set_held(false);
    assert_eq!(drop_new.next_chunk(), b"12");
    drop_new.commit_drain(2).expect("commit first drain");
    assert_eq!(drop_new.next_chunk(), b"34");
    drop_new.commit_drain(2).expect("commit second drain");
    assert_eq!(drop_new.stats().drained, 4);
    assert_eq!(drop_new.stats().dropped, 1);

    let mut blocking = OutputQueue::new(2, 2, QueuePolicy::BlockProducer).expect("block queue");
    let report = blocking.enqueue(b"123");
    assert_eq!((report.accepted, report.dropped, report.blocked), (2, 0, 1));
    blocking.commit_drain(2).expect("drain blocking queue");
    assert_eq!(blocking.enqueue(&b"123"[2..]).accepted, 1);
    assert_eq!(blocking.stats().dropped, 0);
}

#[test]
fn fixture_scripts_expose_hold_fragmentation_delay_crash_and_shutdown() -> Result<()> {
    run_fixture_test(async {
        let mut fixture =
            DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
        fixture.set_hold(true).await?;
        fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
        fixture
            .run_script(vec![
                Action::EmitChunks(vec![b"A".to_vec(), b"BC".to_vec()]),
                Action::Delay(Duration::from_millis(5)),
                Action::Malformed(vec![0xDB, 0x41]),
            ])
            .await?;
        let held = fixture
            .wait_for(WAIT, |snapshot| snapshot.output_pending >= 1)
            .await?;
        assert_eq!(held.output_drained, 0);
        fixture.set_hold(false).await?;
        fixture
            .wait_for(WAIT, |snapshot| snapshot.output_drained == 5)
            .await?;
        fixture.run_script(vec![Action::Crash(42)]).await?;
        let terminal = fixture
            .wait_for(WAIT, |snapshot| snapshot.exit.is_terminal())
            .await?;
        assert_eq!(terminal.exit, FixtureExit::Crashed(42));
        let report = fixture.shutdown().await?;
        assert_eq!(report.snapshot.exit, FixtureExit::Crashed(42));
        assert!(!report.task_aborted);
        Ok(())
    })
}

#[test]
fn public_mcp_ping_hold_disconnect_replace_and_reconnect() -> Result<()> {
    run_fixture_test(async {
        let mut fixture =
            DeviceFixture::spawn(PingPeer::with_boot_banner(), DeviceFixtureConfig::default())
                .await?;
        let stable_path = fixture.port_path().to_string_lossy().into_owned();
        let first_physical_path = fixture.physical_path().to_owned();
        let server = TestServer::start().await;
        let (client, _) = connect_client(&server).await?;
        let connection_id = open_port(&client, &stable_path, true).await?;

        let boot = call_tool(
            &client,
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "match": { "pattern": "test device ready" }
            }),
        )
        .await?;
        assert_success(&boot, "boot banner")?;

        let first_pre_barrier_end = status(&client, &connection_id).await?["rx_end_offset"]
            .as_u64()
            .context("first barrier pre-end offset")?;
        fixture
            .run_script(vec![Action::Emit(b"FIRST-PONG-READY\r\n".to_vec())])
            .await?;
        let first_barrier_end =
            wait_for_rx_barrier(&client, &connection_id, first_pre_barrier_end).await?;

        fixture.set_hold(true).await?;
        fixture.wait_for(WAIT, |snapshot| snapshot.held).await?;
        call_tool(
            &client,
            "write",
            json!({ "connection_id": connection_id, "data": "pi" }),
        )
        .await
        .and_then(|result| assert_success(&result, "first ping fragment"))?;
        call_tool(
            &client,
            "write",
            json!({ "connection_id": connection_id, "data": "ng\r\n" }),
        )
        .await
        .and_then(|result| assert_success(&result, "second ping fragment"))?;
        fixture
            .wait_for(WAIT, |snapshot| {
                snapshot.commands_accepted == 1 && snapshot.output_pending > 0
            })
            .await?;

        let pending_match = {
            let peer = client.peer().clone();
            let connection_id = connection_id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": connection_id,
                        "from": { "type": "now" },
                        "timeout_ms": 2000,
                        "match": { "pattern": "pong seq=1" }
                    }),
                ))
                .await
            })
        };
        assert!(!pending_match.is_finished());
        wait_for_from_now_edge(&client, &connection_id, first_barrier_end).await?;
        assert!(
            !pending_match.is_finished(),
            "pong read completed before held output release"
        );
        fixture.set_hold(false).await?;
        let pong = tokio::time::timeout(WAIT, pending_match)
            .await
            .context("pong read timeout")?
            .context("pong task join")??;
        assert_success(&pong, "pong read")?;
        assert_eq!(structured(&pong)?["stop_reason"], json!("match_found"));

        let no_op_reconnect = call_tool(
            &client,
            "reconnect",
            json!({ "connection_id": connection_id }),
        )
        .await?;
        assert_success(&no_op_reconnect, "reconnect while open")?;
        let no_op_state = structured(&no_op_reconnect)?;
        assert_eq!(no_op_state["connection_id"], json!(connection_id));
        assert_eq!(no_op_state["state"], json!("open"));
        call_tool(
            &client,
            "write",
            json!({ "connection_id": connection_id, "data": "ping\r\n" }),
        )
        .await
        .and_then(|result| assert_success(&result, "post-no-op ping"))?;
        let no_op_pong = call_tool(
            &client,
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "match": { "pattern": "pong seq=2" }
            }),
        )
        .await?;
        assert_success(&no_op_pong, "post-no-op pong")?;
        assert_eq!(
            structured(&no_op_pong)?["stop_reason"],
            json!("match_found")
        );

        let disconnect_pre_barrier_end = status(&client, &connection_id).await?["rx_end_offset"]
            .as_u64()
            .context("disconnect barrier pre-end offset")?;
        fixture
            .run_script(vec![Action::Emit(b"DISCONNECT-READY\r\n".to_vec())])
            .await?;
        let disconnect_barrier_end =
            wait_for_rx_barrier(&client, &connection_id, disconnect_pre_barrier_end).await?;
        let disconnect_read = {
            let peer = client.peer().clone();
            let connection_id = connection_id.clone();
            tokio::spawn(async move {
                peer.call_tool(tool_request(
                    "read",
                    json!({
                        "connection_id": connection_id,
                        "from": { "type": "now" },
                        "timeout_ms": 2000,
                        "match": { "pattern": "never" }
                    }),
                ))
                .await
            })
        };
        wait_for_from_now_edge(&client, &connection_id, disconnect_barrier_end).await?;
        assert!(
            !disconnect_read.is_finished(),
            "read was not pending before fixture disconnect"
        );
        fixture.disconnect_peer().await?;
        let disconnected = tokio::time::timeout(WAIT, disconnect_read)
            .await
            .context("disconnect read timeout")?
            .context("disconnect task join")??;
        assert_success(&disconnected, "disconnect read")?;
        assert_eq!(
            structured(&disconnected)?["stop_reason"],
            json!("connection_closed")
        );

        fixture.replace_endpoint(PingPeer::default()).await?;
        assert_ne!(fixture.physical_path(), first_physical_path);
        let reconnect = call_tool(
            &client,
            "reconnect",
            json!({ "connection_id": connection_id }),
        )
        .await?;
        assert_success(&reconnect, "reconnect to replacement")?;
        call_tool(
            &client,
            "write",
            json!({ "connection_id": connection_id, "data": "ping\r\n" }),
        )
        .await
        .and_then(|result| assert_success(&result, "replacement ping"))?;
        let replacement_pong = call_tool(
            &client,
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "match": { "pattern": "pong seq=1" }
            }),
        )
        .await?;
        assert_success(&replacement_pong, "replacement pong")?;
        assert_eq!(
            structured(&replacement_pong)?["stop_reason"],
            json!("match_found")
        );

        call_tool(&client, "close", json!({ "connection_id": connection_id }))
            .await
            .and_then(|result| assert_success(&result, "close replacement"))?;
        client.cancel().await.ok();
        let report = fixture.shutdown().await?;
        assert_eq!(report.snapshot.exit, FixtureExit::Shutdown);
        assert!(!std::path::Path::new(&stable_path).exists());
        Ok(())
    })
}

#[test]
fn spawned_server_opens_fixture_through_shipped_binary() -> Result<()> {
    run_fixture_test(async {
        let fixture =
            DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
        let stable_path = fixture.port_path().to_string_lossy().into_owned();
        let server = common::spawned::SpawnedServer::start().await;
        let (client, _) = common::spawned::spawn_client(&server).await?;
        let connection_id = open_port(&client, &stable_path, false).await?;
        call_tool(
            &client,
            "write",
            json!({ "connection_id": connection_id, "data": "ping\r\n" }),
        )
        .await
        .and_then(|result| assert_success(&result, "spawned-server ping"))?;
        let pong = call_tool(
            &client,
            "read",
            json!({
                "connection_id": connection_id,
                "timeout_ms": 2000,
                "match": { "pattern": "pong seq=1" }
            }),
        )
        .await?;
        assert_success(&pong, "spawned-server pong")?;
        assert_eq!(
            structured(&pong)?["matched"],
            json!(true),
            "unexpected spawned-server read result: {:?}",
            structured(&pong)?
        );
        call_tool(&client, "close", json!({ "connection_id": connection_id }))
            .await
            .and_then(|result| assert_success(&result, "spawned-server close"))?;
        client.cancel().await.ok();
        fixture.shutdown().await?;
        Ok(())
    })
}

#[cfg(target_os = "linux")]
#[test]
fn repeated_fixture_shutdown_returns_file_descriptors_to_baseline() -> Result<()> {
    run_fixture_test(async {
        let baseline = std::fs::read_dir("/proc/self/fd")?.count();
        for _ in 0..100 {
            let fixture =
                DeviceFixture::spawn(PingPeer::default(), DeviceFixtureConfig::default()).await?;
            let stable_path = fixture.port_path().to_owned();
            let report = fixture.shutdown().await?;
            assert_eq!(report.snapshot.exit, FixtureExit::Shutdown);
            assert!(!report.task_aborted);
            assert!(!stable_path.exists());
        }
        tokio::task::yield_now().await;
        let final_count = std::fs::read_dir("/proc/self/fd")?.count();
        assert_eq!(final_count, baseline, "fixture leaked file descriptors");
        Ok(())
    })
}

async fn open_port<H: rmcp::handler::client::ClientHandler>(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, H>,
    path: &str,
    reconnect: bool,
) -> Result<String> {
    let reconnect_policy = reconnect.then(|| {
        json!({
            "enabled": false,
            "max_attempts": 3,
            "initial_delay_ms": 10,
            "max_delay_ms": 50,
            "backoff_multiplier": 1.5
        })
    });
    let mut args = json!({ "port": path, "baud_rate": 115200 });
    if let (Some(policy), Value::Object(map)) = (reconnect_policy, &mut args) {
        map.insert("reconnect_policy".to_owned(), policy);
    }
    let result = call_tool(client, "open", args).await?;
    assert_success(&result, "open fixture")?;
    structured(&result)?["connection_id"]
        .as_str()
        .map(str::to_owned)
        .context("open result missing connection_id")
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

async fn wait_for_rx_barrier(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    pre_barrier_end: u64,
) -> Result<u64> {
    tokio::time::timeout(WAIT, async {
        loop {
            let state = status(client, connection_id).await?;
            let rx_end = state["rx_end_offset"]
                .as_u64()
                .context("status rx_end_offset")?;
            let rx_cursor = state["rx_cursor"].as_u64().context("status rx_cursor")?;
            if rx_end > pre_barrier_end && rx_end > rx_cursor {
                return Ok(rx_end);
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("timed out waiting for RX barrier")?
}

async fn wait_for_from_now_edge(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, common::TestClientHandler>,
    connection_id: &str,
    barrier_end: u64,
) -> Result<()> {
    tokio::time::timeout(WAIT, async {
        loop {
            let state = status(client, connection_id).await?;
            let rx_end = state["rx_end_offset"]
                .as_u64()
                .context("status rx_end_offset")?;
            let rx_cursor = state["rx_cursor"].as_u64().context("status rx_cursor")?;
            if rx_end >= barrier_end && rx_cursor == rx_end {
                return Ok(());
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .context("timed out waiting for from=now live edge")?
}

async fn call_tool<H: rmcp::handler::client::ClientHandler>(
    client: &rmcp::service::RunningService<rmcp::service::RoleClient, H>,
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
