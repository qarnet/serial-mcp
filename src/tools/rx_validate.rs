//! RX request validation for `read`: `RxLimits`, `ResolvedRxArgs`,
//! `RxRequestArgs`, and `validate_rx_request`. General primitives live in the
//! sibling `helpers` module.

use std::sync::Arc;

use crate::codec::Encoding;
use crate::match_config::{validate_match_request, MatchRequest, Matcher};
use crate::serial::{ConnectionManager, SerialConnection};
use crate::tools::helpers::{
    clamp_or_err, clamp_timeout_or_err, lookup_connection, parse_encoding, require_min_or_err,
    MAX_TIMEOUT_MS,
};
use crate::tools::types::ReadArgs;

/// Per-tool limits and the error-message label for [`validate_rx_request`].
pub struct RxLimits {
    /// Tool name used to prefix error messages.
    pub tool: &'static str,
    /// Minimum allowed `max_buffered_bytes`.
    pub min_buffered: usize,
    /// Maximum allowed `max_buffered_bytes`.
    pub max_buffered: usize,
}

/// The common, validated inputs for `read`.
#[derive(Debug)]
pub struct ResolvedRxArgs {
    pub encoding: Encoding,
    pub connection: Arc<SerialConnection>,
    pub max_buffered_bytes: usize,
    pub matcher: Option<Matcher>,
}

/// Accessors for the request fields for `read`.
pub trait RxRequestArgs {
    fn connection_id(&self) -> &str;
    fn encoding(&self) -> &str;
    fn timeout_ms(&self) -> Option<u64>;
    fn no_new_rx_timeout_ms(&self) -> Option<u64>;
    fn match_request(&self) -> Option<&MatchRequest>;
}

impl RxRequestArgs for ReadArgs {
    fn connection_id(&self) -> &str {
        &self.connection_id
    }
    fn encoding(&self) -> &str {
        &self.encoding
    }
    fn timeout_ms(&self) -> Option<u64> {
        self.timeout_ms
    }
    fn no_new_rx_timeout_ms(&self) -> Option<u64> {
        self.no_new_rx_timeout_ms
    }
    fn match_request(&self) -> Option<&MatchRequest> {
        self.r#match.as_ref()
    }
}

