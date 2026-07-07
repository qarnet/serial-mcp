//! Per-connection RX session core for unified serial data capture.
//!
//! Each open serial connection has at most one [`RxSession`], which owns
//! a single background pump task that reads from the serial port. The pump
//! appends every received byte to a per-connection [`RxRing`] and fans out
//! to registered consumer channels (legacy path; Phase 2 deletes registry).
//!
//! The pump runs from `open` to `close` — bytes are captured regardless of
//! active tool calls. The ring is a budgeted allocation charged at open and
//! released at close.
//!
//! Phase 1.2 scope:
//! - always-on pump from open to close
//! - RxRing capture (all bytes preserved)
//! - budget reservation for the ring at construction
//! - pause on disconnect, resume on reconnect (same ring, same offsets)
//! - ConsumerRegistry + register_blocking/register_streaming still work
//!   (Phase 2 deletes them)
//!
//! ## Consumer drop policy
//!
//! When the pump calls [`ConsumerRegistry::fanout`], any consumer whose
//! channel is full (slow consumer) or closed (dropped receiver) is silently
//! removed from the registry via `Vec::retain`. This prevents the pump from
//! blocking or buffering indefinitely for lagging consumers.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::buffer_budget::{BufferBudget, BufferReservation};
use crate::rx_ring::RxRing;
use crate::serial::SerialConnection;

// ---- Events pumped to consumers -------------------------------------------

/// A chunk of data or a lifecycle event pushed to each consumer.
#[derive(Debug)]
pub enum RxEvent {
    Data(Vec<u8>),
    Closed,
    Error(String),
}

impl Clone for RxEvent {
    fn clone(&self) -> Self {
        match self {
            RxEvent::Data(bytes) => RxEvent::Data(bytes.clone()),
            RxEvent::Closed => RxEvent::Closed,
            RxEvent::Error(msg) => RxEvent::Error(msg.clone()),
        }
    }
}

// ---- Consumer channel wrapper ---------------------------------------------

/// A registered consumer that receives [`RxEvent`]s through an mpsc channel.
pub struct RxConsumer {
    tx: mpsc::Sender<RxEvent>,
}

impl RxConsumer {
    fn new(capacity: usize) -> (Self, mpsc::Receiver<RxEvent>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self { tx }, rx)
    }

    fn try_send(&self, event: RxEvent) -> bool {
        self.tx.try_send(event).is_ok()
    }
}

// ---- Shared consumer registry for pump access -----------------------------

/// Consumer lists shared between `RxSession` and the pump task.
///
/// The pump reads from `connection.read()` and fans out chunks to all
/// registered consumers via `try_send`. Consumers whose channels are full
/// (slow consumer) or closed (dropped receiver) are silently removed from
/// the registry by `retain()`. This is the explicit consumer-drop policy
/// for PLAN 1a; see module-level docs for details.
struct ConsumerRegistry {
    blocking: Vec<RxConsumer>,
    streaming: Vec<RxConsumer>,
}

impl ConsumerRegistry {
    fn new() -> Self {
        Self {
            blocking: Vec::new(),
            streaming: Vec::new(),
        }
    }

    /// Fan out an event to all registered consumers.
    ///
    /// Consumers whose channels are full or closed are silently removed.
    /// This never blocks — `try_send` is used for every consumer.
    fn fanout(&mut self, event: RxEvent) {
        self.blocking.retain(|c| c.try_send(event.clone()));
        self.streaming.retain(|c| c.try_send(event.clone()));
    }

    fn prune_closed(&mut self) {
        self.blocking.retain(|c| !c.tx.is_closed());
        self.streaming.retain(|c| !c.tx.is_closed());
    }

    #[allow(dead_code)] // removed in Phase 2 (registry deletion)
    fn is_empty(&self) -> bool {
        self.blocking.is_empty() && self.streaming.is_empty()
    }
}

// ---- Per-connection session ------------------------------------------------

