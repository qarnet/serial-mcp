use std::sync::Arc;

use rmcp::Json;
use tracing::{debug, info};

use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, PortInfo};
use crate::tools::helpers::log_tool_err;
use crate::tools::helpers::lookup_connection;
use crate::tools::helpers::parse_open_args;
use crate::tools::types::{
    ClearLogArgs, ClearLogResult, CloseArgs, CloseResult, DeleteProfileArgs, DeleteProfileResult,
    ExportLogArgs, ExportLogResult, GetLogArgs, GetLogResult, GetStatusArgs, GetStatusResult,
    ListConnectionsResult, ListPortsResult, ListProfilesResult, OpenArgs, OpenProfileArgs,
    OpenResult, ProfileSummary, ReconfigureArgs, ReconfigureResult, ReconnectArgs, ReconnectResult,
    SaveProfileArgs, SaveProfileResult,
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
    let port = args.port.clone();
    let name = args.name.clone();
    let baud_rate = args.baud_rate;
    debug!("Opening {} @ {}", port, baud_rate);

    if !security.is_port_allowed(&port) {
        return Err(format!(
            "Port '{port}' is not in the allowlist. Allowed patterns: {}",
            security.allowlist_summary()
        ));
    }

    // Capture OS-level port identity before opening so it is available
    // for status snapshots and profile save operations.
    let port_info = PortInfo::list_available()
        .ok()
        .and_then(|ports| ports.into_iter().find(|p| p.name == port));

    let reconnect_policy = args.reconnect_policy.clone();
    let mut config = parse_open_args(args)?;
    config.port_info = port_info;

    let connection_id = connections
        .open(config)
        .await
        .map_err(|e| log_tool_err("open", &format!("Failed to open port {port}"), e))?;

    // Set reconnect policy on the newly opened connection.
    if let Ok(conn) = connections.get(&connection_id).await {
        *conn.reconnect_policy.lock().expect("poisoned") = reconnect_policy;
    }

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
    let conn = lookup_connection(connections, &args.connection_id).await?;

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
        read_ops: status.read_ops,
        write_ops: status.write_ops,
        truncation_count: status.truncation_count,
        notification_drop_count: status.notification_drop_count,
        port_info: status.port_info,
        state: status.state,
        reconnect_attempts: status.reconnect_attempts,
        last_error: status.last_error,
    }))
}

