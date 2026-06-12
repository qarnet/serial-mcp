use std::sync::Arc;

use rmcp::Json;
use tracing::{debug, info};

use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, PortInfo};
use crate::tools::helpers::log_tool_err;
use crate::tools::helpers::parse_open_args;
use crate::tools::types::{
    CloseArgs, CloseResult, GetStatusArgs, GetStatusResult, ListConnectionsResult,
    ListPortsResult, OpenArgs, OpenResult, ReconfigureArgs, ReconfigureResult,
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

pub async fn get_status(
    connections: &Arc<ConnectionManager>,
    args: GetStatusArgs,
) -> Result<Json<GetStatusResult>, String> {
    debug!("Getting status for {}", args.connection_id);
    let conn = connections
        .get(&args.connection_id)
        .await
        .map_err(|_| format!("Connection ID {} not found", args.connection_id))?;

    let status = conn.status_snapshot();
    info!(
        "Status {}: open={} tx={} rx={}",
        args.connection_id, !status.is_closed, status.tx_bytes, status.rx_bytes
    );

    Ok(Json(GetStatusResult {
        connection_id: status.connection_id,
        name: status.name,
        port: status.port,
        baud_rate: status.baud_rate,
        data_bits: status.data_bits,
        stop_bits: status.stop_bits,
        parity: status.parity,
        flow_control: status.flow_control,
        is_open: !status.is_closed,
        tx_bytes: status.tx_bytes,
        rx_bytes: status.rx_bytes,
        last_activity_ms: status.last_activity_ms,
    }))
}

pub async fn reconfigure(
    connections: &Arc<ConnectionManager>,
    args: ReconfigureArgs,
) -> Result<Json<ReconfigureResult>, String> {
    let conn_id = &args.connection_id;
    debug!("Reconfiguring {}", conn_id);

    let conn = connections
        .get(conn_id)
        .await
        .map_err(|_| format!("Connection ID {conn_id} not found"))?;

    let baud_rate = args.baud_rate;
    let data_bits = args
        .data_bits
        .as_deref()
        .map(parse_string_data_bits)
        .transpose()?;
    let stop_bits = args
        .stop_bits
        .as_deref()
        .map(parse_string_stop_bits)
        .transpose()?;
    let parity = args
        .parity
        .as_deref()
        .map(parse_string_parity)
        .transpose()?;
    let flow_control = args
        .flow_control
        .as_deref()
        .map(crate::tools::helpers::parse_flow_control)
        .transpose()?;

    let status = conn
        .reconfigure(baud_rate, data_bits, stop_bits, parity, flow_control)
        .await
        .map_err(|e| log_tool_err("reconfigure", &format!("Failed to reconfigure connection {conn_id}"), e))?;

    info!("Reconfigured {}: baud={}", conn_id, status.baud_rate);

    Ok(Json(ReconfigureResult {
        connection_id: status.connection_id,
        name: status.name,
        port: status.port,
        baud_rate: status.baud_rate,
        data_bits: status.data_bits,
        stop_bits: status.stop_bits,
        parity: status.parity,
        flow_control: status.flow_control,
    }))
}

fn parse_string_data_bits(s: &str) -> Result<crate::serial::DataBits, String> {
    match s {
        "5" => Ok(crate::serial::DataBits::Five),
        "6" => Ok(crate::serial::DataBits::Six),
        "7" => Ok(crate::serial::DataBits::Seven),
        "8" => Ok(crate::serial::DataBits::Eight),
        other => Err(format!("Invalid data_bits: {other}")),
    }
}

fn parse_string_stop_bits(s: &str) -> Result<crate::serial::StopBits, String> {
    match s {
        "1" => Ok(crate::serial::StopBits::One),
        "2" => Ok(crate::serial::StopBits::Two),
        other => Err(format!("Invalid stop_bits: {other}")),
    }
}

fn parse_string_parity(s: &str) -> Result<crate::serial::Parity, String> {
    match s {
        "none" => Ok(crate::serial::Parity::None),
        "odd" => Ok(crate::serial::Parity::Odd),
        "even" => Ok(crate::serial::Parity::Even),
        other => Err(format!("Invalid parity: {other}")),
    }
}
