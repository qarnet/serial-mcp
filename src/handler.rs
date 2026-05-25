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

use crate::codec;
use crate::security::SecurityManager;
use crate::serial::{ConnectionManager, ConnectionSummary, PortInfo, SerialConnection};

use crate::prompts::types::*;
use crate::prompts::{diagnose, interactive};
use crate::tools::helpers::*;
use crate::tools::types::*;

// ---- Handler ---------------------------------------------------------------

#[derive(Clone)]
pub struct SerialHandler {
    pub(crate) connections: Arc<ConnectionManager>,
    /// Per-connection background RX-streaming tasks, indexed by connection id.
    /// Dropping a handle aborts the task.
    streams: Arc<tokio::sync::Mutex<HashMap<String, StreamHandle>>>,
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

/// RAII wrapper around a streaming task. Aborts the task on drop.
struct StreamHandle {
    join: tokio::task::JoinHandle<()>,
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.join.abort();
    }
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

    #[tool(description = "List all available serial ports on the system")]
    async fn list_ports(&self) -> Result<Json<ListPortsResult>, String> {
        debug!("Listing serial ports");
        let ports = PortInfo::list_available()
            .map_err(|e| log_tool_err("list_ports", "Failed to list ports", e))?;
        info!("Found {} serial ports", ports.len());
        Ok(Json(ListPortsResult {
            count: ports.len(),
            ports,
        }))
    }

    #[tool(description = "Open a serial port connection with specified configuration")]
    async fn open(
        &self,
        Parameters(args): Parameters<OpenArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<OpenResult>, String> {
        let config = parse_open_args(args)?;
        let port = config.port.clone();
        let baud_rate = config.baud_rate;
        debug!("Opening {} @ {}", port, baud_rate);

        // Check allowlist
        if !self.security.is_port_allowed(&port) {
            return Err(format!(
                "Port '{}' is not in the allowlist. Allowed patterns: {}",
                port,
                self.security.allowlist_summary()
            ));
        }

        let connection_id = self
            .connections
            .open(config)
            .await
            .map_err(|e| log_tool_err("open", &format!("Failed to open port {port}"), e))?;
        info!("Opened connection {} -> {}", connection_id, port);

        // Notify clients that the resource list has changed
        if let Err(e) = ctx.peer.notify_resource_list_changed().await {
            debug!("Failed to notify resource list changed: {}", e);
        }

        // Notify subscribers to the specific connection resource
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
                debug!("Failed to notify resource updated: {}", e);
            }
        }

