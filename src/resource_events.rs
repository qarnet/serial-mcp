//! Process-wide resource event primitives for modern `subscriptions/listen`.
//!
//! Three pieces:
//!
//! - [`ResourceEvent`] / [`ResourceEventHub`] — a bounded `tokio` broadcast
//!   hub shared by every stdio/HTTP handler and the port watcher. Publish is
//!   synchronous and never awaits; notifications are availability hints, not
//!   a byte ledger.
//! - subscribable-URI helpers built on `resources::parse_resource_uri` —
//!   only `serial://ports`, `serial://connections`, and recognized concrete
//!   connection detail/raw/log URIs are subscribable. Templates, malformed
//!   IDs, unknown schemes, and empty IDs are rejected.
//! - [`PortWatcher`] — one process-wide polling watcher around the shared
//!   `PortProvider` that publishes `Updated(serial://ports)` when the
//!   canonicalized port snapshot changes.
//!
//! Ownership rules:
//!
//! - one hub per server process, cloned into every handler factory and the
//!   watcher (modern HTTP is stateless — a handler-local channel would split
//!   publishers and listeners across handler instances and lose updates);
//! - each `subscriptions/listen` request owns exactly one receiver;
//! - publication happens only AFTER the observable state commit (e.g. ring
//!   append) and never blocks the publisher, the RX pump, another listener,
//!   or serial tools.

use std::cmp::Ordering;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{trace, warn};

use crate::resources::{
    parse_resource_uri, ResourceUriKind, URI_CONNECTIONS, URI_CONNECTION_PREFIX, URI_PORTS,
};
use crate::serial::{PortInfo, PortProvider};

/// Fixed hub capacity: 256 buffered events before a slow listener lags. Lag
/// is recoverable: the listener re-notifies every accepted URI once instead
/// of terminating or blocking publishers.
pub const DEFAULT_HUB_CAPACITY: usize = 256;

/// Production port-watcher poll interval (fixed decision).
pub const PORT_WATCHER_INTERVAL: Duration = Duration::from_secs(1);

/// One resource-update event. Notifications carry only the URI — serial
/// payloads never travel in notifications; `read` remains the data path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResourceEvent {
    /// The resource at `uri` may have changed. Availability hint only.
    Updated(String),
}

/// Process-wide resource event hub. Cloned via `Arc` into every handler and
/// the watcher; each listener owns one [`broadcast::Receiver`].
pub struct ResourceEventHub {
    sender: broadcast::Sender<ResourceEvent>,
    capacity: usize,
}

impl ResourceEventHub {
    /// Create a hub with the given event buffer capacity.
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender, capacity }
    }

    /// Event buffer capacity (the lag window for slow listeners).
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Subscribe a new listener receiver. The receiver sees only events
    /// published after subscription.
    pub fn subscribe(&self) -> broadcast::Receiver<ResourceEvent> {
        self.sender.subscribe()
    }

    /// Synchronously publish an `Updated(uri)` hint. Never awaits; a send
    /// with no receivers is ignored (trace-level). A lagged receiver is
    /// marked and recovers through the listener, never here.
    pub fn publish_updated(&self, uri: impl Into<String>) {
        let uri = uri.into();
        if self
            .sender
            .send(ResourceEvent::Updated(uri.clone()))
            .is_err()
        {
            trace!("resource event published with no receivers: {uri}");
        }
    }

    /// Publish that the `serial://ports` resource may have changed.
    pub fn publish_ports_changed(&self) {
        self.publish_updated(URI_PORTS);
    }

    /// Publish that the `serial://connections` list may have changed.
    pub fn publish_connections_changed(&self) {
        self.publish_updated(URI_CONNECTIONS);
    }

    /// Publish that a connection's detail resource may have changed. No-op
    /// for an invalid connection id.
    pub fn publish_connection_detail_changed(&self, connection_id: &str) {
        if let Some(uri) = detail_uri(connection_id) {
            self.publish_updated(uri);
        }
    }

    /// Publish that a connection's raw resource may have changed. No-op for
    /// an invalid connection id.
    pub fn publish_connection_raw_changed(&self, connection_id: &str) {
        if let Some(uri) = raw_uri(connection_id) {
            self.publish_updated(uri);
        }
    }

    /// Publish that a connection's log resource may have changed. No-op for
    /// an invalid connection id.
    pub fn publish_connection_log_changed(&self, connection_id: &str) {
        if let Some(uri) = log_uri(connection_id) {
            self.publish_updated(uri);
        }
    }

    /// Publish all three per-connection hints (detail, raw, log) — used by
    /// the RX pump after a successful ring append.
    pub fn publish_connection_changed(&self, connection_id: &str) {
        self.publish_connection_detail_changed(connection_id);
        self.publish_connection_raw_changed(connection_id);
        self.publish_connection_log_changed(connection_id);
    }
}

