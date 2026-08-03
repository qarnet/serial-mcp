use std::sync::Arc;

use rmcp::Json;
use tracing::{debug, info};

use crate::learning;
use crate::profiles::{
    canonical_high_selector, high_identity, identity_confidence, rank_candidates,
    selector_matches_high_identity, IdentityConfidence, Profile, ProfileMode,
    ProfilePersistenceOperation, ProfileSelectionSource,
};
use crate::rx_session::RxSessionManager;
use crate::security::SecurityManager;
use crate::serial::{ActiveProfileBinding, ConnectionManager, PortInfo, PortProvider};
use crate::tools::helpers::log_tool_err;
use crate::tools::helpers::lookup_connection;
use crate::tools::helpers::{OpenOverlay, ResolvedOpenSettings};
use crate::tools::types::{
    ClearLogArgs, ClearLogResult, CloseArgs, CloseResult, ConfigureArgs, ConfigureResult,
    DeleteProfileArgs, DeleteProfileResult, ExportLogArgs, ExportLogResult, GetLogArgs,
    GetLogResult, GetStatusArgs, GetStatusResult, ListConnectionsResult, ListPortsResult,
    ListProfilesResult, OpenArgs, OpenProfileArgs, OpenResult, PortProfileMatch,
    ProfileMatchCandidate, ProfileMatchOutcome, ProfileSummary, ReconfigureArgs, ReconfigureResult,
    ReconnectArgs, ReconnectResult, RollbackProfileArgs, RollbackProfileResult, SaveProfileArgs,
    SaveProfileResult,
};

pub async fn list_ports(
    provider: &Arc<dyn PortProvider>,
    store: &Arc<crate::profile_store::ProfileStore>,
) -> Result<Json<ListPortsResult>, String> {
    debug!("Listing serial ports");
    let ports = provider
        .list_available()
        .map_err(|e| log_tool_err("list_ports", "Failed to list ports", e))?;

    // ONE fresh cross-process read for the whole preview: a corrupt or
    // unreadable profile store is a tool error, never a silent "no matches".
    let profiles = store
        .list_fresh()
        .await
        .map_err(|e| log_tool_err("list_ports", "Failed to read profiles", e))?;

    let profile_matches = compute_profile_matches(&ports, &profiles);

    info!("Found {} serial ports", ports.len());
    Ok(Json(ListPortsResult {
        count: ports.len(),
        ports,
        profile_matches,
    }))
}

/// Pure preview of what a bare `open(port=...)` would do for every port,
/// computed over ONE live port list + ONE fresh profile snapshot (the
/// caller performs a single `ProfileStore::list_fresh()`). Never marks a
/// profile used and never mutates the store.
///
/// High identity reuses the open-time selection rules: candidates must pass
/// `Profile::matches` AND carry the target's high identity fields; the
/// unique maximum `last_used_at_ms` wins (`None` sorts oldest); equal top
/// rank is `Ambiguous`. Candidate order is deterministic — newest first,
/// then profile name for display only (a name never breaks a selection
/// tie).
///
/// Medium/low/none identity is never automatically selected: explicitly
/// matching non-empty selectors are listed as `Ineligible` candidates, and
/// empty selectors (which match any port) are excluded. A high fingerprint
/// shared by more than one live port yields `Duplicate` for every such
/// port — settings are never applied to an indistinguishable device.
pub fn compute_profile_matches(ports: &[PortInfo], profiles: &[Profile]) -> Vec<PortProfileMatch> {
    // Count live occurrences of each canonical high fingerprint so every
    // preview for the same device agrees on the duplicate flag.
    let mut high_counts: std::collections::HashMap<crate::profiles::HighIdentity, usize> =
        std::collections::HashMap::new();
    for port in ports {
        if let Some(identity) = high_identity(port) {
            *high_counts.entry(identity).or_insert(0) += 1;
        }
    }

    ports
        .iter()
        .map(|port| {
            let confidence = identity_confidence(port);
            let Some(identity) = high_identity(port) else {
                return weak_identity_profile_match(port, confidence, profiles);
            };

            let duplicated = high_counts.get(&identity).copied().unwrap_or(0) > 1;

            let eligible: Vec<Profile> = profiles
                .iter()
                .filter(|p| {
                    p.matches(port) && selector_matches_high_identity(&p.selector, &identity)
                })
                .cloned()
                .collect();

            if duplicated {
                return PortProfileMatch {
                    port: port.name.clone(),
                    confidence,
                    outcome: ProfileMatchOutcome::Duplicate,
                    selected_profile: None,
                    candidates: Vec::new(),
                };
            }

            if eligible.is_empty() {
                return PortProfileMatch {
                    port: port.name.clone(),
                    confidence,
                    outcome: ProfileMatchOutcome::None,
                    selected_profile: None,
                    candidates: Vec::new(),
                };
            }

            ranked_profile_match(port, confidence, eligible)
        })
        .collect()
}