        Ok(Json(OpenResult {
            connection_id,
            port,
            baud_rate,
        }))
    }

    #[tool(description = "Close an open serial port connection")]
    async fn close(
        &self,
        Parameters(args): Parameters<CloseArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<CloseResult>, String> {
        debug!("Closing {}", args.connection_id);
        self.connections
            .close(&args.connection_id)
            .await
            .map_err(|e| {
                log_tool_err(
                    "close",
                    &format!("Failed to close connection {}", args.connection_id),
                    e,
                )
            })?;
        info!("Closed connection {}", args.connection_id);

        // Notify clients that the resource list has changed
        if let Err(e) = ctx.peer.notify_resource_list_changed().await {
            debug!("Failed to notify resource list changed: {}", e);
        }

        // Notify subscribers to the specific connection resource
        let conn_uri = format!("{URI_CONNECTION_PREFIX}{}", args.connection_id);
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
                debug!("Failed to notify resource updated: {}", e);
            }
        }

        Ok(Json(CloseResult {
            connection_id: args.connection_id,
        }))
    }

    #[tool(description = "Write data to a serial port connection")]
    async fn write(
        &self,
        Parameters(args): Parameters<WriteArgs>,
    ) -> Result<Json<WriteResult>, String> {
        debug!("Write to {} ({})", args.connection_id, args.encoding);
        let encoding = parse_encoding(&args.encoding)?;
        let connection = self.lookup_connection(&args.connection_id).await?;
        let bytes = codec::decode(encoding, &args.data)
            .map_err(|e| format!("Data decoding failed - {e}"))?;
        let bytes_written = connection.write(&bytes).await.map_err(|e| {
            log_tool_err(
                "write",
                &format!("Data sending failed on {}", args.connection_id),
                e,
            )
        })?;
        debug!("Wrote {} bytes to {}", bytes_written, args.connection_id);
        Ok(Json(WriteResult {
            connection_id: args.connection_id,
            bytes_written,
            encoding: encoding.to_string(),
        }))
    }

    #[tool(
        description = "Read data from a serial port connection",
        execution(task_support = "optional")
    )]
    async fn read(
        &self,
        Parameters(args): Parameters<ReadArgs>,
    ) -> Result<Json<ReadResult>, String> {
        debug!(
            "Read from {} (timeout {:?})",
            args.connection_id, args.timeout_ms
        );
        let encoding = parse_encoding(&args.encoding)?;
        let connection = self.lookup_connection(&args.connection_id).await?;
        let outcome = read_bytes(&connection, args.max_bytes, args.timeout_ms).await?;
        build_read_result(outcome, args.connection_id, encoding, args.timeout_ms)
    }

    #[tool(
        description = "Discard buffered serial data. target=input clears OS read buffer (data the device sent that the app hasn't consumed); target=output clears the OS write queue; target=both clears both."
    )]
    async fn flush(
        &self,
        Parameters(args): Parameters<FlushArgs>,
    ) -> Result<Json<FlushResult>, String> {
        debug!("Flush {} target={:?}", args.connection_id, args.target);
        let connection = self.lookup_connection(&args.connection_id).await?;
        connection.flush_buffers(args.target).await.map_err(|e| {
            log_tool_err(
                "flush",
                &format!("Failed to flush {}", args.connection_id),
                e,
            )
        })?;
        info!("Flushed {} ({:?})", args.connection_id, args.target);
        Ok(Json(FlushResult {
            connection_id: args.connection_id,
            target: args.target,
        }))
    }

    #[tool(
        description = "Set the DTR and RTS modem-control lines. Common patterns: pulse DTR low for Arduino auto-reset; hold both low to enter ESP32 bootloader."
    )]
    async fn set_dtr_rts(
        &self,
        Parameters(args): Parameters<SetDtrRtsArgs>,
    ) -> Result<Json<SetDtrRtsResult>, String> {
        debug!(
            "set_dtr_rts {} dtr={} rts={}",
            args.connection_id, args.dtr, args.rts
        );
        let connection = self.lookup_connection(&args.connection_id).await?;
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

    #[tool(
        description = "Assert a BREAK condition on the TX line for duration_ms milliseconds (default 250ms), then release it. Used to signal attention on some legacy serial protocols.",
        execution(task_support = "optional")
    )]
    async fn send_break(
        &self,
        Parameters(args): Parameters<SendBreakArgs>,
    ) -> Result<Json<SendBreakResult>, String> {
        debug!(
            "send_break {} duration={}ms",
            args.connection_id, args.duration_ms
        );
        let connection = self.lookup_connection(&args.connection_id).await?;
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

    #[tool(
        description = "Subscribe to a connection: a background task reads bytes in chunks and forwards them to the client as MCP `notifications/message` events with logger=\"serial:<connection_id>\". Replaces any prior subscription on the same connection. Stop with unsubscribe or by closing the connection."
    )]
    async fn subscribe(
        &self,
        Parameters(args): Parameters<SubscribeArgs>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<Json<SubscribeResult>, String> {
        debug!(
            "subscribe {} encoding={} chunk={} poll={}",
            args.connection_id, args.encoding, args.max_chunk_bytes, args.poll_interval_ms
        );
        let encoding = parse_encoding(&args.encoding)?;
        let connection = self.lookup_connection(&args.connection_id).await?;
        let peer = ctx.peer.clone();

        let id = args.connection_id.clone();
        let chunk_bytes = args.max_chunk_bytes;
        let poll_ms = args.poll_interval_ms;
        let join = tokio::spawn(stream_rx(peer, connection, encoding, chunk_bytes, poll_ms));

        let mut streams = self.streams.lock().await;
        let replaced_previous = streams.insert(id.clone(), StreamHandle { join }).is_some();
        info!(
            "subscribed RX stream for {} (replaced={})",
            id, replaced_previous
        );
        Ok(Json(SubscribeResult {
            connection_id: id,
            encoding: encoding.to_string(),
            max_chunk_bytes: chunk_bytes,
            poll_interval_ms: poll_ms,
            replaced_previous,
        }))
    }

    #[tool(
        description = "Cancel an active RX subscription on a connection. No-op if no subscription exists."
    )]
    async fn unsubscribe(
        &self,
        Parameters(args): Parameters<UnsubscribeArgs>,
    ) -> Result<Json<UnsubscribeResult>, String> {
        debug!("unsubscribe {}", args.connection_id);
        let mut streams = self.streams.lock().await;
        let was_active = streams.remove(&args.connection_id).is_some();
        info!(
            "unsubscribed {} (was_active={})",
            args.connection_id, was_active
        );
        Ok(Json(UnsubscribeResult {
            connection_id: args.connection_id,
            was_active,
        }))
    }

    #[tool(
        description = "Read bytes from a connection until a pattern matches or timeout. Pattern is interpreted with pattern_encoding (utf8/hex/base64). Returns the accumulated bytes (re-encoded with response_encoding) and the byte offset where the match started. Use for prompt/response interactions, e.g. send 'reset\\r\\n' then wait_for pattern='OK>'.",
        execution(task_support = "optional")
    )]
    async fn wait_for(
        &self,
        Parameters(args): Parameters<WaitForArgs>,
    ) -> Result<Json<WaitForResult>, String> {
        debug!(
            "wait_for {} pattern_encoding={} timeout={}ms max_bytes={}",
            args.connection_id, args.pattern_encoding, args.timeout_ms, args.max_bytes
        );
        let pattern_encoding = parse_encoding(&args.pattern_encoding)?;
        let response_encoding = parse_encoding(&args.response_encoding)?;
        let pattern = codec::decode(pattern_encoding, &args.pattern)
            .map_err(|e| format!("Pattern decoding failed - {e}"))?;
        if pattern.is_empty() {
            return Err("Pattern must not be empty".into());
        }

        let connection = self.lookup_connection(&args.connection_id).await?;
        let outcome =
            read_until_pattern(&connection, &pattern, args.timeout_ms, args.max_bytes).await?;
        let bytes_read = outcome.bytes.len();
        let data = codec::encode(response_encoding, &outcome.bytes)
            .map_err(|e| format!("Response encoding failed - {e}"))?;
        Ok(Json(WaitForResult {
            connection_id: args.connection_id,
            matched: outcome.match_index.is_some(),
            timed_out: outcome.timed_out,
            data,
            bytes_read,
            match_index: outcome.match_index,
            timeout_ms: args.timeout_ms,
            response_encoding: response_encoding.to_string(),
        }))
    }
}

