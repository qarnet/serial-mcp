use std::sync::Arc;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tracing::debug;

use crate::codec;
use crate::serial::ConnectionManager;
use crate::tools::helpers::{lookup_connection, parse_encoding, read_until_pattern};
use crate::tools::types::{WaitForArgs, WaitForResult};

pub async fn wait_for(
    connections: &Arc<ConnectionManager>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: WaitForArgs,
) -> Result<Json<WaitForResult>, String> {
    debug!(
        "wait_for {} pattern_encoding={} timeout={}ms max_bytes={}",
        args.connection_id, args.pattern_encoding, args.timeout_ms, args.max_bytes
    );

    let pattern_encoding = parse_encoding(&args.pattern_encoding)?;
    let response_encoding = parse_encoding(&args.response_encoding)?;

    let pattern = codec::decode(pattern_encoding, &args.pattern)
        .map_err(|e| format!("Pattern decoding failed - {e}"))?;
    if pattern.is_empty() {
        return Err("Pattern must not be empty".into());
    }

    let connection = lookup_connection(connections, &args.connection_id).await?;
    let progress_token = meta.get_progress_token();
    let outcome = read_until_pattern(
        &connection,
        &pattern,
        args.timeout_ms,
        args.max_bytes,
        &ct,
        progress_token,
        Some(&peer),
    )
    .await?;

    let bytes_read = outcome.bytes.len();
    let data = codec::encode(response_encoding, &outcome.bytes)
        .map_err(|e| format!("Response encoding failed - {e}"))?;

    Ok(Json(WaitForResult {
        connection_id: args.connection_id,
        matched: outcome.match_index.is_some(),
        timed_out: outcome.timed_out,
        data,
        bytes_read,
        match_index: outcome.match_index,
        timeout_ms: args.timeout_ms,
        response_encoding: response_encoding.to_string(),
    }))
}
