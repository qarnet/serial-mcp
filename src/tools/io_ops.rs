use std::sync::Arc;
use std::time::Instant;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tracing::{debug, info};

use crate::codec;
use crate::serial::ConnectionManager;
use crate::tools::helpers::{
    build_read_result, clamp_or_err, clamp_timeout_or_err, log_tool_err, lookup_connection,
    parse_encoding, read_bytes, read_until_pattern, require_min_or_err, DEFAULT_READ_TIMEOUT_MS,
    MAX_READ_BYTES, MAX_TIMEOUT_MS, MAX_WAIT_BYTES, MAX_WRITE_BYTES, MIN_READ_BYTES,
};
use crate::tools::types::{
    FlushArgs, FlushResult, ReadArgs, ReadLineArgs, ReadLineResult, ReadResult, WriteArgs,
    WriteResult,
};

pub async fn write(
    connections: &Arc<ConnectionManager>,
    args: WriteArgs,
) -> Result<Json<WriteResult>, String> {
    debug!("Write to {} ({})", args.connection_id, args.encoding);

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;
    let bytes =
        codec::decode(encoding, &args.data).map_err(|e| format!("Data decoding failed - {e}"))?;
    clamp_or_err("write.data.len()", bytes.len(), MAX_WRITE_BYTES)?;
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
    let max_bytes = require_min_or_err("read.max_bytes", args.max_bytes.0, MIN_READ_BYTES)?;
    let max_bytes = clamp_or_err("read.max_bytes", max_bytes, MAX_READ_BYTES)?;
    let timeout_ms: Option<u64> = args.timeout_ms.into();
    if let Some(ms) = timeout_ms {
        clamp_timeout_or_err("read.timeout_ms", ms, MAX_TIMEOUT_MS)?;
    }
    let progress_token = meta.get_progress_token();
    let outcome = read_bytes(
        &connection,
        max_bytes,
        timeout_ms,
        &ct,
        progress_token,
        Some(&peer),
    )
    .await?;
    build_read_result(outcome, args.connection_id, encoding, timeout_ms)
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

pub async fn read_line(
    connections: &Arc<ConnectionManager>,
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: ReadLineArgs,
) -> Result<Json<ReadLineResult>, String> {
    debug!(
        "read_line {} (timeout {:?})",
        args.connection_id, args.timeout_ms
    );

    let encoding = parse_encoding(&args.encoding)?;
    let connection = lookup_connection(connections, &args.connection_id).await?;
    let max_bytes = require_min_or_err("read_line.max_bytes", args.max_bytes.0, MIN_READ_BYTES)?;
    let max_bytes = clamp_or_err("read_line.max_bytes", max_bytes, MAX_WAIT_BYTES)?;
    let timeout_ms: u64 =
        Into::<Option<u64>>::into(args.timeout_ms).unwrap_or(DEFAULT_READ_TIMEOUT_MS);
    clamp_timeout_or_err("read_line.timeout_ms", timeout_ms, MAX_TIMEOUT_MS)?;

    let progress_token = meta.get_progress_token();
    let start = Instant::now();
    let outcome = read_until_pattern(
        &connection,
        b"\n",
        timeout_ms,
        max_bytes,
        &ct,
        progress_token,
        Some(&peer),
    )
    .await?;

    let elapsed_ms = start.elapsed().as_millis() as u64;

    if outcome.timed_out {
        return Err(format!(
            "read_line timed out after {timeout_ms}ms on {}",
            args.connection_id
        ));
    }

    let match_idx = outcome.match_index.unwrap_or(outcome.bytes.len());
    let mut line_bytes = Vec::from(&outcome.bytes[..match_idx]);
    let trailing = outcome.bytes.get(match_idx + 1..).unwrap_or(&[]);
    connection.unread(trailing).await;
    if line_bytes.last() == Some(&b'\r') {
        line_bytes.pop();
    }

    let bytes_read = line_bytes.len();
    let data =
        codec::encode(encoding, &line_bytes).map_err(|e| format!("Data encoding failed - {e}"))?;

    info!(
        "read_line {} returned {bytes_read} bytes in {elapsed_ms}ms",
        args.connection_id
    );

    Ok(Json(ReadLineResult {
        connection_id: args.connection_id,
        bytes_read,
        encoding: encoding.to_string(),
        line: data,
        timeout_ms,
        elapsed_ms,
    }))
}
