//! Cross-module wiring tests for TxSession.
//!
//! Low-level unit-tested behaviors live in `src/tx_session.rs`.
//! These tests exercise the wiring between tool handlers →
//! `ConnectionManager` → `TxSessionManager` → `TxSession` that
//! unit tests cannot reach.
//!
//! No hardware required — all connections are in-memory loopbacks.

use std::sync::Arc;
use std::time::Duration;

use serial_mcp::serial::test_support::loopback_connection;
use serial_mcp::serial::{ConnectionManager, FlushTarget};
use serial_mcp::tools::types::FlushArgs;
use serial_mcp::tx_session::TxSessionManager;

use tokio::io::AsyncReadExt;

/// Verifies that the flush tool handler path (`io_ops::flush` with output
/// target) correctly creates a TxSession via `get_or_create`, sequences a
/// flush after writes, and returns the proper tool result.
#[tokio::test]
async fn flush_tool_handler_sequences_through_tx_session() {
    let connections = Arc::new(ConnectionManager::new());
    let (conn, mut peer) = loopback_connection("test-flush-wiring");
    let connection_id = connections.insert(conn).await.unwrap();
    let tx_sessions = Arc::new(TxSessionManager::new());

    // Write 3 bytes through TxSession so there's something to flush.
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

    // Flush output through the tool handler path.
    let result = serial_mcp::tools::io_ops::flush(
        &connections,
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

    // Peer must see "abc" in order (flush was sequenced after writes).
    let mut buf = [0u8; 3];
    tokio::time::timeout(Duration::from_millis(500), peer.read_exact(&mut buf))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&buf, b"abc");
}

/// Verifies that closing a connection via `ConnectionManager::close`
/// propagates to the TxSession worker, so queued writes after close
/// return `ConnectionClosed`.
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

    // Write a byte to confirm session works pre-close.
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

    // Close via ConnectionManager (the tool handler path).
    connections.close(&connection_id).await.unwrap();

    // TxSession worker should see the close; subsequent write must fail.
    let data: Arc<[u8]> = Arc::from(b"z".as_slice());
    let result = session.write(data).await;
    assert!(
        matches!(result, Err(serial_mcp::SerialError::ConnectionClosed(_))),
        "expected ConnectionClosed after ConnectionManager::close, got {result:?}"
    );
}