impl Default for ResourceEventHub {
    fn default() -> Self {
        Self::new(DEFAULT_HUB_CAPACITY)
    }
}

// ---- Subscribable-URI helpers ----------------------------------------------

/// Reject connection ids that cannot form a recognized concrete URI: empty
/// ids, template placeholders (`{id}`), and ids that would change how the
/// URI parses (embedded `/`). UUID connection ids always pass.
fn valid_connection_id(connection_id: &str) -> bool {
    !connection_id.is_empty() && !connection_id.contains(['/', '{', '}'])
}

/// Build `serial://connections/{id}{suffix}` and require the shared parser
/// to recognize the exact kind + id (round-trip predicate).
fn connection_uri(connection_id: &str, suffix: &str) -> Option<String> {
    if !valid_connection_id(connection_id) {
        return None;
    }
    let uri = format!("{URI_CONNECTION_PREFIX}{connection_id}{suffix}");
    match parse_resource_uri(&uri) {
        ResourceUriKind::ConnectionDetail(id) if suffix.is_empty() && id == connection_id => {
            Some(uri)
        }
        ResourceUriKind::ConnectionDetailRaw(id) if suffix == "/raw" && id == connection_id => {
            Some(uri)
        }
        ResourceUriKind::ConnectionLog(id) if suffix == "/log" && id == connection_id => Some(uri),
        _ => None,
    }
}

/// Concrete detail URI for a connection id, or `None` for invalid ids.
pub fn detail_uri(connection_id: &str) -> Option<String> {
    connection_uri(connection_id, "")
}

/// Concrete raw URI for a connection id, or `None` for invalid ids.
pub fn raw_uri(connection_id: &str) -> Option<String> {
    connection_uri(connection_id, "/raw")
}

/// Concrete log URI for a connection id, or `None` for invalid ids.
pub fn log_uri(connection_id: &str) -> Option<String> {
    connection_uri(connection_id, "/log")
}

/// Whether a URI may be requested on `subscriptions/listen`.
///
/// Only `serial://ports`, `serial://connections`, and recognized concrete
/// connection detail/raw/log URIs are subscribable. Templates
/// (`serial://connections/{id}`), malformed/empty ids, and unknown schemes
/// are rejected.
pub fn is_subscribable_uri(uri: &str) -> bool {
    match parse_resource_uri(uri) {
        ResourceUriKind::Ports | ResourceUriKind::ConnectionsList => true,
        ResourceUriKind::ConnectionDetail(id) => detail_uri(&id).is_some(),
        ResourceUriKind::ConnectionDetailRaw(id) => raw_uri(&id).is_some(),
        ResourceUriKind::ConnectionLog(id) => log_uri(&id).is_some(),
        ResourceUriKind::Unknown => false,
    }
}

// ---- Port hotplug watcher ---------------------------------------------------

/// One process-wide polling port watcher.
///
/// Canonicalizes every successful snapshot (sorted full `PortInfo` identity
/// fields, never enumeration order) and publishes `Updated(serial://ports)`
/// only when the canonical snapshot changes. The first successful snapshot
/// establishes the baseline without an event; enumeration failures warn and
/// retain the prior successful baseline so recovery compares against it.
pub struct PortWatcher {
    handle: JoinHandle<()>,
    shutdown: CancellationToken,
}

impl PortWatcher {
    /// Spawn the watcher loop. `interval` is the poll period (production
    /// uses [`PORT_WATCHER_INTERVAL`]; tests inject a short interval).
    pub fn start(
        provider: Arc<dyn PortProvider>,
        hub: Arc<ResourceEventHub>,
        shutdown: CancellationToken,
        interval: Duration,
    ) -> Self {
        let handle = tokio::spawn(port_watcher_loop(provider, hub, shutdown.clone(), interval));
        Self { handle, shutdown }
    }

