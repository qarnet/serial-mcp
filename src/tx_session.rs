//! Per-connection TX worker for serialized write/flush operations.
//!
//! Each open connection can have at most one [`TxSession`], which owns a
//! background task that is the sole caller of `SerialConnection::write()` and
//! `SerialConnection::flush_buffers(Output)`. MCP write and flush tools
//! enqueue requests via an mpsc channel and await a oneshot acknowledgment,
//! returning in microseconds regardless of any active RX pump.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use tokio::sync::{mpsc, oneshot};
use tokio::sync::Mutex as AsyncMutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info};

use crate::error::SerialError;
use crate::serial::{FlushTarget, SerialConnection};

const TX_CHANNEL_CAPACITY: usize = 64;

enum TxRequest {
    Write {
        data: Arc<[u8]>,
        ack: oneshot::Sender<Result<usize, SerialError>>,
    },
    Flush {
        ack: oneshot::Sender<Result<(), SerialError>>,
    },
}

pub struct TxSession {
    connection_id: String,
    tx: StdMutex<Option<mpsc::Sender<TxRequest>>>,
    worker_task: StdMutex<Option<JoinHandle<()>>>,
}

impl TxSession {
    pub fn new(connection: Arc<SerialConnection>) -> Self {
        let connection_id = connection.id().to_string();
        let close_token = connection.cancel_token();
        let (tx, rx) = mpsc::channel(TX_CHANNEL_CAPACITY);
        let handle = tokio::spawn(tx_worker(connection, rx, close_token));
        Self {
            connection_id,
            tx: StdMutex::new(Some(tx)),
            worker_task: StdMutex::new(Some(handle)),
        }
    }

    pub fn connection_id(&self) -> &str {
        &self.connection_id
    }

    pub async fn write(&self, data: Arc<[u8]>) -> Result<usize, SerialError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let tx = self
            .tx
            .lock()
            .expect("tx mutex poisoned")
            .as_ref()
            .cloned()
            .ok_or_else(|| SerialError::ConnectionClosed(self.connection_id.clone()))?;
        tx.send(TxRequest::Write { data, ack: ack_tx })
            .await
            .map_err(|_| SerialError::ConnectionClosed(self.connection_id.clone()))?;
        ack_rx
            .await
            .map_err(|_| SerialError::ConnectionClosed(self.connection_id.clone()))?
    }

    pub async fn flush_output(&self) -> Result<(), SerialError> {
        let (ack_tx, ack_rx) = oneshot::channel();
        let tx = self
            .tx
            .lock()
            .expect("tx mutex poisoned")
            .as_ref()
            .cloned()
            .ok_or_else(|| SerialError::ConnectionClosed(self.connection_id.clone()))?;
        tx.send(TxRequest::Flush { ack: ack_tx })
            .await
            .map_err(|_| SerialError::ConnectionClosed(self.connection_id.clone()))?;
        ack_rx
            .await
            .map_err(|_| SerialError::ConnectionClosed(self.connection_id.clone()))?
    }

    pub async fn shutdown_and_join(&self) {
        let _ = self.tx.lock().expect("tx mutex poisoned").take();
        let handle = self.worker_task.lock().expect("worker_task mutex poisoned").take();
        if let Some(h) = handle {
            let _ = h.await;
        }
    }
}

async fn tx_worker(
    connection: Arc<SerialConnection>,
    mut rx: mpsc::Receiver<TxRequest>,
    close_token: CancellationToken,
) {
    let conn_id = connection.id().to_string();
    info!("tx_session: worker entered for {conn_id}");

    loop {
        let req = tokio::select! {
            _ = close_token.cancelled() => break,
            req = rx.recv() => match req {
                Some(r) => r,
                None => break,
            },
        };
        match req {
            TxRequest::Write { data, ack } => {
                let result = connection.write(&data).await;
                let _ = ack.send(result);
            }
            TxRequest::Flush { ack } => {
                let result = connection.flush_buffers(FlushTarget::Output).await;
                let _ = ack.send(result);
            }
        }
    }

    info!("tx_session: worker exiting for {conn_id}");
}

pub struct TxSessionManager {
    sessions: AsyncMutex<HashMap<String, Arc<TxSession>>>,
}

impl Default for TxSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TxSessionManager {
    pub fn new() -> Self {
        Self {
            sessions: AsyncMutex::new(HashMap::new()),
        }
    }

    pub async fn get_or_create(&self, connection: Arc<SerialConnection>) -> Arc<TxSession> {
        let conn_id = connection.id().to_string();
        let mut sessions = self.sessions.lock().await;
        if let Some(existing) = sessions.get(&conn_id) {
            return Arc::clone(existing);
        }
        let session = Arc::new(TxSession::new(connection));
        sessions.insert(conn_id, Arc::clone(&session));
        debug!(
            "tx_session: created new session for connection {}",
            session.connection_id()
        );
        session
    }