pub async fn reconfigure(
    connections: &Arc<ConnectionManager>,
    args: ReconfigureArgs,
) -> Result<Json<ReconfigureResult>, String> {
    let conn_id = &args.connection_id;
    debug!("Reconfiguring {}", conn_id);

    let conn = lookup_connection(connections, conn_id).await?;

    let baud_rate = args.baud_rate;
    let data_bits = args
        .data_bits
        .as_deref()
        .map(|s| s.parse::<crate::serial::DataBits>())
        .transpose()?;
    let stop_bits = args
        .stop_bits
        .as_deref()
        .map(|s| s.parse::<crate::serial::StopBits>())
        .transpose()?;
    let parity = args
        .parity
        .as_deref()
        .map(|s| s.parse::<crate::serial::Parity>())
        .transpose()?;
    let flow_control = args
        .flow_control
        .as_deref()
        .map(|s| s.parse::<crate::serial::FlowControl>())
        .transpose()?;

    let status = conn
        .reconfigure(baud_rate, data_bits, stop_bits, parity, flow_control)
        .await
        .map_err(|e| {
            log_tool_err(
                "reconfigure",
                &format!("Failed to reconfigure connection {conn_id}"),
                e,
            )
        })?;

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

pub fn list_profiles(
    profiles: &[crate::profiles::Profile],
) -> Result<Json<ListProfilesResult>, String> {
    let summaries: Vec<ProfileSummary> = profiles
        .iter()
        .map(|p| ProfileSummary {
            name: p.name.clone(),
            selector: p.selector.clone(),
            defaults: p.defaults.clone(),
        })
        .collect();
    let count = summaries.len();
    info!("Listed {count} profiles");
    Ok(Json(ListProfilesResult {
        count,
        profiles: summaries,
    }))
}

pub async fn open_profile(
    connections: &Arc<ConnectionManager>,
    security: &SecurityManager,
    profiles: &[crate::profiles::Profile],
    args: OpenProfileArgs,
) -> Result<Json<OpenResult>, String> {
    let profile = profiles
        .iter()
        .find(|p| p.name == args.profile)
        .ok_or_else(|| format!("Profile '{}' not found", args.profile))?;

    let ports = PortInfo::list_available()
        .map_err(|e| log_tool_err("open_profile", "Failed to list ports", e))?;

    let matched = ports.iter().find(|p| profile.matches(p)).ok_or_else(|| {
        format!(
            "No port matches profile '{}' selector: {:?}",
            args.profile, profile.selector
        )
    })?;

    open(
        connections,
        security,
        OpenArgs {
            port: matched.name.clone(),
            name: args.name.or_else(|| {
                profile.defaults.name.as_ref().map(|prefix| {
                    format!(
                        "{}-{}",
                        prefix,
                        matched.name.rsplit('/').next().unwrap_or(&matched.name)
                    )
                })
            }),
            baud_rate: profile.defaults.baud_rate,
            data_bits: profile.defaults.data_bits.clone(),
            stop_bits: profile.defaults.stop_bits.clone(),
            parity: profile.defaults.parity.clone(),
            flow_control: profile.defaults.flow_control.clone(),
            log_capacity: args.log_capacity,
            log_enabled: args.log_enabled,
            reconnect_policy: crate::serial::ReconnectPolicy::default(),
        },
    )
    .await
}

/// Save a new profile by snapshotting an open connection's identity
/// and current configuration.
pub async fn save_profile(
    connections: &Arc<ConnectionManager>,
    profiles: &Arc<tokio::sync::RwLock<Vec<crate::profiles::Profile>>>,
    profiles_path: &std::path::PathBuf,
    args: SaveProfileArgs,
) -> Result<Json<SaveProfileResult>, String> {
    let conn = lookup_connection(connections, &args.connection_id).await?;

    let info = conn
        .port_info()
        .ok_or_else(|| format!("No port identity available for {}", args.connection_id))?;

    let defaults = crate::profiles::ProfileDefaults {
        baud_rate: conn.baud_rate(),
        data_bits: crate::serial::data_bits_to_str(conn.data_bits()),
        stop_bits: crate::serial::stop_bits_to_str(conn.stop_bits()),
        parity: crate::serial::parity_to_str(conn.parity()),
        flow_control: crate::serial::flow_control_to_str(conn.flow_control()),
        name: conn.name().map(str::to_string),
        reconnect_policy: None,
        decoder: None,
        safety_policy: None,
    };

    let selector = crate::profiles::ProfileSelector {
        vid: info.vid,
        pid: info.pid,
        serial_number: info.serial_number.clone(),
        manufacturer: info.manufacturer.clone(),
        product: info.product.clone(),
        interface: info.interface,
        port_pattern: None,
        description_pattern: None,
        transport: Some(info.transport.to_string()),
        hardware_id: info.hardware_id.clone(),
    };

    let profile = crate::profiles::Profile {
        name: args.profile_name.clone(),
        selector,
        defaults,
    };

    let created = crate::profiles::save_profile(profiles_path, &profile, args.overwrite)?;

    // Reload profiles into memory.
    let reloaded = crate::profiles::load_profiles(profiles_path);
    {
        let mut lock = profiles.write().await;
        *lock = reloaded;
    }

    Ok(Json(SaveProfileResult {
        name: profile.name,
        selector: profile.selector,
        defaults: profile.defaults,
        created,
    }))
}

/// Delete a profile by name.
pub async fn delete_profile(
    profiles: &Arc<tokio::sync::RwLock<Vec<crate::profiles::Profile>>>,
    profiles_path: &std::path::PathBuf,
    args: DeleteProfileArgs,
) -> Result<Json<DeleteProfileResult>, String> {
    crate::profiles::delete_profile(profiles_path, &args.profile_name)?;

    // Reload profiles into memory.
    let reloaded = crate::profiles::load_profiles(profiles_path);
    {
        let mut lock = profiles.write().await;
        *lock = reloaded;
    }

    Ok(Json(DeleteProfileResult {
        profile_name: args.profile_name,
    }))
}

// ── Reconnect tool ─────────────────────────────────────────────────────

pub async fn reconnect(
    connections: &Arc<ConnectionManager>,
    args: ReconnectArgs,
) -> Result<Json<ReconnectResult>, String> {
    let conn = lookup_connection(connections, &args.connection_id).await?;

    conn.reconnect()
        .await
        .map_err(|e| format!("Reconnect failed: {e}"))?;

    Ok(Json(ReconnectResult {
        connection_id: args.connection_id,
        name: conn.name().map(str::to_string),
        port: conn.port().to_string(),
        state: conn.state(),
    }))
}

// ── Log tools ──────────────────────────────────────────────────────────

pub async fn get_log(
    connections: &Arc<ConnectionManager>,
    args: GetLogArgs,
) -> Result<Json<GetLogResult>, String> {
    let conn = lookup_connection(connections, &args.connection_id).await?;

    let log = conn.log();
    let all = log.snapshot();
    let total = all.len();

    let filtered: Vec<crate::log_buffer::LogEntry> = all
        .into_iter()
        .filter(|e| args.since_ms.is_none_or(|since| e.timestamp_ms >= since))
        .collect();

    let events = match args.limit {
        Some(limit) if limit < filtered.len() => {
            let start = filtered.len() - limit;
            filtered[start..].to_vec()
        }
        _ => filtered,
    };

    Ok(Json(GetLogResult {
        log_enabled: log.is_enabled(),
        capacity: log.capacity(),
        total_events: total,
        events,
    }))
}

pub async fn clear_log(
    connections: &Arc<ConnectionManager>,
    args: ClearLogArgs,
) -> Result<Json<ClearLogResult>, String> {
    let conn = lookup_connection(connections, &args.connection_id).await?;
    conn.log().clear();
    Ok(Json(ClearLogResult {
        connection_id: args.connection_id,
    }))
}

pub async fn export_log(
    connections: &Arc<ConnectionManager>,
    args: ExportLogArgs,
) -> Result<Json<ExportLogResult>, String> {
    let conn = lookup_connection(connections, &args.connection_id).await?;

    let events = conn.log().snapshot();
    let count = events.len();
    let mut out = String::new();
    for event in &events {
        let line = serde_json::to_string(event)
            .map_err(|e| format!("Failed to serialize log entry: {e}"))?;
        out.push_str(&line);
        out.push('\n');
    }
    std::fs::write(&args.path, out).map_err(|e| format!("Failed to write log export: {e}"))?;

    Ok(Json(ExportLogResult {
        connection_id: args.connection_id,
        path: args.path,
        events_written: count,
    }))
}
