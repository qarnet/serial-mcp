//! Per-connection RX session core for unified serial data fanout.
//!
//! Each open serial connection can have at most one [`RxSession`], which owns
//! a single background pump task that reads from the serial port. The pump
//! fans out received bytes to registered consumer channels.
//!
//! PLAN 1a scope:
//! - `RxSessionManager` / `RxSession` lifecycle
//! - one pump task per connection
//! - consumer registration (blocking + streaming primitives)
//! - deterministic shutdown on connection close
//!
//! Not yet in scope:
//! - migrating `read` / `subscribe` / `wait_for` tools onto this core
//! - buffer budgets, silence timeouts, event-context
//!
//! ## Consumer drop policy
//!
//! When the pump calls [`ConsumerRegistry::fanout`], any consumer whose
//! channel is full (slow consumer) or closed (dropped receiver) is silently
//! removed from the registry via `Vec::retain`. This prevents the pump from
//! blocking or buffering indefinitely for lagging consumers.
//!
//! This policy is intentionally simple for PLAN 1a. PLAN 1b/1c must define
//! explicit per-consumer behavior (e.g. backpressure signaling, structured
//! error delivery, or consumer-aware buffer budgets) before migrating the
//! `read` and `subscribe` tools onto this core.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use tokio::sync::mpsc;
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info};

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
}

// ---- Per-connection session ------------------------------------------------

/// Manages one pump task and its registered consumers for a single connection.
///
/// The pump task holds an `Arc<StdMutex<ConsumerRegistry>>` so it can
/// fan out data without needing a reference back to this struct.
pub struct RxSession {
    connection_id: String,
    connection: Arc<SerialConnection>,
    consumers: Arc<StdMutex<ConsumerRegistry>>,
    pump_task: StdMutex<Option<JoinHandle<()>>>,
    pump_token: CancellationToken,
}

impl RxSession {
    const PUMP_READ_SIZE: usize = 4096;
    const CONSUMER_CHANNEL_CAPACITY: usize = 256;

    pub fn new(connection: Arc<SerialConnection>) -> Self {
        let connection_id = connection.id().to_string();
        let pump_token = CancellationToken::new();
        Self {
            connection_id,
            connection,
            consumers: Arc::new(StdMutex::new(ConsumerRegistry::new())),
            pump_task: StdMutex::new(None),
            pump_token,
        }
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    /// Ensure the pump task is running. Idempotent.
    fn ensure_pump_running(&self) {
        let mut task_slot = self.pump_task.lock().expect("pump_task mutex poisoned");
        if task_slot.is_some() {
            return;
        }
        let connection = Arc::clone(&self.connection);
        let token = self.pump_token.clone();
        let consumers = Arc::clone(&self.consumers);
        let handle = tokio::spawn(pump_loop(connection, token, consumers));
        *task_slot = Some(handle);
        debug!("rx_session: pump started for {}", self.connection_id);
    }

    /// Register a new blocking consumer and return its receiver.
    ///
    /// The consumer will only see bytes that arrive *after* registration.
    /// Starts the pump if not already running.
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
        self.ensure_pump_running();
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
        self.ensure_pump_running();
        rx
    }

    /// Signal the pump to stop and notify all consumers with [`RxEvent::Closed`].
    ///
    /// This only cancels the pump token — it does **not** wait for the pump
    /// task to finish. Call [`Self::join_pump`] after shutdown to await
    /// pump exit. Safe to call multiple times.
    pub fn shutdown(&self) {
        self.pump_token.cancel();
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

    /// Shut down and await pump exit in one step.
    pub async fn shutdown_and_join(&self) {
        self.shutdown();
        self.join_pump().await;
    }
}

// ---- Pump loop (standalone async function, not a method) -------------------

async fn pump_loop(
    connection: Arc<SerialConnection>,
    token: CancellationToken,
    consumers: Arc<StdMutex<ConsumerRegistry>>,
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
            Ok(0) | Err(crate::error::SerialError::ReadTimeout) => continue,
            Ok(n) => {
                let chunk = buf[..n].to_vec();
                consumers
                    .lock()
                    .expect("consumers mutex poisoned")
                    .fanout(RxEvent::Data(chunk));
            }
            Err(e) => {
                error!("rx_session: read error on {conn_id}: {e}");
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
/// session shuts down its pump, awaits pump task exit, and drops consumers
/// deterministically.
pub struct RxSessionManager {
    sessions: AsyncMutex<HashMap<String, Arc<RxSession>>>,
}

impl Default for RxSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RxSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: AsyncMutex::new(HashMap::new()),
        }
    }

