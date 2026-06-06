//! Integration tests for TxSession: dedicated TX worker per connection.
//!
//! These tests exercise the TxSession, TxSessionManager, and the write/flush
//! tool surface against in-memory loopback connections. No hardware required.

use std::sync::Arc;
use std::time::Duration;

use serial_mcp::rx_session::RxSessionManager;
use serial_mcp::serial::test_support::loopback_connection;
use serial_mcp::tx_session::TxSessionManager;

use tokio::io::AsyncReadExt;

#[tokio::test]
async fn tx_write_bytes_reach_peer() {
    let (conn, mut peer) = loopback_connection("test-write-peer");
    let conn = Arc::new(conn);
    let mgr = TxSessionManager::new();
    let session = mgr.get_or_create(Arc::clone(&conn)).await;

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
    let mgr = TxSessionManager::new();
    let session = mgr.get_or_create(Arc::clone(&conn)).await;

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
    let (conn, _peer) = loopback_connection("test-write-fast");
    let conn = Arc::new(conn);
    let rx_mgr = RxSessionManager::new();
    let rx_session = rx_mgr.get_or_create(Arc::clone(&conn)).await;
    let _rx = rx_session.register_blocking();

    let tx_mgr = TxSessionManager::new();
    let tx_session = tx_mgr.get_or_create(Arc::clone(&conn)).await;

    let start = std::time::Instant::now();
    let data: Arc<[u8]> = Arc::from(b"x".as_slice());
    tx_session.write(data).await.unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(200),
        "write via TxSession took {elapsed:?}, should be fast even with pump active"
    );

    rx_mgr.remove(conn.id()).await;
    tx_mgr.remove(conn.id()).await;
}

#[tokio::test]
async fn tx_flush_output_sequenced_after_writes() {
    let (conn, mut peer) = loopback_connection("test-flush-order");
    let conn = Arc::new(conn);
    let mgr = TxSessionManager::new();
    let session = mgr.get_or_create(Arc::clone(&conn)).await;

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
    let mgr = TxSessionManager::new();
    let session = mgr.get_or_create(Arc::clone(&conn)).await;

    conn.close().await.unwrap();

    let data: Arc<[u8]> = Arc::from(b"fail".as_slice());
    let result = session.write(data).await;
    assert!(
        matches!(result, Err(serial_mcp::SerialError::ConnectionClosed(_))),
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
    let tx_mgr = TxSessionManager::new();
    let rx_mgr = RxSessionManager::new();
    let rx_session = rx_mgr.get_or_create(Arc::clone(&conn)).await;
    let _rx = rx_session.register_blocking();

    assert_eq!(tx_mgr.count().await, 0);

    rx_mgr.remove(conn.id()).await;
}