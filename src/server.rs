//! MCP server tool surface for serial communication.
//!
//! Each `#[tool]` method below corresponds to one MCP tool. Tools return
//! structured JSON via [`Json<T>`] so MCP clients can index fields directly
//! instead of parsing free-form text.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use rmcp::{
    handler::server::wrapper::Parameters, model::*, prompt, prompt_handler, prompt_router,
    service::RequestContext, tool, tool_handler, tool_router, ErrorData as McpError, Json,
    RoleServer, ServerHandler,
};

use tracing::{debug, info};

use crate::buffer_budget::BufferBudget;
use crate::rx_session::RxSessionManager;
use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, PortInfo};
use crate::tx_session::TxSessionManager;

use crate::prompts::types::*;
use crate::prompts::{diagnose, interactive};
use crate::tools::types::*;
use crate::tools::{control_ops, io_ops, port_ops, stream_ops};

/// Helper for cursor-based pagination over a vector of items.
///
/// `cursor` is interpreted as a base64-encoded UTF-8 string containing an offset
/// number (e.g. "0", "1").  Returns the sliced items and an optional next
/// cursor when more items remain.
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

// ---- Handler ---------------------------------------------------------------

pub type StreamRegistry = Arc<tokio::sync::Mutex<HashMap<String, stream_ops::StreamHandle>>>;

#[derive(Clone)]
pub struct SerialHandler {
    pub(crate) connections: Arc<ConnectionManager>,
    streams: StreamRegistry,
    security: SecurityManager,
    subscribers: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
    rx_sessions: Arc<RxSessionManager>,
    tx_sessions: Arc<TxSessionManager>,
    budget: Arc<dyn BufferBudget>,
}

#[tool_router]
impl SerialHandler {
    pub fn new() -> Self {
        Self::with_manager_and_security(
            Arc::new(ConnectionManager::new()),
            SecurityManager::from_patterns::<[&str; 0]>([]),
        )
    }

    /// Construct a handler with a caller-supplied [`ConnectionManager`].
    ///
    /// Used by integration tests that want to pre-populate the registry
    /// with a fake (in-memory) connection before exposing the handler over
    /// MCP, instead of going through the OS-level `open` path.
    pub fn with_manager(connections: Arc<ConnectionManager>) -> Self {
        Self::with_manager_and_security(
            connections,
            SecurityManager::from_patterns::<[&str; 0]>([]),
        )
    }

    pub fn with_manager_and_security(
        connections: Arc<ConnectionManager>,
        security: SecurityManager,
    ) -> Self {
        use crate::limits::{DEFAULT_MAX_PROGRAM_BUFFERED_BYTES, DEFAULT_MAX_TOOL_BUFFERED_BYTES};
        let budget: Arc<dyn BufferBudget> = Arc::new(crate::buffer_budget::AtomicBudget::new(
            DEFAULT_MAX_PROGRAM_BUFFERED_BYTES,
            DEFAULT_MAX_TOOL_BUFFERED_BYTES,
        ));
        Self::with_manager_security_streams_and_budget(
            connections,
            security,
            Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            budget,
        )
    }

    pub fn with_manager_security_and_streams(
        connections: Arc<ConnectionManager>,
        security: SecurityManager,
        streams: StreamRegistry,
    ) -> Self {
        use crate::limits::{DEFAULT_MAX_PROGRAM_BUFFERED_BYTES, DEFAULT_MAX_TOOL_BUFFERED_BYTES};
        let budget: Arc<dyn BufferBudget> = Arc::new(crate::buffer_budget::AtomicBudget::new(
            DEFAULT_MAX_PROGRAM_BUFFERED_BYTES,
            DEFAULT_MAX_TOOL_BUFFERED_BYTES,
        ));
        Self::with_manager_security_streams_and_budget(connections, security, streams, budget)
    }

    pub fn with_manager_security_streams_and_budget(
        connections: Arc<ConnectionManager>,
        security: SecurityManager,
        streams: StreamRegistry,
        budget: Arc<dyn BufferBudget>,
    ) -> Self {
        Self {
            connections,
            streams,
            security,
            subscribers: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            rx_sessions: Arc::new(RxSessionManager::new()),
            tx_sessions: Arc::new(TxSessionManager::new()),
            budget,
        }
    }

    #[tool(
        description = "List all available serial ports on the system",
        title = "List Serial Ports",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_ports(&self) -> Result<Json<ListPortsResult>, String> {
        port_ops::list_ports().await
    }