    /// Signal the watcher loop to stop (idempotent).
    pub fn shutdown(&self) {
        self.shutdown.cancel();
    }

    /// Abort the watcher task (test teardown without awaiting).
    pub fn abort(&self) {
        self.handle.abort();
    }

    /// Cancel and deterministically await watcher exit.
    pub async fn shutdown_and_join(self) {
        self.shutdown.cancel();
        let _ = self.handle.await;
    }
}

async fn port_watcher_loop(
    provider: Arc<dyn PortProvider>,
    hub: Arc<ResourceEventHub>,
    shutdown: CancellationToken,
    interval: Duration,
) {
    let mut baseline: Option<Vec<PortInfo>> = None;
    loop {
        // Enumerate immediately, BEFORE any sleep: the first successful
        // baseline is captured as soon as the watcher task starts, so a
        // port change between server startup and the first poll can never
        // become the (silent) baseline. Waits happen between subsequent
        // polls.
        match provider.list_available() {
            Ok(ports) => apply_snapshot(&mut baseline, ports, &hub),
            Err(e) => {
                // Enumeration failure: warn and keep the prior successful
                // baseline so recovery compares against it. If the FIRST
                // call fails, the next success establishes the baseline
                // without notification.
                warn!("port watcher: enumeration failed: {e}");
            }
        }
        // Cancellation-aware wait before the next poll. Cancellation stays
        // prompt and deterministic (the select returns immediately once the
        // token is cancelled, even mid-sleep).
        tokio::select! {
            _ = shutdown.cancelled() => return,
            _ = tokio::time::sleep(interval) => {}
        }
    }
}

/// Apply one successful snapshot to the baseline, publishing a ports update
/// exactly when the canonical snapshot changes. First success sets the
/// baseline without an event; unchanged/reordered snapshots publish nothing.
fn apply_snapshot(
    baseline: &mut Option<Vec<PortInfo>>,
    mut ports: Vec<PortInfo>,
    hub: &ResourceEventHub,
) {
    canonicalize(&mut ports);
    match baseline {
        None => *baseline = Some(ports),
        Some(prev) if *prev != ports => {
            hub.publish_ports_changed();
            *baseline = Some(ports);
        }
        Some(_) => {}
    }
}

/// Sort a snapshot by the full stable `PortInfo` identity fields so OS
/// enumeration order never generates a false update.
fn canonicalize(ports: &mut [PortInfo]) {
    ports.sort_by(port_info_cmp);
}

/// Deterministic total order over every `PortInfo` identity field, in
/// declaration order. `PortTransport` has no deriveable order, so rank it.
fn port_info_cmp(a: &PortInfo, b: &PortInfo) -> Ordering {
    a.name
        .cmp(&b.name)
        .then_with(|| a.display_name.cmp(&b.display_name))
        .then_with(|| a.description.cmp(&b.description))
        .then_with(|| a.hardware_id.cmp(&b.hardware_id))
        .then_with(|| transport_rank(&a.transport).cmp(&transport_rank(&b.transport)))
        .then_with(|| a.vid.cmp(&b.vid))
        .then_with(|| a.pid.cmp(&b.pid))
        .then_with(|| a.serial_number.cmp(&b.serial_number))
        .then_with(|| a.manufacturer.cmp(&b.manufacturer))
        .then_with(|| a.product.cmp(&b.product))
        .then_with(|| a.interface.cmp(&b.interface))
}