/// Manages one pump task and its registered consumers for a single connection.
///
/// The pump task holds an `Arc<StdMutex<ConsumerRegistry>>` so it can
/// fan out data without needing a reference back to this struct.
///
/// The [`RxRing`] captures all RX bytes from open to close. The pump appends
/// to the ring AND fans out to consumer channels simultaneously. Phase 1.3
/// rewrites `read`/`seek`/`flush` onto the ring; Phase 2 rewrites `subscribe`.
pub struct RxSession {
    connection_id: String,
    connection: Arc<SerialConnection>,
    consumers: Arc<StdMutex<ConsumerRegistry>>,
    pump_task: StdMutex<Option<JoinHandle<()>>>,
    pump_token: StdMutex<CancellationToken>,
    /// Per-connection ring buffer capturing all RX bytes from open to close.
    ring: Arc<RxRing>,
    /// Shared read cursor for Phase 1.3 `read`/`seek`/`flush` rewrite.
    /// Absolute u64 offset; starts at 0.
    #[allow(dead_code)] // used by Phase 1.3
    read_cursor: StdMutex<u64>,
    /// Stored ring capacity so close can release the correct byte count.
    ring_capacity: usize,
    /// RAII reservation for the ring's budget allocation. Dropped at shutdown.
    budget_reservation: StdMutex<Option<Box<dyn BufferReservation>>>,
}

impl RxSession {
    const PUMP_READ_SIZE: usize = 4096;
    const CONSUMER_CHANNEL_CAPACITY: usize = 256;