    #[tool(
        description = "List all open serial connections held by this server",
        title = "List Open Connections",
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn list_connections(&self) -> Result<Json<ListConnectionsResult>, String> {
        port_ops::list_connections(&self.connections).await
    }

    #[tool(
        description = "Open a serial port connection with specified configuration",
        title = "Open Serial Port",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<OpenResult>, String> {
        let result = port_ops::open(&self.connections, &self.security, args).await?;
        let connection_id = result.0.connection_id.clone();
        self.notify_resource_changed(&connection_id, &ctx).await;
        Ok(result)
    }

    #[tool(
        description = "Close an open serial port connection",
        title = "Close Serial Port",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn close(
        &self,
        Parameters(args): Parameters<CloseArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<CloseResult>, String> {
        let connection_id = args.connection_id.clone();
        let result = port_ops::close(&self.connections, args).await?;
        // Shut down RX session (pump + consumers) for this connection.
        self.rx_sessions.remove(&connection_id).await;
        // Shut down TX session (worker) for this connection.
        self.tx_sessions.remove(&connection_id).await;
        // Abort any active RX subscription tied to this connection.
        self.streams.lock().await.remove(&connection_id);
        self.notify_resource_changed(&connection_id, &ctx).await;
        Ok(result)
    }

    #[tool(
        description = "Write data to a serial port connection",
        title = "Write Serial Data",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn write(
        &self,
        Parameters(args): Parameters<WriteArgs>,
    ) -> Result<Json<WriteResult>, String> {
        io_ops::write(&self.connections, &self.tx_sessions, args).await
    }

    #[tool(
        description = "Read data from a serial port connection. With no match option, reads available bytes with timeout and burst-settle. With match, accumulates future bytes until the pattern is found, timeout, max_buffered_bytes, connection close, or cancellation. The match option specifies a literal byte-substring pattern to detect; match_encoding controls how the pattern string is decoded to raw bytes.",
        title = "Read Serial Data",
        annotations(read_only_hint = true, open_world_hint = false),
        execution(task_support = "optional")
    )]
    async fn read(
        &self,
        meta: Meta,
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
        description = "Discard buffered serial data. target=input clears OS read buffer (data the device sent that the app hasn't consumed); target=output clears the OS write queue; target=both clears both.",
        title = "Flush Serial Buffers",
        annotations(destructive_hint = true, open_world_hint = false)
    )]
    async fn flush(
        &self,
        Parameters(args): Parameters<FlushArgs>,
    ) -> Result<Json<FlushResult>, String> {
        io_ops::flush(&self.connections, &self.tx_sessions, args).await
    }

    #[tool(
        description = "Set the DTR and RTS modem-control lines. Common patterns: pulse DTR low for Arduino auto-reset; hold both low to enter ESP32 bootloader.",
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
        control_ops::set_dtr_rts(&self.connections, args).await
    }

    #[tool(
        description = "Change hardware/software flow control mode on an open connection. Use flow_control='none' to ignore RTS/CTS for this session.",
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
        control_ops::set_flow_control(&self.connections, args).await
    }

    #[tool(
        description = "Assert a BREAK condition on the TX line for duration_ms milliseconds (default 250ms), then release it. Used to signal attention on some legacy serial protocols.",
        title = "Send BREAK",
        annotations(destructive_hint = true, open_world_hint = false),
        execution(task_support = "optional")
    )]
    async fn send_break(
        &self,
        meta: Meta,
        ct: tokio_util::sync::CancellationToken,
        peer: rmcp::Peer<RoleServer>,
        Parameters(args): Parameters<SendBreakArgs>,
    ) -> Result<Json<SendBreakResult>, String> {
        control_ops::send_break(&self.connections, meta, ct, peer, args).await
    }

    #[tool(
        description = "Subscribe to a connection: starts a background stream that forwards received bytes as MCP `notifications/message` events with logger=\"serial:<connection_id>\". When timeout_ms is set, the stream auto-stops after that duration. When omitted, the stream runs until unsubscribe, connection close, or error. With the match option, the stream also detects the first byte-substring match, emits a final notification with matched=true and match_index, then stops. Replaces any prior subscription on the same connection. A final notification with stop_reason is emitted when the stream ends.",
        title = "Subscribe to RX Stream",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        ),
        execution(task_support = "optional")
    )]
    async fn subscribe(
        &self,
        meta: Meta,
        ct: tokio_util::sync::CancellationToken,
        peer: rmcp::Peer<RoleServer>,
        Parameters(args): Parameters<SubscribeArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<SubscribeResult>, String> {
        stream_ops::subscribe(
            &self.connections,
            &self.rx_sessions,
            &self.budget,
            &self.streams,
            args,
            meta,
            ct,
            peer,
            ctx,
        )
        .await
    }

    #[tool(
        description = "Cancel an active RX subscription on a connection. No-op if no subscription exists.",
        title = "Unsubscribe from RX Stream",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn unsubscribe(
        &self,
        Parameters(args): Parameters<UnsubscribeArgs>,
    ) -> Result<Json<UnsubscribeResult>, String> {
        stream_ops::unsubscribe(&self.connections, &self.rx_sessions, &self.streams, args).await
    }
}

