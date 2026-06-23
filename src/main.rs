use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{transport::stdio, ServiceExt};
use serial_mcp::buffer_budget::AtomicBudget;
use serial_mcp::limits::{DEFAULT_MAX_PROGRAM_BUFFERED_BYTES, DEFAULT_MAX_TOOL_BUFFERED_BYTES};
use serial_mcp::security::SecurityManager;
use serial_mcp::serial::ConnectionManager;
use serial_mcp::server::StreamRegistry;
use serial_mcp::SerialHandler;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

const DEFAULT_HTTP_BIND: &str = "127.0.0.1:8000";
const MOUNT_PATH: &str = "/mcp";

struct Args {
    transport: Transport,
    allowlist: Vec<String>,
    bind: String,
    max_program_buffered_bytes: usize,
    max_tool_buffered_bytes: usize,
}

enum Transport {
    Stdio,
    Http,
}

fn parse_args() -> Result<Args, pico_args::Error> {
    let mut pargs = pico_args::Arguments::from_env();

    if pargs.contains(["-h", "--help"]) {
        print!(
            "serial-mcp {version}

Usage: serial-mcp [OPTIONS]

Options:
  --transport <stdio|http>          Transport to use (default: stdio)
  --allowlist <patterns>            Comma-separated glob patterns for allowed ports
                                     (default: allow all)
  --bind <addr>                     HTTP bind address (default: {bind})
  --max-program-buffered-bytes <N>  Global budget for all in-flight RX tools (default: {prog_default})
  --max-tool-buffered-bytes <N>     Per-tool ceiling for max_buffered_bytes (default: {tool_default})
  -h, --help                        Print this help

Environment:
  RUST_LOG                   Log level (error/warn/info/debug/trace)

Examples:
  serial-mcp --allowlist=/dev/ttyACM*,/dev/ttyUSB*
  serial-mcp --transport=http --bind=0.0.0.0:8000
  serial-mcp --max-tool-buffered-bytes=2097152
",
            version = env!("CARGO_PKG_VERSION"),
            bind = DEFAULT_HTTP_BIND,
            prog_default = DEFAULT_MAX_PROGRAM_BUFFERED_BYTES,
            tool_default = DEFAULT_MAX_TOOL_BUFFERED_BYTES,
        );
        std::process::exit(0);
    }

    let transport_str: Option<String> = pargs.opt_value_from_str("--transport")?;
    let transport = match transport_str.as_deref() {
        Some("http") => Transport::Http,
        Some("stdio") | None => Transport::Stdio,
        Some(other) => {
            eprintln!("error: unknown transport '{other}', expected 'stdio' or 'http'");
            std::process::exit(1);
        }
    };

    let allowlist_str: Option<String> = pargs.opt_value_from_str("--allowlist")?;
    let allowlist = allowlist_str
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let bind = pargs
        .opt_value_from_str("--bind")?
        .unwrap_or_else(|| DEFAULT_HTTP_BIND.to_string());

    let max_program_buffered_bytes: usize = pargs
        .opt_value_from_str("--max-program-buffered-bytes")?
        .unwrap_or(DEFAULT_MAX_PROGRAM_BUFFERED_BYTES);

    let max_tool_buffered_bytes: usize = pargs
        .opt_value_from_str("--max-tool-buffered-bytes")?
        .unwrap_or(DEFAULT_MAX_TOOL_BUFFERED_BYTES);

    let remaining = pargs.finish();
    if !remaining.is_empty() {
        eprintln!(
            "error: unexpected arguments: {}",
            remaining
                .iter()
                .map(|a| a.to_string_lossy())
                .collect::<Vec<_>>()
                .join(", ")
        );
        std::process::exit(1);
    }

    // Validate budget limits.
    if max_program_buffered_bytes == 0 {
        eprintln!("error: --max-program-buffered-bytes must be > 0");
        std::process::exit(1);
    }
    if max_tool_buffered_bytes == 0 {
        eprintln!("error: --max-tool-buffered-bytes must be > 0");
        std::process::exit(1);
    }
    if max_tool_buffered_bytes > max_program_buffered_bytes {
        eprintln!(
            "error: --max-tool-buffered-bytes ({max_tool_buffered_bytes}) must be <= --max-program-buffered-bytes ({max_program_buffered_bytes})"
        );
        std::process::exit(1);
    }

    Ok(Args {
        transport,
        allowlist,
        bind,
        max_program_buffered_bytes,
        max_tool_buffered_bytes,
    })
}

fn init_tracing() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .with_target(true)
        .init();
}

async fn run_stdio(
    security: SecurityManager,
    budget: Arc<dyn serial_mcp::buffer_budget::BufferBudget>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting Serial MCP Server v{}", env!("CARGO_PKG_VERSION"));
    let connections = Arc::new(ConnectionManager::new());
    let streams: StreamRegistry = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let handler = SerialHandler::builder()
        .connections(connections)
        .streams(streams)
        .security(security)
        .budget(budget)
        .build();
    let service = handler.serve(stdio()).await.map_err(|e| {
        error!("Failed to start server: {:?}", e);
        e
    })?;
    info!("Serial MCP Server started");
    service.waiting().await?;
    info!("Serial MCP Server stopped");
    Ok(())
}

async fn run_http(
    security: SecurityManager,
    bind: String,
    budget: Arc<dyn serial_mcp::buffer_budget::BufferBudget>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        "Starting Serial MCP Server (HTTP) v{} on http://{}{}",
        env!("CARGO_PKG_VERSION"),
        bind,
        MOUNT_PATH
    );

    let shutdown = tokio_util::sync::CancellationToken::new();
    let manager = Arc::new(ConnectionManager::new());
    let streams: StreamRegistry = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let manager_for_service = Arc::clone(&manager);
    let streams_for_service = Arc::clone(&streams);
    let budget_for_service = Arc::clone(&budget);

    let service = StreamableHttpService::new(
        move || {
            Ok(SerialHandler::builder()
                .connections(Arc::clone(&manager_for_service))
                .streams(Arc::clone(&streams_for_service))
                .security(security.clone())
                .budget(Arc::clone(&budget_for_service))
                .build())
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    init_tracing();

    let security = SecurityManager::from_patterns(&args.allowlist);
    let budget: Arc<dyn serial_mcp::buffer_budget::BufferBudget> = Arc::new(AtomicBudget::new(
        args.max_program_buffered_bytes,
        args.max_tool_buffered_bytes,
    ));

    info!(
        "Buffer budget: program={} tool={}",
        args.max_program_buffered_bytes, args.max_tool_buffered_bytes,
    );

    match args.transport {
        Transport::Http => run_http(security, args.bind, budget).await,
        Transport::Stdio => run_stdio(security, budget).await,
    }
}
