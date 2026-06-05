use std::sync::Arc;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tracing::debug;

use crate::codec;
use crate::rx_session::RxSessionManager;
use crate::serial::ConnectionManager;
use crate::tools::helpers::{
    clamp_or_err, clamp_timeout_or_err, lookup_connection, parse_encoding, require_min_or_err,
    wait_for_pattern_via_session, MAX_TIMEOUT_MS, MAX_WAIT_BYTES, MIN_WAIT_BYTES,
};
use crate::tools::types::{WaitForArgs, WaitForResult};

pub async fn wait_for(
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: WaitForArgs,
) -> Result<Json<WaitForResult>, String> {
    debug!(
        "wait_for {} pattern_encoding={} timeout={}ms max_bytes={}",
        args.connection_id, args.pattern_encoding, args.timeout_ms.0, args.max_bytes.0
    );

    let pattern_encoding = parse_encoding(&args.pattern_encoding)?;
    let response_encoding = parse_encoding(&args.response_encoding)?;

    let pattern = codec::decode(pattern_encoding, &args.pattern)
        .map_err(|e| format!("Pattern decoding failed - {e}"))?;
    if pattern.is_empty() {
        return Err("Pattern must not be empty".into());
    }

    let max_bytes = require_min_or_err("wait_for.max_bytes", args.max_bytes.0, MIN_WAIT_BYTES)?;
    let max_bytes = clamp_or_err("wait_for.max_bytes", max_bytes, MAX_WAIT_BYTES)?;
    let timeout_ms = args.timeout_ms.0;
    clamp_timeout_or_err("wait_for.timeout_ms", timeout_ms, MAX_TIMEOUT_MS)?;

    let connection = lookup_connection(connections, &args.connection_id).await?;
    let _rx_lease = crate::serial::SerialConnection::acquire_rx(&connection, "wait_for")?;
    let progress_token = meta.get_progress_token();

    let session = rx_sessions.get_or_create(Arc::clone(&connection)).await;
    let event_rx = session.register_blocking();

    let outcome = wait_for_pattern_via_session(
        event_rx,
        &pattern,
        timeout_ms,
        max_bytes,
        &ct,
        progress_token,
        Some(&peer),
    )
    .await?;

    session.prune_consumers();

    if outcome.timed_out {
        return Err(format!(
            "wait_for timed out after {timeout_ms}ms on {}",
            args.connection_id
        ));
    }

    let bytes_read = outcome.bytes.len();
    let data = codec::encode(response_encoding, &outcome.bytes)
        .map_err(|e| format!("Response encoding failed - {e}"))?;

    Ok(Json(WaitForResult {
        connection_id: args.connection_id,
        matched: outcome.match_index.is_some(),
        data,
        bytes_read,
        match_index: outcome.match_index,
        timeout_ms,
        response_encoding: response_encoding.to_string(),
    }))
}