// ---- Tool helpers (extracted to src/tools/helpers.rs) -----------------------

// ---- ServerHandler boilerplate ----------------------------------------------

impl Default for SerialHandler {
    fn default() -> Self {
        Self::new()
    }
}

// ---- Prompt templates ------------------------------------------------------

// ---- Completion helper ------------------------------------------------------

impl SerialHandler {
    /// Generate completion suggestions for tool/resource arguments.
    async fn get_completions(&self, r#ref: &Reference, argument: &ArgumentInfo) -> Vec<String> {
        match r#ref {
            Reference::Resource(resource_ref) => {
                if resource_ref.uri == URI_PORTS && argument.name == "port" {
                    match PortInfo::list_available() {
                        Ok(ports) => ports.into_iter().map(|p| p.name).collect(),
                        Err(_) => vec![],
                    }
                } else {
                    vec![]
                }
            }
            Reference::Prompt(prompt_ref) => {
                if prompt_ref.name == "diagnose_port" && argument.name == "port" {
                    match PortInfo::list_available() {
                        Ok(ports) => ports.into_iter().map(|p| p.name).collect(),
                        Err(_) => vec![],
                    }
                } else {
                    vec![]
                }
            }
        }
    }

    async fn notify_resource_changed(&self, connection_id: &str, ctx: &RequestContext<RoleServer>) {
        if let Err(e) = ctx.peer.notify_resource_list_changed().await {
            debug!("Failed to notify resource list changed: {e}");
        }
        let conn_uri = format!("{URI_CONNECTION_PREFIX}{connection_id}");
        let subs = self.subscribers.lock().await;
        let should_notify = subs.get(&conn_uri).is_some_and(|count| *count > 0);
        drop(subs);
        if should_notify {
            if let Err(e) = ctx
                .peer
                .notify_resource_updated(rmcp::model::ResourceUpdatedNotificationParam::new(
                    conn_uri,
                ))
                .await
            {
                debug!("Failed to notify resource updated: {e}");
            }
        }
    }
}

#[prompt_router]
impl SerialHandler {
    /// Walk through diagnosing an unknown serial port: try common baud
    /// rates, send a benign probe, observe response, narrow down config.
    #[prompt(
        name = "diagnose_port",
        description = "Step-by-step plan to identify an unknown serial device on a given port"
    )]
    async fn diagnose_port_prompt(
        &self,
        Parameters(args): Parameters<DiagnosePortArgs>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        Ok(diagnose::build_diagnose_prompt(args))
    }

    /// Guide an interactive serial REPL session against an already-open
    /// connection, using `write` / `wait_for` to drive a command/response
    /// loop.
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

