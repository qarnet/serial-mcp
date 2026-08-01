use std::sync::Arc;

use rmcp::Json;
use tracing::{debug, info};

use crate::profiles::{
    canonical_high_selector, high_identity, identity_confidence, IdentityConfidence, Profile,
    ProfileMode, ProfileSelectionSource,
};
use crate::rx_session::RxSessionManager;
use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, PortInfo, PortProvider};
use crate::tools::helpers::log_tool_err;
use crate::tools::helpers::lookup_connection;
use crate::tools::helpers::{OpenOverlay, ResolvedOpenSettings};
use crate::tools::types::{
    ClearLogArgs, ClearLogResult, CloseArgs, CloseResult, ConfigureArgs, ConfigureResult,
    DeleteProfileArgs, DeleteProfileResult, ExportLogArgs, ExportLogResult, GetLogArgs,
    GetLogResult, GetStatusArgs, GetStatusResult, ListConnectionsResult, ListPortsResult,
    ListProfilesResult, OpenArgs, OpenProfileArgs, OpenResult, ProfileSummary, ReconfigureArgs,
    ReconfigureResult, ReconnectArgs, ReconnectResult, SaveProfileArgs, SaveProfileResult,
};

pub async fn list_ports(provider: &Arc<dyn PortProvider>) -> Result<Json<ListPortsResult>, String> {
    debug!("Listing serial ports");
    let ports = provider
        .list_available()
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

/// The profile-session plan for a bare `open`, decided BEFORE hardware is
/// opened. Post-open steps (mark used / create generated / attach binding)
/// run only after the hardware open succeeds.
enum SessionPlan {
    /// `profile_mode="none"`: no automatic behavior at all.
    Disabled { confidence: IdentityConfidence },
    /// Weak identity, duplicated live fingerprint, or equal top-ranked
    /// profile timestamps: transient session, never persisted.
    Transient {
        confidence: IdentityConfidence,
        candidates: Vec<String>,
    },
    /// One uniquely most-recently-used high-confidence profile.
    Selected { profile: Profile },
    /// Explicit named selection via `open_profile`.
    Explicit { profile: Profile },
    /// No matching profile yet: create a durable generated profile after
    /// the hardware open succeeds.
    Generate,
}

/// Decide the session plan for a bare `open` without touching hardware.
async fn plan_session(
    store: &Arc<crate::profile_store::ProfileStore>,
    args: &OpenArgs,
    port_info: Option<&PortInfo>,
    live_ports: &[PortInfo],
) -> Result<SessionPlan, String> {
    let confidence = port_info
        .map(identity_confidence)
        .unwrap_or(IdentityConfidence::None);
    if args.profile_mode == Some(ProfileMode::None) {
        return Ok(SessionPlan::Disabled { confidence });
    }

    let Some(port_info) = port_info else {
        return Ok(SessionPlan::Transient {
            confidence,
            candidates: Vec::new(),
        });
    };

    let Some(identity) = high_identity(port_info) else {
        return Ok(SessionPlan::Transient {
            confidence,
            candidates: Vec::new(),
        });
    };

    // If multiple currently enumerated ports share the same high
    // fingerprint, never apply settings to an indistinguishable device.
    let duplicates = live_ports
        .iter()
        .filter(|p| high_identity(p).as_ref() == Some(&identity))
        .count();
    if duplicates > 1 {
        return Ok(SessionPlan::Transient {
            confidence: IdentityConfidence::High,
            candidates: Vec::new(),
        });
    }

    let resolution = store
        .resolve_automatic(port_info)
        .await
        .map_err(|e| log_tool_err("open", "Failed to resolve profiles", e))?;
    match resolution.selected {
        Some(profile) => Ok(SessionPlan::Selected { profile }),
        None if resolution.ambiguous => Ok(SessionPlan::Transient {
            confidence: IdentityConfidence::High,
            candidates: resolution.candidates,
        }),
        None => Ok(SessionPlan::Generate),
    }
}

/// Shared post-open plumbing: attach the session binding computed from the
/// resolved settings and the session plan.
async fn attach_session_binding(
    connections: &Arc<ConnectionManager>,
    store: &Arc<crate::profile_store::ProfileStore>,
    connection_id: &str,
    plan: SessionPlan,
    resolved: &ResolvedOpenSettings,
    port_info: Option<&PortInfo>,
) {
    let conn = match connections.get(connection_id).await {
        Ok(c) => c,
        Err(_) => return,
    };
    let binding = match plan {
        SessionPlan::Disabled { confidence } => Some(crate::serial::ActiveProfileBinding {
            profile_name: String::new(),
            source: ProfileSelectionSource::Disabled,
            confidence,
            persistent: false,
            generated: false,
            revision: None,
            dirty: false,
            candidates: Vec::new(),
            last_persistence_error: None,
        }),
        SessionPlan::Transient {
            confidence,
            candidates,
        } => Some(crate::serial::ActiveProfileBinding {
            profile_name: String::new(),
            source: ProfileSelectionSource::Transient,
            confidence,
            persistent: false,
            generated: false,
            revision: None,
            dirty: false,
            candidates,
            last_persistence_error: None,
        }),
        SessionPlan::Selected { profile } => {
            let dirty = profile_only_differs(resolved, &profile);
            match store.mark_used(&profile.name).await {
                Ok(used) => Some(crate::serial::ActiveProfileBinding {
                    profile_name: used.name.clone(),
                    source: ProfileSelectionSource::Automatic,
                    confidence: IdentityConfidence::High,
                    persistent: true,
                    generated: used.metadata.generated,
                    revision: Some(used.metadata.revision),
                    dirty,
                    candidates: Vec::new(),
                    last_persistence_error: None,
                }),
                Err(e) => Some(crate::serial::ActiveProfileBinding {
                    profile_name: profile.name.clone(),
                    source: ProfileSelectionSource::Automatic,
                    confidence: IdentityConfidence::High,
                    persistent: true,
                    generated: profile.metadata.generated,
                    revision: Some(profile.metadata.revision),
                    dirty,
                    candidates: Vec::new(),
                    last_persistence_error: Some(e),
                }),
            }
        }
        SessionPlan::Explicit { profile } => {
            let dirty = profile_only_differs(resolved, &profile);
            match store.mark_used(&profile.name).await {
                Ok(used) => Some(crate::serial::ActiveProfileBinding {
                    profile_name: used.name.clone(),
                    source: ProfileSelectionSource::Explicit,
                    confidence: IdentityConfidence::High,
                    persistent: true,
                    generated: used.metadata.generated,
                    revision: Some(used.metadata.revision),
                    dirty,
                    candidates: Vec::new(),
                    last_persistence_error: None,
                }),
                Err(e) => Some(crate::serial::ActiveProfileBinding {
                    profile_name: profile.name.clone(),
                    source: ProfileSelectionSource::Explicit,
                    confidence: IdentityConfidence::High,
                    persistent: true,
                    generated: profile.metadata.generated,
                    revision: Some(profile.metadata.revision),
                    dirty,
                    candidates: Vec::new(),
                    last_persistence_error: Some(e),
                }),
            }
        }
        SessionPlan::Generate => {
            // Generated profile defaults equal the effective live settings.
            let defaults = resolved.as_profile_defaults();
            let selector = port_info.and_then(canonical_high_selector);
            let label = generated_label(port_info);
            let Some(selector) = selector else {
                // Cannot happen: Generate requires a high identity.
                return;
            };
            match store.create_generated(label, selector, defaults).await {
                Ok(created) => Some(crate::serial::ActiveProfileBinding {
                    profile_name: created.name.clone(),
                    source: ProfileSelectionSource::Generated,
                    confidence: IdentityConfidence::High,
                    persistent: true,
                    generated: true,
                    revision: Some(created.metadata.revision),
                    dirty: false,
                    candidates: Vec::new(),
                    last_persistence_error: None,
                }),
                // Keep the connection open and bind a transient session
                // carrying the error: do not report open failure or
                // pretend the profile persisted.
                Err(e) => Some(crate::serial::ActiveProfileBinding {
                    profile_name: String::new(),
                    source: ProfileSelectionSource::Transient,
                    confidence: IdentityConfidence::High,
                    persistent: false,
                    generated: false,
                    revision: None,
                    dirty: false,
                    candidates: Vec::new(),
                    last_persistence_error: Some(e),
                }),
            }
        }
    };
    conn.set_active_profile_binding(binding);
}

/// `true` when the resolved effective settings differ from what the
/// selected profile alone would produce (explicit overrides → dirty).
fn profile_only_differs(resolved: &ResolvedOpenSettings, profile: &Profile) -> bool {
    match ResolvedOpenSettings::from_profile(resolved.port.clone(), profile) {
        Ok(profile_only) => resolved != &profile_only,
        Err(_) => false,
    }
}

/// Generated-profile label: product, else manufacturer, else
/// `usb-{vid:04x}-{pid:04x}`.
fn generated_label(port_info: Option<&PortInfo>) -> String {
    match port_info {
        Some(p) => {
            if let Some(product) = p.product.as_deref().filter(|s| !s.is_empty()) {
                product.to_string()
            } else if let Some(mfr) = p.manufacturer.as_deref().filter(|s| !s.is_empty()) {
                mfr.to_string()
            } else if let (Some(vid), Some(pid)) = (p.vid, p.pid) {
                format!("usb-{vid:04x}-{pid:04x}")
            } else {
                "serial-device".to_string()
            }
        }
        None => "serial-device".to_string(),
    }
}

/// Shared open plumbing dependencies.
struct OpenContext<'a> {
    connections: &'a Arc<ConnectionManager>,
    rx_sessions: &'a Arc<RxSessionManager>,
    security: &'a SecurityManager,
    store: &'a Arc<crate::profile_store::ProfileStore>,
}