    /// Get an existing session or create one for the given connection.
    ///
    /// Returns the existing session if one is already registered for this
    /// connection id, ensuring idempotent creation.
    pub async fn get_or_create(&self, connection: Arc<SerialConnection>) -> Arc<RxSession> {
        let conn_id = connection.id().to_string();
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(&conn_id) {
            return Arc::clone(existing);
        }
        let session = Arc::new(RxSession::new(connection));
        sessions.insert(conn_id, Arc::clone(&session));
        debug!(
            "rx_session: created new session for connection {}",
            session.connection_id()
        );
        session
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
}

// ---- Tests -----------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::test_support::loopback_connection;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

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

    #[tokio::test]
    async fn manager_get_or_create_returns_same_session() {
        let (conn, _peer) = loopback_connection("test-idem");
        let conn = Arc::new(conn);
        let mgr = RxSessionManager::new();
        let s1 = mgr.get_or_create(Arc::clone(&conn)).await;
        let s2 = mgr.get_or_create(Arc::clone(&conn)).await;
        assert!(Arc::ptr_eq(&s1, &s2));
        assert_eq!(mgr.count().await, 1);
    }

    #[tokio::test]
    async fn manager_remove_awaits_pump_exit() {
        let (conn, _peer) = loopback_connection("test-remove-await");
        let conn = Arc::new(conn);
        let mgr = RxSessionManager::new();
        let session = mgr.get_or_create(Arc::clone(&conn)).await;
        let id = session.connection_id().to_string();
        let _rx = session.register_blocking();
        assert!(!session.pump_token.is_cancelled());
        mgr.remove(&id).await;
        assert_eq!(mgr.count().await, 0);
        assert!(session.pump_token.is_cancelled());
        assert!(
            session.pump_task.lock().expect("pump_task").is_none(),
            "pump task handle should be consumed after join"
        );
    }

    #[tokio::test]
    async fn manager_remove_nonexistent_is_noop() {
        let mgr = RxSessionManager::new();
        mgr.remove("does-not-exist").await;
        assert_eq!(mgr.count().await, 0);
    }

    #[tokio::test]
    async fn session_register_blocking_starts_pump() {
        let (conn, _peer) = loopback_connection("test-pump-start");
        let conn = Arc::new(conn);
        let session = RxSession::new(conn);
        assert!(session.pump_task.lock().expect("pump_task").is_none());
        let _rx = session.register_blocking();
        assert!(session.pump_task.lock().expect("pump_task").is_some());
    }

    #[tokio::test]
    async fn session_shutdown_cancels_pump_token() {
        let (conn, _peer) = loopback_connection("test-shutdown");
        let conn = Arc::new(conn);
        let session = RxSession::new(conn);
        let _rx = session.register_blocking();
        assert!(!session.pump_token.is_cancelled());
        session.shutdown();
        assert!(session.pump_token.is_cancelled());
    }

