use std::sync::Arc;

use rmcp::Json;
use tracing::{debug, info};

use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, PortInfo};
use crate::tools::helpers::log_tool_err;
use crate::tools::helpers::parse_open_args;
use crate::tools::types::{
    CloseArgs, CloseResult, ListConnectionsResult, ListPortsResult, OpenArgs, OpenResult,
};

pub async fn list_ports() -> Result<Json<ListPortsResult>, String> {
    debug!("Listing serial ports");
    let ports = PortInfo::list_available()
        .map_err(|e| log_tool_err("list_ports", "Failed to list ports", e))?;
    info!("Found {} serial ports", ports.len());
    Ok(Json(ListPortsResult {
        count: ports.len(),
        ports,
    }))
}

pub async fn list_connections(
    connections: &Arc<ConnectionManager>,
) -> Result<Json<ListConnectionsResult>, String> {
    let summaries = connections.list_open().await;
    Ok(Json(ListConnectionsResult {
        count: summaries.len(),
        connections: summaries,
    }))
}

pub async fn open(
    connections: &Arc<ConnectionManager>,
    security: &SecurityManager,
    args: OpenArgs,
) -> Result<Json<OpenResult>, String> {
    let config = parse_open_args(args)?;
    let port = config.port.clone();
    let name = config.name.clone();
    let baud_rate = config.baud_rate;
    debug!("Opening {} @ {}", port, baud_rate);

    if !security.is_port_allowed(&port) {
        return Err(format!(
            "Port '{port}' is not in the allowlist. Allowed patterns: {}",
            security.allowlist_summary()
        ));
    }

    let connection_id = connections
        .open(config)
        .await
        .map_err(|e| log_tool_err("open", &format!("Failed to open port {port}"), e))?;
    info!("Opened connection {} -> {}", connection_id, port);

    Ok(Json(OpenResult {
        connection_id,
        name,
        port,
        baud_rate,
    }))
}

pub async fn close(
    connections: &Arc<ConnectionManager>,
    args: CloseArgs,
) -> Result<Json<CloseResult>, String> {
    debug!("Closing {}", args.connection_id);
    let name = connections
        .get(&args.connection_id)
        .await
        .ok()
        .and_then(|connection| connection.name().map(str::to_string));

    connections.close(&args.connection_id).await.map_err(|e| {
        log_tool_err(
            "close",
            &format!("Failed to close connection {}", args.connection_id),
            e,
        )
    })?;
    info!("Closed connection {}", args.connection_id);

    Ok(Json(CloseResult {
        connection_id: args.connection_id,
        name,
    }))
}
