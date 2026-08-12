//! The multi-connection registry: `ConnectionManager`, its private registry
//! state, duplicate-port lookup, reconnect-task supervision, and
//! manager-only tests. Depends on the `config` and `connection` siblings.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::error::{Result, SerialError};

use super::config::{ConnectionConfig, ConnectionState, ConnectionSummary};
use super::connection::SerialConnection;

/// The connection-opening boundary used by [`ConnectionManager::open`].
///
/// This injects the *connection backend* only: reservation, registry, and
/// lifecycle semantics live in the manager and are identical for every
/// implementation. Production uses the internal `SystemConnectionOpener`;
/// alternate backends (in-memory duplex streams, hardware simulators) can
/// drive the entire public MCP surface without an OS serial port. This keeps
/// such tests cross-platform.
#[async_trait::async_trait]
pub trait ConnectionOpener: Send + Sync {
    /// Open a connection for `config` and return it. The returned
    /// connection must honor the requested config fields (the manager
    /// assumes nothing about the backend beyond this trait).
    ///
    /// The `async_trait` macro supplies the dyn-compatible boxed future
    /// internally, keeping the trait object-safe across the supported
    /// toolchains.
    async fn open(&self, config: ConnectionConfig) -> Result<SerialConnection>;
}

/// Production opener: delegates to [`SerialConnection::open`], which builds
/// the OS serial stream and wraps it in a [`SerialConnection`].
#[derive(Debug)]
struct SystemConnectionOpener;

#[async_trait::async_trait]
impl ConnectionOpener for SystemConnectionOpener {
    async fn open(&self, config: ConnectionConfig) -> Result<SerialConnection> {
        SerialConnection::open(config).await
    }
}

/// Registry of currently open serial connections, indexed by an opaque
/// connection id. Rejects opening the same port twice.
pub struct ConnectionManager {
    /// Injectable connection-opening boundary (production: OS serial open).
    opener: Arc<dyn ConnectionOpener>,
    /// Shared registry state. Wrapped in an `Arc` so the
    /// [`OpeningReservationGuard`] can reach the registry from its `Drop`
    /// (i.e. when a cancelled open must release its reservation).
    state: Arc<Mutex<ConnectionRegistry>>,
}

impl fmt::Debug for ConnectionManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The opener is a backend, not registry state; callers never need
        // to debug-inspect it, and we deliberately do not require
        // implementations to provide a `Debug` impl.
        f.debug_struct("ConnectionManager")
            .field("opener", &"<dyn ConnectionOpener>")
            .field("state", &self.state)
            .finish()
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// RAII owner of one `opening_ports` reservation.
///
/// The reservation is released exactly once, no matter how the owning
/// `open` future ends:
/// - normal success: the caller removes the reservation and disarms via
///   [`clear`](Self::clear) while holding the registry lock (atomic with
///   the connection insert);
/// - normal failure: the caller propagates the error and this guard's
///   `Drop` removes the reservation;
/// - cancellation (the future dropped mid-await): this guard's `Drop`
///   removes the reservation, so a cancelled open can never wedge the port.
///
/// `Drop` removes synchronously via `try_lock` when the registry is
/// uncontended (the common cancellation case — nothing else holds the
/// lock). When the lock is contended it falls back to a bounded async
/// cleanup task on the current runtime, or a synchronous
/// `blocking_lock` when there is no runtime at all (drop outside an
/// async context). The fallback only ever waits for one short registry
/// lock acquisition, never an unbounded queue.
struct OpeningReservationGuard {
    state: Arc<Mutex<ConnectionRegistry>>,
    port: String,
    active: bool,
}

impl OpeningReservationGuard {
    fn new(state: Arc<Mutex<ConnectionRegistry>>, port: String) -> Self {
        Self {
            state,
            port,
            active: true,
        }
    }

    /// Disarm the guard: the reservation was already released explicitly
    /// (under the registry lock, atomically with the connection insert).
    fn clear(&mut self) {
        self.active = false;
    }
}

