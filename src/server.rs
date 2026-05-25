//! MCP server tool surface for serial communication.
//!
//! Each `#[tool]` method below corresponds to one MCP tool. Tools return
//! structured JSON via [`Json<T>`] so MCP clients can index fields directly
//! instead of parsing free-form text.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use rmcp::{
    handler::server::{
        router::{prompt::PromptRouter, tool::ToolRouter},
        wrapper::Parameters,
    },
    model::*,
    prompt, prompt_handler, prompt_router,
    service::RequestContext,
    task_handler,
    task_manager::OperationProcessor,
    tool, tool_handler, tool_router, ErrorData as McpError, Json, RoleServer, ServerHandler,
};

use tracing::{debug, info};

use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, ConnectionSummary, PortInfo};

use crate::prompts::types::*;
use crate::prompts::{diagnose, interactive};
use crate::tools::types::*;
use crate::tools::{control_ops, io_ops, pattern_ops, port_ops, stream_ops};

// ---- Handler ---------------------------------------------------------------

#[derive(Clone)]
pub struct SerialHandler {
    pub(crate) connections: Arc<ConnectionManager>,
    /// Per-connection background RX-streaming tasks, indexed by connection id.
    /// Dropping a handle aborts the task.
    streams: Arc<tokio::sync::Mutex<HashMap<String, stream_ops::StreamHandle>>>,
    /// MCP task manager for opt-in long-running tool invocations
    /// (read, wait_for, send_break). Clients can submit those tools as
    /// tasks and cancel them via the standard MCP tasks/cancel request.
    #[allow(dead_code)]
    processor: Arc<tokio::sync::Mutex<OperationProcessor>>,
    #[allow(dead_code)]
    tool_router: ToolRouter<SerialHandler>,
    #[allow(dead_code)]
    prompt_router: PromptRouter<SerialHandler>,
    security: SecurityManager,
    /// Active resource subscribers by URI (simple reference count).
    subscribers: Arc<tokio::sync::Mutex<HashMap<String, usize>>>,
}

#[tool_router]
impl SerialHandler {
    pub fn new() -> Self {
        Self::with_manager(Arc::new(ConnectionManager::new()))
    }

    /// Construct a handler with a caller-supplied [`ConnectionManager`].
    ///
    /// Used by integration tests that want to pre-populate the registry
    /// with a fake (in-memory) connection before exposing the handler over
    /// MCP, instead of going through the OS-level `open` path.
    pub fn with_manager(connections: Arc<ConnectionManager>) -> Self {
        let security = SecurityManager::from_env();
        Self {
            connections,
            streams: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processor: Arc::new(tokio::sync::Mutex::new(OperationProcessor::new())),
            tool_router: Self::tool_router(),
            prompt_router: Self::prompt_router(),
            security,
            subscribers: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
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
        description = "Open a serial port connection with specified configuration",
        title = "Open Serial Port",
        annotations(destructive_hint = false, open_world_hint = false)
    )]
    async fn open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<OpenResult>, String> {
        port_ops::open(
            &self.connections,
            &self.security,
            &self.subscribers,
            args,
            ctx,
        )
        .await
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
        port_ops::close(&self.connections, &self.subscribers, args, ctx).await
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
        io_ops::write(&self.connections, args).await
    }

    #[tool(
        description = "Read data from a serial port connection",
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
        io_ops::read(&self.connections, meta, ct, peer, args).await
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
        io_ops::flush(&self.connections, args).await
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
        description = "Subscribe to a connection: a background task reads bytes in chunks and forwards them to the client as MCP `notifications/message` events with logger=\"serial:<connection_id>\". Replaces any prior subscription on the same connection. Stop with unsubscribe or by closing the connection.",
        title = "Subscribe to RX Stream",
        annotations(
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn subscribe(
        &self,
        Parameters(args): Parameters<SubscribeArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<SubscribeResult>, String> {
        stream_ops::subscribe(&self.connections, &self.streams, args, ctx).await
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
        stream_ops::unsubscribe(&self.streams, args).await
    }

    #[tool(
        description = "Read bytes from a connection until a pattern matches or timeout. Pattern is interpreted with pattern_encoding (utf8/hex/base64). Returns the accumulated bytes (re-encoded with response_encoding) and the byte offset where the match started. Use for prompt/response interactions, e.g. send 'reset\\r\\n' then wait_for pattern='OK>'.",
        title = "Wait for Serial Pattern",
        annotations(read_only_hint = true, open_world_hint = false),
        execution(task_support = "optional")
    )]
    async fn wait_for(
        &self,
        meta: Meta,
        ct: tokio_util::sync::CancellationToken,
        peer: rmcp::Peer<RoleServer>,
        Parameters(args): Parameters<WaitForArgs>,
    ) -> Result<Json<WaitForResult>, String> {
        pattern_ops::wait_for(&self.connections, meta, ct, peer, args).await
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
#[task_handler]
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
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                RawResource::new(URI_PORTS, "Available serial ports")
                    .with_description(
                        "JSON list of serial ports the OS currently exposes.".to_string(),
                    )
                    .with_mime_type("application/json".to_string())
                    .no_annotation(),
                RawResource::new(URI_CONNECTIONS, "Open serial connections")
                    .with_description(
                        "JSON list of serial connections currently held open by this server."
                            .to_string(),
                    )
                    .with_mime_type("application/json".to_string())
                    .no_annotation(),
            ],
            next_cursor: None,
            meta: None,
        })
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        Ok(ListResourceTemplatesResult {
            resource_templates: vec![
                RawResourceTemplate::new(
                    URI_CONNECTION_TEMPLATE,
                    "Open serial connection by id",
                )
                .with_description(
                    "Per-connection state. Substitute {id} with a connection_id returned by the open tool."
                        .to_string(),
                )
                .with_mime_type("application/json".to_string())
                .no_annotation(),
                RawResourceTemplate::new(
                    URI_CONNECTION_RAW_TEMPLATE,
                    "Raw binary data from a serial connection",
                )
                .with_description(
                    "Base64-encoded bytes recently read from the connection. Substitute {id} with a connection_id."
                        .to_string(),
                )
                .with_mime_type("application/octet-stream".to_string())
                .no_annotation(),
            ],
            next_cursor: None,
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

                // Check if requesting raw binary data
                if uri.ends_with("/raw") {
                    let raw_bytes = conn.read_latest(256).await.map_err(|e| {
                        McpError::internal_error(format!("Failed to read: {e}"), None)
                    })?;
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);
                    Ok(ReadResourceResult::new(vec![ResourceContents::blob(
                        b64, uri,
                    )
                    .with_mime_type("application/octet-stream")]))
                } else {
                    let body = serde_json::to_string_pretty(&ConnectionSummary {
                        connection_id: conn.id().to_string(),
                        port: conn.port().to_string(),
                        latest_read: None,
                    })
                    .map_err(|e| McpError::internal_error(format!("serialize: {e}"), None))?;
                    Ok(ReadResourceResult::new(vec![ResourceContents::text(
                        body, uri,
                    )
                    .with_mime_type("application/json")]))
                }
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
    URI_CONNECTION_RAW_TEMPLATE, URI_CONNECTION_TEMPLATE, URI_PORTS,
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
}
