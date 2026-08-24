use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{transport::stdio, ServiceExt};
use serial_mcp::buffer_budget::AtomicBudget;
use serial_mcp::capture_store::{CaptureLimits, CaptureStore};
use serial_mcp::limits::{
    DEFAULT_CAPTURE_MAX_FILES, DEFAULT_CAPTURE_MAX_FILE_BYTES, DEFAULT_CAPTURE_MAX_TOTAL_BYTES,
    DEFAULT_MAX_PROGRAM_BUFFERED_BYTES, DEFAULT_MAX_TOOL_BUFFERED_BYTES,
};
use serial_mcp::resource_events::{PortWatcher, ResourceEventHub, PORT_WATCHER_INTERVAL};
use serial_mcp::rx_session::RxSessionManager;
use serial_mcp::security::SecurityManager;
use serial_mcp::serial::{ConnectionManager, PortProvider, SystemPortProvider};
use serial_mcp::SerialHandler;
use std::path::PathBuf;
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
    profiles_path: Option<PathBuf>,
    capture_dir: Option<PathBuf>,
    capture_limits: CaptureLimits,
}

enum Transport {
    Stdio,
    Http,
}

fn version_string() -> String {
    format!(
        "serial-mcp {} ({}, {})\n",
        env!("CARGO_PKG_VERSION"),
        option_env!("GIT_HASH").unwrap_or("unknown"),
        option_env!("BUILD_TARGET").unwrap_or("unknown"),
    )
}

fn print_version_and_exit() {
    print!("{}", version_string());
    std::process::exit(0);
}

/// Options that consume the following token as their value. A `--version`
/// token immediately after one is that option's value, not a version request.
/// Cross-reference: this list must stay in sync with the
/// `opt_value_from_str("--<opt>")` calls in `parse_args` below. Adding a
/// value-taking option there without adding it here lets `--version`
/// detection drift, so `--<new-opt> --version` could be misread as a version
/// request. Removing one here without removing the call has the inverse effect.
const VALUE_TAKING_OPTIONS: &[&str] = &[
    "--transport",
    "--allowlist",
    "--bind",
    "--max-program-buffered-bytes",
    "--max-tool-buffered-bytes",
    "--profiles-path",
    "--capture-dir",
    "--capture-max-file-bytes",
    "--capture-max-total-bytes",
    "--capture-max-files",
];

/// Scan argv for a version flag (`-V` / `--version`) that is not in the value
/// position of a preceding value-taking option and not after a `--` separator.
/// A token like `--bind --version` means `--version` is the value of `--bind`,
/// not a version request.
#[allow(clippy::while_let_on_iterator)] // needs manual next() to skip option values
fn argv_has_version_flag() -> bool {
    let mut args = std::env::args().skip(1);
    let mut expect_value = false;
    while let Some(arg) = args.next() {
        if expect_value {
            // This token is the value of the previous option, not a flag.
            expect_value = false;
            continue;
        }
        if arg == "--" {
            // Everything after `--` is positional, not a flag.
            return false;
        }
        // `--opt=value` form: the value is embedded, so the next token is not
        // consumed as a value. Only the bare `--opt` form sets `expect_value`.
        let is_bare_value_taking = VALUE_TAKING_OPTIONS.iter().any(|opt| arg == *opt);
        if is_bare_value_taking {
            expect_value = true;
            continue;
        }
        if arg == "-V" || arg == "--version" {
            return true;
        }
    }
    false
}