/// Shared hardware-open step: allowlist check, resolve settings, open the
/// port, set reconnect policy, start the RX session, then attach the
/// profile-session binding (create/mark profile only after hardware open
/// succeeds).
async fn open_connection(
    ctx: OpenContext<'_>,
    port: String,
    overlay: &OpenOverlay,
    profile_defaults: Option<&crate::profiles::ProfileDefaults>,
    port_info: Option<PortInfo>,
    plan: SessionPlan,
) -> Result<Json<OpenResult>, String> {
    if !ctx.security.is_port_allowed(&port) {
        return Err(format!(
            "Port '{port}' is not in the allowlist. Allowed patterns: {}",
            ctx.security.allowlist_summary()
        ));
    }

    let resolved = ResolvedOpenSettings::resolve(port.clone(), overlay, profile_defaults)?;
    let config = resolved.clone().into_connection_config(port_info.clone());

    let connection_id = ctx
        .connections
        .open(config)
        .await
        .map_err(|e| log_tool_err("open", &format!("Failed to open port {port}"), e))?;

    // Set reconnect policy on the newly opened connection.
    if let Ok(conn) = ctx.connections.get(&connection_id).await {
        *conn.reconnect_policy.lock().expect("poisoned") = resolved.reconnect_policy.clone();
    }

    // Create the RX session and start the always-on pump with a budgeted ring.
    // The session is idempotent — if another code path created one first, this
    // returns the existing session.
    if let Ok(conn) = ctx.connections.get(&connection_id).await {
        let session = ctx
            .rx_sessions
            .get_or_create(conn, resolved.rx_buffer_size)
            .await
            .map_err(|e| log_tool_err("open", "Failed to create RX session", e))?;
        debug!(
            "rx_session: pump started for {} (ring={} bytes)",
            session.connection_id(),
            session.ring_capacity()
        );
    }

    // Post-open profile work: never close a working port merely because
    // profile metadata failed — failures surface as `last_persistence_error`.
    attach_session_binding(
        ctx.connections,
        ctx.store,
        &connection_id,
        plan,
        &resolved,
        port_info.as_ref(),
    )
    .await;

    info!("Opened connection {} -> {}", connection_id, port);

    let binding = ctx
        .connections
        .get(&connection_id)
        .await
        .ok()
        .and_then(|c| c.active_profile_binding())
        .map(|b| b.to_session_result());

    Ok(Json(OpenResult {
        connection_id,
        name: resolved.name,
        port: resolved.port,
        baud_rate: resolved.baud_rate,
        profile: binding,
    }))
}