/// Weak-identity preview (medium/low/none): never automatically selected.
/// Explicitly matching non-empty selectors remain visible candidates,
/// sorted by profile name; empty selectors (which match any port) are
/// excluded.
fn weak_identity_profile_match(
    port: &PortInfo,
    confidence: IdentityConfidence,
    profiles: &[Profile],
) -> PortProfileMatch {
    let mut candidates: Vec<ProfileMatchCandidate> = profiles
        .iter()
        .filter(|p| !p.selector.is_empty() && p.matches(port))
        .map(candidate_of)
        .collect();
    candidates.sort_by(|a, b| a.profile_name.cmp(&b.profile_name));
    PortProfileMatch {
        port: port.name.clone(),
        confidence,
        outcome: if candidates.is_empty() {
            ProfileMatchOutcome::None
        } else {
            ProfileMatchOutcome::Ineligible
        },
        selected_profile: None,
        candidates,
    }
}

/// Ranked high-identity preview over the eligible profiles. Display
/// candidates are ordered newest `last_used_at_ms` first, then name; the
/// selection decision below uses timestamps ONLY — a name never breaks an
/// equal top rank.
fn ranked_profile_match(
    port: &PortInfo,
    confidence: IdentityConfidence,
    eligible: Vec<Profile>,
) -> PortProfileMatch {
    let ranked = rank_candidates(eligible);
    let candidates: Vec<ProfileMatchCandidate> = {
        // Deterministic display order: newest first, then name.
        let mut sorted = ranked.clone();
        sorted.sort_by(|a, b| {
            b.metadata
                .last_used_at_ms
                .unwrap_or(0)
                .cmp(&a.metadata.last_used_at_ms.unwrap_or(0))
                .then_with(|| a.name.cmp(&b.name))
        });
        sorted.iter().map(candidate_of).collect()
    };

    let (selected, outcome) = if ranked.len() == 1 {
        (Some(ranked[0].name.clone()), ProfileMatchOutcome::Selected)
    } else {
        let top_ts = ranked[0].metadata.last_used_at_ms.unwrap_or(0);
        let next_ts = ranked[1].metadata.last_used_at_ms.unwrap_or(0);
        if top_ts != next_ts {
            (Some(ranked[0].name.clone()), ProfileMatchOutcome::Selected)
        } else {
            (None, ProfileMatchOutcome::Ambiguous)
        }
    };

    PortProfileMatch {
        port: port.name.clone(),
        confidence,
        outcome,
        selected_profile: selected,
        candidates,
    }
}

