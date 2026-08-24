//! MCP server tool surface for serial communication.
//!
//! Each `#[tool]` method below corresponds to one MCP tool. Tools return
//! structured JSON via [`Json<T>`] so MCP clients can index fields directly
//! instead of parsing free-form text.

use std::borrow::Cow;
use std::sync::Arc;

use base64::Engine as _;
use rmcp::{
    handler::server::wrapper::Parameters, model::*, prompt, prompt_router, service::RequestContext,
    service::SubscriptionContext, tool, tool_handler, tool_router, ErrorData as McpError, Json,
    RoleServer, ServerHandler,
};
use tokio::sync::broadcast;

use tracing::info;

use crate::buffer_budget::BufferBudget;
use crate::capture_store::CaptureStore;
use crate::mcp_protocol::{cache_fields_for, ProtocolLifecycle, ProtocolPolicy};
use crate::resource_events::{is_subscribable_uri, ResourceEvent, ResourceEventHub};
use crate::rx_session::RxSessionManager;
use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, PortProvider};
use crate::tx_session::TxSessionManager;

use crate::prompts::types::*;
use crate::prompts::{diagnose, interactive};
use crate::tools::types::*;
use crate::tools::{control_ops, io_ops, port_ops, utility_ops};

/// Paginate a vector using a base64-encoded UTF-8 offset cursor.
///
/// Returns one page and an optional cursor when more items remain.
fn paginate<T: Clone>(
    all: &[T],
    cursor: Option<String>,
    page_size: usize,
) -> (Vec<T>, Option<String>) {
    let offset = cursor
        .as_deref()
        .and_then(|c| base64::engine::general_purpose::STANDARD.decode(c).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(0)
        .min(all.len());

    let end = offset.saturating_add(page_size).min(all.len());
    let items = all[offset..end].to_vec();

    let next_cursor = if end < all.len() {
        let next = base64::engine::general_purpose::STANDARD.encode(end.to_string().as_bytes());
        Some(next)
    } else {
        None
    };

    (items, next_cursor)
}

/// Apply SEP-2549 cache fields to `resources/read` only for a policy row with
/// `ImmediatePrivate` cache. Currently that is `2026-07-28`. Other peers,
/// including unknown or future versions, get the bare result. `ttlMs: 0` and
/// `cacheScope: "private"` mark the response immediately stale and
/// client-private.
fn read_result_with_cache_fields(
    result: ReadResourceResult,
    protocol_version: Option<ProtocolVersion>,
) -> ReadResourceResponse {
    if crate::mcp_protocol::cache_fields_for(protocol_version) {
        result.with_ttl_ms(0).with_cache_scope(CacheScope::Private)
    } else {
        result
    }
    .into()
}

// Handler

#[derive(Clone)]
pub struct SerialHandler {
    pub(crate) connections: Arc<ConnectionManager>,
    security: SecurityManager,
    /// Process-wide RX session registry (ring + pump + shared cursor). One
    /// per server process, shared by every handler instance: modern HTTP is
    /// stateless (a fresh `SerialHandler` serves each request), so a
    /// handler-local registry would split the ring across requests and lose
    /// reads. The builder default constructs one for standalone handlers.
    pub(crate) rx_sessions: Arc<RxSessionManager>,
    tx_sessions: Arc<TxSessionManager>,
    budget: Arc<dyn BufferBudget>,
    profile_store: Arc<crate::profile_store::ProfileStore>,
    /// Process-wide capture store. Disabled unless production
    /// `main.rs` configured `--capture-dir`; shared by every stdio/HTTP
    /// session handler.
    capture_store: Arc<CaptureStore>,
    /// Process-wide injectable port enumeration used consistently by tools,
    /// resources, and automatic profile-session identity capture.
    port_provider: Arc<dyn PortProvider>,
    /// Process-wide event hub for modern `subscriptions/listen`. One hub per
    /// server process is shared by every stdio/HTTP handler and the port
    /// watcher. Modern HTTP is stateless, so a handler-local channel would
    /// split publishers and listeners.
    resource_events: Arc<ResourceEventHub>,
}

/// Injectable configuration for [`SerialHandler`].
///
/// Every field has a sensible default produced by [`SerialHandlerOptions::default`].
/// Use [`SerialHandler::builder`] to override individual fields, then `.build()`.
#[derive(Clone)]
pub struct SerialHandlerOptions {
    pub connections: Arc<ConnectionManager>,
    pub security: SecurityManager,
    pub budget: Arc<dyn BufferBudget>,
    /// Process-wide profile store. Defaults to an ephemeral store for
    /// library/test construction; production `main.rs` injects a store
    /// opened at the resolved `--profiles-path`.
    pub profile_store: Arc<crate::profile_store::ProfileStore>,
    /// Process-wide capture store. Defaults to disabled for
    /// library/test construction; production `main.rs` injects a store
    /// opened at the resolved `--capture-dir`.
    pub capture_store: Arc<CaptureStore>,
    /// Process-wide port enumeration. Defaults to the system provider;
    /// tests inject a static provider.
    pub port_provider: Arc<dyn PortProvider>,
    /// Process-wide resource event hub for modern `subscriptions/listen`.
    /// Defaults to a fresh hub for library/test construction; production
    /// `main.rs` creates exactly one hub per server process.
    pub resource_events: Arc<ResourceEventHub>,
    /// Process-wide RX session registry (ring + pump + shared cursor).
    /// `None` (default) lets the builder construct one for standalone
    /// handlers; production `main.rs` and the test harness create exactly
    /// one per server process and inject it so every stateless HTTP handler
    /// instance shares the same ring/cursor state.
    pub rx_sessions: Option<Arc<RxSessionManager>>,
}

impl Default for SerialHandlerOptions {
    fn default() -> Self {
        use crate::limits::{DEFAULT_MAX_PROGRAM_BUFFERED_BYTES, DEFAULT_MAX_TOOL_BUFFERED_BYTES};
        Self {
            connections: Arc::new(ConnectionManager::new()),
            security: SecurityManager::from_patterns::<[&str; 0]>([]),
            budget: Arc::new(crate::buffer_budget::AtomicBudget::new(
                DEFAULT_MAX_PROGRAM_BUFFERED_BYTES,
                DEFAULT_MAX_TOOL_BUFFERED_BYTES,
            )),
            profile_store: Arc::new(crate::profile_store::ProfileStore::ephemeral()),
            capture_store: Arc::new(CaptureStore::disabled()),
            port_provider: Arc::new(crate::serial::SystemPortProvider),
            resource_events: Arc::new(ResourceEventHub::default()),
            rx_sessions: None,
        }
    }
}

/// Builder for [`SerialHandler`]. Start with [`SerialHandler::builder`].
#[derive(Default)]
pub struct SerialHandlerBuilder {
    options: SerialHandlerOptions,
}

impl SerialHandlerBuilder {
    pub fn connections(mut self, connections: Arc<ConnectionManager>) -> Self {
        self.options.connections = connections;
        self
    }
    pub fn security(mut self, security: SecurityManager) -> Self {
        self.options.security = security;
        self
    }
    pub fn budget(mut self, budget: Arc<dyn BufferBudget>) -> Self {
        self.options.budget = budget;
        self
    }
    pub fn profile_store(mut self, profile_store: Arc<crate::profile_store::ProfileStore>) -> Self {
        self.options.profile_store = profile_store;
        self
    }
    pub fn capture_store(mut self, capture_store: Arc<CaptureStore>) -> Self {
        self.options.capture_store = capture_store;
        self
    }
    pub fn port_provider(mut self, port_provider: Arc<dyn PortProvider>) -> Self {
        self.options.port_provider = port_provider;
        self
    }
    pub fn resource_events(mut self, resource_events: Arc<ResourceEventHub>) -> Self {
        self.options.resource_events = resource_events;
        self
    }
    /// Inject the process-wide RX session registry (ring + pump + shared
    /// cursor). One per server process; clone the same `Arc` into every
    /// HTTP handler factory so stateless requests share ring/cursor state.
    pub fn rx_sessions(mut self, rx_sessions: Arc<RxSessionManager>) -> Self {
        self.options.rx_sessions = Some(rx_sessions);
        self
    }

    /// Build a [`SerialHandler`].
    ///
    /// Starts one reconnect supervisor. If no
    /// [`SerialHandlerBuilder::rx_sessions`] was injected, constructs a manager
    /// for the standalone handler.
    pub fn build(self) -> SerialHandler {
        let SerialHandlerOptions {
            connections,
            security,
            budget,
            profile_store,
            capture_store,
            port_provider,
            resource_events,
            rx_sessions,
        } = self.options;
        let rx_sessions = match rx_sessions {
            Some(manager) => manager,
            None => Arc::new(RxSessionManager::new(
                Arc::clone(&budget),
                Arc::clone(&resource_events),
            )),
        };
        let handler = SerialHandler {
            connections,
            security,
            rx_sessions,
            tx_sessions: Arc::new(TxSessionManager::new()),
            budget,
            profile_store,
            capture_store,
            port_provider,
            resource_events,
        };
        handler.spawn_reconnect_supervisor();
        handler
    }
}

#[tool_router]
impl SerialHandler {
    /// Entry point for the builder.
    pub fn builder() -> SerialHandlerBuilder {
        SerialHandlerBuilder::default()
    }

    /// Create a handler with default connections, an empty allowlist, the
    /// default budget, profiles from the default path, and a reconnect
    /// supervisor.
    ///
    /// Falls back to an ephemeral store with a warning when the default path
    /// cannot be resolved or contains invalid data. Production `main.rs`
    /// resolves `--profiles-path` and fails startup on invalid persistent data.
    pub fn new() -> Self {
        let store = crate::profiles::default_profiles_path()
            .map_err(|e| format!("Cannot resolve default profiles path: {e}"))
            .and_then(crate::profile_store::ProfileStore::open);
        let store = match store {
            Ok(store) => Arc::new(store),
            Err(e) => {
                tracing::warn!("profiles store unavailable, using ephemeral store: {e}");
                Arc::new(crate::profile_store::ProfileStore::ephemeral())
            }
        };
        Self::builder().profile_store(store).build()
    }

    /// Start the background reconnect supervisor task.
    /// Requires a Tokio runtime to be active (panics otherwise).
    pub fn spawn_reconnect_supervisor(&self) {
        let connections = Arc::clone(&self.connections);
        let rx_sessions = Arc::clone(&self.rx_sessions);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                let all = connections.list_all().await;
                for (id, conn) in all {
                    let state = conn.state();
                    if state == crate::serial::ConnectionState::Disconnected {
                        let policy = conn.reconnect_policy.lock().expect("poisoned").clone();
                        if policy.enabled {
                            connections
                                .start_reconnect(&id, Arc::clone(&rx_sessions))
                                .await;
                        }
                    }
                }
            }
        });
    }

    #[tool(
        description = "List serial ports with a profile-match preview. `profile_matches` has the same length and order as `ports`. Outcomes: `selected` means bare `open(port=...)` reuses the profile named by `selected_profile`; `ambiguous` means equal-ranked high-confidence profiles, so bare open stays transient and callers choose with `open_profile`; `duplicate` means another live port shares the identity and prevents automatic selection; `ineligible` means weak identity has explicitly matching candidates, so use `open_profile`; `none` means a high-confidence port starts a generated session while weaker identity starts a transient session. Call this first, then open.",
        title = "List Serial Ports",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_ports(&self) -> Result<Json<ListPortsResult>, String> {
        port_ops::list_ports(&self.port_provider, &self.profile_store).await
    }

    #[tool(
        description = "List all open serial connections held by this server.",
        title = "List Open Connections",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_connections(&self) -> Result<Json<ListConnectionsResult>, String> {
        port_ops::list_connections(&self.connections).await
    }

    #[tool(
        description = "Open a serial port. The common call is `open(port=...)`; baud defaults to 115200/8-N-1. For a known high-confidence device, the server reuses the most recently used profile (see `list_ports` `profile_matches`). It creates a durable generated profile only for a new high-confidence device. Weak, ambiguous, or duplicate identity stays transient and receives no automatic durable profile. The result's `profile` shows the choice. Explicit fields such as baud, framing, parser, or protocol override the selected profile and are learned back into it. Use `open_profile` for explicit named selection.",
        title = "Open Serial Port",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
    ) -> Result<Json<OpenResult>, String> {
        let result = port_ops::open(
            &self.connections,
            &self.rx_sessions,
            &self.security,
            &self.profile_store,
            &self.port_provider,
            args,
        )
        .await?;
        // Publish only after the successful open: connections list + detail.
        self.resource_events.publish_connections_changed();
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Close an open serial port connection.",
        title = "Close Serial Port",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn close(
        &self,
        Parameters(args): Parameters<CloseArgs>,
    ) -> Result<Json<CloseResult>, String> {
        let connection_id = args.connection_id.clone();
        // Cancel any running reconnect task so it doesn't try to reopen.
        self.connections.cancel_reconnect(&connection_id).await;
        let result = port_ops::close(&self.connections, &self.profile_store, args).await?;
        // Publish only after the successful close: connections list + the
        // (now-closed) detail URI as a final hint.
        self.resource_events.publish_connections_changed();
        self.resource_events
            .publish_connection_detail_changed(&connection_id);
        // Shut down RX session (pump + consumers) for this connection.
        self.rx_sessions.remove(&connection_id).await;
        // Shut down TX session (worker) for this connection.
        self.tx_sessions.remove(&connection_id).await;
        Ok(result)
    }

    #[tool(
        description = "Send data to a serial port (send-only); it sends without waiting for a response. For request/response traffic, use `transact`; it writes and awaits the response. `tx_framing` supports line terminator, delimiter, length prefix, SLIP, COBS, and start/end markers. `protocol` supports `at_command`, `slip`, `json_lines`, `cobs`, `ndjson`, `nmea0183`, and `modbus_ascii`. These overrides apply when connection defaults do not fit. The `nmea0183` preset automatically appends the `*XX` XOR checksum. With `tx_framing`, `decoded_bytes` is the payload length before framing and `bytes_written` is the total framed length.",
        title = "Write Serial Data",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn write(
        &self,
        Parameters(args): Parameters<WriteArgs>,
    ) -> Result<Json<WriteResult>, String> {
        let result = io_ops::write(&self.connections, &self.tx_sessions, args).await?;
        // A successful write changes connection state (counters); hint the
        // detail URI. RX-side hints come from the pump.
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Write data and await the response in one call. This is the request/response primitive for AT/Modbus/GRBL traffic; prefer it over separate `write` and `read`. The read half starts at the live edge (`from: {\"type\":\"now\"}`), so it awaits only post-write bytes. Use `match` to stop on a prompt and `no_new_rx_timeout_ms` or `timeout_ms` for bounded waits. `protocol` fills framing defaults for both directions; explicit `tx_framing`, `rx_framing`, and `rx_parser` override it per direction.",
        title = "Transact (Write + Read)",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn transact(
        &self,
        meta: RequestMetaObject,
        ct: tokio_util::sync::CancellationToken,
        peer: rmcp::Peer<RoleServer>,
        Parameters(args): Parameters<TransactArgs>,
    ) -> Result<Json<TransactResult>, String> {
        let result = io_ops::transact(
            &self.connections,
            &self.tx_sessions,
            &self.rx_sessions,
            &self.budget,
            meta,
            ct,
            peer,
            args,
        )
        .await?;
        // The write half changed connection state; hint the detail URI.
        // RX-side hints come from the pump.
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Read buffered serial data or wait for unsolicited data or patterns. Returns unread bytes from the connection cursor immediately (`cat` semantics). `from` selects the start: `from: {\"type\":\"cursor\"}` is the default, `from: {\"type\":\"now\"}` starts at the live edge, `from: {\"type\":\"buffer_start\"}` replays retained history, and `from: {\"type\":\"offset\",\"offset\":N}` seeks an absolute offset. `match` checks buffered history before waiting for new data. `rx_framing`, `rx_parser`, and `protocol` presets apply when connection defaults do not fit. With `validate:true`, checksum-mismatched frames are dropped and counted. Set `no_new_rx_timeout_ms` to stop on silence. Results include `from_offset`, `next_offset`, `bytes_lost`, `buffered_remaining`, `start_offset`, and `end_offset`.",
        title = "Read Serial Data",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn read(
        &self,
        meta: RequestMetaObject,
        ct: tokio_util::sync::CancellationToken,
        peer: rmcp::Peer<RoleServer>,
        Parameters(args): Parameters<ReadArgs>,
    ) -> Result<Json<ReadResult>, String> {
        io_ops::read(
            &self.connections,
            &self.rx_sessions,
            &self.budget,
            meta,
            ct,
            peer,
            args,
        )
        .await
    }

    #[tool(
        description = "Flush serial buffers. `target=input` clears the OS read buffer and discards unread retained RX data. To skip buffered data without destroying it, use `read` with `from: {\"type\":\"now\"}`. `target=output` clears the OS write queue. `target=both` flushes output first, then performs the input discard for the OS read buffer and retained RX backlog.",
        title = "Flush Serial Buffers",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn flush(
        &self,
        Parameters(args): Parameters<FlushArgs>,
    ) -> Result<Json<FlushResult>, String> {
        let result = io_ops::flush(
            &self.connections,
            &self.rx_sessions,
            &self.tx_sessions,
            args,
        )
        .await?;
        // Input-side flushes change the ring; hint detail + raw.
        match result.0.target {
            crate::serial::FlushTarget::Input | crate::serial::FlushTarget::Both => {
                self.resource_events
                    .publish_connection_detail_changed(&result.0.connection_id);
                self.resource_events
                    .publish_connection_raw_changed(&result.0.connection_id);
            }
            crate::serial::FlushTarget::Output => {}
        }
        Ok(result)
    }

    #[tool(
        description = "Set the DTR and RTS modem-control lines. Common patterns are pulsing DTR low for Arduino auto-reset and holding both low to enter an ESP32 bootloader.",
        title = "Set DTR/RTS",
        annotations(
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_dtr_rts(
        &self,
        Parameters(args): Parameters<SetDtrRtsArgs>,
    ) -> Result<Json<SetDtrRtsResult>, String> {
        let result = control_ops::set_dtr_rts(&self.connections, args).await?;
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Change hardware or software flow control on an open connection. Use `flow_control='none'` to ignore RTS/CTS for this session.",
        title = "Set Flow Control",
        annotations(
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn set_flow_control(
        &self,
        Parameters(args): Parameters<SetFlowControlArgs>,
    ) -> Result<Json<SetFlowControlResult>, String> {
        let result =
            control_ops::set_flow_control(&self.connections, &self.profile_store, args).await?;
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Assert BREAK on the TX line for `duration_ms` milliseconds (default 250ms), then release it. Some legacy serial protocols use this as an attention signal.",
        title = "Send BREAK",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn send_break(
        &self,
        meta: RequestMetaObject,
        ct: tokio_util::sync::CancellationToken,
        peer: rmcp::Peer<RoleServer>,
        Parameters(args): Parameters<SendBreakArgs>,
    ) -> Result<Json<SendBreakResult>, String> {
        let result = control_ops::send_break(&self.connections, meta, ct, peer, args).await?;
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Inspect a connection's live configuration, RX ring state (size, offsets, cursor, unread bytes, and wrap loss), profile binding, and reconnect state. Use for diagnostics, not routine reads; read results already include offsets.",
        title = "Get Connection Status",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_status(
        &self,
        Parameters(args): Parameters<GetStatusArgs>,
    ) -> Result<Json<GetStatusResult>, String> {
        port_ops::get_status(&self.connections, &self.rx_sessions, args).await
    }

    #[tool(
        description = "Change baud rate, data bits, stop bits, parity, or flow control on a live connection without reopening. Omitted parameters stay unchanged. Changes are learned into the bound profile; see result `profile` and `profile_persistence`.",
        title = "Reconfigure Serial Port",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn reconfigure(
        &self,
        Parameters(args): Parameters<ReconfigureArgs>,
    ) -> Result<Json<ReconfigureResult>, String> {
        let result = port_ops::reconfigure(&self.connections, &self.profile_store, args).await?;
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "List saved device profiles with selectors, defaults, metadata, and revision history. Cross-check `list_ports` `profile_matches` to see which profiles match live devices before opening.",
        title = "List Profiles",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_profiles(&self) -> Result<Json<ListProfilesResult>, String> {
        let profiles = self.profile_store.list().await;
        port_ops::list_profiles(&profiles)
    }

    #[tool(
        description = "Open a device by named profile. Use when `list_ports` reports `ambiguous`, `ineligible`, or `duplicate`, when identity is weak, or to override automatic last-used selection. The selector must match exactly one live port, and the profile's defaults apply.",
        title = "Open by Profile",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn open_profile(
        &self,
        Parameters(args): Parameters<OpenProfileArgs>,
    ) -> Result<Json<OpenResult>, String> {
        // Clone the profile out of the store first; do not hold a store
        // lock while opening serial hardware.
        let profile = self.profile_store.get(&args.profile).await;
        let result = port_ops::open_profile(
            &self.connections,
            &self.rx_sessions,
            &self.security,
            &self.profile_store,
            &self.port_provider,
            profile,
            args,
        )
        .await?;
        // Publish only after the successful open: connections list + detail.
        self.resource_events.publish_connections_changed();
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Save a profile by snapshotting an open connection's port identity and current serial configuration. Use after opening a device to create a reusable profile selected by name in later sessions.",
        title = "Save Profile",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn save_profile(
        &self,
        Parameters(args): Parameters<SaveProfileArgs>,
    ) -> Result<Json<SaveProfileResult>, String> {
        port_ops::save_profile(&self.connections, &self.profile_store, args).await
    }

    #[tool(
        description = "Delete a named profile from the profiles configuration file.",
        title = "Delete Profile",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn delete_profile(
        &self,
        Parameters(args): Parameters<DeleteProfileArgs>,
    ) -> Result<Json<DeleteProfileResult>, String> {
        port_ops::delete_profile(&self.connections, &self.profile_store, args).await
    }

    #[tool(
        description = "Restore a retained prior profile revision after a bad learned setting. `list_profiles` shows `revisions`, the newest five snapshots. Pass the current profile revision as `expected_revision` to guard against concurrent modification. The restore creates a new monotonic revision. Bound connections keep live state and become stale until reopened. A wrong `expected_revision` or evicted target revision is a tool error and leaves the file unchanged.",
        title = "Roll Back Profile",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn rollback_profile(
        &self,
        Parameters(args): Parameters<RollbackProfileArgs>,
    ) -> Result<Json<RollbackProfileResult>, String> {
        port_ops::rollback_profile(&self.connections, &self.profile_store, args).await
    }

    #[tool(
        description = "Set defaults in two modes. Profile mode: `configure(profile=..., defaults=...)` writes a named profile to profiles TOML for future `open_profile` calls; `overwrite=true` replaces an existing profile. Connection mode: `configure(connection_id=..., defaults=...)` changes live framing, parser, protocol, reconnect_policy, and max_buffered_bytes defaults. It does not persist to disk; reopen to apply rx_buffer_size, serial-line parameters, log_capacity, or log_enabled. The `defaults` object is the full desired state; omitted fields use their defaults.",
        title = "Configure Defaults",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn configure(
        &self,
        Parameters(args): Parameters<ConfigureArgs>,
    ) -> Result<Json<ConfigureResult>, String> {
        // Connection mode mutates live defaults and changes the detail URI.
        // Profile mode has no live connection URI, so it emits no event.
        let connection_id = args.connection_id.clone();
        let result = port_ops::configure(&self.connections, &self.profile_store, args).await?;
        if result.0.mode == "connection" {
            if let Some(connection_id) = connection_id.as_deref() {
                self.resource_events
                    .publish_connection_detail_changed(connection_id);
            }
        }
        Ok(result)
    }

    #[tool(
        description = "Retrieve an open connection's event log. Returns timestamped JSONL entries for RX, TX, matches, errors, and lifecycle events. Use `since_ms` to filter by time and `limit` to cap entries.",
        title = "Get Log",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn get_log(
        &self,
        Parameters(args): Parameters<GetLogArgs>,
    ) -> Result<Json<GetLogResult>, String> {
        port_ops::get_log(&self.connections, args).await
    }

    #[tool(
        description = "Clear a connection's event log buffer.",
        title = "Clear Log",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn clear_log(
        &self,
        Parameters(args): Parameters<ClearLogArgs>,
    ) -> Result<Json<ClearLogResult>, String> {
        let result = port_ops::clear_log(&self.connections, args).await?;
        self.resource_events
            .publish_connection_log_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Export a connection's event log to JSONL in the configured capture directory. Persistent capture is disabled unless the server started with `--capture-dir <absolute-directory>`. `path` is an ASCII 1-120 character `.jsonl` filename relative to that root, not a path: filename only, no separators, subdirectories, or traversal, and ending in `.jsonl`. The server never overwrites an existing file, rejects symlink targets, enforces per-file, total-byte, and file-count quotas from a fresh scan under an advisory cross-process lock, and atomically commits the complete bounded snapshot. Pre-commit failure creates no file and changes no existing capture. Success returns exact event and byte counts, the canonical absolute final path, and post-commit quota usage. On Unix, a post-commit root-directory sync failure is reported in `durability_warning`; the file is committed, counted, and never deleted.",
        title = "Export Log",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn export_log(
        &self,
        Parameters(args): Parameters<ExportLogArgs>,
    ) -> Result<Json<ExportLogResult>, String> {
        port_ops::export_log(&self.connections, &self.capture_store, args).await
    }

    #[tool(
        description = "Reconnect a disconnected serial connection. Rebuilds the port stream from its original configuration while preserving connection_id, counters, and log buffer. Succeeds immediately when already connected.",
        title = "Reconnect",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn reconnect(
        &self,
        Parameters(args): Parameters<ReconnectArgs>,
    ) -> Result<Json<ReconnectResult>, String> {
        let result = port_ops::reconnect(&self.connections, args).await?;
        // Reconnect/state change: hint the detail URI.
        self.resource_events
            .publish_connection_detail_changed(&result.0.connection_id);
        Ok(result)
    }

    #[tool(
        description = "Atomic boot/reset capture: purges unread OS input, marks the RX live edge under the pump gate so pre-mark bytes cannot leak in, optionally pulses DTR/RTS with release guaranteed on completion, cancellation, or failure, then captures only post-mark bytes through the existing match/framing/parser/timeout/silence pipeline. Uses a private read cursor; the shared `read` cursor and ring history stay untouched. `reset=null` arms capture for externally reset devices without touching lines. The result is bounded in memory by the connection's `max_buffered_bytes`; no file output. `read.from_offset` equals `mark_offset` unless the ring wraps, when `bytes_lost` reports loss. Destructive: configured reset lines may reboot hardware.",
        title = "Capture Boot Output",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn capture_boot(
        &self,
        meta: RequestMetaObject,
        ct: tokio_util::sync::CancellationToken,
        peer: rmcp::Peer<RoleServer>,
        Parameters(args): Parameters<CaptureBootArgs>,
    ) -> Result<Json<CaptureBootResult>, String> {
        control_ops::capture_boot(
            &self.connections,
            &self.rx_sessions,
            &self.budget,
            meta,
            ct,
            peer,
            args,
        )
        .await
    }

    #[tool(
        description = "Compute a checksum over caller-supplied bytes. Algorithms are xor (NMEA-0183 `*XX`) and lrc (Modbus ASCII). Input is decoded from `utf8`, `hex`, or `base64` before checksumming. Returns a hex checksum string and raw integer. Use this when hand-crafting binary frames without a protocol preset.",
        title = "Compute Checksum",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn compute_checksum(
        &self,
        Parameters(args): Parameters<ComputeChecksumArgs>,
    ) -> Result<Json<ComputeChecksumResult>, String> {
        utility_ops::compute_checksum(args).await
    }
}

// Tool catalog. Tool logic lives in src/tools/*.rs.

/// The public MCP tool catalog served by `tools/list`.
///
/// This list uses the generated attributes from every `#[tool]` method. Schema
/// tests (`src/tools/mod.rs`) and xtask `agent-eval` therefore measure the same
/// attributes as the MCP router. Keep it in sync with the methods above;
/// `tool_catalog_has_exactly_twenty_five_tools` guards the count.
pub fn tool_catalog() -> Vec<rmcp::model::Tool> {
    vec![
        SerialHandler::list_ports_tool_attr(),
        SerialHandler::list_connections_tool_attr(),
        SerialHandler::open_tool_attr(),
        SerialHandler::close_tool_attr(),
        SerialHandler::write_tool_attr(),
        SerialHandler::transact_tool_attr(),
        SerialHandler::read_tool_attr(),
        SerialHandler::capture_boot_tool_attr(),
        SerialHandler::flush_tool_attr(),
        SerialHandler::set_dtr_rts_tool_attr(),
        SerialHandler::set_flow_control_tool_attr(),
        SerialHandler::send_break_tool_attr(),
        SerialHandler::get_status_tool_attr(),
        SerialHandler::reconfigure_tool_attr(),
        SerialHandler::list_profiles_tool_attr(),
        SerialHandler::open_profile_tool_attr(),
        SerialHandler::save_profile_tool_attr(),
        SerialHandler::delete_profile_tool_attr(),
        SerialHandler::configure_tool_attr(),
        SerialHandler::rollback_profile_tool_attr(),
        SerialHandler::get_log_tool_attr(),
        SerialHandler::clear_log_tool_attr(),
        SerialHandler::export_log_tool_attr(),
        SerialHandler::reconnect_tool_attr(),
        SerialHandler::compute_checksum_tool_attr(),
    ]
}

// ServerHandler implementation

/// Build capabilities for one policy row. Every row enables
/// tools/resources/prompts/completions. Resource subscriptions follow the row;
/// logging, list-change flags, tasks, and unrelated capabilities stay disabled.
fn capabilities_for(policy: &ProtocolPolicy) -> ServerCapabilities {
    let mut builder = ServerCapabilities::builder()
        .enable_tools()
        .enable_resources()
        .enable_prompts()
        .enable_completions();
    if policy.resource_subscriptions {
        builder = builder.enable_resources_subscribe();
    }
    builder.build()
}

/// Build server info from one policy row. Identity, instructions, capabilities,
/// protocol version, and version/feature decisions all come from the policy
/// table.
fn server_info_for(policy: &ProtocolPolicy) -> ServerInfo {
    ServerInfo::new(capabilities_for(policy))
        .with_server_info(Implementation::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
        .with_protocol_version(policy.version.clone())
        .with_instructions(
            "Serial port MCP server. Workflow:\n\
             1. Call `list_ports` and inspect `profile_matches`, which parallels \
             `ports` and previews which profile bare `open(port=...)` would reuse.\n\
             2. Normally call bare `open` with only the port. Baud defaults to \
             115200/8-N-1. The server reuses the most recently used high-confidence \
             profile for a known device. It creates a durable generated profile only \
             for a new high-confidence device. Weak, ambiguous, or duplicate identity \
             uses a transient session. The result reports the choice in `profile`.\n\
             3. Use `transact` for command/response, `read` for buffered or unsolicited \
             data with `match` when needed, and `write` for send-only.\n\
             4. Use `capture_boot` for boot/reset capture such as an Arduino reset, \
             power-cycle banner, or boot prompt. It marks the live edge atomically, \
             can pulse DTR/RTS, and captures only post-mark bytes with a private \
             cursor in memory. `reset=null` arms capture for externally reset devices.\n\
             5. After durable changes, inspect `profile` and `profile_persistence`, or \
             use `get_status` to inspect the live binding.\n\
             6. Use `open_profile` for explicit choice or weak identity. Use \
             `rollback_profile` to restore a retained configuration after a bad \
             learned change.\n\
             7. Use framing/parser, cursor replay, reconnect, line-control, and log \
             tools only when needed. `export_log` persists JSONL only when the server \
             started with `--capture-dir`; it accepts a portable filename, not a path, \
             never overwrites, and enforces quotas.\n\
             8. Modern clients may call `subscriptions/listen` with resource URIs \
             (`serial://ports`, `serial://connections`, or \
             `serial://connections/{id}[/raw|/log]`) for change hints. Notifications \
             carry no data; `read` remains the primary lossless data path.\n\
             Resources: serial://ports (same preview as list_ports), serial://connections, \
             serial://connections/{id}. Prompts: diagnose_port, interactive_terminal."
                .to_string(),
        )
}

impl Default for SerialHandler {
    fn default() -> Self {
        Self::new()
    }
}

// Prompt templates

// Completion helper

impl SerialHandler {
    /// Generate completion suggestions for tool and resource arguments.
    async fn get_completions(&self, r#ref: &Reference, argument: &ArgumentInfo) -> Vec<String> {
        match r#ref {
            Reference::Resource(resource_ref) => {
                if resource_ref.uri == URI_PORTS && argument.name == "port" {
                    match self.port_provider.list_available() {
                        Ok(ports) => ports.into_iter().map(|p| p.name).collect(),
                        Err(_) => vec![],
                    }
                } else {
                    vec![]
                }
            }
            Reference::Prompt(prompt_ref) => {
                if prompt_ref.name == "diagnose_port" && argument.name == "port" {
                    match self.port_provider.list_available() {
                        Ok(ports) => ports.into_iter().map(|p| p.name).collect(),
                        Err(_) => vec![],
                    }
                } else {
                    vec![]
                }
            }
            _ => vec![],
        }
    }
}

#[prompt_router]
impl SerialHandler {
    /// Plan diagnosis for an unknown serial port by trying common baud rates,
    /// sending a benign probe, and narrowing the configuration.
    #[prompt(
        name = "diagnose_port",
        description = "Identify an unknown serial device on a given port"
    )]
    async fn diagnose_port_prompt(
        &self,
        Parameters(args): Parameters<DiagnosePortArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        Ok(diagnose::build_diagnose_prompt(args))
    }

    /// Guide an interactive serial REPL over an open connection using `write`,
    /// `read(match=...)`, or `transact`.
    #[prompt(
        name = "interactive_terminal",
        description = "Run a REPL-style command/response session over an open serial connection"
    )]
    async fn interactive_terminal_prompt(
        &self,
        Parameters(args): Parameters<InteractiveTerminalArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        Ok(interactive::build_interactive_prompt(args))
    }
}

// Do not add `#[prompt_handler]` here. rmcp-macros 3.1.0 unconditionally
// replaces `list_prompts` and `get_prompt` in an annotated block and leaves
// SEP-2549 cache fields unset. Unlike `#[tool_handler]`, it does not honor
// existing methods. The two methods below therefore use the same
// `Self::prompt_router()` explicitly.
#[tool_handler]
impl ServerHandler for SerialHandler {
    fn get_info(&self) -> ServerInfo {
        // rmcp intersects `subscriptions/listen` filters with
        // `get_info().capabilities`, so this view advertises the preferred
        // `2026-07-28` resource-subscription capability. `initialize()` still
        // returns the legacy row's view for `2025-11-25` clients.
        server_info_for(crate::mcp_protocol::preferred_policy())
    }

    async fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        // Exact policy lookup admits only the `InitializeSession` lifecycle.
        // Modern `2026-07-28` clients negotiate through `server/discover`.
        // Modern `initialize` and unknown or unsupported versions are rejected
        // before peer bookkeeping, so no session state is established. rmcp
        // maps the handler's METHOD_NOT_FOUND through transport routing.
        let Some(policy) = crate::mcp_protocol::policy_for(&request.protocol_version) else {
            return Err(McpError::method_not_found::<InitializeResultMethod>());
        };
        if policy.lifecycle != ProtocolLifecycle::InitializeSession {
            return Err(McpError::method_not_found::<InitializeResultMethod>());
        }
        // Preserve rmcp peer bookkeeping. The session worker's peer_info()
        // must contain this client's initialize parameters so later requests
        // use the negotiated legacy protocol version.
        context.peer.set_peer_info(request.clone());
        info!("Serial MCP server initialized");
        Ok(server_info_for(policy))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(crate::mcp_protocol::supported_protocol_versions())
    }

    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, McpError> {
        Ok(DiscoverResult::from_server_info(
            crate::mcp_protocol::supported_protocol_versions(),
            server_info_for(crate::mcp_protocol::preferred_policy()),
        ))
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        // Keep valid, deduplicated resource URIs in first-request order.
        // Strip all three list-change flags because serial-mcp never emits
        // them. Reject templates, malformed or empty ids, and unknown URIs via
        // `is_subscribable_uri`. An empty resource list becomes `None`; return
        // an available handler so rmcp can acknowledge the accepted filter.
        let mut seen = std::collections::HashSet::new();
        let mut accepted_uris = Vec::new();
        if let Some(requested_uris) = requested.resource_subscriptions.as_ref() {
            for uri in requested_uris {
                if is_subscribable_uri(uri) && seen.insert(uri.clone()) {
                    accepted_uris.push(uri.clone());
                }
            }
        }
        let mut builder = SubscriptionFilter::builder();
        for uri in accepted_uris {
            builder = builder.resource_subscription(uri);
        }
        Some(builder.build())
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        // Snapshot accepted URIs in first-occurrence order. rmcp computes the
        // accepted filter as
        // `requested.intersection(&candidate).intersection(&advertised)`,
        // left-biased over the requested list, so context.accepted() may echo
        // duplicates. Deduplicate again so matching and lag recovery emit no
        // duplicate hints.
        let accepted: Vec<String> = {
            let mut seen = std::collections::HashSet::new();
            context
                .accepted()
                .resource_subscriptions
                .clone()
                .unwrap_or_default()
                .into_iter()
                .filter(|uri| seen.insert(uri.clone()))
                .collect()
        };
        // Each listener gets its own receiver.
        let mut receiver = self.resource_events.subscribe();
        let cancelled = context.cancelled();
        tokio::pin!(cancelled);
        loop {
            tokio::select! {
                // Cancellation completes the stream cleanly.
                _ = &mut cancelled => return Ok(()),
                result = receiver.recv() => {
                    match result {
                        // Notify matching updates and ignore unrelated events.
                        Ok(ResourceEvent::Updated(uri)) => {
                            if accepted.contains(&uri) {
                                if let Err(error) =
                                    context.sink().notify_resource_updated(uri).await
                                {
                                    // The peer or subscription ended. Stop
                                    // without retrying.
                                    tracing::debug!("listen: sink terminated: {error}");
                                    return Ok(());
                                }
                            }
                        }
                        // On lag, notify each accepted URI once in order. The
                        // bounded hub and synchronous publish keep publishers
                        // and the pump non-blocking.
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            for uri in &accepted {
                                if let Err(error) =
                                    context.sink().notify_resource_updated(uri.clone()).await
                                {
                                    tracing::debug!(
                                        "listen: sink terminated during lag recovery: {error}"
                                    );
                                    return Ok(());
                                }
                            }
                        }
                        // A closed hub completes successfully.
                        Err(broadcast::error::RecvError::Closed) => return Ok(()),
                    }
                }
            }
        }
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        const PAGE_SIZE: usize = 100;

        let port_count = self
            .port_provider
            .list_available()
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        let conn_count = self.connections.count().await as u64;

        let all = vec![
            Resource::new(URI_PORTS, "Available serial ports")
                .with_description(
                    "JSON list of serial ports currently exposed by the OS.".to_string(),
                )
                .with_mime_type("application/json".to_string())
                .with_size(port_count)
                .with_annotations(
                    Annotations::default()
                        .with_priority(0.9)
                        .with_audience(vec![Role::User, Role::Assistant]),
                ),
            Resource::new(URI_CONNECTIONS, "Open serial connections")
                .with_description(
                    "JSON list of serial connections currently held open by this server."
                        .to_string(),
                )
                .with_mime_type("application/json".to_string())
                .with_size(conn_count)
                .with_annotations(
                    Annotations::default()
                        .with_priority(0.8)
                        .with_audience(vec![Role::User, Role::Assistant]),
                ),
        ];
        let (resources, next_cursor) = paginate(&all, request.and_then(|r| r.cursor), PAGE_SIZE);
        let mut result = ListResourcesResult::with_all_items(resources);
        result.next_cursor = next_cursor;
        if cache_fields_for(ctx.protocol_version()) {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        const PAGE_SIZE: usize = 100;
        let all = vec![
            ResourceTemplate::new(
                URI_CONNECTION_TEMPLATE,
                "Open serial connection by id",
            )
            .with_description(
                "Per-connection state. Replace {id} with a connection_id returned by open."
                    .to_string(),
            )
            .with_mime_type("application/json".to_string())
            .with_annotations(
                Annotations::default()
                    .with_priority(0.7)
                    .with_audience(vec![Role::User, Role::Assistant]),
            ),
            ResourceTemplate::new(
                URI_CONNECTION_RAW_TEMPLATE,
                "Raw binary data from a serial connection",
            )
            .with_description(
                "Base64-encoded bytes from the connection. Reading consumes up to 256 pending bytes. Replace {id} with a connection_id."
                    .to_string(),
            )
            .with_mime_type("application/octet-stream".to_string())
            .with_annotations(
                Annotations::default()
                    .with_priority(0.6)
                    .with_audience(vec![Role::User, Role::Assistant]),
            ),
            ResourceTemplate::new(
                URI_CONNECTION_LOG_TEMPLATE,
                "Event log for a serial connection",
            )
            .with_description(
                "JSONL event log for the connection. Replace {id} with a connection_id. Each line is a JSON object with timestamp, direction, and event fields."
                    .to_string(),
            )
            .with_mime_type("application/x-ndjson".to_string())
            .with_annotations(
                Annotations::default()
                    .with_priority(0.5)
                    .with_audience(vec![Role::User, Role::Assistant]),
            ),
        ];
        let (resource_templates, next_cursor) =
            paginate(&all, request.and_then(|r| r.cursor), PAGE_SIZE);
        let mut result = ListResourceTemplatesResult::with_all_items(resource_templates);
        result.next_cursor = next_cursor;
        if cache_fields_for(ctx.protocol_version()) {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        let uri = request.uri;
        match parse_resource_uri(&uri) {
            ResourceUriKind::Ports => {
                let ports = self.port_provider.list_available().map_err(|e| {
                    McpError::internal_error(format!("Failed to list ports: {e}"), None)
                })?;
                // Same fresh profile-store preview as the `list_ports` tool,
                // so the resource and the tool never disagree.
                let profiles = self.profile_store.list_fresh().await.map_err(|e| {
                    McpError::internal_error(format!("Failed to read profiles: {e}"), None)
                })?;
                let profile_matches =
                    crate::tools::port_ops::compute_profile_matches(&ports, &profiles);
                let body = serde_json::to_string_pretty(&ListPortsResult {
                    count: ports.len(),
                    ports,
                    profile_matches,
                })
                .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
                Ok(read_result_with_cache_fields(
                    ReadResourceResult::new(vec![
                        ResourceContents::text(body, uri).with_mime_type("application/json")
                    ]),
                    ctx.protocol_version(),
                ))
            }
            ResourceUriKind::ConnectionsList => {
                let summaries = self.connections.list_open().await;
                let body = serde_json::to_string_pretty(&ConnectionsResource {
                    count: summaries.len(),
                    connections: summaries,
                })
                .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
                Ok(read_result_with_cache_fields(
                    ReadResourceResult::new(vec![
                        ResourceContents::text(body, uri).with_mime_type("application/json")
                    ]),
                    ctx.protocol_version(),
                ))
            }
            ResourceUriKind::ConnectionDetail(id) => {
                let conn = self.connections.get(&id).await.map_err(|_| {
                    McpError::resource_not_found(
                        "connection_not_found",
                        Some(serde_json::json!({ "uri": uri, "connection_id": id })),
                    )
                })?;

                let body = serde_json::to_string_pretty(&conn.summary())
                    .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
                Ok(read_result_with_cache_fields(
                    ReadResourceResult::new(vec![
                        ResourceContents::text(body, uri).with_mime_type("application/json")
                    ]),
                    ctx.protocol_version(),
                ))
            }
            ResourceUriKind::ConnectionDetailRaw(id) => {
                let conn = self.connections.get(&id).await.map_err(|_| {
                    McpError::resource_not_found(
                        "connection_not_found",
                        Some(serde_json::json!({ "uri": uri, "connection_id": id })),
                    )
                })?;
                let raw_bytes = conn
                    .read_latest(256)
                    .await
                    .map_err(|e| McpError::internal_error(format!("Failed to read: {e}"), None))?;
                let b64 = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);
                Ok(read_result_with_cache_fields(
                    ReadResourceResult::new(vec![
                        ResourceContents::blob(b64, uri).with_mime_type("application/octet-stream")
                    ]),
                    ctx.protocol_version(),
                ))
            }
            ResourceUriKind::ConnectionLog(id) => {
                let conn = self.connections.get(&id).await.map_err(|_| {
                    McpError::resource_not_found(
                        "connection_not_found",
                        Some(serde_json::json!({ "uri": uri, "connection_id": id })),
                    )
                })?;
                let events = conn.log().snapshot();
                let mut body = String::new();
                for event in &events {
                    let line = serde_json::to_string(event)
                        .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
                    body.push_str(&line);
                    body.push('\n');
                }
                Ok(read_result_with_cache_fields(
                    ReadResourceResult::new(vec![
                        ResourceContents::text(body, uri).with_mime_type("application/x-ndjson")
                    ]),
                    ctx.protocol_version(),
                ))
            }
            ResourceUriKind::Unknown => Err(McpError::resource_not_found(
                "resource_not_found",
                Some(serde_json::json!({ "uri": uri })),
            )),
        }
    }

    async fn complete(
        &self,
        request: CompleteRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        let suggestions = self
            .get_completions(&request.r#ref, &request.argument)
            .await;
        let completion = CompletionInfo::with_all_values(suggestions)
            .map_err(|e| McpError::internal_error(format!("Completion error: {e}"), None))?;
        Ok(CompleteResult::new(completion))
    }

    // Explicit `tools/list` and `prompts/list` handlers. The
    // `#[tool_handler]` macro generates `list_tools` when absent without
    // SEP-2549 cache fields, while `#[prompt_handler]` replaces `list_prompts`.
    // These methods use the same routers, catalog, titles, schemas, and prompt
    // definitions, add cursor pagination, and set `ttlMs: 0` /
    // `cacheScope: "private"` only for the `2026-07-28` immediate-private row.
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        const PAGE_SIZE: usize = 100;
        let all = Self::tool_router().list_all();
        let (tools, next_cursor) = paginate(&all, request.and_then(|r| r.cursor), PAGE_SIZE);
        let mut result = ListToolsResult::with_all_items(tools);
        result.next_cursor = next_cursor;
        if cache_fields_for(ctx.protocol_version()) {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }

    /// Route `prompts/get` through the same `prompt_router` used by
    /// `#[prompt_handler]`. This stays hand-written because that macro replaces
    /// the method; see the impl attribute note.
    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        let prompt_context = rmcp::handler::server::prompt::PromptContext::new(
            self,
            request.name,
            request.arguments,
            context,
        );
        Self::prompt_router().get_prompt(prompt_context).await
    }

    async fn list_prompts(
        &self,
        request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        const PAGE_SIZE: usize = 100;
        let all = Self::prompt_router().list_all();
        let (prompts, next_cursor) = paginate(&all, request.and_then(|r| r.cursor), PAGE_SIZE);
        let mut result = ListPromptsResult::with_all_items(prompts);
        result.next_cursor = next_cursor;
        if cache_fields_for(ctx.protocol_version()) {
            result = result.with_ttl_ms(0).with_cache_scope(CacheScope::Private);
        }
        Ok(result)
    }
}

// Resource URI handling

use crate::resources::{
    parse_resource_uri, ConnectionsResource, ResourceUriKind, URI_CONNECTIONS,
    URI_CONNECTION_LOG_TEMPLATE, URI_CONNECTION_RAW_TEMPLATE, URI_CONNECTION_TEMPLATE, URI_PORTS,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_result_with_cache_fields_sets_fields_only_for_modern() {
        use rmcp::model::ReadResourceResponse;

        let modern = read_result_with_cache_fields(
            ReadResourceResult::new(vec![]),
            Some(ProtocolVersion::V_2026_07_28),
        );
        let ReadResourceResponse::Complete(modern) = modern else {
            panic!("expected complete read result");
        };
        assert_eq!(modern.ttl_ms, Some(0), "modern ttlMs must be 0");
        assert_eq!(
            modern.cache_scope,
            Some(CacheScope::Private),
            "modern cacheScope must be private"
        );

        let legacy = read_result_with_cache_fields(
            ReadResourceResult::new(vec![]),
            Some(ProtocolVersion::V_2025_11_25),
        );
        let ReadResourceResponse::Complete(legacy) = legacy else {
            panic!("expected complete read result");
        };
        assert_eq!(legacy.ttl_ms, None, "legacy must omit ttlMs");
        assert_eq!(legacy.cache_scope, None, "legacy must omit cacheScope");
    }

    #[test]
    fn read_result_with_cache_fields_gives_future_version_no_fields() {
        // A custom future version has no policy row, so it must not inherit
        // modern cache fields merely because its date sorts after 2026-07-28.
        use rmcp::model::ReadResourceResponse;

        let future: ProtocolVersion =
            serde_json::from_value(serde_json::json!("2099-01-01")).unwrap();
        let result = read_result_with_cache_fields(ReadResourceResult::new(vec![]), Some(future));
        let ReadResourceResponse::Complete(result) = result else {
            panic!("expected complete read result");
        };
        assert_eq!(result.ttl_ms, None, "future version must omit ttlMs");
        assert_eq!(
            result.cache_scope, None,
            "future version must omit cacheScope"
        );
    }

    #[test]
    fn prompt_router_advertises_both_prompts() {
        let router = SerialHandler::prompt_router();
        assert!(router.has_route("diagnose_port"));
        assert!(router.has_route("interactive_terminal"));
        assert_eq!(router.list_all().len(), 2);
    }

    #[test]
    fn paginate_handles_offset_past_end_without_panic() {
        let items: Vec<i32> = vec![1, 2, 3, 4, 5];

        // Normal case: offset 0
        let (page, next) = paginate(&items, None, 2);
        assert_eq!(page, vec![1, 2]);
        assert!(next.is_some());

        // Offset at end of items
        let cursor = base64::engine::general_purpose::STANDARD.encode("5".as_bytes());
        let (page, next) = paginate(&items, Some(cursor), 2);
        assert_eq!(page, vec![] as Vec<i32>);
        assert!(next.is_none());

        // Offset past end (would have panicked before fix)
        let cursor = base64::engine::general_purpose::STANDARD.encode("999".as_bytes());
        let (page, next) = paginate(&items, Some(cursor), 2);
        assert_eq!(page, vec![] as Vec<i32>);
        assert!(next.is_none());

        // Offset at max usize (would have overflowed before fix)
        let cursor = base64::engine::general_purpose::STANDARD.encode(usize::MAX.to_string());
        let (page, _next) = paginate(&items, Some(cursor), usize::MAX);
        assert_eq!(page, vec![] as Vec<i32>);
    }

    use crate::buffer_budget::{AtomicBudget, BufferBudget};
    use crate::limits::{DEFAULT_MAX_PROGRAM_BUFFERED_BYTES, DEFAULT_MAX_TOOL_BUFFERED_BYTES};
    use crate::security::SecurityManager;

    /// Assert two `SerialHandler`s have field-for-field equivalent *configuration*.
    fn assert_handler_configs_match(a: &SerialHandler, b: &SerialHandler) {
        assert_eq!(
            a.budget.tool_limit(),
            b.budget.tool_limit(),
            "budget.tool_limit mismatch"
        );
        assert_eq!(
            a.budget.program_limit(),
            b.budget.program_limit(),
            "budget.program_limit mismatch"
        );
        assert_eq!(
            a.security.allowlist_summary(),
            b.security.allowlist_summary(),
            "security summary mismatch"
        );
    }

    #[tokio::test]
    async fn builder_default_matches_new_defaults() {
        let from_new = SerialHandler::new();
        let from_builder = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .security(SecurityManager::from_patterns::<[&str; 0]>([]))
            .build();

        assert_handler_configs_match(&from_new, &from_builder);
        assert_eq!(
            from_builder.budget.tool_limit(),
            DEFAULT_MAX_TOOL_BUFFERED_BYTES
        );
        assert_eq!(
            from_builder.budget.program_limit(),
            DEFAULT_MAX_PROGRAM_BUFFERED_BYTES
        );
    }

    #[tokio::test]
    async fn builder_custom_budget_overrides_default() {
        let budget: Arc<dyn BufferBudget> = Arc::new(AtomicBudget::new(4096, 2048));
        let handler = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .budget(budget)
            .build();
        assert_eq!(handler.budget.tool_limit(), 2048);
        assert_eq!(handler.budget.program_limit(), 4096);
    }
    #[tokio::test]
    async fn builder_injected_profile_store_is_used() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        let store = Arc::new(crate::profile_store::ProfileStore::open(path).unwrap());
        let handler = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .profile_store(Arc::clone(&store))
            .build();
        // The injected store (not a fresh empty vector) is the one served.
        assert!(Arc::ptr_eq(&handler.profile_store, &store));
        assert!(handler.profile_store.list().await.is_empty());

        // A mutation through the handler's store is visible via list().
        let profile = crate::profiles::Profile {
            name: "injected".into(),
            selector: Default::default(),
            defaults: Default::default(),
            metadata: Default::default(),
            revisions: Vec::new(),
        };
        store.upsert(profile, false).await.unwrap();
        let listed = handler.profile_store.list().await;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "injected");
    }

    #[tokio::test]
    async fn builder_default_uses_ephemeral_store() {
        let handler = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .build();
        assert!(
            handler.profile_store.path().is_none(),
            "builder default must be an ephemeral store"
        );
    }

    // Dual MCP lifecycle

    #[tokio::test]
    async fn supported_protocol_versions_are_exact_modern_then_legacy() {
        let handler = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .build();
        let versions: Vec<ProtocolVersion> = handler.supported_protocol_versions().to_vec();
        assert_eq!(versions.len(), 2, "exactly two supported versions");
        assert_eq!(versions[0], ProtocolVersion::V_2026_07_28);
        assert_eq!(versions[1], ProtocolVersion::V_2025_11_25);
    }

    #[test]
    fn modern_capability_view_advertises_resource_subscriptions() {
        let json = serde_json::to_value(
            server_info_for(crate::mcp_protocol::preferred_policy()).capabilities,
        )
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "completions": {},
                "prompts": {},
                "resources": {"subscribe": true},
                "tools": {},
            }),
            "preferred discovery view must advertise resource subscriptions"
        );
    }

    #[test]
    fn legacy_capability_view_keeps_subscription_disabled() {
        let legacy_policy = crate::mcp_protocol::policy_for(&ProtocolVersion::V_2025_11_25)
            .expect("legacy policy row must exist");
        let json = serde_json::to_value(server_info_for(legacy_policy).capabilities).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "completions": {},
                "prompts": {},
                "resources": {},
                "tools": {},
            }),
            "legacy initialize view must keep resource subscriptions disabled"
        );
    }

    #[test]
    fn capability_views_omit_logging_list_change_and_tasks() {
        let legacy_policy = crate::mcp_protocol::policy_for(&ProtocolVersion::V_2025_11_25)
            .expect("legacy policy row must exist");
        for capabilities in [
            server_info_for(crate::mcp_protocol::preferred_policy()).capabilities,
            server_info_for(legacy_policy).capabilities,
        ] {
            let json = serde_json::to_value(&capabilities).unwrap();
            let object = json.as_object().expect("capabilities serialize as object");
            for forbidden in [
                "logging",
                "experimental",
                "extensions",
                "roots",
                "sampling",
                "elicitation",
            ] {
                assert!(
                    !object.contains_key(forbidden),
                    "capability view must not advertise `{forbidden}`: {json}"
                );
            }
            // tools/prompts advertise no list-change flags.
            assert_eq!(object["tools"], serde_json::json!({}));
            assert_eq!(object["prompts"], serde_json::json!({}));
        }
        // resources: preferred carries only `subscribe`, legacy carries none;
        // neither advertises `listChanged`.
        let modern = serde_json::to_value(
            server_info_for(crate::mcp_protocol::preferred_policy()).capabilities,
        )
        .unwrap();
        assert_eq!(modern["resources"], serde_json::json!({"subscribe": true}));
        let legacy = serde_json::to_value(server_info_for(legacy_policy).capabilities).unwrap();
        assert_eq!(legacy["resources"], serde_json::json!({}));
    }

    #[test]
    fn policy_infos_carry_their_own_protocol_version() {
        let legacy_policy = crate::mcp_protocol::policy_for(&ProtocolVersion::V_2025_11_25)
            .expect("legacy policy row must exist");
        assert_eq!(
            server_info_for(legacy_policy).protocol_version,
            ProtocolVersion::V_2025_11_25
        );
        assert_eq!(
            server_info_for(crate::mcp_protocol::preferred_policy()).protocol_version,
            ProtocolVersion::V_2026_07_28
        );
    }

    #[tokio::test]
    async fn get_info_returns_modern_view_for_listen_intersection() {
        // rmcp intersects `subscriptions/listen` filters against
        // `get_info().capabilities`; the modern listen lifecycle requires the
        // modern view there while `initialize()` returns the legacy view.
        let handler = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .build();
        assert_eq!(
            serde_json::to_value(handler.get_info().capabilities).unwrap(),
            serde_json::json!({
                "completions": {},
                "prompts": {},
                "resources": {"subscribe": true},
                "tools": {},
            })
        );
        assert_eq!(
            handler.get_info().protocol_version,
            ProtocolVersion::V_2026_07_28
        );
    }

    #[tokio::test]
    async fn accepted_subscription_filter_keeps_valid_dedup_and_strips_unsupported() {
        let handler = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .build();
        let requested = SubscriptionFilter::builder()
            .resource_subscriptions([
                "serial://ports",
                "serial://connections",
                "serial://ports", // duplicate; first order preserved
                "serial://connections/abc-123",
                "serial://connections/{id}", // template -> stripped
                "serial://connections/",     // empty id -> stripped
                "https://example.com/x",     // unknown scheme -> stripped
            ])
            .tools_list_changed()
            .prompts_list_changed()
            .resources_list_changed()
            .build();
        let accepted = handler
            .accepted_subscription_filter(&requested)
            .expect("handler always available");
        assert_eq!(
            accepted.resource_subscriptions.as_deref(),
            Some(
                [
                    "serial://ports".to_string(),
                    "serial://connections".to_string(),
                    "serial://connections/abc-123".to_string(),
                ]
                .as_slice()
            ),
            "valid + deduplicated URIs in first-request order"
        );
        assert!(
            accepted.tools_list_changed.is_none()
                && accepted.prompts_list_changed.is_none()
                && accepted.resources_list_changed.is_none(),
            "all list-change flags must be stripped"
        );
    }

    #[tokio::test]
    async fn accepted_subscription_filter_empty_accepted_resources_is_none_not_fake_uri() {
        let handler = SerialHandler::builder()
            .connections(Arc::new(ConnectionManager::new()))
            .build();
        // Everything invalid -> accepted resource list is None.
        let requested = SubscriptionFilter::builder()
            .resource_subscriptions(["serial://connections/{id}", "https://example.com/x"])
            .build();
        let accepted = handler
            .accepted_subscription_filter(&requested)
            .expect("handler always available");
        assert!(accepted.resource_subscriptions.is_none());

        // Nothing requested at all -> same empty accepted filter, and the
        // handler stays available (Some).
        let accepted_empty = handler
            .accepted_subscription_filter(&SubscriptionFilter::new())
            .expect("handler always available");
        assert!(accepted_empty.resource_subscriptions.is_none());
        assert!(
            accepted_empty.tools_list_changed.is_none()
                && accepted_empty.prompts_list_changed.is_none()
                && accepted_empty.resources_list_changed.is_none()
        );
    }
}