// Lookup is split out so the macro-generated tool methods stay focused.
impl SerialHandler {
    /// Resolve an MCP connection id into a live [`SerialConnection`].
    async fn lookup_connection(&self, id: &str) -> Result<Arc<SerialConnection>, String> {
        self.connections
            .get(id)
            .await
            .map_err(|_| format!("Connection ID {id} not found"))
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
    URI_CONNECTION_PREFIX, URI_CONNECTION_RAW_TEMPLATE, URI_CONNECTION_TEMPLATE, URI_PORTS,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::Encoding;

    #[test]
    fn open_args_parsed_strictly() {
        let args = OpenArgs {
            port: "/dev/ttyUSB0".into(),
            baud_rate: 115200,
            data_bits: "8".into(),
            stop_bits: "1".into(),
            parity: "none".into(),
            flow_control: "none".into(),
        };
        let config = parse_open_args(args).unwrap();
        assert_eq!(config.port, "/dev/ttyUSB0");
        assert_eq!(config.baud_rate, 115200);
    }

    #[test]
    fn open_args_reject_invalid_data_bits() {
        let args = OpenArgs {
            port: "X".into(),
            baud_rate: 9600,
            data_bits: "9".into(),
            stop_bits: "1".into(),
            parity: "none".into(),
            flow_control: "none".into(),
        };
        let err = parse_open_args(args).unwrap_err();
        assert!(err.contains("data_bits"));
    }

    #[test]
    fn open_args_reject_invalid_parity() {
        assert!(parse_parity("weird").is_err());
        assert!(parse_parity("none").is_ok());
        assert!(parse_parity("Even").is_ok());
    }

    #[test]
    fn parse_encoding_rejects_garbage() {
        assert!(parse_encoding("rot13").is_err());
        assert!(parse_encoding("utf-8").is_ok());
    }

    #[test]
    fn build_read_result_timeout_branch() {
        let outcome = ReadOutcome {
            bytes: Vec::new(),
            timed_out: true,
        };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Utf8, Some(250))
            .expect("timeout result must build");
        assert!(result.timed_out);
        assert_eq!(result.bytes_read, 0);
        assert_eq!(result.timeout_ms, 250);
        assert!(result.data.is_empty());
    }

