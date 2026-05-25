use std::sync::Arc;

use rmcp::Json;
use tracing::{debug, info};

use crate::serial::ConnectionManager;
use crate::tools::helpers::{log_tool_err, lookup_connection};
use crate::tools::types::{SendBreakArgs, SendBreakResult, SetDtrRtsArgs, SetDtrRtsResult};

pub async fn set_dtr_rts(
    connections: &Arc<ConnectionManager>,
    args: SetDtrRtsArgs,
) -> Result<Json<SetDtrRtsResult>, String> {
    debug!(
        "set_dtr_rts {} dtr={} rts={}",
        args.connection_id, args.dtr, args.rts
    );

    let connection = lookup_connection(connections, &args.connection_id).await?;
    connection
        .set_dtr_rts(args.dtr, args.rts)
        .await
        .map_err(|e| {
            log_tool_err(
                "set_dtr_rts",
                &format!("Failed to set control lines on {}", args.connection_id),
                e,
            )
        })?;

    info!(
        "Control lines on {} set to dtr={} rts={}",
        args.connection_id, args.dtr, args.rts
    );

    Ok(Json(SetDtrRtsResult {
        connection_id: args.connection_id,
        dtr: args.dtr,
        rts: args.rts,
    }))
}

pub async fn send_break(
    connections: &Arc<ConnectionManager>,
    args: SendBreakArgs,
) -> Result<Json<SendBreakResult>, String> {
    debug!(
        "send_break {} duration={}ms",
        args.connection_id, args.duration_ms
    );

    let connection = lookup_connection(connections, &args.connection_id).await?;
    connection.send_break(args.duration_ms).await.map_err(|e| {
        log_tool_err(
            "send_break",
            &format!("Failed to send break on {}", args.connection_id),
            e,
        )
    })?;

    info!(
        "Sent break on {} for {}ms",
        args.connection_id, args.duration_ms
    );

    Ok(Json(SendBreakResult {
        connection_id: args.connection_id,
        duration_ms: args.duration_ms,
    }))
}