    /// Create a new RxSession with a budgeted ring buffer.
    ///
    /// The ring captures `ring_capacity` bytes of RX history. Bytes are
    /// charged against `budget` and released at [`shutdown_and_join`](Self::shutdown_and_join).
    ///
    /// Returns `Err` if `ring_capacity` is 0 or the budget cannot be reserved.
    pub fn new(
        connection: Arc<SerialConnection>,
        ring_capacity: usize,
        budget: &Arc<dyn BufferBudget>,
    ) -> Result<Self, String> {
        if ring_capacity == 0 {
            return Err(format!(
                "open.rx_buffer_size: ring_capacity must be > 0, got {ring_capacity}"
            ));
        }
        let reservation = budget
            .try_reserve(ring_capacity)
            .map_err(|e| crate::tools::helpers::map_budget_err("open.rx_buffer_size", e))?;
        let connection_id = connection.id().to_string();
        Ok(Self {
            connection_id,
            connection,
            consumers: Arc::new(StdMutex::new(ConsumerRegistry::new())),
            pump_task: StdMutex::new(None),
            pump_token: StdMutex::new(CancellationToken::new()),
            ring: Arc::new(RxRing::new(ring_capacity)),
            read_cursor: StdMutex::new(0),
            ring_capacity,
            budget_reservation: StdMutex::new(Some(reservation)),
        })
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    // ── Phase 1.3 accessors (read/seek/flush rewrite) ─────────────────────

    /// Borrow the ring buffer. Phase 1.3 `read`/`seek`/`flush` use this.
    #[allow(dead_code)] // used by Phase 1.3
    pub(crate) fn ring(&self) -> &RxRing {
        &self.ring
    }

    /// Current shared read cursor (absolute u64 offset).
    #[allow(dead_code)] // used by Phase 1.3
    pub(crate) fn read_cursor(&self) -> u64 {
        *self.read_cursor.lock().expect("read_cursor mutex poisoned")
    }

    /// Set the shared read cursor. Phase 1.3 `seek` uses this.
    #[allow(dead_code)] // used by Phase 1.3
    pub(crate) fn set_read_cursor(&self, off: u64) {
        *self.read_cursor.lock().expect("read_cursor mutex poisoned") = off;
    }

    /// Ring capacity in bytes.
    pub(crate) fn ring_capacity(&self) -> usize {
        self.ring_capacity
    }

    /// Ensure the pump task is running. Idempotent.
    ///
    /// Called once at open (via `get_or_create`) and on reconnect. Under the
    /// always-on model the pump is started at open and runs until close;
    /// this method is a no-op if already running and restarts if the previous
    /// pump finished (e.g. after a fatal non-disconnect error).
    pub(crate) fn ensure_pump_running(&self) {
        let mut task_slot = self.pump_task.lock().expect("pump_task mutex poisoned");
        if let Some(handle) = task_slot.take() {
            if handle.is_finished() {
                debug!(
                    "rx_session: previous pump for {} finished, restarting",
                    self.connection_id
                );
            } else {
                *task_slot = Some(handle);
                return;
            }
        }
        // If the previous pump was cancelled, the token is stale. Replace.
        {
            let mut token = self.pump_token.lock().expect("pump_token mutex poisoned");
            if token.is_cancelled() {
                *token = CancellationToken::new();
            }
        }
        let connection = Arc::clone(&self.connection);
        let token = self
            .pump_token
            .lock()
            .expect("pump_token mutex poisoned")
            .clone();
        let consumers = Arc::clone(&self.consumers);
        let ring = Arc::clone(&self.ring);
        let handle = tokio::spawn(pump_loop(connection, token, consumers, ring));
        *task_slot = Some(handle);
        debug!("rx_session: pump started for {}", self.connection_id);
    }

    /// Register a new blocking consumer and return its receiver.
    ///
    /// The consumer will only see bytes that arrive *after* registration.
    /// The pump is assumed already running (started at open time).
    pub fn register_blocking(&self) -> mpsc::Receiver<RxEvent> {
        let (consumer, rx) = RxConsumer::new(Self::CONSUMER_CHANNEL_CAPACITY);
        self.consumers
            .lock()
            .expect("consumers mutex poisoned")
            .blocking
            .push(consumer);
        debug!(
            "rx_session: blocking consumer registered for {}",
            self.connection_id
        );
        rx
    }

    /// Register a new streaming consumer and return its receiver.
    ///
    /// Same future-only semantics as [`Self::register_blocking`].
    pub fn register_streaming(&self) -> mpsc::Receiver<RxEvent> {
        let (consumer, rx) = RxConsumer::new(Self::CONSUMER_CHANNEL_CAPACITY);
        self.consumers
            .lock()
            .expect("consumers mutex poisoned")
            .streaming
            .push(consumer);
        debug!(
            "rx_session: streaming consumer registered for {}",
            self.connection_id
        );
        rx
    }

    /// Prune consumers whose receivers have been dropped.
    ///
    /// Under the always-on pump model the pump keeps running regardless of
    /// consumer count — bytes are captured to the ring from open to close.
    /// Always returns `false` (pump is never cancelled by prune).
    ///
    /// Call this after removing a stream handle (e.g., unsubscribe) to clean
    /// up closed consumer channels so the pump doesn't waste work on dead
    /// fanout targets.
    pub fn prune_consumers(&self) -> bool {
        let mut reg = self.consumers.lock().expect("consumers mutex poisoned");
        reg.prune_closed();
        debug!(
            "rx_session: pruned consumers for {} (pump stays alive)",
            self.connection_id
        );
        false
    }

    /// Signal the pump to stop and notify all consumers with [`RxEvent::Closed`].
    ///
    /// This only cancels the pump token — it does **not** wait for the pump
    /// task to finish. Call [`Self::join_pump`] after shutdown to await
    /// pump exit. Safe to call multiple times.
    pub fn shutdown(&self) {
        self.pump_token
            .lock()
            .expect("pump_token mutex poisoned")
            .cancel();
        self.consumers
            .lock()
            .expect("consumers mutex poisoned")
            .fanout(RxEvent::Closed);
        info!("rx_session: shut down for {}", self.connection_id);
    }

    /// Wait for the pump task to finish. Call after `shutdown` for
    /// deterministic cleanup.
    pub async fn join_pump(&self) {
        let handle = self
            .pump_task
            .lock()
            .expect("pump_task mutex poisoned")
            .take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }

    /// Shut down and await pump exit in one step, then release the ring
    /// budget reservation.
    pub async fn shutdown_and_join(&self) {
        self.shutdown();
        self.join_pump().await;
        // Release the ring's budget reservation so bytes are available
        // for the next open.
        let reservation = self
            .budget_reservation
            .lock()
            .expect("budget_reservation mutex poisoned")
            .take();
        drop(reservation);
        debug!(
            "rx_session: budget released for {} ({} bytes)",
            self.connection_id, self.ring_capacity
        );
    }
}

// ---- Pump loop (standalone async function, not a method) -------------------

/// Background task that reads from the serial port, appends to the ring,
/// and fans out to consumers.
///
/// Runs from open to close. On fatal disconnect, pauses (50ms sleep loop)
/// until the connection state returns to `Open` (reconnect succeeded) or
/// `Closed` (reconnect exhausted/disabled). The ring persists across
/// disconnect/reconnect cycles — `end_offset` is monotonic.
async fn pump_loop(
    connection: Arc<SerialConnection>,
    token: CancellationToken,
    consumers: Arc<StdMutex<ConsumerRegistry>>,
    ring: Arc<RxRing>,
) {
    let conn_id = connection.id().to_string();
    let mut buf = vec![0u8; RxSession::PUMP_READ_SIZE];
    info!("rx_session: pump entered for {conn_id}");

    loop {
        if token.is_cancelled() {
            break;
        }
        let read_result = tokio::select! {
            _ = token.cancelled() => break,
            res = connection.read(&mut buf, Some(100)) => res,
        };
        match read_result {
            Ok(0) | Err(crate::error::SerialError::ReadTimeout) => {
                // No data this cycle. Prune dead consumers but keep running.
                let mut reg = consumers.lock().expect("consumers mutex poisoned");
                reg.prune_closed();
                continue;
            }
            Ok(n) => {
                connection.log().rx_data(n);
                let chunk = buf[..n].to_vec();

                // Append to the ring first (capture all bytes).
                ring.append(&chunk);

                // Fan out to legacy consumers (read/subscribe still work).
                let mut reg = consumers.lock().expect("consumers mutex poisoned");
                reg.fanout(RxEvent::Data(chunk));
            }
            Err(e) => {
                error!("rx_session: read error on {conn_id}: {e}");
                let is_fatal = if let crate::error::SerialError::IoError(ref io_err) = e {
                    crate::serial::is_fatal_disconnect(io_err)
                } else {
                    false
                };
                if is_fatal {
                    connection
                        .mark_disconnected(format!("Read error: {e}"))
                        .await;

                    // Pause: wait for reconnect or close. The ring persists
                    // across disconnect/reconnect — offsets stay monotonic.
                    let pause_start = std::time::Instant::now();
                    debug!("rx_session: pump for {conn_id} pausing (disconnected)");
                    loop {
                        if token.is_cancelled() {
                            info!(
                                "rx_session: pump for {conn_id} exiting during pause (cancelled)"
                            );
                            return;
                        }
                        let state = connection.state();
                        match state {
                            crate::serial::ConnectionState::Open => {
                                info!(
                                    "rx_session: pump for {conn_id} resumed after {:.0?}",
                                    pause_start.elapsed()
                                );
                                break; // resume the outer read loop
                            }
                            crate::serial::ConnectionState::Closed => {
                                info!(
                                    "rx_session: pump for {conn_id} exiting (connection closed during pause)"
                                );
                                return;
                            }
                            _ => {
                                // Disconnected or Reconnecting — keep waiting.
                                tokio::time::sleep(Duration::from_millis(50)).await;
                            }
                        }
                    }
                    // Resume reading from the (now reopened) connection.
                    // The ring's end_offset is where we left off.
                    continue;
                }

                // Non-fatal error: fan out error event and exit.
                consumers
                    .lock()
                    .expect("consumers mutex poisoned")
                    .fanout(RxEvent::Error(e.to_string()));
                break;
            }
        }
    }

    info!("rx_session: pump exiting for {conn_id}");
}

// ---- Session manager -------------------------------------------------------

/// Manages [`RxSession`] instances keyed by connection id.
///
/// One session per connection. Creating a session is idempotent. Removing a
/// session shuts down its pump, awaits pump task exit, releases the ring
/// budget, and drops consumers deterministically.
pub struct RxSessionManager {
    sessions: AsyncMutex<HashMap<String, Arc<RxSession>>>,
    budget: Arc<dyn BufferBudget>,
}

impl RxSessionManager {
    pub fn new(budget: Arc<dyn BufferBudget>) -> Self {
        Self {
            sessions: AsyncMutex::new(HashMap::new()),
            budget,
        }
    }

