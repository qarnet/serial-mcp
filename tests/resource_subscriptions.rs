//! Modern `subscriptions/listen` resource subscriptions.
//!
//! These tests use real in-process HTTP MCP transport with typed modern
//! `2026-07-28` clients. Each request may reach a fresh handler instance, so
//! receiving shared updates proves the process-wide event hub. Watcher
//! behavior is driven through an injected mutable [`MutexPortProvider`] with
//! a short poll interval.
//!
//! Notifications are availability hints only. This suite asserts notification
//! URIs and verifies that referenced state (ring bytes, cursor, and resources)
//! is immediately observable through public tools rather than private `Arc`
//! identity or helper call counts.

mod common;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rmcp::model::{ServerNotification, SubscriptionFilter};
use rmcp::service::{RoleClient, Subscription, SubscriptionEnd};
use serde_json::json;

use common::controlled::controlled_connection;
use common::{connect_2025_11_25_client, connect_2026_07_28_client, TestServer};

/// Start a server around a fresh manager and controlled in-memory connection.
/// Return `(server, connection_id, rx-injector state)`.
async fn controlled_server() -> (TestServer, String, Arc<common::controlled::ControlledState>) {
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let (conn, state) = controlled_connection("/dev/controlled", 65536);
    let connection_id = manager.insert(conn).await.unwrap();
    let server = TestServer::builder(manager).start().await;
    (server, connection_id, state)
}

/// Start the connection's RX pump through a public read (creates the RX
/// session; the pump runs from then on).
async fn start_pump(
    client: &rmcp::service::RunningService<RoleClient, common::VersionedClientHandler>,
    connection_id: &str,
) {
    let result = client
        .peer()
        .call_tool(common::tool_request(
            "read",
            json!({"connection_id": connection_id, "timeout_ms": 150}),
        ))
        .await
        .expect("initial read call");
    assert_eq!(result.is_error, Some(false), "{result:?}");
}

/// Receive exactly `n` resource-updated notifications and return their URIs in
/// arrival order. The timeout guards against hangs.
async fn collect_updates(sub: &mut Subscription, n: usize) -> Vec<String> {
    let mut uris = Vec::new();
    for _ in 0..n {
        let notif = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("resource update within the hang guard")
            .expect("subscription must not error while collecting")
            .expect("subscription must not end while collecting");
        match notif {
            ServerNotification::ResourceUpdatedNotification(update) => uris.push(update.params.uri),
            other => panic!("unexpected notification: {other:?}"),
        }
    }
    uris
}

/// Assert that a received URI set equals the expected set without considering
/// order. Hints are availability signals, not a ledger.
fn assert_hint_set(received: Vec<String>, expected: &[&str]) {
    let mut received: Vec<&str> = received.iter().map(String::as_str).collect();
    let mut expected: Vec<&str> = expected.to_vec();
    received.sort_unstable();
    expected.sort_unstable();
    assert_eq!(received, expected, "expected URI hints");
}

/// Assert that no notification arrives within `window`. This bounds a
/// wall-clock window over short intervals in the watcher and hub paths under
/// test.
async fn assert_no_notification(sub: &mut Subscription, window: Duration) {
    match tokio::time::timeout(window, sub.next()).await {
        Err(_) => {}
        Ok(Ok(Some(notif))) => panic!("unexpected notification: {notif:?}"),
        Ok(Ok(None)) => panic!("subscription ended unexpectedly"),
        Ok(Err(e)) => panic!("subscription error: {e}"),
    }
}