fn parse_args() -> Result<Args, pico_args::Error> {
    // Short-circuit version requests before parsing so pargs.finish() does not
    // reject them as unexpected arguments.
    //
    // Scan argv with value-position awareness. A token is treated as a version
    // flag only when it is not the value of a preceding value-taking option
    // and not after a `--` separator. This prevents
    // `serial-mcp --bind --version` from printing the version instead of
    // erroring (`--version` is the value of `--bind`).
    if argv_has_version_flag() {
        print_version_and_exit();
    }
    if std::env::args().nth(1).as_deref() == Some("version") {
        print_version_and_exit();
    }

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
  --profiles-path <path>            Profile store file path
                                     (default: OS user config dir + serial-mcp/profiles.toml)
  --capture-dir <absolute-dir>      Enable persistent export_log capture into an existing
                                     absolute directory (disabled by default; no fallback)
  --capture-max-file-bytes <N>      Per-file quota for a capture JSONL snapshot
                                     (default: {cap_file})
  --capture-max-total-bytes <N>     Total-byte quota across committed capture files
                                     (default: {cap_total})
  --capture-max-files <N>           File-count quota across committed capture files
                                     (default: {cap_files})
  -V, --version                     Print version and exit
  -h, --help                        Print this help

Commands:
  version                           Print version and exit

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
            cap_file = DEFAULT_CAPTURE_MAX_FILE_BYTES,
            cap_total = DEFAULT_CAPTURE_MAX_TOTAL_BYTES,
            cap_files = DEFAULT_CAPTURE_MAX_FILES,
        );
        std::process::exit(0);
    }

    // Value-taking options parsed below. Cross-reference: every
    // `opt_value_from_str("--<opt>")` call here must have a matching entry in
    // `VALUE_TAKING_OPTIONS` above, or `argv_has_version_flag` will misclassify
    // a `--<opt> --version` invocation.
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

    let profiles_path: Option<std::path::PathBuf> = pargs.opt_value_from_str("--profiles-path")?;

    let capture_dir: Option<std::path::PathBuf> = pargs.opt_value_from_str("--capture-dir")?;
    let capture_max_file_bytes: Option<u64> =
        pargs.opt_value_from_str("--capture-max-file-bytes")?;
    let capture_max_total_bytes: Option<u64> =
        pargs.opt_value_from_str("--capture-max-total-bytes")?;
    let capture_max_files: Option<usize> = pargs.opt_value_from_str("--capture-max-files")?;

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

    // Capture quota options are meaningless without a capture root. An
    // explicitly supplied quota without `--capture-dir` is a startup error,
    // not a silent disable.
    let quotas_supplied = capture_max_file_bytes.is_some()
        || capture_max_total_bytes.is_some()
        || capture_max_files.is_some();
    if quotas_supplied && capture_dir.is_none() {
        eprintln!(
            "error: --capture-max-file-bytes/--capture-max-total-bytes/--capture-max-files \
             require --capture-dir"
        );
        std::process::exit(1);
    }
    let capture_limits = CaptureLimits {
        max_file_bytes: capture_max_file_bytes.unwrap_or(DEFAULT_CAPTURE_MAX_FILE_BYTES),
        max_total_bytes: capture_max_total_bytes.unwrap_or(DEFAULT_CAPTURE_MAX_TOTAL_BYTES),
        max_files: capture_max_files.unwrap_or(DEFAULT_CAPTURE_MAX_FILES),
    };
    // `CaptureStore::open` revalidates the same rules; checking here keeps
    // the CLI errors local to argument parsing.
    if capture_limits.max_file_bytes == 0
        || capture_limits.max_total_bytes == 0
        || capture_limits.max_files == 0
    {
        eprintln!("error: capture quota limits must all be > 0");
        std::process::exit(1);
    }
    if capture_limits.max_file_bytes > capture_limits.max_total_bytes {
        eprintln!(
            "error: --capture-max-file-bytes ({}) must be <= --capture-max-total-bytes ({})",
            capture_limits.max_file_bytes, capture_limits.max_total_bytes
        );
        std::process::exit(1);
    }

    Ok(Args {
        transport,
        allowlist,
        bind,
        max_program_buffered_bytes,
        max_tool_buffered_bytes,
        profiles_path,
        capture_dir,
        capture_limits,
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
    profile_store: Arc<serial_mcp::profile_store::ProfileStore>,
    capture_store: Arc<CaptureStore>,
    port_provider: Arc<dyn PortProvider>,
    hub: Arc<ResourceEventHub>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Starting Serial MCP Server v{}", env!("CARGO_PKG_VERSION"));
    let connections = Arc::new(ConnectionManager::new());
    // One process-wide RX session registry per server (ring + pump + shared
    // cursor): stateless HTTP creates a fresh handler per request, so every
    // handler must share the same manager.
    let rx_sessions = Arc::new(RxSessionManager::new(Arc::clone(&budget), Arc::clone(&hub)));
    let handler = SerialHandler::builder()
        .connections(connections)
        .security(security)
        .budget(budget)
        .profile_store(profile_store)
        .capture_store(capture_store)
        .port_provider(Arc::clone(&port_provider))
        .resource_events(Arc::clone(&hub))
        .rx_sessions(rx_sessions)
        .build();
    // One proactive port hotplug watcher per process, cancelled and joined
    // on server shutdown.
    let watcher = PortWatcher::start(
        port_provider,
        hub,
        tokio_util::sync::CancellationToken::new(),
        PORT_WATCHER_INTERVAL,
    );
    let service = handler.serve(stdio()).await.map_err(|e| {
        error!("Failed to start server: {:?}", e);
        e
    })?;
    info!("Serial MCP Server started");
    service.waiting().await?;
    watcher.shutdown_and_join().await;
    info!("Serial MCP Server stopped");
    Ok(())
}

async fn run_http(
    security: SecurityManager,
    bind: String,
    budget: Arc<dyn serial_mcp::buffer_budget::BufferBudget>,
    profile_store: Arc<serial_mcp::profile_store::ProfileStore>,
    capture_store: Arc<CaptureStore>,
    port_provider: Arc<dyn PortProvider>,
    hub: Arc<ResourceEventHub>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!(
        "Starting Serial MCP Server (HTTP) v{} on http://{}{}",
        env!("CARGO_PKG_VERSION"),
        bind,
        MOUNT_PATH
    );

    let shutdown = tokio_util::sync::CancellationToken::new();
    let manager = Arc::new(ConnectionManager::new());
    // One process-wide RX session registry per server (ring + pump + shared
    // cursor): cloned into every stateless HTTP handler factory so
    // sequential reads share ring/cursor state across requests.
    let rx_sessions = Arc::new(RxSessionManager::new(Arc::clone(&budget), Arc::clone(&hub)));
    let manager_for_service = Arc::clone(&manager);
    let budget_for_service = Arc::clone(&budget);
    let profile_store_for_service = Arc::clone(&profile_store);
    let capture_store_for_service = Arc::clone(&capture_store);
    let port_provider_for_service = Arc::clone(&port_provider);
    let hub_for_service = Arc::clone(&hub);
    let rx_sessions_for_service = Arc::clone(&rx_sessions);

    let service = StreamableHttpService::new(
        move || {
            Ok(SerialHandler::builder()
                .connections(Arc::clone(&manager_for_service))
                .security(security.clone())
                .budget(Arc::clone(&budget_for_service))
                .profile_store(Arc::clone(&profile_store_for_service))
                .capture_store(Arc::clone(&capture_store_for_service))
                .port_provider(Arc::clone(&port_provider_for_service))
                .resource_events(Arc::clone(&hub_for_service))
                .rx_sessions(Arc::clone(&rx_sessions_for_service))
                .build())
        },
        LocalSessionManager::default().into(),
        // Exact protocol policy: `2026-07-28` modern stateless lifecycle
        // plus `2025-11-25` initialized sessions. Strict validation
        // (`stateless_protocol_metadata_required`) rejects statelessly routed
        // modern requests missing required protocol signals. A request
        // carrying modern per-request `_meta` but no `MCP-Protocol-Version`
        // header gets HTTP 400 / JSON-RPC -32020 before tool dispatch. Valid
        // initialized `2025-11-25` sessions remain supported. Requests
        // missing both signals are classified legacy by rmcp and rejected
        // earlier (HTTP 422), never reaching this validator.
        StreamableHttpServerConfig::default()
            .with_cancellation_token(shutdown.child_token())
            .with_stateless_protocol_metadata_required(true),
    );

    // One proactive port hotplug watcher per process, sharing the same
    // SystemPortProvider and hub as every handler instance.
    let watcher = PortWatcher::start(
        port_provider,
        hub,
        shutdown.child_token(),
        PORT_WATCHER_INTERVAL,
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

    // Cancel and deterministically join the watcher on server shutdown.
    watcher.shutdown_and_join().await;

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

    // Resolve the profile store path. Without --profiles-path the OS user
    // config path is the default; an unavailable config directory is a
    // startup error (no silent cwd fallback). Invalid persistent data
    // (corrupt file, unsupported future schema version) also fails startup,
    // never an empty store.
    let profiles_path = match args.profiles_path {
        Some(p) => p,
        None => serial_mcp::profiles::default_profiles_path().unwrap_or_else(|e| {
            eprintln!("error: {e}");
            std::process::exit(1);
        }),
    };
    let profile_store = Arc::new(
        serial_mcp::profile_store::ProfileStore::open(profiles_path.clone()).unwrap_or_else(|e| {
            eprintln!(
                "error: failed to load profiles from {}: {e}",
                profiles_path.display()
            );
            std::process::exit(1);
        }),
    );
    info!("Profiles store: {}", profiles_path.display());

    // Persistent capture is disabled unless an explicit absolute capture
    // directory was configured. `CaptureStore::open` validates the root
    // (absolute, existing directory, not a symlink, working advisory lock)
    // and the quota relation at startup.
    let capture_store = match &args.capture_dir {
        Some(dir) => Arc::new(
            CaptureStore::open(dir.clone(), args.capture_limits).unwrap_or_else(|e| {
                eprintln!("error: {e}");
                std::process::exit(1);
            }),
        ),
        None => {
            info!("Persistent capture disabled (no --capture-dir)");
            Arc::new(CaptureStore::disabled())
        }
    };
    if let Some(root) = capture_store.root() {
        info!(
            "Capture store: {} (file={} total={} files={})",
            root.display(),
            args.capture_limits.max_file_bytes,
            args.capture_limits.max_total_bytes,
            args.capture_limits.max_files,
        );
    }

    // One shared port provider and one process-wide resource event hub per
    // server process: cloned into every handler factory and the port
    // watcher in both transports.
    let port_provider: Arc<dyn PortProvider> = Arc::new(SystemPortProvider);
    let hub = Arc::new(ResourceEventHub::default());

    match args.transport {
        Transport::Http => {
            run_http(
                security,
                args.bind,
                budget,
                profile_store,
                capture_store,
                port_provider,
                hub,
            )
            .await
        }
        Transport::Stdio => {
            run_stdio(
                security,
                budget,
                profile_store,
                capture_store,
                port_provider,
                hub,
            )
            .await
        }
    }
}