pub async fn open(
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    security: &SecurityManager,
    store: &Arc<crate::profile_store::ProfileStore>,
    provider: &Arc<dyn PortProvider>,
    args: OpenArgs,
) -> Result<Json<OpenResult>, String> {
    let port = args.port.clone();
    debug!("Opening {}", port);

    // Enumerate once through the injectable provider: identity capture,
    // duplicate-fingerprint detection, and automatic resolution all use
    // the same live view.
    let live_ports = provider
        .list_available()
        .map_err(|e| log_tool_err("open", "Failed to list ports", e))?;
    let port_info = live_ports.iter().find(|p| p.name == port).cloned();

    let plan = plan_session(store, &args, port_info.as_ref(), &live_ports).await?;
    let overlay = OpenOverlay::from_open_args(&args);
    let profile_defaults = match &plan {
        SessionPlan::Selected { profile } => Some(profile.defaults.clone()),
        _ => None,
    };

    open_connection(
        OpenContext {
            connections,
            rx_sessions,
            security,
            store,
        },
        port,
        &overlay,
        profile_defaults.as_ref(),
        port_info,
        plan,
    )
    .await
}

pub async fn open_profile(
    connections: &Arc<ConnectionManager>,
    rx_sessions: &Arc<RxSessionManager>,
    security: &SecurityManager,
    store: &Arc<crate::profile_store::ProfileStore>,
    provider: &Arc<dyn PortProvider>,
    profile: Option<Profile>,
    args: OpenProfileArgs,
) -> Result<Json<OpenResult>, String> {
    let profile = profile.ok_or_else(|| format!("Profile '{}' not found", args.profile))?;

    let ports = provider
        .list_available()
        .map_err(|e| log_tool_err("open_profile", "Failed to list ports", e))?;

    let matched: Vec<PortInfo> = ports
        .iter()
        .filter(|p| profile.matches(p))
        .cloned()
        .collect();

    if matched.is_empty() {
        return Err(format!(
            "No port matches profile '{}' selector: {:?}",
            args.profile, profile.selector
        ));
    }
    if matched.len() > 1 {
        let names: Vec<String> = matched.iter().map(|p| p.name.clone()).collect();
        return Err(format!(
            "Profile '{}' selector matches {} live ports ({}) — refusing to choose. \
             Narrow the selector so exactly one port matches.",
            args.profile,
            matched.len(),
            names.join(", ")
        ));
    }

    let port = matched.into_iter().next().expect("exactly one match");
    let overlay = OpenOverlay::from_open_profile_args(&args);
    let defaults = profile.defaults.clone();

    open_connection(
        OpenContext {
            connections,
            rx_sessions,
            security,
            store,
        },
        port.name.clone(),
        &overlay,
        Some(&defaults),
        Some(port),
        SessionPlan::Explicit { profile },
    )
    .await
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
    rx_sessions: &Arc<RxSessionManager>,
    args: GetStatusArgs,
) -> Result<Json<GetStatusResult>, String> {
    debug!("Getting status for {}", args.connection_id);
    let conn = lookup_connection(connections, &args.connection_id).await?;

    let status = conn.status_snapshot();
    info!(
        "Status {}: open={} tx={} rx={}",
        args.connection_id, !status.is_closed, status.tx_bytes, status.rx_bytes
    );

    // Gather ring fields if a session exists.
    let (
        rx_buffer_size,
        rx_start_offset,
        rx_end_offset,
        rx_cursor,
        rx_buffered_unread,
        rx_bytes_wrapped_total,
    ) = if let Some(session) = rx_sessions.get(&args.connection_id).await {
        let ring = session.ring();
        let start = ring.start_offset();
        let end = ring.end_offset();
        let cur = session.read_cursor();
        let unread = end.saturating_sub(cur);
        (
            session.ring_capacity(),
            start,
            end,
            cur,
            unread,
            ring.bytes_wrapped_total(),
        )
    } else {
        (0, 0, 0, 0, 0, 0)
    };

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
        rx_buffer_size,
        rx_start_offset,
        rx_end_offset,
        rx_cursor,
        rx_buffered_unread,
        rx_bytes_wrapped_total,
        profile: status.profile,
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
            metadata: p.metadata.clone(),
            revisions: p.revisions.clone(),
        })
        .collect();
    let count = summaries.len();
    info!("Listed {count} profiles");
    Ok(Json(ListProfilesResult {
        count,
        profiles: summaries,
    }))
}