/// Validate and resolve `read` inputs: encoding, connection lookup,
/// `max_buffered_bytes` bounds, `timeout_ms`, nonzero
/// `no_new_rx_timeout_ms`, and matcher. Prefix errors with `limits.tool` to
/// preserve each tool's wording.
///
/// The handler passes `max_buffered_bytes`, resolved from the connection
/// default. The caller reserves the buffer budget.
pub async fn validate_rx_request<A: RxRequestArgs>(
    connections: &Arc<ConnectionManager>,
    args: &A,
    limits: RxLimits,
    max_buffered_bytes: usize,
) -> Result<ResolvedRxArgs, String> {
    let encoding = parse_encoding(args.encoding())?;
    let connection = lookup_connection(connections, args.connection_id()).await?;

    let max_buffered_bytes = require_min_or_err(
        &format!("{}.max_buffered_bytes", limits.tool),
        max_buffered_bytes,
        limits.min_buffered,
    )?;
    let max_buffered_bytes = clamp_or_err(
        &format!("{}.max_buffered_bytes", limits.tool),
        max_buffered_bytes,
        limits.max_buffered,
    )?;

    if let Some(timeout_ms) = args.timeout_ms() {
        clamp_timeout_or_err(
            &format!("{}.timeout_ms", limits.tool),
            timeout_ms,
            MAX_TIMEOUT_MS,
        )?;
    }
    if let Some(silence_ms) = args.no_new_rx_timeout_ms() {
        if silence_ms == 0 {
            return Err(format!("{}.no_new_rx_timeout_ms must be > 0", limits.tool));
        }
        clamp_timeout_or_err(
            &format!("{}.no_new_rx_timeout_ms", limits.tool),
            silence_ms,
            MAX_TIMEOUT_MS,
        )?;
    }

    let matcher = match args.match_request() {
        Some(m) => Some(validate_match_request(m)?),
        None => None,
    };

    Ok(ResolvedRxArgs {
        encoding,
        connection,
        max_buffered_bytes,
        matcher,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::tools::helpers::{MAX_READ_BYTES, MIN_READ_BYTES};

    struct TestRxArgs {
        connection_id: String,
        encoding: String,
        timeout_ms: Option<u64>,
        no_new_rx_timeout_ms: Option<u64>,
        match_request: Option<MatchRequest>,
    }

    impl RxRequestArgs for TestRxArgs {
        fn connection_id(&self) -> &str {
            &self.connection_id
        }
        fn encoding(&self) -> &str {
            &self.encoding
        }
        fn timeout_ms(&self) -> Option<u64> {
            self.timeout_ms
        }
        fn no_new_rx_timeout_ms(&self) -> Option<u64> {
            self.no_new_rx_timeout_ms
        }
        fn match_request(&self) -> Option<&MatchRequest> {
            self.match_request.as_ref()
        }
    }

    fn valid_args(id: &str) -> TestRxArgs {
        TestRxArgs {
            connection_id: id.into(),
            encoding: "utf-8".into(),
            timeout_ms: Some(1000),
            no_new_rx_timeout_ms: None,
            match_request: None,
        }
    }

    fn read_limits() -> RxLimits {
        RxLimits {
            tool: "read",
            min_buffered: MIN_READ_BYTES,
            max_buffered: MAX_READ_BYTES,
        }
    }

    async fn fake_conn() -> (Arc<ConnectionManager>, String, tokio::io::DuplexStream) {
        let connections = Arc::new(ConnectionManager::new());
        let (conn, peer) = crate::serial::test_support::loopback_connection("/dev/fake-validate");
        let id = connections.insert(conn).await.unwrap();
        (connections, id, peer)
    }

    #[tokio::test]
    async fn validate_rx_request_ok() {
        let (connections, id, _peer) = fake_conn().await;
        let resolved = validate_rx_request(&connections, &valid_args(&id), read_limits(), 256)
            .await
            .unwrap();
        assert_eq!(resolved.max_buffered_bytes, 256);
        assert!(resolved.matcher.is_none());
        assert_eq!(resolved.connection.port(), "/dev/fake-validate");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_bad_encoding() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.encoding = "rot13".into();
        let err = validate_rx_request(&connections, &a, read_limits(), 256)
            .await
            .unwrap_err();
        assert!(err.to_lowercase().contains("encoding"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_unknown_connection() {
        let connections = Arc::new(ConnectionManager::new());
        let err = validate_rx_request(&connections, &valid_args("nope"), read_limits(), 256)
            .await
            .unwrap_err();
        assert!(err.contains("Connection ID nope not found"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_buffered_below_min() {
        let (connections, id, _peer) = fake_conn().await;
        let a = valid_args(&id);
        let err = validate_rx_request(&connections, &a, read_limits(), 0)
            .await
            .unwrap_err();
        assert!(err.contains("read.max_buffered_bytes"), "got: {err}");
        assert!(err.contains("below minimum"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_buffered_above_max() {
        let (connections, id, _peer) = fake_conn().await;
        let a = valid_args(&id);
        let err = validate_rx_request(&connections, &a, read_limits(), MAX_READ_BYTES + 1)
            .await
            .unwrap_err();
        assert!(err.contains("exceeds maximum"), "got: {err}");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_zero_silence_with_tool_prefix() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.no_new_rx_timeout_ms = Some(0);
        let read_limits = RxLimits {
            tool: "read",
            min_buffered: MIN_READ_BYTES,
            max_buffered: MAX_READ_BYTES,
        };
        let err = validate_rx_request(&connections, &a, read_limits, 256)
            .await
            .unwrap_err();
        assert_eq!(err, "read.no_new_rx_timeout_ms must be > 0");
    }

    #[tokio::test]
    async fn validate_rx_request_rejects_oversized_timeout() {
        let (connections, id, _peer) = fake_conn().await;
        let mut a = valid_args(&id);
        a.timeout_ms = Some(MAX_TIMEOUT_MS + 1);
        let err = validate_rx_request(&connections, &a, read_limits(), 256)
            .await
            .unwrap_err();
        assert!(err.contains("read.timeout_ms"), "got: {err}");
        assert!(err.contains("exceeds maximum"), "got: {err}");
    }
}