    /// Get an existing session or create one for the given connection.
    ///
    /// The session is created with a budgeted ring of `ring_capacity` bytes.
    /// The pump is started automatically on first creation. Returns the
    /// existing session if one is already registered (ring_capacity is
    /// ignored in that case — the ring was already allocated at open time).
    ///
    /// Returns `Err(String)` if ring budget reservation fails.
    pub async fn get_or_create(
        &self,
        connection: Arc<SerialConnection>,
        ring_capacity: usize,
    ) -> Result<Arc<RxSession>, String> {
        let conn_id = connection.id().to_string();
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(&conn_id) {
            return Ok(Arc::clone(existing));
        }
        let session = Arc::new(RxSession::new(connection, ring_capacity, &self.budget)?);
        session.ensure_pump_running();
        sessions.insert(conn_id, Arc::clone(&session));
        debug!(
            "rx_session: created new session for connection {}",
            session.connection_id()
        );
        Ok(session)
    }

    /// Remove and shut down a session by connection id.
    ///
    /// Cancels the pump, sends [`RxEvent::Closed`] to consumers, and awaits
    /// pump task exit. Does nothing if no session exists for the given id.
    pub async fn remove(&self, connection_id: &str) {
        let session = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(connection_id)
        };
        if let Some(session) = session {
            session.shutdown_and_join().await;
            info!("rx_session: removed session for connection {connection_id}");
        }
    }