/// Configure connection defaults. Two modes: profile (persist through the
/// shared store) and connection (mutate live connection defaults).
pub async fn configure(
    connections: &Arc<ConnectionManager>,
    store: &Arc<crate::profile_store::ProfileStore>,
    args: ConfigureArgs,
) -> Result<Json<ConfigureResult>, String> {
    // Validate: exactly one of profile / connection_id.
    match (args.profile.as_ref(), args.connection_id.as_ref()) {
        (Some(_), Some(_)) => {
            return Err(
                "configure: provide exactly one of `profile` or `connection_id`, not both".into(),
            )
        }
        (None, None) => {
            return Err("configure: provide exactly one of `profile` or `connection_id`".into())
        }
        _ => {}
    }

    if let Some(profile_name) = args.profile.as_ref() {
        // Profile mode: the store reloads under lock, preserves the
        // on-disk selector, and persists before updating its cache. The
        // effective profile (created flag + defaults) is returned from the
        // same transaction — no racy second lookup.
        let (created, profile) = store
            .update_defaults_preserving_selector(
                profile_name.clone(),
                args.defaults.clone(),
                args.overwrite,
            )
            .await?;
        Ok(Json(ConfigureResult {
            mode: "profile".into(),
            defaults: profile.defaults,
            created: Some(created),
        }))
    } else {
        // Connection mode: mutate the live connection's defaults.
        let conn_id = args.connection_id.as_ref().unwrap();
        let conn = lookup_connection(connections, conn_id).await?;
        // Apply framing defaults.
        conn.set_tx_framing_default(args.defaults.tx_framing.clone());
        conn.set_rx_framing_default(args.defaults.rx_framing.clone());
        conn.set_rx_parser_default(args.defaults.rx_parser.clone());
        conn.set_protocol_default(args.defaults.protocol);
        // Apply reconnect_policy (already StdMutex).
        *conn.reconnect_policy.lock().expect("poisoned") = args.defaults.reconnect_policy.clone();
        // Apply scalar defaults (Atomic).
        conn.set_max_buffered_bytes_default(args.defaults.max_buffered_bytes);
        conn.set_poll_interval_ms_default(args.defaults.poll_interval_ms);
        // log_capacity/log_enabled: LogBuffer has NO live setters. Documented as
        // profile-only. rx_buffer_size: ring is fixed at open. Also profile-only.
        Ok(Json(ConfigureResult {
            mode: "connection".into(),
            defaults: args.defaults,
            created: None,
        }))
    }
}