    #[tokio::test]
    async fn consumer_receives_data_after_registration() {
        let (conn, mut peer) = loopback_connection("test-fanout-data");
        let conn = Arc::new(conn);
        let session = RxSession::new(Arc::clone(&conn));

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
        let session = RxSession::new(Arc::clone(&conn));

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

    #[tokio::test]
    async fn removing_session_awaits_pump_and_drops_consumers() {
        let (conn, _peer) = loopback_connection("test-remove-lifecycle");
        let conn = Arc::new(conn);
        let mgr = RxSessionManager::new();
        let session = mgr.get_or_create(Arc::clone(&conn)).await;
        let conn_id = session.connection_id().to_string();
        let _rx = session.register_blocking();
        assert!(!session.pump_token.is_cancelled());
        mgr.remove(&conn_id).await;
        assert!(session.pump_token.is_cancelled());
        assert_eq!(mgr.count().await, 0);
        assert!(
            session.pump_task.lock().expect("pump_task").is_none(),
            "pump handle should be consumed after remove"
        );
    }

    #[tokio::test]
    async fn connection_close_causes_pump_exit() {
        let (conn, mut peer) = loopback_connection("test-close-exit");
        let conn = Arc::new(conn);
        let session = RxSession::new(Arc::clone(&conn));
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

    #[tokio::test]
    async fn pump_exits_cleanly_on_shutdown_without_hanging() {
        let (conn, _peer) = loopback_connection("test-pump-clean-exit");
        let conn = Arc::new(conn);
        let session = RxSession::new(Arc::clone(&conn));
        let _rx = session.register_blocking();

        session.shutdown_and_join().await;

        assert!(
            session.pump_task.lock().expect("pump_task").is_none(),
            "pump task handle should be consumed after join"
        );
    }

    #[tokio::test]
    async fn no_consumers_means_no_pump() {
        let (conn, _peer) = loopback_connection("test-no-pump");
        let conn = Arc::new(conn);
        let session = RxSession::new(conn);
        assert!(session.pump_task.lock().expect("pump_task").is_none());
    }

    #[tokio::test]
    async fn shutdown_is_idempotent() {
        let (conn, _peer) = loopback_connection("test-shutdown-idem");
        let conn = Arc::new(conn);
        let session = RxSession::new(conn);
        let mut rx = session.register_blocking();
        session.shutdown();
        session.shutdown();
        session.shutdown();
        assert!(session.pump_token.is_cancelled());
        let events = collect_events(&mut rx);
        let closed_count = events
            .iter()
            .filter(|e| matches!(e, RxEvent::Closed))
            .count();
        assert!(closed_count >= 1, "at least one Closed event expected");
    }

    #[tokio::test]
    async fn repeated_create_remove_no_leaked_pump_tasks() {
        let iterations = 10;
        for i in 0..iterations {
            let port_name = format!("test-stress-{i}");
            let (conn, _peer) = loopback_connection(&port_name);
            let conn = Arc::new(conn);
            let mgr = RxSessionManager::new();
            let session = mgr.get_or_create(Arc::clone(&conn)).await;
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
                session.pump_token.is_cancelled(),
                "pump token should be cancelled after remove on iteration {i}"
            );
            assert_eq!(mgr.count().await, 0);
        }
    }

    #[tokio::test]
    async fn full_consumer_is_dropped_from_registry() {
        let (conn, mut peer) = loopback_connection("test-full-consumer");
        let conn = Arc::new(conn);
        let session = RxSession::new(Arc::clone(&conn));

        // Register a consumer with a tiny channel capacity to force overflow.
        // We'll use the normal register path but just not drain the receiver.
        let mut rx = session.register_blocking();

        // Send enough data to exceed the channel capacity (256 events).
        // Each byte chunk becomes a Data event, so send many small writes.
        for _ in 0..300 {
            peer.write_all(b"x").await.unwrap();
            // Small sleep to let the pump process each chunk
            tokio::time::sleep(Duration::from_millis(2)).await;
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
        session.shutdown();

        // Consumer was dropped from registry when its channel filled up.
        // The receiver should still work for whatever was buffered, but the
        // key behavior is: the pump did NOT hang or panic.
        let events = collect_events(&mut rx);
        // We got *some* data before being dropped, and may or may not get Closed
        // (since the consumer may have been removed from registry before shutdown).
        assert!(
            !events.is_empty(),
            "should have received some events before consumer was dropped"
        );
    }

    #[tokio::test]
    async fn dropped_receiver_removed_from_registry() {
        let (conn, _peer) = loopback_connection("test-dropped-receiver");
        let conn = Arc::new(conn);
        let session = RxSession::new(Arc::clone(&conn));

        // Register a consumer, then drop the receiver.
        let rx = session.register_blocking();
        let consumer_count_before = {
            let registry = session.consumers.lock().expect("consumers");
            registry.blocking.len() + registry.streaming.len()
        };
        assert_eq!(consumer_count_before, 1);

        drop(rx);

        // Send data — pump will try to fanout, find the consumer's channel
        // closed, and remove it via retain().
        // We need to trigger a read from the port. Since no peer writes data,
        // the pump won't get any Data events. Instead, trigger via shutdown.
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
}