fn transport_rank(transport: &crate::serial::PortTransport) -> u8 {
    match transport {
        crate::serial::PortTransport::Usb => 0,
        crate::serial::PortTransport::Pci => 1,
        crate::serial::PortTransport::Bluetooth => 2,
        crate::serial::PortTransport::Unknown => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::{PortTransport, SystemPortProvider};
    use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
    use std::sync::Mutex as StdMutex;

    fn usb_port(name: &str, serial: &str) -> PortInfo {
        PortInfo {
            name: name.into(),
            display_name: name.rsplit('/').next().unwrap_or(name).into(),
            description: "Synthetic USB device".into(),
            hardware_id: Some("USB VID:1234 PID:5678".into()),
            transport: PortTransport::Usb,
            vid: Some(0x1234),
            pid: Some(0x5678),
            serial_number: Some(serial.into()),
            manufacturer: Some("Synthetic".into()),
            product: Some("Test Device".into()),
            interface: None,
        }
    }

    fn weak_port(name: &str) -> PortInfo {
        PortInfo {
            name: name.into(),
            display_name: name.rsplit('/').next().unwrap_or(name).into(),
            description: "PTY".into(),
            hardware_id: None,
            transport: PortTransport::Unknown,
            vid: None,
            pid: None,
            serial_number: None,
            manufacturer: None,
            product: None,
            interface: None,
        }
    }

    // ── Hub semantics ─────────────────────────────────────────────────────

    #[test]
    fn hub_publishes_to_independent_receivers() {
        let hub = ResourceEventHub::new(16);
        let mut rx1 = hub.subscribe();
        let mut rx2 = hub.subscribe();
        hub.publish_updated("serial://ports");
        assert_eq!(
            rx1.try_recv().unwrap(),
            ResourceEvent::Updated("serial://ports".into())
        );
        assert_eq!(
            rx2.try_recv().unwrap(),
            ResourceEvent::Updated("serial://ports".into())
        );
        // A receiver created after the publish sees nothing.
        let mut rx3 = hub.subscribe();
        assert!(matches!(
            rx3.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn hub_ignores_publication_without_receivers() {
        let hub = ResourceEventHub::new(16);
        // No receivers: publish must not panic and must complete synchronously.
        hub.publish_updated("serial://ports");
        hub.publish_updated("serial://connections");
        // A later subscriber must not receive the pre-subscription events.
        let mut rx = hub.subscribe();
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn hub_default_capacity_is_256() {
        assert_eq!(ResourceEventHub::default().capacity(), DEFAULT_HUB_CAPACITY);
        assert_eq!(ResourceEventHub::new(2).capacity(), 2);
    }

    #[tokio::test]
    async fn lagging_receiver_is_marked_and_recovers_from_latest() {
        let hub = ResourceEventHub::new(2);
        let mut lagging = hub.subscribe();
        // Publish more events than the buffer holds; the lagging receiver
        // never drains, so it lags.
        for i in 0..5 {
            hub.publish_updated(format!("serial://connections/evt-{i}"));
        }
        match lagging.recv().await {
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            other => panic!("expected Lagged, got {other:?}"),
        }
        // After lag, the receiver re-syncs to the oldest retained event
        // (broadcast semantics) and keeps consuming.
        match lagging.recv().await {
            Ok(ResourceEvent::Updated(uri)) => assert_eq!(uri, "serial://connections/evt-3"),
            other => panic!("expected oldest retained event after lag, got {other:?}"),
        }
        match lagging.recv().await {
            Ok(ResourceEvent::Updated(uri)) => assert_eq!(uri, "serial://connections/evt-4"),
            other => panic!("expected latest event after lag, got {other:?}"),
        }
        // A receiver created after the burst sees only later events (no
        // replay of pre-subscription history).
        let mut fresh = hub.subscribe();
        hub.publish_updated("serial://connections/after");
        match fresh.recv().await {
            Ok(ResourceEvent::Updated(uri)) => assert_eq!(uri, "serial://connections/after"),
            other => panic!("expected post-subscription event, got {other:?}"),
        }
    }

    // ── URI helpers ───────────────────────────────────────────────────────

    #[test]
    fn uri_helpers_accept_only_concrete_subscribable_uris() {
        // Accepted: static lists + concrete connection URIs.
        assert!(is_subscribable_uri("serial://ports"));
        assert!(is_subscribable_uri("serial://connections"));
        assert!(is_subscribable_uri("serial://connections/abc-123"));
        assert!(is_subscribable_uri("serial://connections/abc-123/raw"));
        assert!(is_subscribable_uri("serial://connections/abc-123/log"));
        assert_eq!(
            detail_uri("abc-123"),
            Some("serial://connections/abc-123".into())
        );
        assert_eq!(
            raw_uri("abc-123"),
            Some("serial://connections/abc-123/raw".into())
        );
        assert_eq!(
            log_uri("abc-123"),
            Some("serial://connections/abc-123/log".into())
        );

        // Rejected: templates, malformed/empty ids, unknown schemes/URIs.
        for uri in [
            "serial://connections/{id}",
            "serial://connections/{id}/raw",
            "serial://connections/{id}/log",
            "serial://connections/",
            "serial://connections//raw",
            "serial://connections/abc/extra",
            "serial://other",
            "https://example.com/x",
            "",
            "not-a-uri",
        ] {
            assert!(!is_subscribable_uri(uri), "must reject {uri:?}");
        }
        for id in ["", "{id}", "a/b"] {
            assert!(detail_uri(id).is_none(), "must reject id {id:?}");
            assert!(raw_uri(id).is_none(), "must reject id {id:?}");
            assert!(log_uri(id).is_none(), "must reject id {id:?}");
        }
    }

    // ── Watcher snapshot logic (pure) ─────────────────────────────────────

    #[test]
    fn first_success_establishes_baseline_without_event() {
        let hub = ResourceEventHub::new(16);
        let mut baseline = None;
        apply_snapshot(&mut baseline, vec![usb_port("/dev/ttyUSB0", "SN-1")], &hub);
        assert_eq!(baseline.as_ref().unwrap().len(), 1);
        let mut rx = hub.subscribe();
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn unchanged_and_reordered_snapshots_publish_nothing() {
        let hub = ResourceEventHub::new(16);
        let mut rx = hub.subscribe();
        let mut baseline = None;
        let set = vec![usb_port("/dev/ttyUSB0", "SN-1"), weak_port("/dev/ttyS0")];
        apply_snapshot(&mut baseline, set.clone(), &hub);
        // Identical snapshot: no event.
        apply_snapshot(&mut baseline, set.clone(), &hub);
        // Reordered snapshot (OS enumeration order changed): no event.
        apply_snapshot(&mut baseline, vec![set[1].clone(), set[0].clone()], &hub);
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn changed_snapshot_publishes_ports_once_and_updates_baseline() {
        let hub = ResourceEventHub::new(16);
        let mut rx = hub.subscribe();
        let mut baseline = None;
        apply_snapshot(&mut baseline, vec![usb_port("/dev/ttyUSB0", "SN-1")], &hub);
        // Add a port: exactly one ports event.
        apply_snapshot(
            &mut baseline,
            vec![
                usb_port("/dev/ttyUSB0", "SN-1"),
                usb_port("/dev/ttyUSB1", "SN-2"),
            ],
            &hub,
        );
        assert_eq!(
            rx.try_recv().unwrap(),
            ResourceEvent::Updated("serial://ports".into())
        );
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        // Identity change (same name, new serial): event + baseline updated.
        apply_snapshot(
            &mut baseline,
            vec![
                usb_port("/dev/ttyUSB0", "SN-X"),
                usb_port("/dev/ttyUSB1", "SN-2"),
            ],
            &hub,
        );
        assert_eq!(
            rx.try_recv().unwrap(),
            ResourceEvent::Updated("serial://ports".into())
        );
        assert_eq!(
            baseline.as_ref().unwrap()[0].serial_number.as_deref(),
            Some("SN-X")
        );
    }

    // ── Watcher loop (integration, short interval) ────────────────────────

    /// Mutable test provider: the test swaps the snapshot and injects
    /// enumeration failures.
    struct MutPortProvider {
        ports: StdMutex<Vec<PortInfo>>,
        fail: AtomicBool,
    }

    impl PortProvider for MutPortProvider {
        fn list_available(&self) -> crate::error::Result<Vec<PortInfo>> {
            if self.fail.load(AtomicOrdering::SeqCst) {
                Err(crate::error::SerialError::IoError(std::io::Error::other(
                    "injected enumeration failure",
                )))
            } else {
                Ok(self.ports.lock().expect("ports poisoned").clone())
            }
        }
    }

    impl MutPortProvider {
        fn new(ports: Vec<PortInfo>) -> Arc<Self> {
            Arc::new(Self {
                ports: StdMutex::new(ports),
                fail: AtomicBool::new(false),
            })
        }
        fn set_ports(&self, ports: Vec<PortInfo>) {
            *self.ports.lock().expect("ports poisoned") = ports;
        }
        fn set_fail(&self, fail: bool) {
            self.fail.store(fail, AtomicOrdering::SeqCst);
        }
    }

    #[tokio::test]
    async fn watcher_captures_immediate_baseline_before_first_interval() {
        // The FIRST poll must run immediately when the watcher task starts
        // — never after one interval. Proven with an interval far longer
        // than the mutation window: the baseline is captured during the
        // yield below (no interval elapses), a mutation issued right after
        // it emits an update. A sleep-first loop would swallow that mutation
        // as the silent baseline and this test would time out.
        let provider = MutPortProvider::new(vec![usb_port("/dev/ttyUSB0", "SN-1")]);
        let hub = Arc::new(ResourceEventHub::new(16));
        let mut rx = hub.subscribe();
        let shutdown = CancellationToken::new();
        let watcher = PortWatcher::start(
            Arc::clone(&provider) as Arc<dyn PortProvider>,
            Arc::clone(&hub),
            shutdown.clone(),
            Duration::from_millis(1000), // one second: no interval may elapse here
        );

        // Yield so the spawned watcher runs its FIRST (immediate) poll and
        // establishes the baseline [a] without sleeping an interval.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        // Mutate immediately — well before the first 1s interval would have
        // elapsed. The mutation must emit an update (baseline was [a], not
        // the mutated set).
        provider.set_ports(vec![
            usb_port("/dev/ttyUSB0", "SN-1"),
            usb_port("/dev/ttyUSB1", "SN-2"),
        ]);
        let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("mutation after immediate baseline must emit an update")
            .expect("hub not closed");
        assert_eq!(event, ResourceEvent::Updated("serial://ports".into()));

        watcher.shutdown_and_join().await;
    }

    #[tokio::test]
    async fn watcher_loop_emits_on_change_recovers_after_failure_and_ignores_reorder() {
        let provider = MutPortProvider::new(vec![usb_port("/dev/ttyUSB0", "SN-1")]);
        let hub = Arc::new(ResourceEventHub::new(16));
        let mut rx = hub.subscribe();
        let shutdown = CancellationToken::new();
        let watcher = PortWatcher::start(
            Arc::clone(&provider) as Arc<dyn PortProvider>,
            Arc::clone(&hub),
            shutdown.clone(),
            Duration::from_millis(15),
        );

        // The FIRST poll runs immediately (no interval wait): yield lets the
        // spawned watcher establish the baseline [a] right away.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
        // Unchanged snapshots over a few polls: still no event.
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        // Add a port -> exactly one ports event.
        provider.set_ports(vec![
            usb_port("/dev/ttyUSB0", "SN-1"),
            usb_port("/dev/ttyUSB1", "SN-2"),
        ]);
        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("ports update within hang guard")
            .expect("hub not closed");
        assert_eq!(event, ResourceEvent::Updated("serial://ports".into()));

        // Failure: warn and retain baseline, no event.
        provider.set_fail(true);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        // Recovery compares against the retained baseline: reorder of the
        // retained snapshot is NOT a change...
        provider.set_fail(false);
        provider.set_ports(vec![
            usb_port("/dev/ttyUSB1", "SN-2"),
            usb_port("/dev/ttyUSB0", "SN-1"),
        ]);
        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        // ...but a real change after recovery IS an event.
        provider.set_ports(vec![
            usb_port("/dev/ttyUSB1", "SN-2"),
            usb_port("/dev/ttyUSB0", "SN-1"),
            weak_port("/dev/ttyS0"),
        ]);
        let event = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("post-recovery ports update")
            .expect("hub not closed");
        assert_eq!(event, ResourceEvent::Updated("serial://ports".into()));

        // Deterministic shutdown/join.
        watcher.shutdown_and_join().await;
    }

    #[tokio::test]
    async fn watcher_shutdown_is_deterministic_and_idempotent() {
        let provider = Arc::new(SystemPortProvider) as Arc<dyn PortProvider>;
        let hub = Arc::new(ResourceEventHub::new(16));
        let shutdown = CancellationToken::new();
        let watcher =
            PortWatcher::start(provider, hub, shutdown.clone(), Duration::from_millis(10));
        watcher.shutdown();
        watcher.shutdown();
        watcher.shutdown_and_join().await;
        // Double join is safe via a fresh handle only; the consumed watcher
        // already exited, so nothing further to assert besides no panic.
        assert!(shutdown.is_cancelled());
    }
}
