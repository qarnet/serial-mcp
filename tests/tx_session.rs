//! Integration tests for TxSession wiring through tool handlers and
//! `ConnectionManager`.
//!
//! Unit tests for TxSession behavior live in `src/tx_session.rs`; these tests
//! cover the cross-module path with in-memory loopbacks and no hardware.

use std::sync::Arc;
use std::time::Duration;

use serial_mcp::serial::test_support::loopback_connection;
use serial_mcp::serial::{ConnectionManager, FlushTarget};
use serial_mcp::tools::types::FlushArgs;
use serial_mcp::tx_session::TxSessionManager;

use tokio::io::AsyncReadExt;

/// Flush handler creates a TxSession, queues writes before flush, and returns
/// after ordered delivery.
#[tokio::test]
async fn flush_tool_handler_sequences_through_tx_session() {
    let connections = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("test-flush-wiring");
    let connection_id = connections.insert(conn).await.unwrap();
    let tx_sessions = Arc::new(TxSessionManager::new());

    // Queue bytes before invoking flush.
    {
        let session = tx_sessions
            .get_or_create(
                connections
                    .get(&connection_id)
                    .await
                    .expect("connection found"),
            )
            .await;
        for ch in b"abc" {
            let data: Arc<[u8]> = Arc::from(vec![*ch].as_slice());
            session.write(data).await.unwrap();
        }
    }

    let rx_sessions = std::sync::Arc::new(serial_mcp::rx_session::RxSessionManager::new(
        std::sync::Arc::new(serial_mcp::buffer_budget::AtomicBudget::new(
            1024 * 1024,
            1024 * 1024,
        )),
        std::sync::Arc::new(serial_mcp::resource_events::ResourceEventHub::new(64)),
    ));
    let result = serial_mcp::tools::io_ops::flush(
        &connections,
        &rx_sessions,
        &tx_sessions,
        FlushArgs {
            connection_id: connection_id.clone(),
            target: FlushTarget::Output,
        },
    )
    .await
    .unwrap();

    assert_eq!(result.0.connection_id, connection_id);
    assert_eq!(result.0.target, FlushTarget::Output);

    // Flush must deliver queued bytes to peer in write order.
    let mut buf = [0u8; 3];
    tokio::time::timeout(Duration::from_millis(500), peer.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"abc");
}

/// `ConnectionManager::close` propagates to the TxSession worker, so a write
/// after close returns `ConnectionClosed`.
#[tokio::test]
async fn close_via_connection_manager_propagates_to_tx_session() {
    let connections = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("test-close-mgr-wiring");
    let connection_id = connections.insert(conn).await.unwrap();
    let tx_sessions = Arc::new(TxSessionManager::new());

    let session = tx_sessions
        .get_or_create(
            connections
                .get(&connection_id)
                .await
                .expect("connection found"),
        )
        .await;

    // Verify write succeeds before close.
    {
        let data: Arc<[u8]> = Arc::from(b"x".as_slice());
        session.write(data).await.unwrap();
        let mut buf = [0u8; 1];
        tokio::time::timeout(Duration::from_millis(500), peer.read_exact(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(buf[0], b'x');
    }

    // Close through ConnectionManager to exercise handler-path propagation.
    connections.close(&connection_id).await.unwrap();

    // A write after close must return ConnectionClosed.
    let data: Arc<[u8]> = Arc::from(b"z".as_slice());
    let result = session.write(data).await;
    assert!(
        matches!(result, Err(serial_mcp::SerialError::ConnectionClosed(_))),
        "expected ConnectionClosed after ConnectionManager::close, got {result:?}"
    );
}