fn candidate_of(p: &Profile) -> ProfileMatchCandidate {
    ProfileMatchCandidate {
        profile_name: p.name.clone(),
        generated: p.metadata.generated,
        revision: p.metadata.revision,
        last_used_at_ms: p.metadata.last_used_at_ms,
    }
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

/// Binding for a `profile_mode="none"` session: never persists, no
/// candidates, no error.
fn disabled_binding(confidence: IdentityConfidence) -> ActiveProfileBinding {
    ActiveProfileBinding {
        profile_name: String::new(),
        source: ProfileSelectionSource::Disabled,
        confidence,
        persistent: false,
        generated: false,
        revision: None,
        dirty: false,
        stale: false,
        candidates: Vec::new(),
        last_persistence_error: None,
    }
}

/// Binding for a transient session: never persists, keeps the candidate
/// list the caller could have selected from, and optionally carries a
/// persistence error (generated-profile creation failure).
fn transient_binding(
    confidence: IdentityConfidence,
    candidates: Vec<String>,
    persistence_error: Option<String>,
) -> ActiveProfileBinding {
    ActiveProfileBinding {
        profile_name: String::new(),
        source: ProfileSelectionSource::Transient,
        confidence,
        persistent: false,
        generated: false,
        revision: None,
        dirty: false,
        stale: false,
        candidates,
        last_persistence_error: persistence_error,
    }
}

/// Binding for a persistent selected/generated profile. Name, generated
/// flag, and revision derive from the supplied profile; persistence
/// failures keep the connection on the original metadata and surface as
/// `last_persistence_error`.
fn persistent_binding(
    profile: &Profile,
    source: ProfileSelectionSource,
    confidence: IdentityConfidence,
    dirty: bool,
    persistence_error: Option<String>,
) -> ActiveProfileBinding {
    ActiveProfileBinding {
        profile_name: profile.name.clone(),
        source,
        confidence,
        persistent: true,
        generated: profile.metadata.generated,
        revision: Some(profile.metadata.revision),
        dirty,
        stale: false,
        candidates: Vec::new(),
        last_persistence_error: persistence_error,
    }
}

/// Mark the selected profile used and build its persistent binding. On
/// mark-used failure the connection stays on the original profile metadata
/// and the error is carried on the binding — never a hard open failure.
async fn mark_used_binding(
    store: &crate::profile_store::ProfileStore,
    profile: Profile,
    source: ProfileSelectionSource,
    confidence: IdentityConfidence,
    dirty: bool,
) -> ActiveProfileBinding {
    match store.mark_used(&profile.name).await {
        Ok(used) => persistent_binding(&used, source, confidence, dirty, None),
        Err(e) => persistent_binding(&profile, source, confidence, dirty, Some(e)),
    }
}

/// Shared post-open plumbing: attach the session binding computed from the
/// resolved settings and the session plan to the already-open connection.
///
/// Post-open PROFILE-METADATA failures (mark used / create generated) keep
/// the connection open and surface as `last_persistence_error` — they are
/// partial success, not open failure. The only error return is the
/// unreachable case where a `Generate` plan has no high identity.
async fn attach_session_binding(
    store: &Arc<crate::profile_store::ProfileStore>,
    conn: &Arc<crate::serial::SerialConnection>,
    plan: SessionPlan,
    resolved: &ResolvedOpenSettings,
    port_info: Option<&PortInfo>,
    dirty: Option<bool>,
) -> Result<ActiveProfileBinding, String> {
    let confidence = port_info
        .map(identity_confidence)
        .unwrap_or(IdentityConfidence::None);
    let binding = match plan {
        SessionPlan::Disabled { confidence } => disabled_binding(confidence),
        SessionPlan::Transient {
            confidence,
            candidates,
        } => transient_binding(confidence, candidates, None),
        SessionPlan::Selected { profile } => {
            let dirty = dirty.unwrap_or(false);
            mark_used_binding(
                store,
                profile,
                ProfileSelectionSource::Automatic,
                IdentityConfidence::High,
                dirty,
            )
            .await
        }
        SessionPlan::Explicit { profile } => {
            let dirty = dirty.unwrap_or(false);
            // Explicit selection reports the matched port's own identity
            // confidence — weak selectors are an explicit caller choice.
            mark_used_binding(
                store,
                profile,
                ProfileSelectionSource::Explicit,
                confidence,
                dirty,
            )
            .await
        }
        SessionPlan::Generate => {
            // Generated profile defaults equal the effective live settings.
            let defaults = resolved.as_profile_defaults();
            let selector = port_info.and_then(canonical_high_selector).ok_or_else(|| {
                "Cannot create generated profile: no high-confidence identity".to_string()
            })?;
            let label = generated_label(port_info);
            match store.create_generated(label, selector, defaults).await {
                Ok(created) => persistent_binding(
                    &created,
                    ProfileSelectionSource::Generated,
                    IdentityConfidence::High,
                    false,
                    None,
                ),
                // Keep the connection open and bind a transient session
                // carrying the error: do not report open failure or
                // pretend the profile persisted.
                Err(e) => transient_binding(IdentityConfidence::High, Vec::new(), Some(e)),
            }
        }
    };
    conn.set_active_profile_binding(Some(binding.clone()));
    Ok(binding)
}

/// Whether the resolved effective settings differ from what the selected
/// profile alone would produce (explicit overrides → dirty). Parse failures
/// in the profile's defaults (invalid data bits etc.) propagate as errors so
/// they surface BEFORE hardware open instead of silently mapping to clean.
fn profile_only_differs(
    resolved: &ResolvedOpenSettings,
    profile: &Profile,
) -> Result<bool, String> {
    let profile_only = ResolvedOpenSettings::from_profile(resolved.port.clone(), profile)?;
    Ok(resolved != &profile_only)
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

/// Shared hardware-open step: allowlist check, resolve settings, compute
/// the selected-profile dirty flag (invalid profile defaults are a tool
/// error BEFORE opening), open the port, set reconnect policy, start the RX
/// session, then attach the profile-session binding (create/mark profile
/// only after hardware open succeeds). Every successful open carries a
/// binding; a missing connection or binding is an operational error, never
/// a silently `None` result.
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

    // Dirty comparison happens BEFORE hardware open so invalid profile
    // defaults fail the call instead of producing a wrong binding.
    let dirty = match &plan {
        SessionPlan::Selected { profile } | SessionPlan::Explicit { profile } => {
            Some(profile_only_differs(&resolved, profile)?)
        }
        _ => None,
    };

    let config = resolved.clone().into_connection_config(port_info.clone());

    let connection_id = ctx
        .connections
        .open(config)
        .await
        .map_err(|e| log_tool_err("open", &format!("Failed to open port {port}"), e))?;

    // Obtain the connection exactly once; absence after a successful open
    // is an operational error, not a silent binding loss.
    let connection = ctx.connections.get(&connection_id).await.map_err(|e| {
        log_tool_err(
            "open",
            &format!("Failed to access opened connection {connection_id}"),
            e,
        )
    })?;

    // Set reconnect policy on the newly opened connection.
    *connection.reconnect_policy.lock().expect("poisoned") = resolved.reconnect_policy.clone();

    // Create the RX session and start the always-on pump with a budgeted ring.
    // The session is idempotent — if another code path created one first, this
    // returns the existing session.
    let session = ctx
        .rx_sessions
        .get_or_create(Arc::clone(&connection), resolved.rx_buffer_size)
        .await
        .map_err(|e| log_tool_err("open", "Failed to create RX session", e))?;
    debug!(
        "rx_session: pump started for {} (ring={} bytes)",
        session.connection_id(),
        session.ring_capacity()
    );

    // Post-open profile work: never close a working port merely because
    // profile metadata failed — failures surface as `last_persistence_error`.
    let binding = attach_session_binding(
        ctx.store,
        &connection,
        plan,
        &resolved,
        port_info.as_ref(),
        dirty,
    )
    .await?;

    // Open-override learning: a selected/explicit binding that is dirty
    // (explicit fields differ from the profile's defaults) is persisted
    // write-through before the open result returns. Persistence failure
    // keeps the open a success — the result carries `failed` state and the
    // binding stays dirty. Generated/transient/disabled sessions have
    // nothing to persist.
    let mut session = binding.to_session_result();
    let mut profile_persistence = None;
    if binding.dirty {
        let (learned, persistence) = learning::learn(
            ctx.store,
            &connection,
            ProfilePersistenceOperation::OpenOverride,
        )
        .await;
        if let Some(learned) = learned {
            session = learned;
        }
        profile_persistence = Some(persistence);
    }

    info!("Opened connection {} -> {}", connection_id, port);

    Ok(Json(OpenResult {
        connection_id,
        name: resolved.name,
        port: resolved.port,
        baud_rate: resolved.baud_rate,
        profile: Some(session),
        profile_persistence,
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

    let mut matched: Vec<PortInfo> = ports
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

    let port = matched.pop().ok_or_else(|| {
        format!(
            "No port matches profile '{}' selector: {:?}",
            args.profile, profile.selector
        )
    })?;
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
    store: &Arc<crate::profile_store::ProfileStore>,
    args: CloseArgs,
) -> Result<Json<CloseResult>, String> {
    debug!("Closing {}", args.connection_id);

    // Retain the connection Arc (and its binding) BEFORE the registry
    // removes it, so the close snapshot can still read effective state.
    let conn = lookup_connection(connections, &args.connection_id).await?;
    let name = conn.name().map(str::to_string);

    // Hold the learning lock across the clean hardware close and the
    // close-snapshot persistence so no concurrent durable mutation can
    // interleave.
    let _learning_guard = conn.learning_lock().lock().await;

    connections.close(&args.connection_id).await.map_err(|e| {
        log_tool_err(
            "close",
            &format!("Failed to close connection {}", args.connection_id),
            e,
        )
    })?;
    info!("Closed connection {}", args.connection_id);

    // Close snapshot: only after successful hardware close, persist the
    // effective defaults when the persistent binding is dirty or differs.
    // A no-op is `NotNeeded`; persistence failure neither reopens hardware
    // nor turns close into a tool error.
    let (profile, persistence) =
        learning::learn(store, &conn, ProfilePersistenceOperation::CloseSnapshot).await;

    Ok(Json(CloseResult {
        connection_id: args.connection_id,
        name,
        profile,
        profile_persistence: Some(persistence),
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
    store: &Arc<crate::profile_store::ProfileStore>,
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

    // Hold the learning lock across live mutation → effective snapshot →
    // CAS persistence → binding update.
    let _learning_guard = conn.learning_lock().lock().await;

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

    // Write-through learning: hardware mutation succeeded; persist the
    // effective defaults through the bound profile (if any). Persistence
    // failure keeps the tool result successful with `state="failed"`.
    let (profile, persistence) =
        learning::learn(store, &conn, ProfilePersistenceOperation::Learned).await;

    Ok(Json(ReconfigureResult {
        connection_id: status.connection_id,
        name: status.name,
        port: status.port,
        baud_rate: status.baud_rate,
        data_bits: status.data_bits,
        stop_bits: status.stop_bits,
        parity: status.parity,
        flow_control: status.flow_control,
        profile,
        profile_persistence: Some(persistence),
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
        // same transaction — no racy second lookup. Profile mode never
        // touches live connections, so `profile`/`profile_persistence`
        // fields stay None.
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
            profile: None,
            profile_persistence: None,
        }))
    } else {
        // Connection mode: mutate the live connection's defaults.
        let conn_id = args.connection_id.as_ref().unwrap();
        let conn = lookup_connection(connections, conn_id).await?;

        // Hold the learning lock across the setters, the effective
        // snapshot, and the CAS persistence attempt so concurrent durable
        // requests cannot snapshot half-applied state.
        let _learning_guard = conn.learning_lock().lock().await;

        // Apply framing defaults.
        conn.set_tx_framing_default(args.defaults.tx_framing.clone());
        conn.set_rx_framing_default(args.defaults.rx_framing.clone());
        conn.set_rx_parser_default(args.defaults.rx_parser.clone());
        conn.set_protocol_default(args.defaults.protocol);
        // Apply reconnect_policy (already StdMutex).
        *conn.reconnect_policy.lock().expect("poisoned") = args.defaults.reconnect_policy.clone();
        // Apply scalar defaults (Atomic).
        conn.set_max_buffered_bytes_default(args.defaults.max_buffered_bytes);
        // log_capacity/log_enabled: LogBuffer has NO live setters. Documented as
        // profile-only. rx_buffer_size: ring is fixed at open. Also profile-only.

        // Write-through learning for connection mode: persist the full
        // effective defaults through the bound profile (if any). Failure
        // keeps the result successful with `state="failed"`.
        let (profile, persistence) =
            learning::learn(store, &conn, ProfilePersistenceOperation::Learned).await;

        Ok(Json(ConfigureResult {
            mode: "connection".into(),
            defaults: args.defaults,
            created: None,
            profile,
            profile_persistence: Some(persistence),
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

    // Hold the learning lock across the effective-defaults snapshot and
    // the store upsert so a concurrent reconfigure/configure cannot yield
    // a mixed snapshot (e.g. new baud with old framing defaults).
    let _learning_guard = conn.learning_lock().lock().await;

    let info = conn
        .port_info()
        .ok_or_else(|| format!("No port identity available for {}", args.connection_id))?;

    // Snapshot the connection's full effective defaults from the shared
    // helper (never a handler-local session manager); it covers serial
    // parameters, framing/parser/protocol defaults, the stored RX buffer
    // size, read/subscribe defaults, reconnect policy, log config, and the
    // connection name.
    let defaults = conn.effective_defaults();

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
///
/// Rejects deletion while any same-process open connection binds the
/// profile (the error lists the connection IDs). Cross-process active
/// ownership cannot be known here; a later missing-profile CAS protects
/// those processes.
pub async fn delete_profile(
    connections: &Arc<ConnectionManager>,
    store: &Arc<crate::profile_store::ProfileStore>,
    args: DeleteProfileArgs,
) -> Result<Json<DeleteProfileResult>, String> {
    let bound: Vec<String> = connections
        .list_all()
        .await
        .into_iter()
        .filter(|(_, conn)| {
            conn.active_profile_binding()
                .map(|b| b.persistent && b.profile_name == args.profile_name)
                .unwrap_or(false)
        })
        .map(|(id, _)| id)
        .collect();
    if !bound.is_empty() {
        return Err(format!(
            "Cannot delete profile '{}': bound to open connection(s) {}",
            args.profile_name,
            bound.join(", ")
        ));
    }

    store.delete(&args.profile_name).await?;

    Ok(Json(DeleteProfileResult {
        profile_name: args.profile_name,
    }))
}

/// Roll a profile back to a prior retained revision.
///
/// Restores the target revision's selector/defaults as a NEW monotonic
/// revision (`current + 1`), preserving generated/usage metadata. Live
/// hardware is never touched: same-process connections bound to the
/// profile are marked stale+dirty (so neither learning nor close can
/// overwrite the rollback) and counted in the result. Wrong
/// `expected_revision` or an evicted target revision is a tool error that
/// leaves the file unchanged.
pub async fn rollback_profile(
    connections: &Arc<ConnectionManager>,
    store: &Arc<crate::profile_store::ProfileStore>,
    args: RollbackProfileArgs,
) -> Result<Json<RollbackProfileResult>, String> {
    let rolled_back = store
        .rollback(
            args.profile_name.clone(),
            args.expected_revision,
            args.revision,
        )
        .await?;

    let mut active_connections_unchanged = 0usize;
    for (_, conn) in connections.list_all().await {
        let is_bound = conn
            .active_profile_binding()
            .map(|b| b.persistent && b.profile_name == args.profile_name)
            .unwrap_or(false);
        if is_bound {
            let error = format!(
                "profile '{}' rolled back to revision {} by rollback_profile; \
                 connection stays on its live state until reopened",
                args.profile_name, args.revision
            );
            conn.update_active_profile_binding(|b| {
                b.stale = true;
                b.dirty = true;
                b.last_persistence_error = Some(error);
            });
            active_connections_unchanged += 1;
        }
    }

    let revision = rolled_back.metadata.revision;
    Ok(Json(RollbackProfileResult {
        profile_name: args.profile_name,
        restored_from_revision: args.revision,
        previous_revision: args.expected_revision,
        revision,
        selector: rolled_back.selector,
        defaults: rolled_back.defaults,
        metadata: rolled_back.metadata,
        active_connections_unchanged,
        persistence: crate::profiles::ProfilePersistenceResult {
            state: crate::profiles::ProfilePersistenceState::Persisted,
            operation: crate::profiles::ProfilePersistenceOperation::Rollback,
            profile_name: Some(rolled_back.name),
            revision: Some(revision),
            error: None,
        },
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
    capture_store: &Arc<crate::capture_store::CaptureStore>,
    args: ExportLogArgs,
) -> Result<Json<ExportLogResult>, String> {
    // Order matters: store enabled check and path validity run BEFORE any
    // connection or file work — a disabled store or bad filename must fail
    // without touching the connection or the capture root.
    if !capture_store.is_enabled() {
        return Err(crate::capture_store::CAPTURE_DISABLED_ERROR.to_string());
    }
    crate::capture_store::validate_capture_filename(&args.path)?;
    let conn = lookup_connection(connections, &args.connection_id).await?;

    // Serialize the bounded snapshot in a blocking context so a large log
    // never stalls a Tokio worker thread. The store then commits in its own
    // spawn_blocking under the process-local + advisory locks.
    let max_file_bytes = capture_store.max_file_bytes();
    let log = Arc::clone(conn.log());
    let snapshot = tokio::task::spawn_blocking(move || log.jsonl_snapshot(max_file_bytes))
        .await
        .map_err(|e| format!("capture snapshot task failed: {e}"))??;

    let write = capture_store
        .write_new(args.path.clone(), snapshot.bytes)
        .await?;

    Ok(Json(ExportLogResult {
        connection_id: args.connection_id,
        path: write.path.display().to_string(),
        events_written: snapshot.events,
        bytes_written: write.bytes_written,
        files_used: write.files_used,
        total_bytes_used: write.total_bytes_used,
        durability_warning: write.durability_warning,
    }))
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;
    use std::sync::Arc;
    use std::task::{Context, Poll};
    use std::time::Duration;

    use tokio::io::{AsyncRead, AsyncWrite};

    use crate::profile_store::ProfileStore;
    use crate::profiles::{
        IdentityConfidence, Profile, ProfileDefaults, ProfileMetadata, ProfileSelectionSource,
        ProfileSelector,
    };
    use crate::serial::{
        ConnectionConfig, ConnectionManager, DataBits, FlowControl, FlushTarget, Parity, PortInfo,
        PortTransport, SerialConnection, SerialIo, StopBits,
    };

    /// Minimal `SerialIo` for in-crate tool tests: no real hardware, all
    /// control/reconfigure operations are no-ops. I/O returns EOF/0 — the
    /// tests here never exchange bytes.
    struct FakeIo;

    impl AsyncRead for FakeIo {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            // EOF: no bytes ever become available (tests never exchange
            // bytes over this fake).
            Poll::Ready(Ok(()))
        }
    }

    impl AsyncWrite for FakeIo {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Poll::Ready(Ok(0))
        }
        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl SerialIo for FakeIo {
        fn clear_os_buffers(&self, _target: FlushTarget) -> std::io::Result<()> {
            Ok(())
        }
        fn set_dtr_rts(&mut self, _dtr: bool, _rts: bool) -> std::io::Result<()> {
            Ok(())
        }
        fn set_flow_control(&mut self, _flow_control: FlowControl) -> std::io::Result<()> {
            Ok(())
        }
        fn set_break_state(&self, _asserted: bool) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn fake_port_info() -> PortInfo {
        PortInfo {
            name: "/dev/fake".into(),
            display_name: "fake".into(),
            description: "Fake".into(),
            hardware_id: Some("USB VID:1234 PID:5678".into()),
            transport: PortTransport::Usb,
            vid: Some(0x1234),
            pid: Some(0x5678),
            serial_number: Some("SN-LOCK".into()),
            manufacturer: Some("Synthetic".into()),
            product: Some("Fake USB Serial".into()),
            interface: None,
        }
    }

    /// Build a manager with one connection that carries a persistent
    /// binding to profile `dev` (baud 115200) and an identity, so
    /// `save_profile` has everything it needs.
    async fn bound_connection(store: &Arc<ProfileStore>) -> (Arc<ConnectionManager>, String) {
        let profile = Profile {
            name: "dev".into(),
            selector: ProfileSelector::default(),
            defaults: ProfileDefaults {
                baud_rate: 115200,
                ..Default::default()
            },
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        store.upsert(profile, false).await.unwrap();

        let manager = Arc::new(ConnectionManager::new());
        let conn = SerialConnection::from_io_with_config(
            ConnectionConfig {
                port: "/dev/fake".into(),
                name: Some("fake-dev".into()),
                baud_rate: 115200,
                data_bits: DataBits::Eight,
                stop_bits: StopBits::One,
                parity: Parity::None,
                flow_control: FlowControl::None,
                port_info: Some(fake_port_info()),
                log_capacity: 1024,
                log_enabled: true,
                tx_framing: None,
                rx_framing: None,
                rx_parser: None,
                protocol: None,
                rx_buffer_size: crate::limits::DEFAULT_RX_BUFFER_SIZE,
                max_buffered_bytes: 32768,
            },
            Box::new(FakeIo),
        );
        conn.set_active_profile_binding(Some(crate::serial::ActiveProfileBinding {
            profile_name: "dev".into(),
            source: ProfileSelectionSource::Automatic,
            confidence: IdentityConfidence::High,
            persistent: true,
            generated: false,
            revision: Some(1),
            dirty: false,
            stale: false,
            candidates: Vec::new(),
            last_persistence_error: None,
        }));
        let id = manager.insert(conn).await.unwrap();
        (manager, id)
    }

    /// `save_profile` must hold the connection's learning lock across the
    /// effective-defaults snapshot and the store upsert. While the lock is
    /// held by a concurrent durable operation, a spawned save_profile must
    /// block; it completes only after the lock is released.
    #[tokio::test]
    async fn save_profile_holds_learning_lock_across_snapshot_and_upsert() {
        let store = Arc::new(ProfileStore::ephemeral());
        let (manager, connection_id) = bound_connection(&store).await;
        let conn = manager.get(&connection_id).await.unwrap();

        let guard = conn.learning_lock().lock().await;

        let manager_task = Arc::clone(&manager);
        let store_task = Arc::clone(&store);
        let task = tokio::spawn(async move {
            super::save_profile(
                &manager_task,
                &store_task,
                crate::tools::types::SaveProfileArgs {
                    connection_id,
                    profile_name: "snap".into(),
                    overwrite: false,
                },
            )
            .await
        });

        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            !task.is_finished(),
            "save_profile must block while the learning lock is held"
        );

        drop(guard);
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("save_profile completes after lock release")
            .expect("task did not panic")
            .expect("save_profile succeeds");
        assert_eq!(result.0.name, "snap");

        // The snapshot is consistent with the connection's live state.
        let saved = store.get("snap").await.unwrap();
        assert_eq!(saved.defaults.baud_rate, conn.baud_rate());
        assert_eq!(saved.selector.serial_number.as_deref(), Some("SN-LOCK"));
        assert!(!saved.metadata.generated, "save_profile creates user-owned");
    }
}