    /// Number of active sessions.
    pub async fn count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    /// Look up an existing session by connection id, if one exists.
    pub async fn get(&self, connection_id: &str) -> Option<Arc<RxSession>> {
        self.sessions.lock().await.get(connection_id).cloned()
    }
}

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer_budget::AtomicBudget;
    use crate::serial::test_support::loopback_connection;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    /// Convenience: an unlimited budget for tests where budget isn't under test.
    fn test_budget() -> Arc<dyn BufferBudget> {
        Arc::new(AtomicBudget::new(1024 * 1024 * 1024, 1024 * 1024 * 1024))
    }

    /// Convenience: create an RxSession directly with a 1024-byte ring.
    fn test_session(conn: Arc<SerialConnection>) -> RxSession {
        let budget = test_budget();
        RxSession::new(conn, 1024, &budget).expect("test session creation")
    }

    /// Convenience: create an RxSessionManager with a test budget.
    fn test_manager() -> RxSessionManager {
        RxSessionManager::new(test_budget())
    }

    fn collect_events(rx: &mut mpsc::Receiver<RxEvent>) -> Vec<RxEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    #[test]
    fn rx_event_clone_copies_data() {
        let event = RxEvent::Data(b"hello".to_vec());
        let cloned = event.clone();
        assert!(matches!(cloned, RxEvent::Data(ref b) if b == b"hello"));
    }

    // ── Manager idempotency ────────────────────────────────────────────────

    #[tokio::test]
    async fn manager_get_or_create_returns_same_session() {
        let (conn, _peer) = loopback_connection("test-idem");
        let conn = Arc::new(conn);
        let mgr = test_manager();
        let s1 = mgr.get_or_create(Arc::clone(&conn), 1024).await.unwrap();
        let s2 = mgr.get_or_create(Arc::clone(&conn), 2048).await.unwrap();
        assert!(Arc::ptr_eq(&s1, &s2));
        assert_eq!(mgr.count().await, 1);
    }

    // ── Manager remove ─────────────────────────────────────────────────────

    #[tokio::test]
    async fn manager_remove_awaits_pump_exit() {
        let (conn, _peer) = loopback_connection("test-remove-await");
        let conn = Arc::new(conn);
        let mgr = test_manager();
        let session = mgr.get_or_create(Arc::clone(&conn), 1024).await.unwrap();
        let id = session.connection_id().to_string();
        let _rx = session.register_blocking();
        assert!(!session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
        mgr.remove(&id).await;
        assert_eq!(mgr.count().await, 0);
        assert!(session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
        assert!(
            session.pump_task.lock().expect("pump_task").is_none(),
            "pump task handle should be consumed after join"
        );
    }

    #[tokio::test]
    async fn manager_remove_nonexistent_is_noop() {
        let mgr = test_manager();
        mgr.remove("does-not-exist").await;
        assert_eq!(mgr.count().await, 0);
    }

    // ── Pump lifecycle (always-on model) ───────────────────────────────────

    #[tokio::test]
    async fn get_or_create_starts_pump() {
        let (conn, _peer) = loopback_connection("test-pump-start");
        let conn = Arc::new(conn);
        let mgr = test_manager();
        let session = mgr.get_or_create(Arc::clone(&conn), 1024).await.unwrap();
        assert!(session.pump_task.lock().expect("pump_task").is_some());
    }

    #[tokio::test]
    async fn session_shutdown_cancels_pump_token() {
        let (conn, _peer) = loopback_connection("test-shutdown");
        let conn = Arc::new(conn);
        let session = test_session(conn);
        session.ensure_pump_running();
        assert!(!session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
        session.shutdown();
        assert!(session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
    }

    // ── Consumer fanout ────────────────────────────────────────────────────

    #[tokio::test]
    async fn consumer_receives_data_after_registration() {
        let (conn, mut peer) = loopback_connection("test-fanout-data");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

        let mut rx = session.register_blocking();

        peer.write_all(b"hello").await.unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;
        session.shutdown();

        let received = collect_events(&mut rx);
        let has_data = received.iter().any(|e| matches!(e, RxEvent::Data(_)));
        assert!(
            has_data,
            "consumer should have received at least one Data event"
        );
    }

    #[tokio::test]
    async fn two_consumers_both_receive_future_data() {
        let (conn, mut peer) = loopback_connection("test-two-consumers");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

        let mut rx1 = session.register_blocking();
        let mut rx2 = session.register_streaming();

        peer.write_all(b"abc").await.unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;
        session.shutdown();

        let events1 = collect_events(&mut rx1);
        let events2 = collect_events(&mut rx2);

        let has_data1 = events1.iter().any(|e| matches!(e, RxEvent::Data(_)));
        let has_data2 = events2.iter().any(|e| matches!(e, RxEvent::Data(_)));
        assert!(has_data1, "blocking consumer should have received data");
        assert!(has_data2, "streaming consumer should have received data");
    }

    // ── Remove lifecycle ───────────────────────────────────────────────────

    #[tokio::test]
    async fn removing_session_awaits_pump_and_drops_consumers() {
        let (conn, _peer) = loopback_connection("test-remove-lifecycle");
        let conn = Arc::new(conn);
        let mgr = test_manager();
        let session = mgr.get_or_create(Arc::clone(&conn), 1024).await.unwrap();
        let conn_id = session.connection_id().to_string();
        let _rx = session.register_blocking();
        assert!(!session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
        mgr.remove(&conn_id).await;
        assert!(session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
        assert_eq!(mgr.count().await, 0);
        assert!(
            session.pump_task.lock().expect("pump_task").is_none(),
            "pump handle should be consumed after remove"
        );
    }

    // ── Close causes pump exit ─────────────────────────────────────────────

    #[tokio::test]
    async fn connection_close_causes_pump_exit() {
        let (conn, mut peer) = loopback_connection("test-close-exit");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();
        let mut rx = session.register_blocking();

        peer.write_all(b"bye").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        conn.close().await.unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;

        let events = collect_events(&mut rx);
        let has_closed_or_error = events
            .iter()
            .any(|e| matches!(e, RxEvent::Closed | RxEvent::Error(_)));
        assert!(
            has_closed_or_error,
            "consumer should receive Closed or Error event when connection closes"
        );
    }

    // ── Shutdown clean exit ────────────────────────────────────────────────

    #[tokio::test]
    async fn pump_exits_cleanly_on_shutdown_without_hanging() {
        let (conn, _peer) = loopback_connection("test-pump-clean-exit");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();
        let _rx = session.register_blocking();

        session.shutdown_and_join().await;

        assert!(
            session.pump_task.lock().expect("pump_task").is_none(),
            "pump task handle should be consumed after join"
        );
    }

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let (conn, _peer) = loopback_connection("test-shutdown-idem");
        let conn = Arc::new(conn);
        let session = test_session(conn);
        session.ensure_pump_running();
        let mut rx = session.register_blocking();
        session.shutdown();
        session.shutdown();
        session.shutdown();
        assert!(session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
        let events = collect_events(&mut rx);
        let closed_count = events
            .iter()
            .filter(|e| matches!(e, RxEvent::Closed))
            .count();
        assert!(closed_count >= 1, "at least one Closed event expected");
    }

    // ── Stress: repeated create + remove ──────────────────────────────────

    #[tokio::test]
    async fn repeated_create_remove_no_leaked_pump_tasks() {
        let iterations = 10;
        for i in 0..iterations {
            let port_name = format!("test-stress-{i}");
            let (conn, _peer) = loopback_connection(&port_name);
            let conn = Arc::new(conn);
            let mgr = test_manager();
            let session = mgr.get_or_create(Arc::clone(&conn), 1024).await.unwrap();
            let conn_id = session.connection_id().to_string();
            let _rx = session.register_blocking();

            // Verify pump is running
            assert!(
                session.pump_task.lock().expect("pump_task").is_some(),
                "pump should be running on iteration {i}"
            );

            mgr.remove(&conn_id).await;

            // Verify pump task handle was consumed
            assert!(
                session.pump_task.lock().expect("pump_task").is_none(),
                "pump handle should be consumed after remove on iteration {i}"
            );
            assert!(
                session
                    .pump_token
                    .lock()
                    .expect("pump_token")
                    .is_cancelled(),
                "pump token should be cancelled after remove on iteration {i}"
            );
            assert_eq!(mgr.count().await, 0);
        }
    }

    // ── Consumer overflow / drop ───────────────────────────────────────────

    #[tokio::test]
    async fn full_consumer_is_dropped_from_registry() {
        let (conn, mut peer) = loopback_connection("test-full-consumer");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

        // Register a consumer with a tiny channel capacity to force overflow.
        let mut rx = session.register_blocking();

        // Send enough data to exceed the channel capacity (256 events).
        for _ in 0..300 {
            peer.write_all(b"x").await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        session.shutdown();

        let events = collect_events(&mut rx);
        assert!(
            !events.is_empty(),
            "should have received some events before consumer was dropped"
        );
    }

    #[tokio::test]
    async fn dropped_receiver_removed_from_registry() {
        let (conn, _peer) = loopback_connection("test-dropped-receiver");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

        let rx = session.register_blocking();
        let consumer_count_before = {
            let registry = session.consumers.lock().expect("consumers");
            registry.blocking.len() + registry.streaming.len()
        };
        assert_eq!(consumer_count_before, 1);

        drop(rx);

        session.shutdown();

        let consumer_count_after = {
            let registry = session.consumers.lock().expect("consumers");
            registry.blocking.len() + registry.streaming.len()
        };
        assert_eq!(
            consumer_count_after, 0,
            "dropped receiver should be removed from registry after fanout attempt"
        );
    }

    // ── NEW: Ring capture without consumers ────────────────────────────────

    #[tokio::test]
    async fn ring_captures_bytes_without_consumers() {
        let (conn, mut peer) = loopback_connection("test-ring-no-consumers");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

        // No consumers registered. Write data; the pump must still capture to ring.
        peer.write_all(b"hi").await.unwrap();

        // Wait for pump to process the bytes.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let end = session.ring().end_offset();
        assert_eq!(end, 2, "ring should have captured 2 bytes");
        let slice = session.ring().read_from(0, 10);
        assert_eq!(slice.bytes, b"hi");
        assert_eq!(slice.bytes_lost, 0);

        session.shutdown_and_join().await;
    }

    #[tokio::test]
    async fn ring_captures_bytes_alongside_consumers() {
        let (conn, mut peer) = loopback_connection("test-ring-with-consumers");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();
        let mut rx = session.register_blocking();

        peer.write_all(b"hello").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Ring must have the data.
        let end = session.ring().end_offset();
        assert_eq!(end, 5);
        let slice = session.ring().read_from(0, 10);
        assert_eq!(slice.bytes, b"hello");

        // Consumer must also have the data.
        let events = collect_events(&mut rx);
        let has_data = events.iter().any(|e| matches!(e, RxEvent::Data(_)));
        assert!(has_data, "consumer should have received data too");

        session.shutdown_and_join().await;
    }

    // ── NEW: Pump continues without consumers ─────────────────────────────

    #[tokio::test]
    async fn pump_keeps_running_when_no_consumers() {
        let (conn, mut peer) = loopback_connection("test-pump-no-consumers");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

        // No consumers registered. Write some data.
        peer.write_all(b"before").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let end_before = session.ring().end_offset();
        assert_eq!(end_before, 6, "ring should have 'before'");

        // Pump must keep running — write more data.
        peer.write_all(b"after").await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;

        let end_after = session.ring().end_offset();
        assert_eq!(end_after, 11, "ring should have 'before'+'after'");
        let slice = session.ring().read_from(0, 20);
        assert_eq!(&slice.bytes[..6], b"before");
        assert_eq!(&slice.bytes[6..], b"after");

        session.shutdown_and_join().await;
    }

    // ── NEW: prune_consumers does not cancel pump ──────────────────────────

    #[tokio::test]
    async fn prune_consumers_returns_false_pump_keeps_running() {
        let (conn, _peer) = loopback_connection("test-prune-no-cancel");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

        // Register a consumer then drop the receiver.
        let rx = session.register_blocking();
        drop(rx);

        // Prune should return false (pump is not cancelled).
        let cancelled = session.prune_consumers();
        assert!(!cancelled, "prune_consumers must not cancel pump");
        assert!(
            !session
                .pump_token
                .lock()
                .expect("pump_token")
                .is_cancelled(),
            "pump token must still be active after prune"
        );

        session.shutdown_and_join().await;
    }

    // ── NEW: Budget charged at open, released at close ────────────────────

    #[tokio::test]
    async fn budget_charged_at_open_released_at_close() {
        let budget: Arc<dyn BufferBudget> = Arc::new(AtomicBudget::new(4096, 4096));
        let avail_before = budget.available();
        assert_eq!(avail_before, 4096);

        let (conn, _peer) = loopback_connection("test-budget-lifecycle");
        let conn = Arc::new(conn);
        let session = RxSession::new(conn, 1024, &budget).expect("session create");

        let avail_after_open = budget.available();
        assert_eq!(avail_after_open, 4096 - 1024);

        // Ring accessor works.
        assert_eq!(session.ring_capacity(), 1024);
        assert_eq!(session.read_cursor(), 0);

        session.shutdown_and_join().await;

        let avail_after_close = budget.available();
        assert_eq!(avail_after_close, 4096);
    }

    #[tokio::test]
    async fn budget_open_fails_if_insufficient() {
        // program_limit=512, tool_limit=1024 so request of 768 passes tool check
        // but exceeds program budget.
        let budget: Arc<dyn BufferBudget> = Arc::new(AtomicBudget::new(1024, 1024));
        // Consume some budget first so available < requested.
        let _hold = budget.try_reserve(900).expect("pre-reserve");
        let result = RxSession::new(
            Arc::new(loopback_connection("test-budget-fail").0),
            200,
            &budget,
        );
        match result {
            Err(e) => assert!(
                e.contains("insufficient program buffer budget"),
                "expected budget exhaustion error, got: {e}"
            ),
            Ok(_) => panic!("expected error for insufficient budget"),
        }
    }

    #[tokio::test]
    async fn budget_open_fails_if_zero_capacity() {
        let budget = test_budget();
        let result = RxSession::new(
            Arc::new(loopback_connection("test-zero-capacity").0),
            0,
            &budget,
        );
        match result {
            Err(e) => assert!(
                e.contains("ring_capacity must be > 0"),
                "expected zero capacity error, got: {e}"
            ),
            Ok(_) => panic!("expected error for zero capacity"),
        }
    }

    // ── NEW: Read cursor accessors ─────────────────────────────────────────

    #[tokio::test]
    async fn read_cursor_starts_at_zero() {
        let (conn, _peer) = loopback_connection("test-cursor-zero");
        let conn = Arc::new(conn);
        let session = test_session(conn);
        assert_eq!(session.read_cursor(), 0);
    }

    #[tokio::test]
    async fn set_read_cursor_updates_value() {
        let (conn, _peer) = loopback_connection("test-cursor-set");
        let conn = Arc::new(conn);
        let session = test_session(conn);
        session.set_read_cursor(42);
        assert_eq!(session.read_cursor(), 42);
        session.set_read_cursor(0);
        assert_eq!(session.read_cursor(), 0);
    }

    // ── NEW: ensure_pump_running is idempotent ─────────────────────────────

    #[tokio::test]
    async fn ensure_pump_running_is_idempotent() {
        let (conn, _peer) = loopback_connection("test-pump-idem");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));

        assert!(session.pump_task.lock().expect("pump_task").is_none());
        session.ensure_pump_running();
        assert!(session.pump_task.lock().expect("pump_task").is_some());

        // Call again — verify pump is still running (not restarted).
        session.ensure_pump_running();
        assert!(session.pump_task.lock().expect("pump_task").is_some());
        assert!(!session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());

        session.shutdown_and_join().await;
    }

    // Verify ensure_pump_running starts a new pump if previous one completed.
    #[tokio::test]
    async fn ensure_pump_running_restarts_after_pump_finishes() {
        let (conn, _peer) = loopback_connection("test-pump-restart");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));

        session.ensure_pump_running();
        // Cancel to simulate pump finishing abnormally.
        session.shutdown();
        session.join_pump().await;

        // Pump handle was taken by join_pump.
        assert!(session.pump_task.lock().expect("pump_task").is_none());

        // Calling ensure_pump_running again should start a new pump.
        session.ensure_pump_running();
        assert!(session.pump_task.lock().expect("pump_task").is_some());

        session.shutdown_and_join().await;
    }
}