    pub async fn remove(&self, connection_id: &str) {
        let session = {
            let mut sessions = self.sessions.lock().await;
            sessions.remove(connection_id)
        };
        if let Some(session) = session {
            session.shutdown_and_join().await;
            info!("tx_session: removed session for connection {connection_id}");
        }
    }

    pub async fn count(&self) -> usize {
        self.sessions.lock().await.len()
    }

    pub async fn get(&self, connection_id: &str) -> Option<Arc<TxSession>> {
        self.sessions.lock().await.get(connection_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::test_support::loopback_connection;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn tx_write_bytes_reach_peer() {
        let (conn, mut peer) = loopback_connection("test-write-peer");
        let conn = Arc::new(conn);
        let session = TxSession::new(Arc::clone(&conn));

        let data: Arc<[u8]> = Arc::from(b"hello".as_slice());
        let n = session.write(data).await.unwrap();
        assert_eq!(n, 5);

        let mut buf = [0u8; 5];
        tokio::time::timeout(Duration::from_millis(500), peer.read_exact(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf, b"hello");
    }

    #[tokio::test]
    async fn tx_sequential_writes_preserve_order() {
        let (conn, mut peer) = loopback_connection("test-write-order");
        let conn = Arc::new(conn);
        let session = TxSession::new(Arc::clone(&conn));

        for ch in b"ABCDE" {
            let data: Arc<[u8]> = Arc::from(vec![*ch].as_slice());
            session.write(data).await.unwrap();
        }

        let mut buf = [0u8; 5];
        tokio::time::timeout(Duration::from_millis(500), peer.read_exact(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf, b"ABCDE");
    }

    #[tokio::test]
    async fn tx_write_during_active_pump_returns_quickly() {
        use crate::rx_session::RxSessionManager;

        let (conn, _peer) = loopback_connection("test-write-fast");
        let conn = Arc::new(conn);
        let rx_mgr = RxSessionManager::new();
        let rx_session = rx_mgr.get_or_create(Arc::clone(&conn)).await;
        let _rx = rx_session.register_blocking();

        let tx_session = TxSession::new(Arc::clone(&conn));

        let start = std::time::Instant::now();
        let data: Arc<[u8]> = Arc::from(b"x".as_slice());
        tx_session.write(data).await.unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(200),
            "write via TxSession took {elapsed:?}, should be fast even with pump active"
        );

        rx_mgr.remove(conn.id()).await;
    }

    #[tokio::test]
    async fn tx_flush_output_sequenced_after_writes() {
        let (conn, mut peer) = loopback_connection("test-flush-order");
        let conn = Arc::new(conn);
        let session = TxSession::new(Arc::clone(&conn));

        for ch in b"abc" {
            let data: Arc<[u8]> = Arc::from(vec![*ch].as_slice());
            session.write(data).await.unwrap();
        }
        session.flush_output().await.unwrap();

        let mut buf = [0u8; 3];
        tokio::time::timeout(Duration::from_millis(500), peer.read_exact(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&buf, b"abc");
    }

    #[tokio::test]
    async fn tx_close_while_writes_queued_returns_connection_closed() {
        let (conn, _peer) = loopback_connection("test-close-queued");
        let conn = Arc::new(conn);
        let session = TxSession::new(Arc::clone(&conn));

        conn.close().await.unwrap();

        let data: Arc<[u8]> = Arc::from(b"fail".as_slice());
        let result = session.write(data).await;
        assert!(
            matches!(result, Err(SerialError::ConnectionClosed(_))),
            "expected ConnectionClosed after close, got {result:?}"
        );
    }

    #[tokio::test]
    async fn tx_get_or_create_is_idempotent() {
        let (conn, _peer) = loopback_connection("test-idempotent");
        let conn = Arc::new(conn);
        let mgr = TxSessionManager::new();
        let s1 = mgr.get_or_create(Arc::clone(&conn)).await;
        let s2 = mgr.get_or_create(Arc::clone(&conn)).await;
        assert!(Arc::ptr_eq(&s1, &s2));
        assert_eq!(mgr.count().await, 1);
    }

    #[tokio::test]
    async fn tx_no_session_for_read_only_connection() {
        let (conn, _peer) = loopback_connection("test-read-only");
        let conn = Arc::new(conn);
        let mgr = TxSessionManager::new();
        assert_eq!(mgr.count().await, 0);

        use crate::rx_session::RxSessionManager;
        let rx_mgr = RxSessionManager::new();
        let rx_session = rx_mgr.get_or_create(Arc::clone(&conn)).await;
        let _rx = rx_session.register_blocking();

        assert_eq!(mgr.count().await, 0);

        rx_mgr.remove(conn.id()).await;
    }
}