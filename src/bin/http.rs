//! Serial MCP Server with the streamable-HTTP transport.
//!
//! Same handler as the stdio binary, but reachable over HTTP on
//! `${SERIAL_MCP_HTTP_BIND:-127.0.0.1:8000}/mcp`. Useful for running the
//! server in a container alongside the USB-serial dongle while a remote
//! MCP client connects over the network.

use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use std::collections::HashMap;
use std::sync::Arc;

use serial_mcp_server::security::SecurityManager;
use serial_mcp_server::serial::ConnectionManager;
use serial_mcp_server::server::StreamRegistry;
use serial_mcp_server::SerialHandler;

const DEFAULT_BIND: &str = "127.0.0.1:8000";
const ENV_BIND: &str = "SERIAL_MCP_HTTP_BIND";
const MOUNT_PATH: &str = "/mcp";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_target(true)
        .init();

    let bind: String = std::env::var(ENV_BIND).unwrap_or_else(|_| DEFAULT_BIND.to_string());
    info!(
        "Starting Serial MCP Server (HTTP) v{} on http://{}{}",
        env!("CARGO_PKG_VERSION"),
        bind,
        MOUNT_PATH
    );

    let shutdown = tokio_util::sync::CancellationToken::new();

    // Share a single ConnectionManager across HTTP sessions so that two
    // clients cannot independently open the same physical port.
    let manager = Arc::new(ConnectionManager::new());
    let streams: StreamRegistry = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let security = SecurityManager::from_env();
    let manager_for_service = Arc::clone(&manager);
    let streams_for_service = Arc::clone(&streams);

    let service = StreamableHttpService::new(
        move || {
            Ok(SerialHandler::with_manager_security_and_streams(
                Arc::clone(&manager_for_service),
                security.clone(),
                Arc::clone(&streams_for_service),
            ))
        },
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default().with_cancellation_token(shutdown.child_token()),
    );

    let router = axum::Router::new().nest_service(MOUNT_PATH, service);
    let listener = tokio::net::TcpListener::bind(&bind).await.map_err(|e| {
        error!("Failed to bind {}: {}", bind, e);
        e
    })?;

    let server_shutdown = shutdown.clone();
    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                info!("Ctrl-C received, shutting down");
            }
            server_shutdown.cancel();
        })
        .await?;

    info!("Serial MCP Server (HTTP) stopped");
    Ok(())
}
