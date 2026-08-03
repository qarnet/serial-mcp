//! Phase 3 — modern `subscriptions/listen` resource subscriptions.
//!
//! Real in-process HTTP MCP transport with typed modern `2026-07-28`
//! clients (stateless — every request may hit a fresh handler instance, so
//! receiving shared updates proves the process-wide event hub). Watcher
//! behavior is driven through an injected mutable [`MutexPortProvider`] with
//! a short poll interval.
//!
//! Notifications are availability hints only: this suite asserts the
//! notification URIs and that the referenced state (ring bytes, cursor,
//! resources) is immediately observable through the public tools — never
//! private Arc identity or helper call counts.

mod common;

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rmcp::model::{ServerNotification, SubscriptionFilter};
use rmcp::service::{RoleClient, Subscription, SubscriptionEnd};
use serde_json::json;

use common::controlled::controlled_connection;
use common::{connect_modern_client, TestServer};

/// Start a server around a fresh manager plus a controlled (in-memory)
/// connection, returning `(server, connection_id, rx-injector state)`.
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
    client: &rmcp::service::RunningService<RoleClient, common::TestClientHandler>,
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

/// Receive exactly `n` resource-updated notifications, returning their URIs
/// in arrival order. Hangs guarded.
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

/// Assert that a received URI set equals the expected set (order-insensitive
/// — hints are availability signals, not a ledger).
fn assert_hint_set(received: Vec<String>, expected: &[&str]) {
    let mut received: Vec<&str> = received.iter().map(String::as_str).collect();
    let mut expected: Vec<&str> = expected.to_vec();
    received.sort_unstable();
    expected.sort_unstable();
    assert_eq!(received, expected, "expected URI hints");
}

/// Assert that NO notification arrives within `window` (bounded wall-clock
/// window over short intervals — the watcher/hub paths under test).
async fn assert_no_notification(sub: &mut Subscription, window: Duration) {
    match tokio::time::timeout(window, sub.next()).await {
        Err(_) => {}
        Ok(Ok(Some(notif))) => panic!("unexpected notification: {notif:?}"),
        Ok(Ok(None)) => panic!("subscription ended unexpectedly"),
        Ok(Err(e)) => panic!("subscription error: {e}"),
    }
}

// =============================================================================
// Discovery + capability split
// =============================================================================

#[tokio::test]
async fn modern_discovery_advertises_resource_subscriptions_legacy_initialize_does_not(
) -> Result<()> {
    let server = TestServer::start().await;

    let (modern, _) = connect_modern_client(&server).await?;
    let caps = modern
        .peer_info()
        .expect("modern peer info")
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
        "modern discovery advertises resources.subscribe and no listChanged"
    );

    let (legacy, _) = common::connect_legacy_client(&server).await?;
    let caps = legacy
        .peer_info()
        .expect("legacy peer info")
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
        "legacy initialize keeps resource subscriptions disabled"
    );
    Ok(())
}

// =============================================================================
// Accepted filter
// =============================================================================

