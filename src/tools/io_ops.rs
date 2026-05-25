use std::sync::Arc;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tracing::{debug, info};

use crate::codec::{self, Encoding};
use crate::serial::ConnectionManager;
use crate::tools::helpers::{
    build_read_result, log_tool_err, lookup_connection, parse_encoding, read_bytes,
};
use crate::tools::types::{FlushArgs, FlushResult, ReadArgs, ReadResult, WriteArgs, WriteResult};

pub async fn write(
    connections: &Arc<ConnectionManager>,
    args: WriteArgs,
) -> Result<Json<WriteResult>, String> {
    debug!("Write to {} ({})", args.connection_id, args.encoding);

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;
    let bytes =
        codec::decode(encoding, &args.data).map_err(|e| format!("Data decoding failed - {e}"))?;
    let bytes_written = connection.write(&bytes).await.map_err(|e| {
        log_tool_err(
            "write",
            &format!("Data sending failed on {}", args.connection_id),
            e,
        )
    })?;

    debug!("Wrote {} bytes to {}", bytes_written, args.connection_id);
    Ok(Json(WriteResult {
        connection_id: args.connection_id,
        bytes_written,
        encoding: encoding.to_string(),
    }))
}

pub async fn read(
    connections: &Arc<ConnectionManager>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: ReadArgs,
) -> Result<Json<ReadResult>, String> {
    debug!(
        "Read from {} (timeout {:?})",
        args.connection_id, args.timeout_ms
    );

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;
    let progress_token = meta.get_progress_token();
    let outcome = read_bytes(
        &connection,
        args.max_bytes,
        args.timeout_ms,
        &ct,
        progress_token,
        Some(&peer),
    )
    .await?;
    build_read_result(outcome, args.connection_id, encoding, args.timeout_ms)
}

pub async fn flush(
    connections: &Arc<ConnectionManager>,
    args: FlushArgs,
) -> Result<Json<FlushResult>, String> {
    debug!("Flush {} target={:?}", args.connection_id, args.target);

    let connection = lookup_connection(connections, &args.connection_id).await?;
    connection.flush_buffers(args.target).await.map_err(|e| {
        log_tool_err(
            "flush",
            &format!("Failed to flush {}", args.connection_id),
            e,
        )
    })?;
    info!("Flushed {} ({:?})", args.connection_id, args.target);

    Ok(Json(FlushResult {
        connection_id: args.connection_id,
        target: args.target,
    }))
}

pub fn encoding_from_str(raw: &str) -> Result<Encoding, String> {
    parse_encoding(raw)
}
