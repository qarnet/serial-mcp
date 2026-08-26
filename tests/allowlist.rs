//! Port allowlist tests use in-process HTTP requests with fake paths.
//!
//! They verify allowlist decisions before OS open errors without child
//! processes or hardware.

use std::sync::Arc;

use serde_json::json;

use serial_mcp::security::SecurityManager;
use serial_mcp::serial::ConnectionManager;

mod common;
use common::{connect_client, tool_request, TestServer};

#[tokio::test]
async fn empty_allowlist_allows_any_port() {
    let manager = Arc::new(ConnectionManager::new());

    let security = SecurityManager::from_patterns([] as [&str; 0]);
    let server = TestServer::builder(manager)
        .security(security)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "/tmp/test-fake-port",
                "baud_rate": 9600,
            }),
        ))
        .await
        .unwrap();

    // An empty allowlist permits any path to reach OS open. The fake path
    // therefore produces an OS-level error without an allowlist rejection.
    assert_eq!(
        result.is_error,
        Some(true),
        "Expected OS-level error for non-existent port"
    );
    let content = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(
        !content.contains("allowlist"),
        "Empty allowlist should not reject any port. Got: {content}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn exact_match_blocks_unauthorized_port() {
    let manager = Arc::new(ConnectionManager::new());

    let security = SecurityManager::from_patterns(["/dev/ttyACM1"]);
    let server = TestServer::builder(manager)
        .security(security)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "/dev/ttyACM0",
                "baud_rate": 115200,
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        result.is_error,
        Some(true),
        "Expected unauthorized port to be rejected: {result:?}"
    );

    let content = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(
        content.contains("allowlist") || content.contains("not"),
        "Error message should mention allowlist. Got: {content}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn exact_match_allows_authorized_port() {
    let manager = Arc::new(ConnectionManager::new());

    let security = SecurityManager::from_patterns(["/dev/ttyACM0"]);
    let server = TestServer::builder(manager)
        .security(security)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "/dev/ttyACM0",
                "baud_rate": 115200,
            }),
        ))
        .await
        .unwrap();

    // An allowed path may still fail when OS open cannot find it, but it must
    // not fail authorization.
    if result.is_error == Some(true) {
        let content = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");
        assert!(
            !content.contains("allowlist"),
            "Should not fail due to allowlist. Got: {content}"
        );
    }

    client.cancel().await.ok();
}

#[tokio::test]
async fn glob_pattern_matches_multiple_ports() {
    let manager = Arc::new(ConnectionManager::new());

    let security = SecurityManager::from_patterns(["/dev/ttyACM*", "/dev/ttyUSB*"]);
    let server = TestServer::builder(manager)
        .security(security)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "/dev/ttyACM0",
                "baud_rate": 115200,
            }),
        ))
        .await
        .unwrap();

    if result.is_error == Some(true) {
        let content = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");
        assert!(
            !content.contains("allowlist"),
            "Glob /dev/ttyACM* should match /dev/ttyACM0. Got: {content}"
        );
    }

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "/dev/ttyUSB5",
                "baud_rate": 115200,
            }),
        ))
        .await
        .unwrap();

    if result.is_error == Some(true) {
        let content = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");
        assert!(
            !content.contains("allowlist"),
            "Glob /dev/ttyUSB* should match /dev/ttyUSB5. Got: {content}"
        );
    }

    // A path outside both glob patterns must fail authorization before OS open.
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "/dev/ttyS0",
                "baud_rate": 115200,
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        result.is_error,
        Some(true),
        "Expected /dev/ttyS0 to be rejected by globs. Got: {result:?}"
    );

    let content = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(
        content.contains("allowlist") || content.contains("not"),
        "Error message should mention allowlist. Got: {content}"
    );

    client.cancel().await.ok();
}

#[tokio::test]
async fn comma_separated_multiple_exact_ports() {
    let manager = Arc::new(ConnectionManager::new());

    let security = SecurityManager::from_patterns(["COM1", "COM3", "COM5"]);
    let server = TestServer::builder(manager)
        .security(security)
        .start()
        .await;
    let (client, _rx) = connect_client(&server).await.unwrap();

    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "COM3",
                "baud_rate": 9600,
            }),
        ))
        .await
        .unwrap();

    if result.is_error == Some(true) {
        let content = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");
        assert!(
            !content.contains("allowlist"),
            "COM3 should be allowed. Got: {content}"
        );
    }

    // A path outside the list must fail authorization before OS open.
    let result = client
        .peer()
        .call_tool(tool_request(
            "open",
            json!({
                "port": "COM2",
                "baud_rate": 9600,
            }),
        ))
        .await
        .unwrap();

    assert_eq!(
        result.is_error,
        Some(true),
        "Expected COM2 to be rejected: {result:?}"
    );

    let content = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.as_str())
        .unwrap_or("");
    assert!(
        content.contains("allowlist") || content.contains("not"),
        "Error should mention allowlist. Got: {content}"
    );

    client.cancel().await.ok();
}
