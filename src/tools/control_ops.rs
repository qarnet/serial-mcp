use std::sync::Arc;

use rmcp::{model::Meta, Json, Peer, RoleServer};
use tokio::time::{Duration, Instant};
use tracing::{debug, info};

use crate::serial::{ConnectionManager, SerialConnection};
use crate::tools::helpers::log_tool_err;
use crate::tools::helpers::lookup_connection;
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
    meta: Meta,
    ct: tokio_util::sync::CancellationToken,
    peer: Peer<RoleServer>,
    args: SendBreakArgs,
) -> Result<Json<SendBreakResult>, String> {
    debug!(
        "send_break {} duration={}ms",
        args.connection_id, args.duration_ms
    );

    let connection = lookup_connection(connections, &args.connection_id).await?;

    struct BreakResetGuard {
        connection: Arc<SerialConnection>,
    }

    impl Drop for BreakResetGuard {
        fn drop(&mut self) {
            let connection = Arc::clone(&self.connection);
            tokio::spawn(async move {
                let _ = connection.set_break_state(false).await;
            });
        }
    }

    connection
        .set_break_state(true)
        .await
        .map_err(|e| log_tool_err("send_break", "Failed to assert BREAK", e))?;
    let _guard = BreakResetGuard {
        connection: Arc::clone(&connection),
    };

    let progress_token = meta.get_progress_token();
    if let Some(token) = progress_token.clone() {
        let _ = peer
            .notify_progress(rmcp::model::ProgressNotificationParam {
                progress_token: token,
                progress: 0.0,
                total: Some(args.duration_ms as f64),
                message: Some("break asserted".into()),
            })
            .await;
    }

    let start = Instant::now();
    let mut ticker = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = ct.cancelled() => return Err("Cancelled".into()),
            _ = ticker.tick() => {
                let elapsed = start.elapsed().as_millis() as u64;
                if elapsed >= args.duration_ms {
                    break;
                }
                if let Some(token) = progress_token.clone() {
                    let _ = peer
                        .notify_progress(rmcp::model::ProgressNotificationParam {
                            progress_token: token,
                            progress: elapsed as f64,
                            total: Some(args.duration_ms as f64),
                            message: Some("holding break".into()),
                        })
                        .await;
                }
            }
        }
    }

    connection
        .set_break_state(false)
        .await
        .map_err(|e| log_tool_err("send_break", "Failed to release BREAK", e))?;

    info!(
        "Sent break on {} for {}ms",
        args.connection_id, args.duration_ms
    );

    if let Some(token) = progress_token {
        let _ = peer
            .notify_progress(rmcp::model::ProgressNotificationParam {
                progress_token: token,
                progress: args.duration_ms as f64,
                total: Some(args.duration_ms as f64),
                message: Some("break released".into()),
            })
            .await;
    }

    Ok(Json(SendBreakResult {
        connection_id: args.connection_id,
        duration_ms: args.duration_ms,
    }))
}