#[tool_handler]
#[prompt_handler]
impl ServerHandler for SerialHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .enable_resources_list_changed()
                .enable_resources_subscribe()
                .enable_prompts()
                .enable_logging()
                .enable_completions()
                .build(),
        )
        .with_server_info(Implementation::new(
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
        ))
        .with_protocol_version(ProtocolVersion::V_2025_11_25)
        .with_instructions(
            "A serial port communication MCP server. Use list_ports to discover available serial ports, then open connections to communicate with serial devices. Resources: serial://ports, serial://connections, serial://connections/{id}. Prompts: diagnose_port, interactive_terminal. Subscribe to live RX bytes with the subscribe tool; the server emits notifications/message events with logger=\"serial:<connection_id>\"."
                .to_string(),
        )
    }

    async fn initialize(
        &self,
        _req: InitializeRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<InitializeResult, McpError> {
        info!("Serial MCP server initialized");
        Ok(self.get_info())
    }

    async fn list_resources(
        &self,
        request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        const PAGE_SIZE: usize = 100;

        let port_count = PortInfo::list_available()
            .map(|v| v.len() as u32)
            .unwrap_or(0);
        let conn_count = self.connections.count().await as u32;

        let all = vec![
            RawResource::new(URI_PORTS, "Available serial ports")
                .with_description("JSON list of serial ports the OS currently exposes.".to_string())
                .with_mime_type("application/json".to_string())
                .with_size(port_count)
                .with_priority(0.9)
                .with_audience(vec![Role::User, Role::Assistant]),
            RawResource::new(URI_CONNECTIONS, "Open serial connections")
                .with_description(
                    "JSON list of serial connections currently held open by this server."
                        .to_string(),
                )
                .with_mime_type("application/json".to_string())
                .with_size(conn_count)
                .with_priority(0.8)
                .with_audience(vec![Role::User, Role::Assistant]),
        ];
        let (resources, next_cursor) = paginate(&all, request.and_then(|r| r.cursor), PAGE_SIZE);
        Ok(ListResourcesResult {
            resources,
            next_cursor,
            meta: None,
        })
    }

    async fn list_resource_templates(
        &self,
        request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        const PAGE_SIZE: usize = 100;
        let all = vec![
            RawResourceTemplate::new(
                URI_CONNECTION_TEMPLATE,
                "Open serial connection by id",
            )
            .with_description(
                "Per-connection state. Substitute {id} with a connection_id returned by the open tool."
                    .to_string(),
            )
            .with_mime_type("application/json".to_string())
            .with_priority(0.7)
            .with_audience(vec![Role::User, Role::Assistant]),
            RawResourceTemplate::new(
                URI_CONNECTION_RAW_TEMPLATE,
                "Raw binary data from a serial connection",
            )
            .with_description(
                "Base64-encoded bytes read from the connection. This resource consumes up to 256 pending bytes; substitute {id} with a connection_id."
                    .to_string(),
            )
            .with_mime_type("application/octet-stream".to_string())
            .with_priority(0.6)
            .with_audience(vec![Role::User, Role::Assistant]),
        ];
        let (resource_templates, next_cursor) =
            paginate(&all, request.and_then(|r| r.cursor), PAGE_SIZE);
        Ok(ListResourceTemplatesResult {
            resource_templates,
            next_cursor,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let uri = request.uri;
        match parse_resource_uri(&uri) {
            ResourceUriKind::Ports => {
                let ports = PortInfo::list_available().map_err(|e| {
                    McpError::internal_error(format!("Failed to list ports: {e}"), None)
                })?;
                let body = serde_json::to_string_pretty(&ListPortsResult {
                    count: ports.len(),
                    ports,
                })
                .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    body, uri,
                )
                .with_mime_type("application/json")]))
            }
            ResourceUriKind::ConnectionsList => {
                let summaries = self.connections.list_open().await;
                let body = serde_json::to_string_pretty(&ConnectionsResource {
                    count: summaries.len(),
                    connections: summaries,
                })
                .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    body, uri,
                )
                .with_mime_type("application/json")]))
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
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    body, uri,
                )
                .with_mime_type("application/json")]))
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
                Ok(ReadResourceResult::new(vec![ResourceContents::blob(
                    b64, uri,
                )
                .with_mime_type("application/octet-stream")]))
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

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let uri = request.uri;
        let mut subscribers = self.subscribers.lock().await;
        *subscribers.entry(uri.clone()).or_insert(0) += 1;
        debug!("Client subscribed to resource {}", uri);
        Ok(())
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let uri = request.uri;
        let mut subscribers = self.subscribers.lock().await;
        if let Some(count) = subscribers.get_mut(&uri) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                subscribers.remove(&uri);
            }
        }
        debug!("Client unsubscribed from resource {}", uri);
        Ok(())
    }
}

// ---- Resource URI handling --------------------------------------------------

use crate::resources::{
    parse_resource_uri, ConnectionsResource, ResourceUriKind, URI_CONNECTIONS,
    URI_CONNECTION_PREFIX, URI_CONNECTION_RAW_TEMPLATE, URI_CONNECTION_TEMPLATE, URI_PORTS,
};

#[cfg(test)]
mod tests {
    use super::*;

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
}
