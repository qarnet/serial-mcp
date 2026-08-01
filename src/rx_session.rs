//! Per-connection RX session core for unified serial data capture.
//!
//! Each open serial connection has at most one [`RxSession`], which owns
//! a single background pump task that reads from the serial port. The pump
//! appends every received byte to a per-connection [`RxRing`].
//!
//! The pump runs from `open` to `close` — bytes are captured regardless of
//! active tool calls. The ring is a budgeted allocation charged at open and
//! released at close.
//!
//! Both `read` and `subscribe` read from the ring via private or shared
//! cursors. `read` advances a shared cursor; `subscribe` has per-call
//! private cursors that do not move the shared read cursor.
//!
//! - always-on pump from open to close
//! - RxRing capture (all bytes preserved)
//! - budget reservation for the ring at construction
//! - pause on disconnect, resume on reconnect (same ring, same offsets)

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

use crate::buffer_budget::{BufferBudget, BufferReservation};
use crate::rx_ring::RxRing;
use crate::serial::SerialConnection;

// ---- Per-connection session ------------------------------------------------

/// Manages one pump task and its registered consumers for a single connection.
///
/// The [`RxRing`] captures all RX bytes from open to close. The pump appends
/// to the ring. Both `read` and `subscribe` read from the ring via cursors.
pub struct RxSession {
    connection_id: String,
    connection: Arc<SerialConnection>,
    pump_task: StdMutex<Option<JoinHandle<()>>>,
    pump_token: StdMutex<CancellationToken>,
    /// Async pump gate (Phase 5): the pump holds this across one complete
    /// read + ring append. `capture_boot` acquires the same gate so its mark
    /// cannot race an in-flight pump read/append — a byte physically read
    /// before the reset can never append after the mark.
    pump_gate: Arc<AsyncMutex<()>>,
    /// Per-connection ring buffer capturing all RX bytes from open to close.
    ring: Arc<RxRing>,
    /// Shared read cursor for `read`/`flush`. Absolute u64 offset.
    read_cursor: StdMutex<u64>,
    /// Stored ring capacity so close can release the correct byte count.
    ring_capacity: usize,
    /// RAII reservation for the ring's budget allocation. Dropped at shutdown.
    budget_reservation: StdMutex<Option<Box<dyn BufferReservation>>>,
}