/// Save a new profile by snapshotting an open connection's identity
/// and current configuration.
pub async fn save_profile(
    connections: &Arc<ConnectionManager>,
    store: &Arc<crate::profile_store::ProfileStore>,
    args: SaveProfileArgs,
) -> Result<Json<SaveProfileResult>, String> {
    let conn = lookup_connection(connections, &args.connection_id).await?;

    let info = conn
        .port_info()
        .ok_or_else(|| format!("No port identity available for {}", args.connection_id))?;

    // Snapshot rx_buffer_size from the connection's stored effective value;
    // never from a handler-local session manager (a later request may land
    // on a different manager).
    let rx_buffer_size = conn.rx_buffer_size();

    let defaults = crate::profiles::ProfileDefaults {
        baud_rate: conn.baud_rate(),
        data_bits: crate::serial::data_bits_to_str(conn.data_bits()),
        stop_bits: crate::serial::stop_bits_to_str(conn.stop_bits()),
        parity: crate::serial::parity_to_str(conn.parity()),
        flow_control: crate::serial::flow_control_to_str(conn.flow_control()),
        name: conn.name().map(str::to_string),
        tx_framing: conn.tx_framing_default(),
        rx_framing: conn.rx_framing_default(),
        rx_parser: conn.rx_parser_default(),
        protocol: conn.protocol_default(),
        rx_buffer_size,
        max_buffered_bytes: conn.max_buffered_bytes_default(),
        poll_interval_ms: conn.poll_interval_ms_default(),
        reconnect_policy: conn.reconnect_policy.lock().expect("poisoned").clone(),
        log_capacity: conn.log().capacity(),
        log_enabled: conn.log().is_enabled(),
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
        metadata: crate::profiles::ProfileMetadata::default(),
        revisions: Vec::new(),
    };

    let created = store.upsert(profile.clone(), args.overwrite).await?;

    Ok(Json(SaveProfileResult {
        name: profile.name,
        selector: profile.selector,
        defaults: profile.defaults,
        created,
    }))
}

/// Delete a profile by name.
pub async fn delete_profile(
    store: &Arc<crate::profile_store::ProfileStore>,
    args: DeleteProfileArgs,
) -> Result<Json<DeleteProfileResult>, String> {
    store.delete(&args.profile_name).await?;

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