    #[test]
    fn build_read_result_timeout_uses_default() {
        let outcome = ReadOutcome {
            bytes: Vec::new(),
            timed_out: true,
        };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Hex, None)
            .expect("timeout result must build");
        assert_eq!(result.timeout_ms, DEFAULT_READ_TIMEOUT_MS);
    }

    #[test]
    fn build_read_result_data_branch_encodes_hex() {
        let outcome = ReadOutcome {
            bytes: b"Hi".to_vec(),
            timed_out: false,
        };
        let Json(result) = build_read_result(outcome, "abc".into(), Encoding::Hex, Some(500))
            .expect("data result must build");
        assert!(!result.timed_out);
        assert_eq!(result.bytes_read, 2);
        assert_eq!(result.encoding, "hex");
        assert_eq!(result.data, "48 69");
    }

    #[test]
    fn find_subslice_locates_pattern() {
        assert_eq!(find_subslice(b"hello OK> world", b"OK>"), Some(6));
        assert_eq!(find_subslice(b"OK>at-start", b"OK>"), Some(0));
        assert_eq!(find_subslice(b"trailing OK>", b"OK>"), Some(9));
    }

    #[test]
    fn find_subslice_missing_returns_none() {
        assert_eq!(find_subslice(b"hello world", b"OK>"), None);
        assert_eq!(find_subslice(b"", b"x"), None);
    }

    #[test]
    fn find_subslice_empty_needle_returns_none() {
        assert_eq!(find_subslice(b"hello", b""), None);
    }

    #[test]
    fn find_subslice_needle_longer_than_haystack() {
        assert_eq!(find_subslice(b"hi", b"hello"), None);
    }

    // ---- read_until_pattern integration with the loopback backend ----------

    use crate::serial::test_support::loopback_connection;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn read_until_pattern_matches_when_pattern_arrives() {
        let (conn, mut peer) = loopback_connection("test");
        let writer = tokio::spawn(async move {
            peer.write_all(b"junk before OK> and tail").await.unwrap();
        });
        let outcome = read_until_pattern(&conn, b"OK>", 1_000, 1024)
            .await
            .unwrap();
        writer.await.unwrap();
        assert_eq!(outcome.match_index, Some(12));
        assert!(!outcome.timed_out);
        assert!(outcome.bytes.starts_with(b"junk before OK>"));
    }

    #[tokio::test]
    async fn read_until_pattern_times_out_with_no_match() {
        let (conn, mut peer) = loopback_connection("test");
        peer.write_all(b"only noise here").await.unwrap();
        let outcome = read_until_pattern(&conn, b"OK>", 60, 1024).await.unwrap();
        assert!(outcome.timed_out);
        assert!(outcome.match_index.is_none());
        assert!(outcome.bytes.windows(3).all(|w| w != b"OK>"));
    }

    #[test]
    fn resource_uri_known_targets() {
        assert_eq!(parse_resource_uri("serial://ports"), ResourceUriKind::Ports);
        assert_eq!(
            parse_resource_uri("serial://connections"),
            ResourceUriKind::ConnectionsList
        );
        assert_eq!(
            parse_resource_uri("serial://connections/abc-123"),
            ResourceUriKind::ConnectionDetail("abc-123".into())
        );
    }

    #[test]
    fn resource_uri_unknown_targets() {
        assert_eq!(
            parse_resource_uri("serial://other"),
            ResourceUriKind::Unknown
        );
        assert_eq!(
            parse_resource_uri("serial://connections/"),
            ResourceUriKind::Unknown
        );
        assert_eq!(
            parse_resource_uri("serial://connections/abc/extra"),
            ResourceUriKind::Unknown
        );
        assert_eq!(
            parse_resource_uri("https://example.com"),
            ResourceUriKind::Unknown
        );
    }

    #[test]
    fn prompt_router_advertises_both_prompts() {
        let router = SerialHandler::prompt_router();
        assert!(router.has_route("diagnose_port"));
        assert!(router.has_route("interactive_terminal"));
        assert_eq!(router.list_all().len(), 2);
    }

    #[tokio::test]
    async fn read_until_pattern_stops_at_max_bytes() {
        let (conn, mut peer) = loopback_connection("test");
        let blob = vec![b'.'; 600];
        peer.write_all(&blob).await.unwrap();
        let outcome = read_until_pattern(&conn, b"OK>", 1_000, 256).await.unwrap();
        assert!(!outcome.timed_out);
        assert!(outcome.match_index.is_none());
        assert_eq!(outcome.bytes.len(), 256);
    }
}