impl RxSession {
    const PUMP_READ_SIZE: usize = 4096;

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
            pump_task: StdMutex::new(None),
            pump_token: StdMutex::new(CancellationToken::new()),
            pump_gate: Arc::new(AsyncMutex::new(())),
            ring: Arc::new(RxRing::new(ring_capacity)),
            read_cursor: StdMutex::new(0),
            ring_capacity,
            budget_reservation: StdMutex::new(Some(reservation)),
        })
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    // ── Ring + cursor accessors ─────────────────────────────────────────────

    /// Borrow the ring buffer.
    pub(crate) fn ring(&self) -> &RxRing {
        &self.ring
    }

    /// Current shared read cursor (absolute u64 offset).
    pub(crate) fn read_cursor(&self) -> u64 {
        *self.read_cursor.lock().expect("read_cursor mutex poisoned")
    }

    /// Set the shared read cursor.
    pub(crate) fn set_read_cursor(&self, off: u64) {
        *self.read_cursor.lock().expect("read_cursor mutex poisoned") = off;
    }

    /// Ring capacity in bytes.
    pub(crate) fn ring_capacity(&self) -> usize {
        self.ring_capacity
    }

    /// Acquire the pump gate (Phase 5): waits for any in-flight pump
    /// read+append to finish, then blocks the pump from reading again until
    /// the returned guard is dropped. `capture_boot` holds this across the
    /// OS-input purge, the live-edge mark, and the reset-line assertion so
    /// bytes predating the mark can never be appended after it.
    pub(crate) async fn pump_gate_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.pump_gate.lock().await
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
        let ring = Arc::clone(&self.ring);
        let pump_gate = Arc::clone(&self.pump_gate);
        let handle = tokio::spawn(pump_loop(connection, token, ring, pump_gate));
        *task_slot = Some(handle);
        debug!("rx_session: pump started for {}", self.connection_id);
    }

    /// Signal the pump to stop.
    ///
    /// This only cancels the pump token — it does **not** wait for the pump
    /// task to finish. Call [`Self::join_pump`] after shutdown to await
    /// pump exit. Safe to call multiple times.
    pub fn shutdown(&self) {
        self.pump_token
            .lock()
            .expect("pump_token mutex poisoned")
            .cancel();
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

/// Background task that reads from the serial port and appends to the ring.
///
/// Runs from open to close. On fatal disconnect, pauses (50ms sleep loop)
/// until the connection state returns to `Open` (reconnect succeeded) or
/// `Closed` (reconnect exhausted/disabled). The ring persists across
/// disconnect/reconnect cycles — `end_offset` is monotonic.
///
/// The pump holds `pump_gate` across one complete read + ring append and
/// releases it before the disconnect pause/sleep. `capture_boot` acquires
/// the same gate to establish an atomic live-edge mark (Phase 5): a byte
/// physically read before the mark can never be appended after it.
async fn pump_loop(
    connection: Arc<SerialConnection>,
    token: CancellationToken,
    ring: Arc<RxRing>,
    pump_gate: Arc<AsyncMutex<()>>,
) {
    let conn_id = connection.id().to_string();
    let mut buf = vec![0u8; RxSession::PUMP_READ_SIZE];
    info!("rx_session: pump entered for {conn_id}");

    loop {
        if token.is_cancelled() {
            break;
        }
        // Gate held across the read AND the ring append: this is the
        // atomic unit capture_boot waits on.
        let gate = pump_gate.lock().await;
        let read_result = tokio::select! {
            _ = token.cancelled() => break,
            res = connection.read(&mut buf, Some(100)) => res,
        };
        match read_result {
            Ok(0) | Err(crate::error::SerialError::ReadTimeout) => {
                // No data this cycle. Keep running.
                drop(gate);
                continue;
            }
            Ok(n) => {
                connection.log().rx_data(n);
                let chunk = buf[..n].to_vec();

                // Append to the ring (capture all bytes).
                ring.append(&chunk);
                drop(gate);
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
                    // Release the gate before the pause/sleep loop so
                    // capture_boot cannot block behind a paused pump.
                    drop(gate);
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

                // Non-fatal error: pump exits.
                error!("rx_session: non-fatal read error on {conn_id}, pump exiting: {e}");
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
/// session shuts down its pump, awaits pump task exit, and releases the ring
/// budget.
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
    /// Cancels the pump and awaits pump task exit.
    /// Does nothing if no session exists for the given id.
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

    // ── Remove lifecycle ───────────────────────────────────────────────────

    #[tokio::test]
    async fn removing_session_awaits_pump_and_drops_consumers() {
        let (conn, _peer) = loopback_connection("test-remove-lifecycle");
        let conn = Arc::new(conn);
        let mgr = test_manager();
        let session = mgr.get_or_create(Arc::clone(&conn), 1024).await.unwrap();
        let conn_id = session.connection_id().to_string();
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

        peer.write_all(b"bye").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        conn.close().await.unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;

        // After close, the pump should have exited. Verify by checking the
        // pump task handle is consumed.
        assert!(
            session.pump_task.lock().expect("pump_task").is_none()
                || session
                    .pump_task
                    .lock()
                    .expect("pump_task")
                    .as_ref()
                    .map(|h| h.is_finished())
                    .unwrap_or(true),
            "pump should finish after connection close"
        );
    }

    // ── Shutdown clean exit ────────────────────────────────────────────────

    #[tokio::test]
    async fn pump_exits_cleanly_on_shutdown_without_hanging() {
        let (conn, _peer) = loopback_connection("test-pump-clean-exit");
        let conn = Arc::new(conn);
        let session = test_session(Arc::clone(&conn));
        session.ensure_pump_running();

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
        session.shutdown();
        session.shutdown();
        session.shutdown();
        assert!(session
            .pump_token
            .lock()
            .expect("pump_token")
            .is_cancelled());
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