impl Drop for OpeningReservationGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        match self.state.try_lock() {
            Ok(mut state) => {
                state.opening_ports.remove(&self.port);
            }
            Err(_) => {
                // Contended: bounded async cleanup on the current runtime.
                // The spawned task only waits for one registry lock
                // acquisition, then removes the reservation.
                let state = Arc::clone(&self.state);
                let port = self.port.clone();
                if let Ok(handle) = tokio::runtime::Handle::try_current() {
                    // The JoinHandle is dropped immediately, which detaches
                    // the task (it keeps running to completion).
                    std::mem::drop(handle.spawn(async move {
                        state.lock().await.opening_ports.remove(&port);
                    }));
                } else {
                    // No runtime (drop in a plain sync context):
                    // synchronous acquisition is safe here and cannot
                    // deadlock the runtime.
                    self.state.blocking_lock().opening_ports.remove(&self.port);
                }
            }
        }
    }
}

#[derive(Debug, Default)]
struct ConnectionRegistry {
    connections: HashMap<String, Arc<SerialConnection>>,
    opening_ports: HashSet<String>,
    closing_ports: HashSet<String>,
    reconnect_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::with_opener(Arc::new(SystemConnectionOpener))
    }

    /// Build a manager whose connection opening is performed by `opener`
    /// instead of the production OS serial backend.
    ///
    /// Production never calls this — [`Self::new`] / [`Default`] use the
    /// system opener. It exists so alternate connection backends (in-memory
    /// duplex streams, hardware simulators) can exercise the whole public
    /// MCP surface without an OS serial port, which also keeps such tests
    /// cross-platform. This injects the connection backend only; port
    /// reservation, registry, and lifecycle semantics are unchanged.
    pub fn with_opener(opener: Arc<dyn ConnectionOpener>) -> Self {
        Self {
            opener,
            state: Arc::new(Mutex::new(ConnectionRegistry::default())),
        }
    }

    /// Open a new connection and store it. Returns the new connection id.
    pub async fn open(&self, config: ConnectionConfig) -> Result<String> {
        let port = config.port.clone();
        let mut guard = {
            let mut state = self.state.lock().await;
            if let Some(connection) = find_connection_by_port(&state.connections, &port) {
                return Err(SerialError::PortAlreadyOpen {
                    port,
                    connection_id: Some(connection.id().to_string()),
                    name: connection.name().map(str::to_string),
                });
            }
            if state.opening_ports.contains(&port) || state.closing_ports.contains(&port) {
                return Err(SerialError::PortAlreadyOpening(port));
            }
            state.opening_ports.insert(port.clone());
            OpeningReservationGuard::new(Arc::clone(&self.state), port.clone())
        };

        let opened = self.opener.open(config).await;

        // Success or failure: the reservation is removed exactly once.
        // On error, `?` propagates and the guard's `Drop` removes it. On
        // success, removal happens here under the registry lock, atomic
        // with the connection insert, then the guard is disarmed so its
        // `Drop` becomes a no-op.
        let mut state = self.state.lock().await;
        let connection = Arc::new(opened?);
        let id = connection.id().to_string();
        state.opening_ports.remove(&port);
        state.connections.insert(id.clone(), connection);
        guard.clear();
        Ok(id)
    }

    /// Insert an already-built [`SerialConnection`] (typically one backed
    /// by an in-memory loopback) into the registry. Honours the same
    /// port-uniqueness invariant as [`Self::open`].
    ///
    /// Exposed for integration tests that want to drive the MCP surface
    /// against a fake connection without going through the OS serial layer.
    pub async fn insert(&self, connection: SerialConnection) -> Result<String> {
        let mut state = self.state.lock().await;
        if let Some(existing) = find_connection_by_port(&state.connections, connection.port()) {
            return Err(SerialError::PortAlreadyOpen {
                port: connection.port().to_string(),
                connection_id: Some(existing.id().to_string()),
                name: existing.name().map(str::to_string),
            });
        }
        if state.opening_ports.contains(connection.port())
            || state.closing_ports.contains(connection.port())
        {
            return Err(SerialError::PortAlreadyOpening(
                connection.port().to_string(),
            ));
        }
        let id = connection.id().to_string();
        state.connections.insert(id.clone(), Arc::new(connection));
        Ok(id)
    }

    /// Remove a connection, cancel in-flight operations, flush RX, and close
    /// the underlying port before allowing a reopen.
    pub async fn close(&self, id: &str) -> Result<()> {
        let connection = {
            let mut state = self.state.lock().await;
            let connection = state
                .connections
                .remove(id)
                .ok_or_else(|| SerialError::ConnectionNotFound(id.to_string()))?;
            state.closing_ports.insert(connection.port().to_string());
            connection
        };

        let port = connection.port().to_string();
        // Abort any running reconnect task for this connection.
        {
            let mut state = self.state.lock().await;
            if let Some(handle) = state.reconnect_tasks.remove(id) {
                handle.abort();
            }
        }
        connection.log().closed();
        let result = connection.close().await;

        self.state.lock().await.closing_ports.remove(&port);
        result
    }

    /// Look up an existing connection by id.
    pub async fn get(&self, id: &str) -> Result<Arc<SerialConnection>> {
        self.state
            .lock()
            .await
            .connections
            .get(id)
            .cloned()
            .ok_or_else(|| SerialError::ConnectionNotFound(id.to_string()))
    }

    /// Return all currently-registered connections with their ids.
    pub async fn list_all(&self) -> Vec<(String, Arc<SerialConnection>)> {
        self.state
            .lock()
            .await
            .connections
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// Start a background reconnect task for the given connection.
    /// The task retries `reconnect()` with exponential backoff,
    /// respecting the connection's `ReconnectPolicy`. On success,
    /// restarts the RX pump via `rx_sessions`.
    pub async fn start_reconnect(
        &self,
        id: &str,
        rx_sessions: Arc<crate::rx_session::RxSessionManager>,
    ) {
        let conn = match self.get(id).await {
            Ok(c) => c,
            Err(_) => return,
        };
        let policy = conn.reconnect_policy.lock().expect("poisoned").clone();
        if !policy.enabled {
            return;
        }
        // Avoid spawning a duplicate task. Prune finished handles first.
        {
            let mut state = self.state.lock().await;
            state.reconnect_tasks.retain(|_, h| !h.is_finished());
            if state.reconnect_tasks.contains_key(id) {
                return;
            }
        }

        let id_owned = id.to_string();
        let conn_clone = Arc::clone(&conn);
        let handle = tokio::spawn(async move {
            let mut delay_ms = policy.initial_delay_ms;
            let mut attempts: u32 = 0;
            loop {
                // Check if still disconnected / not cancelled.
                let state = conn_clone.state();
                if state == ConnectionState::Open || state == ConnectionState::Closed {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                match conn_clone.reconnect().await {
                    Ok(()) => {
                        // Reset attempt counter after successful reconnect.
                        conn_clone.reset_reconnect_attempts();
                        // Restart the RX pump so data flows again.
                        if let Some(session) = rx_sessions.get(&id_owned).await {
                            session.ensure_pump_running();
                        }
                        break;
                    }
                    Err(_e) => {
                        attempts += 1;
                        if policy.max_attempts > 0 && attempts >= policy.max_attempts {
                            conn_clone
                                .log()
                                .record(None, crate::log_buffer::LogEvent::ReconnectExhausted);
                            break;
                        }
                        // Exponential backoff with cap.
                        delay_ms = ((delay_ms as f64) * policy.backoff_multiplier)
                            .min(policy.max_delay_ms as f64)
                            as u64;
                    }
                }
            }
            // Task completes: handle stays in reconnect_tasks; supervisor
            // prunes finished handles on its next poll.
        });

        let mut state = self.state.lock().await;
        state.reconnect_tasks.insert(id.to_string(), handle);
    }

    /// Cancel a running reconnect task for the given connection.
    pub async fn cancel_reconnect(&self, id: &str) {
        let mut state = self.state.lock().await;
        if let Some(handle) = state.reconnect_tasks.remove(id) {
            handle.abort();
        }
    }

    /// Number of currently open connections.
    pub async fn count(&self) -> usize {
        self.state.lock().await.connections.len()
    }

    /// Lightweight snapshot of all currently-open connections. Cheap because
    /// it only clones the id + port pair, not the underlying IO.
    pub async fn list_open(&self) -> Vec<ConnectionSummary> {
        self.state
            .lock()
            .await
            .connections
            .values()
            .map(|c| c.summary())
            .collect()
    }
}

fn find_connection_by_port<'a>(
    connections: &'a HashMap<String, Arc<SerialConnection>>,
    port: &str,
) -> Option<&'a Arc<SerialConnection>> {
    connections.values().find(|c| c.port() == port)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use tokio::sync::{watch, Notify};

    use super::*;
    use crate::serial::test_support::{loopback_connection, loopback_connection_with_config};
    use crate::serial::{DataBits, FlowControl, Parity, StopBits};

    fn test_config(port: &str) -> ConnectionConfig {
        ConnectionConfig {
            port: port.into(),
            name: Some("opener-test".into()),
            baud_rate: 9600,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            port_info: None,
            log_capacity: 1024,
            log_enabled: true,
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: 8192,
            max_buffered_bytes: 4096,
        }
    }

    #[tokio::test]
    async fn manager_rejects_duplicate_port() {
        let mgr = ConnectionManager::new();
        let (c1, _p1) = loopback_connection("port-a");
        mgr.insert(c1).await.unwrap();
        let (c2, _p2) = loopback_connection("port-a");
        let err = mgr.insert(c2).await.unwrap_err();
        assert!(matches!(err, SerialError::PortAlreadyOpen { .. }));
    }

    #[tokio::test]
    async fn manager_duplicate_port_error_includes_owner_metadata() {
        let mgr = ConnectionManager::new();
        let (c1, _peer_a) = loopback_connection_with_config(ConnectionConfig {
            port: "port-owner".into(),
            name: Some("console".into()),
            baud_rate: 115200,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            port_info: None,
            log_capacity: 1024,
            log_enabled: true,
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: crate::limits::DEFAULT_RX_BUFFER_SIZE,
            max_buffered_bytes: 32768,
        });
        let owner_id = mgr.insert(c1).await.unwrap();

        let (c2, _p2) = loopback_connection("port-owner");
        let err = mgr.insert(c2).await.unwrap_err();
        match err {
            SerialError::PortAlreadyOpen {
                port,
                connection_id,
                name,
            } => {
                assert_eq!(port, "port-owner");
                assert_eq!(connection_id.as_deref(), Some(owner_id.as_str()));
                assert_eq!(name.as_deref(), Some("console"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manager_close_then_get_returns_connection_not_found() {
        let mgr = ConnectionManager::new();
        let (c, _p) = loopback_connection("port-z");
        let id = mgr.insert(c).await.unwrap();
        mgr.close(&id).await.unwrap();
        let err = mgr.get(&id).await.unwrap_err();
        assert!(matches!(err, SerialError::ConnectionNotFound(_)));
    }

    #[tokio::test]
    async fn manager_get_unknown_id_returns_connection_not_found() {
        let mgr = ConnectionManager::new();
        let err = mgr.get("does-not-exist").await.unwrap_err();
        assert!(matches!(err, SerialError::ConnectionNotFound(_)));
    }

    /// The injected opener receives the exact config passed to the public
    /// `open`, and the successful open becomes retrievable through the
    /// manager (observable behavior, not opener helper-call counts alone).
    #[tokio::test]
    async fn injected_opener_receives_requested_config_and_open_is_retrievable() {
        struct RecordingOpener {
            configs: std::sync::Mutex<Vec<ConnectionConfig>>,
        }
        #[async_trait::async_trait]
        impl ConnectionOpener for RecordingOpener {
            async fn open(&self, config: ConnectionConfig) -> Result<SerialConnection> {
                self.configs
                    .lock()
                    .expect("opener configs poisoned")
                    .push(config.clone());
                let (conn, _peer) = loopback_connection_with_config(config);
                Ok(conn)
            }
        }

        let opener = Arc::new(RecordingOpener {
            configs: std::sync::Mutex::new(Vec::new()),
        });
        let mgr = ConnectionManager::with_opener(opener.clone());
        let config = test_config("opener-port");
        let id = mgr.open(config.clone()).await.unwrap();

        // The opener received and used the requested config.
        let received = opener
            .configs
            .lock()
            .expect("opener configs poisoned")
            .clone();
        assert_eq!(received.len(), 1, "opener invoked exactly once");
        let received_cfg = &received[0];
        assert_eq!(received_cfg.port, "opener-port");
        assert_eq!(received_cfg.name.as_deref(), Some("opener-test"));
        assert_eq!(received_cfg.baud_rate, 9600);
        assert_eq!(received_cfg.rx_buffer_size, 8192);
        assert_eq!(received_cfg.max_buffered_bytes, 4096);

        // The opened connection is retrievable and reflects the config.
        let conn = mgr.get(&id).await.unwrap();
        assert_eq!(conn.port(), "opener-port");
        assert_eq!(conn.name(), Some("opener-test"));
        assert_eq!(conn.baud_rate(), 9600);
    }

    /// An opener failure must release the same-port opening reservation so
    /// a later open of the same port succeeds (a failed open must never
    /// wedge the port).
    #[tokio::test]
    async fn opener_failure_releases_opening_reservation_for_later_open() {
        struct FailFirstOpener {
            failures: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl ConnectionOpener for FailFirstOpener {
            async fn open(&self, config: ConnectionConfig) -> Result<SerialConnection> {
                // One-shot: `swap(0)` never wraps (unlike `fetch_sub`,
                // which would wrap to usize::MAX at zero).
                if self.failures.swap(0, Ordering::SeqCst) > 0 {
                    return Err(SerialError::IoError(std::io::Error::other(
                        "injected opener failure",
                    )));
                }
                let (conn, _peer) = loopback_connection_with_config(config);
                Ok(conn)
            }
        }

        let mgr = ConnectionManager::with_opener(Arc::new(FailFirstOpener {
            failures: AtomicUsize::new(1),
        }));
        let config = test_config("reservation-port");

        let first = mgr.open(config.clone()).await;
        assert!(
            matches!(first, Err(SerialError::IoError(_))),
            "first open fails through the opener: {first:?}"
        );

        // The reservation was released: the same port opens successfully now.
        let id = mgr
            .open(config)
            .await
            .expect("opening reservation released after opener failure");
        let conn = mgr.get(&id).await.unwrap();
        assert_eq!(conn.port(), "reservation-port");
    }

    /// While an open is in flight, the opening reservation must stop a
    /// concurrent same-port open BEFORE it reaches the opener, and the
    /// in-flight open must still complete and register its connection.
    /// The opener invocation count is paired with the observable results
    /// (second open rejected, first open retrievable) so the assertion is
    /// not a bare helper-call-count.
    #[tokio::test]
    async fn concurrent_same_port_open_reserves_port_and_second_open_is_rejected() {
        struct GatedOpener {
            entered: watch::Sender<bool>,
            release: Arc<Notify>,
            invocations: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl ConnectionOpener for GatedOpener {
            async fn open(&self, config: ConnectionConfig) -> Result<SerialConnection> {
                self.invocations.fetch_add(1, Ordering::SeqCst);
                let _ = self.entered.send(true);
                // Hold the reservation until the test proves the second
                // open is rejected without invoking the opener again.
                self.release.notified().await;
                let (conn, _peer) = loopback_connection_with_config(config);
                Ok(conn)
            }
        }

        let (entered_tx, mut entered_rx) = watch::channel(false);
        let release = Arc::new(Notify::new());
        let opener = Arc::new(GatedOpener {
            entered: entered_tx,
            release: Arc::clone(&release),
            invocations: AtomicUsize::new(0),
        });
        let mgr = Arc::new(ConnectionManager::with_opener(opener.clone()));
        let config = test_config("concurrent-port");

        // First open runs on its own task and signals once the opener is
        // inside (reservation held).
        let mgr_for_task = Arc::clone(&mgr);
        let config_for_task = config.clone();
        let first_task = tokio::spawn(async move { mgr_for_task.open(config_for_task).await });

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            entered_rx.wait_for(|e| *e),
        )
        .await
        .expect("first open reached the opener within the hang guard")
        .expect("opener signalled entry");

        // Second open on the same port: rejected as PortAlreadyOpening
        // WITHOUT invoking the opener again.
        let second = mgr.open(config).await.unwrap_err();
        assert!(
            matches!(second, SerialError::PortAlreadyOpening(ref port) if port == "concurrent-port"),
            "concurrent same-port open rejected while opening: {second:?}"
        );
        assert_eq!(
            opener.invocations.load(Ordering::SeqCst),
            1,
            "the reservation prevents a second opener invocation"
        );

        // Release the in-flight open: it completes and registers.
        release.notify_one();
        let first_id = first_task
            .await
            .expect("first open task")
            .expect("first open succeeds after release");
        let conn = mgr.get(&first_id).await.unwrap();
        assert_eq!(conn.port(), "concurrent-port");
        assert_eq!(mgr.count().await, 1, "exactly one connection registered");
    }

    /// A cancelled open must release its `opening_ports` reservation so a
    /// later open of the same port succeeds — and a connection the opener
    /// was about to produce must never register after the cancel.
    ///
    /// The cleanup is deterministic, not polled: `JoinHandle::await` only
    /// resolves after the aborted task's future (and with it the
    /// reservation guard) is dropped, so by the time the handle returns the
    /// reservation is already gone and the immediate retry succeeds.
    #[tokio::test]
    async fn cancelled_open_releases_opening_reservation_for_later_open() {
        struct CancelGatedOpener {
            entered: watch::Sender<bool>,
            invocations: AtomicUsize,
        }
        #[async_trait::async_trait]
        impl ConnectionOpener for CancelGatedOpener {
            async fn open(&self, config: ConnectionConfig) -> Result<SerialConnection> {
                if self.invocations.fetch_add(1, Ordering::SeqCst) == 0 {
                    // First call: signal entry and wait forever. The
                    // test aborts this task; the guard's `Drop` then
                    // releases the reservation.
                    let _ = self.entered.send(true);
                    std::future::pending::<()>().await;
                    unreachable!("aborted before pending resolves");
                }
                // Second (and later) calls: succeed immediately — the
                // opener must never wait on a cancelled predecessor.
                let (conn, _peer) = loopback_connection_with_config(config);
                Ok(conn)
            }
        }

        let (entered_tx, mut entered_rx) = watch::channel(false);
        let opener = Arc::new(CancelGatedOpener {
            entered: entered_tx,
            invocations: AtomicUsize::new(0),
        });
        let mgr = Arc::new(ConnectionManager::with_opener(opener.clone()));
        let config = test_config("cancel-port");

        let mgr_for_task = Arc::clone(&mgr);
        let config_for_task = config.clone();
        let task = tokio::spawn(async move { mgr_for_task.open(config_for_task).await });

        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            entered_rx.wait_for(|e| *e),
        )
        .await
        .expect("hang guard: first open reached the opener")
        .expect("opener signalled entry");

        // Cancel the in-flight open; the reservation guard releases the
        // port during task cancellation.
        task.abort();
        let join = tokio::time::timeout(std::time::Duration::from_secs(5), task)
            .await
            .expect("hang guard: cancelled open task joined");
        assert!(
            join.is_err(),
            "open task was aborted, not completed: {join:?}"
        );

        // The same port opens successfully immediately after the cancel,
        // through the opener again, and registers exactly one connection.
        let id = tokio::time::timeout(std::time::Duration::from_secs(5), mgr.open(config))
            .await
            .expect("hang guard: second open after cancellation")
            .expect("second open succeeds after cancellation cleanup");
        let conn = mgr.get(&id).await.unwrap();
        assert_eq!(conn.port(), "cancel-port");
        assert_eq!(mgr.count().await, 1, "exactly one connection registered");
        assert_eq!(
            opener.invocations.load(Ordering::SeqCst),
            2,
            "the retry invoked the opener again"
        );
    }
}