#[tokio::test]
async fn acknowledgement_contains_only_accepted_valid_resource_uris_in_first_occurrence_order(
) -> Result<()> {
    let server = TestServer::start().await;
    let (client, _) = connect_modern_client(&server).await?;

    let requested = SubscriptionFilter::builder()
        .resource_subscriptions([
            "serial://ports",
            "serial://connections",
            "serial://ports",            // duplicate — first order preserved
            "serial://connections/{id}", // template -> stripped
            "serial://connections/",     // empty id -> stripped
            "https://example.com/x",     // unknown scheme -> stripped
            "serial://connections/abc-123",
            "serial://other", // unknown URI -> stripped
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

    // The deduplicated accepted set (server-side contract, first-occurrence
    // order).
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

    // No invalid/unsupported URI may leak into the raw acknowledged Vec.
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

    // rmcp 3.0.1 computes the final accepted filter via
    // `requested.intersection(&candidate).intersection(&advertised)`, both
    // left-biased over the REQUESTED list — so a repeated requested VALID
    // URI may legitimately appear more than once in the raw acknowledged
    // Vec. The server-side contract (accepted_subscription_filter) and the
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
    let (client, _) = connect_modern_client(&server).await?;

    // Everything invalid: acknowledged resource list is None (no fake URI).
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

    // Nothing requested at all: same empty accepted filter; the listen
    // still acknowledges (handler stays available).
    let mut sub = client.peer().listen(SubscriptionFilter::new()).await?;
    assert!(sub.acknowledged().resource_subscriptions.is_none());
    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn legacy_listen_is_method_not_found() -> Result<()> {
    let server = TestServer::start().await;
    let (legacy, _) = common::connect_legacy_client(&server).await?;
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
        "legacy clients must not reach the modern listen surface: {result:?}"
    );
    legacy.cancel().await.ok();
    Ok(())
}

// =============================================================================
// RX append + cursor semantics
// =============================================================================

#[tokio::test]
async fn rx_append_notification_arrives_only_after_bytes_readable_and_does_not_move_cursor(
) -> Result<()> {
    let (server, connection_id, state) = controlled_server().await;
    let (client, _) = connect_modern_client(&server).await?;

    // Start the pump with a drained read (no data yet; shared cursor stays 0).
    start_pump(&client, &connection_id).await;

    // Subscribe to the connection's detail URI.
    let detail_uri = format!("serial://connections/{connection_id}");
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription(detail_uri.clone())
                .build(),
        )
        .await?;

    // Nothing pending before any RX.
    assert_no_notification(&mut sub, Duration::from_millis(200)).await;

    // Inject bytes; the pump appends and only then publishes.
    state.inject_rx(b"first");
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &[&detail_uri]);

    // The bytes are readable immediately and the shared read cursor was NOT
    // moved by the notification: a read from the cursor (still 0 — no read
    // consumed anything) returns the appended bytes from offset 0.
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
    let (client, _) = connect_modern_client(&server).await?;

    // No subscription exists anywhere; the read path must be fully usable.
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

// =============================================================================
// Listener independence + shared hub
// =============================================================================

#[tokio::test]
async fn two_stateless_listeners_receive_same_update_independently() -> Result<()> {
    let (server, connection_id, state) = controlled_server().await;
    let (client_a, _) = connect_modern_client(&server).await?;
    let (client_b, _) = connect_modern_client(&server).await?;
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
    // Both see the same bytes after their own notification. The shared read
    // cursor is process-wide (covered by `stateless_requests_share_session_ring_and_cursor`),
    // so each client replays the same absolute offset non-destructively.
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
    let (client_a, _) = connect_modern_client(&server).await?;
    let (client_b, _) = connect_modern_client(&server).await?;
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

    // Cancel A; B must keep receiving.
    sub_a.cancel().await?;
    assert_eq!(sub_a.end(), Some(&SubscriptionEnd::Cancelled));

    state.inject_rx(b"b-only");
    let b_uris = collect_updates(&mut sub_b, 1).await;
    assert_hint_set(b_uris, &[&detail_uri]);
    // A's stream is over: next() completes (None), never a hang or error.
    let ended = tokio::time::timeout(Duration::from_secs(5), sub_a.next())
        .await
        .expect("A subscription must terminate after cancel")
        .expect("no service error after cancel");
    assert!(ended.is_none(), "A subscription ended, got {ended:?}");

    // B is still live: another RX yields another update.
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
    let (client, _) = connect_modern_client(&server).await?;
    start_pump(&client, &connection_id).await;

    // Listen ONLY to serial://ports. RX appends publish detail/raw/log —
    // never ports — so no notification may arrive.
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

// =============================================================================
// Lag recovery
// =============================================================================

#[tokio::test]
async fn forced_hub_lag_yields_conservative_per_uri_recovery_without_blocking_publisher(
) -> Result<()> {
    // Tiny hub so a burst of synchronous publishes forces broadcast lag.
    let hub = Arc::new(serial_mcp::resource_events::ResourceEventHub::new(2));
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let server = TestServer::builder(manager)
        .resource_hub(Arc::clone(&hub))
        .start()
        .await;
    let (client, _) = connect_modern_client(&server).await?;

    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscriptions(["serial://ports", "serial://connections"])
                .build(),
        )
        .await?;

    // Burst of synchronous publishes with NO await between them: the
    // current-thread runtime cannot schedule the server's listen task, so
    // its receiver (capacity 2) is guaranteed to lag. publish_updated is
    // synchronous and never blocks — this loop cannot stall.
    for i in 0..10 {
        hub.publish_updated("serial://ports");
        hub.publish_updated("serial://connections/evt");
        let _ = i;
    }

    // Recovery: exactly one update per ACCEPTED URI, in accepted order —
    // not a flood, and never a deadlock of the publisher/pump.
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
    // rmcp 3.0.1 echoes a repeated requested URI into the acknowledged
    // filter (left-biased requested.intersection(candidate)); the listen
    // loop deduplicates again, so lag recovery must notify each accepted
    // URI exactly once even when the request repeated it.
    let hub = Arc::new(serial_mcp::resource_events::ResourceEventHub::new(2));
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let server = TestServer::builder(manager)
        .resource_hub(Arc::clone(&hub))
        .start()
        .await;
    let (client, _) = connect_modern_client(&server).await?;

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

    // Force broadcast lag with a synchronous publish burst (current-thread
    // runtime cannot schedule the listen task mid-loop).
    for _ in 0..10 {
        hub.publish_ports_changed();
        hub.publish_connections_changed();
    }

    // Recovery: exactly one notification per DISTINCT accepted URI, in
    // first-occurrence order — the repeated "serial://ports" request must
    // not produce a second ports notification.
    let uris = collect_updates(&mut sub, 2).await;
    assert_eq!(
        uris,
        vec![
            "serial://ports".to_string(),
            "serial://connections".to_string()
        ],
        "lag recovery notifies each distinct accepted URI once, first-occurrence order"
    );

    // Normal matching also never duplicates: one ports publish -> one
    // notification (the sink's accepted filter still contains the echoed
    // duplicate, but the listen loop's deduplicated set drives emission).
    hub.publish_ports_changed();
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}

#[tokio::test]
async fn stateless_requests_share_session_ring_and_cursor() -> Result<()> {
    // Modern HTTP is stateless — every request is served by a fresh handler
    // instance. The process-wide RxSessionManager makes the session, ring,
    // and shared read cursor visible across distinct requests (public
    // behavior, not Arc identity).
    let (server, connection_id, state) = controlled_server().await;
    let (client, _) = connect_modern_client(&server).await?;

    // Request 1 (handler instance A): start the pump; nothing to drain.
    start_pump(&client, &connection_id).await;

    // Request 2 (handler instance B): get_status sees the SAME session —
    // no fresh empty ring.
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

    // Inject bytes; the shared pump appends to the shared ring.
    state.inject_rx(b"shared-ring");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Request 3: the shared ring advanced; the shared cursor is untouched
    // (still 0), so the bytes are buffered-but-unread.
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

    // Request 4: reading from the shared cursor consumes the bytes.
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

    // Request 5: the shared cursor ADVANCED (the read moved it) — the next
    // request observes the moved cursor, not a fresh one.
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

    // Request 6: another read from the cursor returns nothing (drained).
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

// =============================================================================
// Open/close + state/log operation hints
// =============================================================================

#[cfg(unix)]
#[tokio::test]
async fn open_and_close_emit_expected_uri_hints() -> Result<()> {
    use common::pty::PtyPair;

    let pair = PtyPair::open().expect("open PTY pair");
    let slave = pair
        .slave_path
        .to_str()
        .expect("slave path utf8")
        .to_string();
    let provider = common::StaticPortProvider::new(vec![common::StaticPortProvider::usb_port(
        &slave,
        0x1234,
        0x5678,
        "SN-OPEN-CLOSE",
        Some("Test Device"),
        None,
    )]);
    let manager = Arc::new(serial_mcp::serial::ConnectionManager::new());
    let server = TestServer::builder(manager)
        .port_provider(provider)
        .start()
        .await;
    let (client, _) = connect_modern_client(&server).await?;

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
        .call_tool(common::tool_request("open", json!({"port": slave})))
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

    // open -> serial://connections (detail URI unknown until now).
    let uris = collect_updates(&mut conn_sub, 1).await;
    assert_hint_set(uris, &["serial://connections"]);

    // Now subscribe to the concrete detail URI and close.
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

    // close -> serial://connections on the list subscription + the closed
    // detail URI on the detail subscription (both are hints).
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
    let (client, _) = connect_modern_client(&server).await?;
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

    // write -> detail (state changed).
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

    // transact -> detail (write half changed state).
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

    // send_break -> detail.
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

    // set_dtr_rts -> detail.
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

    // reconfigure -> detail.
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

    // set_flow_control -> detail.
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

    // configure (connection mode) -> detail.
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

    // reconnect -> detail (state change).
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

    // clear_log -> log URI only.
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

    // flush(both) -> detail + raw (ring state changed).
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

// =============================================================================
// Port hotplug watcher (public boundary)
// =============================================================================

/// Distinct port identities for watcher tests.
fn watcher_port(name: &str, serial: &str) -> serial_mcp::serial::PortInfo {
    common::StaticPortProvider::usb_port(name, 0xABCD, 0x1234, serial, Some("Watch Device"), None)
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
    let (client, _) = connect_modern_client(&server).await?;
    let mut sub = client
        .peer()
        .listen(
            SubscriptionFilter::builder()
                .resource_subscription("serial://ports")
                .build(),
        )
        .await?;

    // Let the first successful snapshot establish the baseline.
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_no_notification(&mut sub, Duration::from_millis(200)).await;

    // Unchanged: no event.
    assert_no_notification(&mut sub, Duration::from_millis(200)).await;

    // Reorder (OS enumeration order changed, same devices): no event.
    provider.set_ports(vec![b.clone(), a.clone()]);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;

    // Add a device: exactly one ports update.
    provider.set_ports(vec![a.clone(), b.clone(), c.clone()]);
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    // Identity change (same path, new serial): one update. The retained
    // baseline is now [a2, b, c] (three devices).
    provider.set_ports(vec![
        watcher_port("/dev/ttyWatch0", "SN-A2"),
        b.clone(),
        c.clone(),
    ]);
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    // Enumeration failure: no event, prior successful baseline retained.
    provider.set_fail(true);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;

    // Recovery compares against the retained baseline: an unchanged
    // snapshot after the error is NOT a change...
    provider.set_fail(false);
    provider.set_ports(vec![
        watcher_port("/dev/ttyWatch0", "SN-A2"),
        b.clone(),
        c.clone(),
    ]);
    assert_no_notification(&mut sub, Duration::from_millis(250)).await;

    // ...and neither is a pure reorder of the retained snapshot...
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

    // ...but a real change after recovery IS an event.
    provider.set_ports(vec![b.clone(), c.clone()]);
    let uris = collect_updates(&mut sub, 1).await;
    assert_hint_set(uris, &["serial://ports"]);

    sub.cancel().await?;
    client.cancel().await.ok();
    Ok(())
}