#[tokio::test]
async fn discovery_2026_07_28_advertises_subscriptions_2025_11_25_initialize_does_not() -> Result<()>
{
    let server = TestServer::start().await;

    let (client_2026_07_28, _) = connect_2026_07_28_client(&server).await?;
    let caps = client_2026_07_28
        .peer_info()
        .expect("2026-07-28 peer info")
        .capabilities
        .clone();
    assert_eq!(
        serde_json::to_value(caps).unwrap(),
        json!({
            "completions": {},
            "prompts": {},
            "resources": {"subscribe": true},
            "tools": {},
        }),
        "2026-07-28 discovery advertises resources.subscribe and no listChanged"
    );

    let (client_2025_11_25, _) = connect_2025_11_25_client(&server).await?;
    let caps = client_2025_11_25
        .peer_info()
        .expect("2025-11-25 peer info")
        .capabilities
        .clone();
    assert_eq!(
        serde_json::to_value(caps).unwrap(),
        json!({
            "completions": {},
            "prompts": {},
            "resources": {},
            "tools": {},
        }),
        "2025-11-25 initialize keeps resource subscriptions disabled"
    );
    Ok(())
}

#[tokio::test]
async fn acknowledgement_contains_only_accepted_valid_resource_uris_in_first_occurrence_order(
) -> Result<()> {
    let server = TestServer::start().await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    let requested = SubscriptionFilter::builder()
        .resource_subscriptions([
            "serial://ports",
            "serial://connections",
            "serial://ports", // Duplicate preserves first-occurrence order.
            "serial://connections/{id}", // Template must be stripped.
            "serial://connections/", // Empty identifier must be stripped.
            "https://example.com/x", // Unknown scheme must be stripped.
            "serial://connections/abc-123",
            "serial://other", // Unknown URI must be stripped.
        ])
        .tools_list_changed()
        .prompts_list_changed()
        .resources_list_changed()
        .build();

    let mut sub = client.peer().listen(requested).await?;
    let acknowledged = sub.acknowledged().clone();
    let accepted_set = [
        "serial://ports".to_string(),
        "serial://connections".to_string(),
        "serial://connections/abc-123".to_string(),
    ];

    // The server-side contract is a deduplicated accepted set in
    // first-occurrence order.
    assert_eq!(
        {
            let mut seen = std::collections::HashSet::new();
            acknowledged
                .resource_subscriptions
                .iter()
                .flatten()
                .filter(|uri| seen.insert((*uri).clone()))
                .cloned()
                .collect::<Vec<String>>()
        },
        accepted_set,
        "acknowledged URIs, deduplicated, equal the accepted set in first-occurrence order"
    );

    // No invalid or unsupported URI may leak into the raw acknowledged Vec.
    let acknowledged_uris = acknowledged
        .resource_subscriptions
        .as_deref()
        .unwrap_or(&[]);
    for uri in acknowledged_uris {
        assert!(
            accepted_set.contains(uri),
            "acknowledged URI must be accepted+valid, got {uri:?}"
        );
    }

    // rmcp computes the final accepted filter with
    // `requested.intersection(&candidate).intersection(&advertised)`. The
    // intersections are left-biased over the requested list, so a repeated
    // requested valid URI may legitimately appear more than once in the raw
    // acknowledged Vec. The server-side `accepted_subscription_filter` and
    // listen loop both deduplicate, so the echo is harmless.
    assert!(
        acknowledged_uris.contains(&"serial://ports".to_string()),
        "acknowledged set contains the accepted ports URI"
    );

    assert!(
        acknowledged.tools_list_changed.is_none()
            && acknowledged.prompts_list_changed.is_none()
            && acknowledged.resources_list_changed.is_none(),
        "all list-change flags stripped from the accepted filter"
    );
    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn acknowledgement_with_no_valid_resources_is_an_empty_accepted_filter() -> Result<()> {
    let server = TestServer::start().await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    // With every requested URI invalid, the acknowledged resource list is
    // None rather than a fake URI.
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscriptions(["serial://connections/{id}", "https://example.com/x"])
                .build(),
        )
        .await?;
    assert!(
        sub.acknowledged().resource_subscriptions.is_none(),
        "no valid resource accepted -> None, not a fake URI"
    );
    sub.cancel().await?;

    // With nothing requested, the accepted filter is also empty. Listen still
    // acknowledges because the handler remains available.
    let mut sub = client.peer().listen(SubscriptionFilter::new()).await?;
    assert!(sub.acknowledged().resource_subscriptions.is_none());
    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn listen_2025_11_25_is_method_not_found() -> Result<()> {
    let server = TestServer::start().await;
    let (legacy, _) = connect_2025_11_25_client(&server).await?;
    let result = legacy
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("serial://ports")
                .build(),
        )
        .await;
    assert!(
        result.is_err(),
        "2025-11-25 clients must not reach the modern listen surface: {result:?}"
    );
    legacy.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn rx_append_notification_arrives_only_after_bytes_readable_and_does_not_move_cursor(
) -> Result<()> {
    let (server, connection_id, state) = controlled_server().await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    // Start the pump with a drained read. No data is available, so the shared
    // cursor stays at 0.
    start_pump(&client, &connection_id).await;

    // Subscribe to the connection detail URI.
    let detail_uri = format!("serial://connections/{connection_id}");
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription(detail_uri.clone())
                .build(),
        )
        .await?;

    // No data is pending before RX.
    assert_no_notification(&mut sub, Duration::from_millis(200)).await;

    // Inject bytes. The pump appends them before publishing the notification.
    state.inject_rx(b"first");
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &[&detail_uri]);

    // The bytes are readable immediately, and the notification does not move
    // the shared read cursor. The cursor remains 0 because no read consumed
    // data, so a read from the cursor returns appended bytes from offset 0.
    let read = client
        .peer()
        .call_tool(common::tool_request(
            "read",
            json!({"connection_id": connection_id, "from": {"type": "cursor"}}),
        ))
        .await
        .expect("read after notification");
    assert_eq!(read.is_error, Some(false), "{read:?}");
    let structured = read.structured_content.expect("structured content");
    assert_eq!(structured["data"], json!("first"));
    assert_eq!(structured["from_offset"], json!(0));
    assert_eq!(structured["next_offset"], json!(5));

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn read_works_without_any_listener() -> Result<()> {
    let (server, connection_id, state) = controlled_server().await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    // No subscription exists, so the read path must remain fully usable.
    start_pump(&client, &connection_id).await;
    state.inject_rx(b"unsolicited");
    tokio::time::sleep(Duration::from_millis(200)).await;

    let read = tokio::time::timeout(
        Duration::from_secs(5),
        client.peer().call_tool(common::tool_request(
            "read",
            json!({"connection_id": connection_id, "from": {"type": "cursor"}}),
        )),
    )
    .await
    .expect("read call within hang guard")
    .expect("read call");
    assert_eq!(read.is_error, Some(false), "{read:?}");
    let structured = read.structured_content.expect("structured content");
    assert_eq!(structured["data"], json!("unsolicited"));

    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn two_stateless_listeners_receive_same_update_independently() -> Result<()> {
    let (server, connection_id, state) = controlled_server().await;
    let (client_a, _) = connect_2026_07_28_client(&server).await?;
    let (client_b, _) = connect_2026_07_28_client(&server).await?;
    start_pump(&client_a, &connection_id).await;

    let detail_uri = format!("serial://connections/{connection_id}");
    let mut sub_a = client_a
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription(detail_uri.clone())
                .build(),
        )
        .await?;
    let mut sub_b = client_b
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription(detail_uri.clone())
                .build(),
        )
        .await?;

    state.inject_rx(b"shared");
    let a_uris = collect_updates(&mut sub_a, 1).await;
    let b_uris = collect_updates(&mut sub_b, 1).await;
    assert_hint_set(a_uris, &[&detail_uri]);
    assert_hint_set(b_uris, &[&detail_uri]);
    // Both clients see the same bytes after their own notification. The
    // shared read cursor is process-wide (covered by
    // `stateless_requests_share_session_ring_and_cursor`), so each client
    // replays the same absolute offset non-destructively.
    for client in [&client_a, &client_b] {
        let read = client
            .peer()
            .call_tool(common::tool_request(
                "read",
                json!({"connection_id": connection_id, "from": {"type": "offset", "offset": 0}}),
            ))
            .await
            .expect("read after shared notification");
        let structured = read.structured_content.expect("structured content");
        assert_eq!(structured["data"], json!("shared"));
    }

    sub_a.cancel().await?;
    sub_b.cancel().await?;
    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn cancelling_one_listener_leaves_the_other_active() -> Result<()> {
    let (server, connection_id, state) = controlled_server().await;
    let (client_a, _) = connect_2026_07_28_client(&server).await?;
    let (client_b, _) = connect_2026_07_28_client(&server).await?;
    start_pump(&client_a, &connection_id).await;

    let detail_uri = format!("serial://connections/{connection_id}");
    let mut sub_a = client_a
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription(detail_uri.clone())
                .build(),
        )
        .await?;
    let mut sub_b = client_b
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription(detail_uri.clone())
                .build(),
        )
        .await?;

    // Cancelling A must leave B active.
    sub_a.cancel().await?;
    assert_eq!(sub_a.end(), Some(&SubscriptionEnd::Cancelled));

    state.inject_rx(b"b-only");
    let b_uris = collect_updates(&mut sub_b, 1).await;
    assert_hint_set(b_uris, &[&detail_uri]);
    // A's stream is over: `next()` completes with `None`, without hanging or
    // returning an error.
    let ended = tokio::time::timeout(Duration::from_secs(5), sub_a.next())
        .await
        .expect("A subscription must terminate after cancel")
        .expect("no service error after cancel");
    assert!(ended.is_none(), "A subscription ended, got {ended:?}");

    // B remains live, so another RX yields another update.
    state.inject_rx(b"b-again");
    let b_uris = collect_updates(&mut sub_b, 1).await;
    assert_hint_set(b_uris, &[&detail_uri]);

    sub_b.cancel().await?;
    client_a.cancel().await.ok();
    client_b.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn listener_ignores_unrelated_events() -> Result<()> {
    let (server, connection_id, state) = controlled_server().await;
    let (client, _) = connect_2026_07_28_client(&server).await?;
    start_pump(&client, &connection_id).await;

    // Listen to `serial://ports` only. RX appends publish detail, raw, and log
    // hints, not ports hints, so no notification should arrive.
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("serial://ports")
                .build(),
        )
        .await?;

    state.inject_rx(b"ignored-by-ports-listener");
    assert_no_notification(&mut sub, Duration::from_millis(300)).await;

    // The listener is still healthy: a ports event still arrives.
    server.hub.publish_ports_changed();
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn forced_hub_lag_yields_conservative_per_uri_recovery_without_blocking_publisher(
) -> Result<()> {
    // A tiny hub makes a synchronous publish burst force broadcast lag.
    let hub = Arc::new(serial_mcp::resource_events::ResourceEventHub::new(2));
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let server = TestServer::builder(manager)
        .resource_hub(Arc::clone(&hub))
        .start()
        .await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscriptions(["serial://ports", "serial://connections"])
                .build(),
        )
        .await?;

    // No await occurs between publishes, so the current-thread runtime cannot
    // schedule the server's listen task and its capacity-2 receiver must lag.
    // `publish_updated` is synchronous and never blocks, so this loop cannot
    // stall.
    for i in 0..10 {
        hub.publish_updated("serial://ports");
        hub.publish_updated("serial://connections/evt");
        let _ = i;
    }

    // Recovery emits exactly one update per accepted URI in accepted order. It
    // must not flood the listener or deadlock the publisher or pump.
    let uris = collect_updates(&mut sub, 2).await;
    assert_eq!(
        uris,
        vec![
            "serial://ports".to_string(),
            "serial://connections".to_string()
        ],
        "lag recovery notifies every accepted URI once, in accepted order"
    );

    // The listener remains healthy after recovery.
    hub.publish_ports_changed();
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn repeated_requested_uri_does_not_cause_duplicate_lag_recovery_notifications() -> Result<()>
{
    // rmcp echoes a repeated requested URI into the acknowledged filter
    // through the left-biased `requested.intersection(&candidate)`. The listen
    // loop deduplicates again, so lag recovery must notify each accepted URI
    // exactly once when the request repeats it.
    let hub = Arc::new(serial_mcp::resource_events::ResourceEventHub::new(2));
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let server = TestServer::builder(manager)
        .resource_hub(Arc::clone(&hub))
        .start()
        .await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscriptions([
                    "serial://ports",
                    "serial://ports",
                    "serial://connections",
                ])
                .build(),
        )
        .await?;

    // Force broadcast lag with a synchronous publish burst. The current-thread
    // runtime cannot schedule the listen task during the loop.
    for _ in 0..10 {
        hub.publish_ports_changed();
        hub.publish_connections_changed();
    }

    // Recovery emits exactly one notification per distinct accepted URI in
    // first-occurrence order. The repeated "serial://ports" request must not
    // produce a second ports notification.
    let uris = collect_updates(&mut sub, 2).await;
    assert_eq!(
        uris,
        vec![
            "serial://ports".to_string(),
            "serial://connections".to_string()
        ],
        "lag recovery notifies each distinct accepted URI once, first-occurrence order"
    );

    // Normal matching also emits one notification per ports publish. The
    // sink's accepted filter still contains the echoed duplicate, but the
    // listen loop emits from its deduplicated set.
    hub.publish_ports_changed();
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn stateless_requests_share_session_ring_and_cursor() -> Result<()> {
    // Modern HTTP is stateless: each request uses a fresh handler instance.
    // The process-wide RxSessionManager makes the session, ring, and shared
    // read cursor visible across distinct requests. This tests public behavior
    // rather than `Arc` identity.
    let (server, connection_id, state) = controlled_server().await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    // Request 1 uses handler instance A to start the pump. Nothing is available
    // to drain.
    start_pump(&client, &connection_id).await;

    // Request 2 uses handler instance B. `get_status` must see the same session
    // rather than a fresh empty ring.
    let status = client
        .peer()
        .call_tool(common::tool_request(
            "get_status",
            json!({"connection_id": connection_id}),
        ))
        .await
        .expect("get_status call");
    let s = status.structured_content.expect("status content");
    assert!(
        s["rx_buffer_size"].as_u64().unwrap() > 0,
        "session created by a previous stateless request must be visible"
    );

    // Inject bytes. The shared pump appends them to the shared ring.
    state.inject_rx(b"shared-ring");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Request 3 verifies that the shared ring advanced while the shared cursor
    // stayed at 0, leaving the bytes buffered but unread.
    let status = client
        .peer()
        .call_tool(common::tool_request(
            "get_status",
            json!({"connection_id": connection_id}),
        ))
        .await
        .expect("get_status after inject");
    let s = status.structured_content.expect("status content");
    assert_eq!(s["rx_end_offset"], json!(11));
    assert_eq!(s["rx_buffered_unread"], json!(11), "shared cursor still 0");
    assert_eq!(s["rx_cursor"], json!(0));

    // Request 4 reads from the shared cursor and consumes the bytes.
    let read = client
        .peer()
        .call_tool(common::tool_request(
            "read",
            json!({"connection_id": connection_id, "from": {"type": "cursor"}}),
        ))
        .await
        .expect("read call");
    assert_eq!(read.is_error, Some(false), "{read:?}");
    let structured = read.structured_content.expect("structured content");
    assert_eq!(structured["data"], json!("shared-ring"));
    assert_eq!(structured["from_offset"], json!(0));

    // Request 5 verifies that the read advanced the shared cursor. The next
    // request must observe the moved cursor rather than a fresh one.
    let status = client
        .peer()
        .call_tool(common::tool_request(
            "get_status",
            json!({"connection_id": connection_id}),
        ))
        .await
        .expect("get_status after read");
    let s = status.structured_content.expect("status content");
    assert_eq!(
        s["rx_cursor"],
        json!(11),
        "shared cursor advanced by the read"
    );
    assert_eq!(s["rx_buffered_unread"], json!(0));

    // Request 6 reads from the drained cursor and returns nothing.
    let read2 = client
        .peer()
        .call_tool(common::tool_request(
            "read",
            json!({"connection_id": connection_id, "from": {"type": "cursor"}, "timeout_ms": 200}),
        ))
        .await
        .expect("drained read call");
    assert_eq!(read2.is_error, Some(false), "{read2:?}");
    let structured = read2.structured_content.expect("structured content");
    assert_eq!(structured["data"], json!(""), "cursor consumed everything");

    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn open_and_close_emit_expected_uri_hints() -> Result<()> {
    // This cross-platform regression exercises resource hints from public
    // `open` and `close` through the real HTTP MCP surface. An injected
    // in-memory connection opener replaces the OS serial layer, so the test
    // has no PTY dependence: Linux PTYs are Linux-only, and opening a macOS
    // tty fails with ENOTTY. The opener builds its connection from the exact
    // config resolved by public `open`, so allowlist, identity,
    // profile-session, and hint behavior remain covered.
    let opener = common::controlled::ControlledConnectionOpener::new();
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::with_opener(opener));
    let port = "/dev/controlled-open-close";
    let provider = common::StaticPortProvider::new(vec![common::StaticPortProvider::usb_port(
        port,
        0x1234,
        0x5678,
        "SN-OPEN-CLOSE",
        Some("Test Device"),
        None,
    )]);
    let server = TestServer::builder(manager)
        .port_provider(provider)
        .start()
        .await;
    let (client, _) = connect_2026_07_28_client(&server).await?;

    // Listen for the connections list while opening.
    let mut conn_sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("serial://connections")
                .build(),
        )
        .await?;

    let open = client
        .peer()
        .call_tool(common::tool_request("open", json!({"port": port})))
        .await
        .expect("open call");
    assert_eq!(open.is_error, Some(false), "{open:?}");
    let connection_id = open
        .structured_content
        .as_ref()
        .expect("open structured content")["connection_id"]
        .as_str()
        .expect("connection_id string")
        .to_string();

    // Opening emits `serial://connections`; the detail URI is unknown until
    // the response arrives.
    let uris = collect_updates(&mut conn_sub, 1).await;
    assert_hint_set(uris, &["serial://connections"]);

    // Subscribe to the concrete detail URI before closing.
    let detail_uri = format!("serial://connections/{connection_id}");
    let mut detail_sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription(detail_uri.clone())
                .build(),
        )
        .await?;

    let close = client
        .peer()
        .call_tool(common::tool_request(
            "close",
            json!({"connection_id": connection_id}),
        ))
        .await
        .expect("close call");
    assert_eq!(close.is_error, Some(false), "{close:?}");

    // Closing emits `serial://connections` on the list subscription and the
    // closed detail URI on the detail subscription. Both are hints.
    let list_uris = collect_updates(&mut conn_sub, 1).await;
    assert_hint_set(list_uris, &["serial://connections"]);
    let detail_uris = collect_updates(&mut detail_sub, 1).await;
    assert_hint_set(detail_uris, &[&detail_uri]);

    // The closed connection's detail resource is gone from the registry.
    let read = client
        .peer()
        .call_tool(common::tool_request(
            "read",
            json!({"connection_id": connection_id, "timeout_ms": 100}),
        ))
        .await
        .expect("read on closed connection");
    assert_eq!(read.is_error, Some(true), "closed connection read errors");

    conn_sub.cancel().await?;
    detail_sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn state_and_log_operations_emit_expected_uri_hints() -> Result<()> {
    let (server, connection_id, _state) = controlled_server().await;
    let (client, _) = connect_2026_07_28_client(&server).await?;
    start_pump(&client, &connection_id).await;

    let detail_uri = format!("serial://connections/{connection_id}");
    let raw_uri = format!("{detail_uri}/raw");
    let log_uri = format!("{detail_uri}/log");
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscriptions([detail_uri.clone(), raw_uri.clone(), log_uri.clone()])
                .build(),
        )
        .await?;

    // Writing emits a detail hint because state changed.
    let write = client
        .peer()
        .call_tool(common::tool_request(
            "write",
            json!({"connection_id": connection_id, "data": "ping"}),
        ))
        .await
        .expect("write call");
    assert_eq!(write.is_error, Some(false), "{write:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Transacting emits a detail hint because its write half changes state.
    let transact = client
        .peer()
        .call_tool(common::tool_request(
            "transact",
            json!({"connection_id": connection_id, "data": "ping", "timeout_ms": 200}),
        ))
        .await
        .expect("transact call");
    assert_eq!(transact.is_error, Some(false), "{transact:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Sending a break emits a detail hint.
    let brk = client
        .peer()
        .call_tool(common::tool_request(
            "send_break",
            json!({"connection_id": connection_id, "duration_ms": 100}),
        ))
        .await
        .expect("send_break call");
    assert_eq!(brk.is_error, Some(false), "{brk:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Setting DTR and RTS emits a detail hint.
    let lines = client
        .peer()
        .call_tool(common::tool_request(
            "set_dtr_rts",
            json!({"connection_id": connection_id, "dtr": true, "rts": false}),
        ))
        .await
        .expect("set_dtr_rts call");
    assert_eq!(lines.is_error, Some(false), "{lines:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Reconfiguring emits a detail hint.
    let reconf = client
        .peer()
        .call_tool(common::tool_request(
            "reconfigure",
            json!({"connection_id": connection_id, "baud_rate": 9600}),
        ))
        .await
        .expect("reconfigure call");
    assert_eq!(reconf.is_error, Some(false), "{reconf:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Setting flow control emits a detail hint.
    let flow = client
        .peer()
        .call_tool(common::tool_request(
            "set_flow_control",
            json!({"connection_id": connection_id, "flow_control": "none"}),
        ))
        .await
        .expect("set_flow_control call");
    assert_eq!(flow.is_error, Some(false), "{flow:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Configuring connection defaults emits a detail hint.
    let cfg = client
        .peer()
        .call_tool(common::tool_request(
            "configure",
            json!({"connection_id": connection_id, "defaults": {"max_buffered_bytes": 16384}}),
        ))
        .await
        .expect("configure call");
    assert_eq!(cfg.is_error, Some(false), "{cfg:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Reconnecting emits a detail hint because state changed.
    let reconn = client
        .peer()
        .call_tool(common::tool_request(
            "reconnect",
            json!({"connection_id": connection_id}),
        ))
        .await
        .expect("reconnect call");
    assert_eq!(reconn.is_error, Some(false), "{reconn:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&detail_uri]);

    // Clearing the log emits only a log URI hint.
    let cleared = client
        .peer()
        .call_tool(common::tool_request(
            "clear_log",
            json!({"connection_id": connection_id}),
        ))
        .await
        .expect("clear_log call");
    assert_eq!(cleared.is_error, Some(false), "{cleared:?}");
    assert_hint_set(collect_updates(&mut sub, 1).await, &[&log_uri]);

    // Flushing both targets emits detail and raw hints because ring state
    // changed.
    let flushed = client
        .peer()
        .call_tool(common::tool_request(
            "flush",
            json!({"connection_id": connection_id, "target": "both"}),
        ))
        .await
        .expect("flush call");
    assert_eq!(flushed.is_error, Some(false), "{flushed:?}");
    assert_hint_set(collect_updates(&mut sub, 2).await, &[&detail_uri, &raw_uri]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

/// Return a distinct port identity for watcher tests.
fn watcher_port(name: &str, serial: &str) -> serial_mcp::serial::PortInfo {
    common::StaticPortProvider::usb_port(name, 0xABCD, 0x1234, serial, Some("Watch Device"), None)
}

#[tokio::test]
async fn port_watcher_baseline_is_captured_immediately_not_after_first_interval() -> Result<()> {
    // With a 1s poll interval, the baseline must be captured by the first
    // immediate poll rather than after one interval elapses. A mutation issued
    // right after connect/listen, before the first 1s interval, must therefore
    // emit an update. A sleep-first loop would swallow it as the silent
    // baseline, causing this test to time out.
    let a = watcher_port("/dev/ttyWatch0", "SN-A");
    let b = watcher_port("/dev/ttyWatch1", "SN-B");

    let provider = common::MutexPortProvider::new(vec![a.clone()]);
    let provider_trait: Arc<dyn serial_mcp::serial::PortProvider> = provider.clone();
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let server = TestServer::builder(manager)
        .port_provider(provider_trait)
        .port_watcher_interval(Duration::from_millis(1000))
        .start()
        .await;
    let (client, _) = connect_2026_07_28_client(&server).await?;
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("serial://ports")
                .build(),
        )
        .await?;

    // Mutate immediately. The watcher's first immediate poll already
    // established the baseline during connect/listen, so the update arrives at
    // the second poll (~1s). A sleep-first baseline would never emit it.
    provider.set_ports(vec![a.clone(), b.clone()]);
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn port_watcher_emits_update_on_mutation_and_none_on_reorder_unchanged_or_error() -> Result<()>
{
    let a = watcher_port("/dev/ttyWatch0", "SN-A");
    let b = watcher_port("/dev/ttyWatch1", "SN-B");
    let c = watcher_port("/dev/ttyWatch2", "SN-C");

    let provider = common::MutexPortProvider::new(vec![a.clone(), b.clone()]);
    let provider_trait: Arc<dyn serial_mcp::serial::PortProvider> = provider.clone();
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let server = TestServer::builder(manager)
        .port_provider(provider_trait)
        .port_watcher_interval(Duration::from_millis(25))
        .start()
        .await;
    let (client, _) = connect_2026_07_28_client(&server).await?;
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("serial://ports")
                .build(),
        )
        .await?;

    // The first poll runs immediately at watcher start, capturing the
    // baseline during connect/listen without an interval wait. No spurious
    // event should arrive while it settles; mutation-after-baseline checks
    // follow.
    assert_no_notification(&mut sub, Duration::from_millis(80)).await;

    // An unchanged snapshot emits no event.
    assert_no_notification(&mut sub, Duration::from_millis(80)).await;

    // Reorder changes OS enumeration order but not devices, so no event.
    provider.set_ports(vec![b.clone(), a.clone()]);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;

    // Adding a device emits exactly one ports update.
    provider.set_ports(vec![a.clone(), b.clone(), c.clone()]);
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    // Changing identity with the same path and a new serial emits one update.
    // The retained baseline is now [a2, b, c] (three devices).
    provider.set_ports(vec![
        watcher_port("/dev/ttyWatch0", "SN-A2"),
        b.clone(),
        c.clone(),
    ]);
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    // Enumeration failure emits no event and retains the prior successful
    // baseline.
    provider.set_fail(true);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;

    // Recovery compares against the retained baseline, so an unchanged
    // snapshot after the error is not a change.
    provider.set_fail(false);
    provider.set_ports(vec![
        watcher_port("/dev/ttyWatch0", "SN-A2"),
        b.clone(),
        c.clone(),
    ]);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;

    // A pure reorder of the retained snapshot is not a change either.
    provider.set_ports(vec![
        c.clone(),
        b.clone(),
        watcher_port("/dev/ttyWatch0", "SN-A2"),
    ]);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;
    provider.set_ports(vec![
        b.clone(),
        c.clone(),
        watcher_port("/dev/ttyWatch0", "SN-A2"),
    ]);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;

    // A real change after recovery is an event.
    provider.set_ports(vec![b.clone(), c.clone()]);
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}
