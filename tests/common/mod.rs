//! Shared test harness for HTTP integration tests.
//!
//! Spins up the real `SerialHandler` behind an `axum` server on an
//! auto-assigned port and connects to it with the real `rmcp` HTTP client
//! transport. The harness optionally pre-populates the
//! [`ConnectionManager`] with an in-memory loopback connection so the
//! HTTP surface can be exercised end-to-end without an OS-level serial
//! port.

#![allow(dead_code)]

pub mod binaries;
pub mod controlled;
pub mod firmware;
pub mod spawned;

/// Absolute path to the `serial-mcp` workspace root.
///
/// Resolved at first call by reading `CARGO_MANIFEST_DIR` (always
/// populated by cargo when running tests) and walking up to the
/// directory that contains `Cargo.toml`.
pub fn workspace_root() -> &'static std::path::PathBuf {
    static WORKSPACE_ROOT: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    WORKSPACE_ROOT.get_or_init(|| {
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        debug_assert!(
            manifest.join("Cargo.toml").is_file(),
            "CARGO_MANIFEST_DIR does not point at a Cargo workspace root: {}",
            manifest.display()
        );
        manifest
    })
}

/// Explicit expected tool list shared by the HTTP and stdio transport
/// tests. Kept independent of the production `tool_catalog` so the
/// transport tests verify the actual wire surface.
pub const EXPECTED_TOOLS: &[&str] = &[
    "list_ports",
    "list_connections",
    "open",
    "close",
    "write",
    "transact",
    "read",
    "capture_boot",
    "flush",
    "set_dtr_rts",
    "set_flow_control",
    "send_break",
    "subscribe",
    "unsubscribe",
    "get_status",
    "reconfigure",
    "list_profiles",
    "open_profile",
    "save_profile",
    "delete_profile",
    "configure",
    "rollback_profile",
    "get_log",
    "clear_log",
    "export_log",
    "reconnect",
    "compute_checksum",
];

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use rmcp::handler::client::ClientHandler;
use rmcp::model::{
    CallToolRequestParams, LoggingMessageNotificationParam, ProgressNotificationParam,
};
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::Map;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use serial_mcp::capture_store::CaptureStore;
use serial_mcp::security::SecurityManager;
use serial_mcp::serial::ConnectionManager;
use serial_mcp::serial::PortProvider;
use serial_mcp::server::StreamRegistry;
use serial_mcp::SerialHandler;

/// Static [`PortProvider`] used by Phase 3A tests: returns a fixed list of
/// ports. `PortInfo.name` typically points at a real PTY slave so the full
/// public `open` path and real serial I/O run while identity fields
/// describe a synthetic USB device.
#[derive(Debug, Clone)]
pub struct StaticPortProvider {
    pub ports: Vec<serial_mcp::serial::PortInfo>,
}

impl PortProvider for StaticPortProvider {
    fn list_available(&self) -> serial_mcp::error::Result<Vec<serial_mcp::serial::PortInfo>> {
        Ok(self.ports.clone())
    }
}

impl StaticPortProvider {
    pub fn new(ports: Vec<serial_mcp::serial::PortInfo>) -> Arc<Self> {
        Arc::new(Self { ports })
    }

    /// Build a synthetic high-confidence USB `PortInfo` pointing at a real
    /// port path (a PTY slave in tests).
    pub fn usb_port(
        name: &str,
        vid: u16,
        pid: u16,
        serial: &str,
        product: Option<&str>,
        interface: Option<u8>,
    ) -> serial_mcp::serial::PortInfo {
        serial_mcp::serial::PortInfo {
            name: name.into(),
            display_name: name.rsplit('/').next().unwrap_or(name).into(),
            description: "Synthetic USB device".into(),
            hardware_id: Some(format!("USB VID:{vid:04X} PID:{pid:04X}")),
            transport: serial_mcp::serial::PortTransport::Usb,
            vid: Some(vid),
            pid: Some(pid),
            serial_number: Some(serial.into()),
            manufacturer: Some("Synthetic".into()),
            product: product.map(str::to_string),
            interface,
        }
    }

    /// Build a weak-identity `PortInfo` (no USB identity) pointing at a
    /// real port path.
    pub fn weak_port(name: &str) -> serial_mcp::serial::PortInfo {
        serial_mcp::serial::PortInfo {
            name: name.into(),
            display_name: name.rsplit('/').next().unwrap_or(name).into(),
            description: "PTY".into(),
            hardware_id: None,
            transport: serial_mcp::serial::PortTransport::Unknown,
            vid: None,
            pid: None,
            serial_number: None,
            manufacturer: None,
            product: None,
            interface: None,
        }
    }
}

/// In-process HTTP MCP server bound to `127.0.0.1` on an OS-assigned
/// port. The shared [`ConnectionManager`] is exposed so tests can insert
/// in-memory connections before the client connects.
pub struct TestServer {
    pub url: String,
    pub manager: Arc<ConnectionManager>,
    shutdown: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
}

impl TestServer {
    /// Start a server with a fresh empty [`ConnectionManager`].
    pub async fn start() -> Self {
        Self::builder(Arc::new(ConnectionManager::new()))
            .start()
            .await
    }

    /// Start a server reusing a caller-supplied [`ConnectionManager`].
    /// Useful when the test wants to insert a loopback connection before
    /// the server is up.
    pub async fn start_with(manager: Arc<ConnectionManager>) -> Self {
        Self::builder(manager).start().await
    }

    /// Begin building a test server around a caller-supplied
    /// [`ConnectionManager`]. See [`TestServerBuilder`] for the defaults
    /// and injectable dependencies.
    pub fn builder(manager: Arc<ConnectionManager>) -> TestServerBuilder {
        TestServerBuilder {
            manager,
            security: SecurityManager::from_patterns::<[&str; 0]>([]),
            profile_store: None,
            provider: None,
            capture_store: None,
        }
    }

    /// Shared construction: one `Arc<ProfileStore>` per server, cloned into
    /// every session handler factory so all HTTP MCP sessions observe the
    /// same profile state. `None` selects the ephemeral store default. An
    /// injected `provider` (defaults to the system provider) is shared the
    /// same way.
    async fn start_inner(
        manager: Arc<ConnectionManager>,
        security: SecurityManager,
        profile_store: Option<Arc<serial_mcp::profile_store::ProfileStore>>,
        provider: Option<Arc<dyn PortProvider>>,
        capture_store: Option<Arc<CaptureStore>>,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/mcp");
        let shutdown = CancellationToken::new();

        let streams: StreamRegistry = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let profile_store = profile_store
            .unwrap_or_else(|| Arc::new(serial_mcp::profile_store::ProfileStore::ephemeral()));
        let provider = provider.unwrap_or_else(|| {
            Arc::new(serial_mcp::serial::SystemPortProvider) as Arc<dyn PortProvider>
        });
        let capture_store = capture_store.unwrap_or_else(|| Arc::new(CaptureStore::disabled()));
        let manager_for_service = Arc::clone(&manager);
        let streams_for_service = Arc::clone(&streams);
        let profile_store_for_service = Arc::clone(&profile_store);
        let provider_for_service = Arc::clone(&provider);
        let capture_store_for_service = Arc::clone(&capture_store);
        let shutdown_for_service = shutdown.child_token();
        let service = StreamableHttpService::new(
            move || {
                Ok(SerialHandler::builder()
                    .connections(Arc::clone(&manager_for_service))
                    .streams(Arc::clone(&streams_for_service))
                    .security(security.clone())
                    .profile_store(Arc::clone(&profile_store_for_service))
                    .capture_store(Arc::clone(&capture_store_for_service))
                    .port_provider(Arc::clone(&provider_for_service))
                    .build())
            },
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default().with_cancellation_token(shutdown_for_service),
        );
        let router = axum::Router::new().nest_service("/mcp", service);

        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        TestServer {
            url,
            manager,
            shutdown,
            handle,
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.handle.abort();
    }
}

/// Test-only builder for [`TestServer`] with injectable dependencies.
///
/// Defaults, all overridable via the builder methods:
/// - caller-supplied [`ConnectionManager`] (constructor argument)
/// - empty-allowlist [`SecurityManager`]
/// - ephemeral profile store
/// - system port provider
/// - disabled [`CaptureStore`]
///
/// The profile store opened from a [`TestServerBuilder::profiles_path`]
/// lives for the server's lifetime, exactly like production startup.
pub struct TestServerBuilder {
    manager: Arc<ConnectionManager>,
    security: SecurityManager,
    profile_store: Option<Arc<serial_mcp::profile_store::ProfileStore>>,
    provider: Option<Arc<dyn PortProvider>>,
    capture_store: Option<Arc<CaptureStore>>,
}

impl TestServerBuilder {
    /// Inject a custom [`SecurityManager`]; its allowlist will govern
    /// `open` calls during the test. Defaults to an empty allowlist.
    pub fn security(mut self, security: SecurityManager) -> Self {
        self.security = security;
        self
    }

    /// Use the profile store at `profiles_path` (for tests that exercise
    /// `configure`/`save_profile`/`delete_profile` without polluting the
    /// user's real `$XDG_CONFIG_HOME/serial-mcp/profiles.toml`) instead of
    /// the ephemeral default. A pre-written file (legacy migration tests,
    /// restart tests) is loaded and validated like production startup
    /// would. The caller owns the tempdir and must keep it alive for the
    /// test's duration.
    pub fn profiles_path(mut self, profiles_path: std::path::PathBuf) -> Self {
        let store = Arc::new(
            serial_mcp::profile_store::ProfileStore::open(profiles_path)
                .expect("open profiles store for test server"),
        );
        self.profile_store = Some(store);
        self
    }

    /// Inject a static port provider (Phase 3A: synthetic USB identity over
    /// a real PTY slave path) instead of the system provider default.
    pub fn port_provider(mut self, provider: Arc<dyn PortProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Inject a capture store (Phase 6 export_log tests) instead of the
    /// disabled default.
    pub fn capture_store(mut self, store: Arc<CaptureStore>) -> Self {
        self.capture_store = Some(store);
        self
    }

    /// Build and start the server.
    pub async fn start(self) -> TestServer {
        TestServer::start_inner(
            self.manager,
            self.security,
            self.profile_store,
            self.provider,
            self.capture_store,
        )
        .await
    }
}

/// [`ClientHandler`] that forwards every received `notifications/message`
/// onto an unbounded mpsc channel. The receiver half is returned from
/// [`connect_client`] so tests can await events.
#[derive(Clone)]
pub struct NotificationCollector {
    tx: mpsc::UnboundedSender<LoggingMessageNotificationParam>,
}

impl ClientHandler for NotificationCollector {
    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _ctx: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let tx = self.tx.clone();
        async move {
            let _ = tx.send(params);
        }
    }
}

/// Connect an `rmcp` HTTP client to the given test server. Returns the
/// running client service plus the receiving end of the notification
/// collector.
pub async fn connect_client(
    server: &TestServer,
) -> Result<(
    RunningService<RoleClient, NotificationCollector>,
    mpsc::UnboundedReceiver<LoggingMessageNotificationParam>,
)> {
    connect_to_url(server.url.as_str()).await
}

/// Connect an `rmcp` HTTP client to a server URL (in-process or
/// spawned-binary). Returns the running client service plus the
/// receiving end of the notification collector.
pub async fn connect_to_url(
    url: &str,
) -> Result<(
    RunningService<RoleClient, NotificationCollector>,
    mpsc::UnboundedReceiver<LoggingMessageNotificationParam>,
)> {
    let (tx, rx) = mpsc::unbounded_channel();
    let handler = NotificationCollector { tx };
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = handler.serve(transport).await?;
    Ok((client, rx))
}

#[derive(Clone)]
pub struct ProgressNotificationCollector {
    log_tx: mpsc::UnboundedSender<LoggingMessageNotificationParam>,
    progress_tx: mpsc::UnboundedSender<ProgressNotificationParam>,
}

impl ClientHandler for ProgressNotificationCollector {
    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _ctx: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let tx = self.log_tx.clone();
        async move {
            let _ = tx.send(params);
        }
    }

    fn on_progress(
        &self,
        params: ProgressNotificationParam,
        _ctx: NotificationContext<RoleClient>,
    ) -> impl Future<Output = ()> + Send + '_ {
        let tx = self.progress_tx.clone();
        async move {
            let _ = tx.send(params);
        }
    }
}

pub async fn connect_client_with_progress(
    server: &TestServer,
) -> Result<(
    RunningService<RoleClient, ProgressNotificationCollector>,
    mpsc::UnboundedReceiver<LoggingMessageNotificationParam>,
    mpsc::UnboundedReceiver<ProgressNotificationParam>,
)> {
    let (log_tx, log_rx) = mpsc::unbounded_channel();
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let handler = ProgressNotificationCollector {
        log_tx,
        progress_tx,
    };
    let transport = StreamableHttpClientTransport::from_uri(server.url.as_str());
    let client = handler.serve(transport).await?;
    Ok((client, log_rx, progress_rx))
}

/// Build a `CallToolRequestParams::arguments` JSON object from a
/// `serde_json::Value`. Panics if the value is not a JSON object.
pub fn args_object(value: serde_json::Value) -> Map<String, serde_json::Value> {
    value
        .as_object()
        .expect("args must serialize to a JSON object")
        .clone()
}

/// Convenience: build a tool-call request with named arguments.
pub fn tool_request(name: &'static str, args: serde_json::Value) -> CallToolRequestParams {
    CallToolRequestParams::new(name).with_arguments(args_object(args))
}

/// Receive the next notification from the collector with a timeout.
pub async fn next_notification(
    rx: &mut mpsc::UnboundedReceiver<LoggingMessageNotificationParam>,
    within: Duration,
) -> Result<LoggingMessageNotificationParam> {
    tokio::time::timeout(within, rx.recv())
        .await
        .map_err(|_| anyhow::anyhow!("no notification arrived within {within:?}"))?
        .ok_or_else(|| anyhow::anyhow!("notification channel closed"))
}

// ---- Unix PTY pair (Layer 3) ------------------------------------------------
//
// `openpty` on Linux/macOS gives back a master fd and a slave fd whose
// device path (`/dev/pts/N`) can be opened by `tokio_serial::SerialStream`
// exactly like a real serial port. The test holds the master and plays
// the role of the device.

#[cfg(unix)]
pub mod pty {
    use std::os::fd::OwnedFd;
    use std::path::PathBuf;

    use anyhow::{Context, Result};
    use nix::pty::{openpty, OpenptyResult};
    use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
    use nix::unistd::ttyname;
    use tokio::fs::File;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One half of a PTY pair: the master end (driven by the test) plus
    /// the slave path (opened by the server via `tokio_serial`).
    pub struct PtyPair {
        pub slave_path: PathBuf,
        master: File,
        // Kept alive until drop so the kernel doesn't reclaim the slave.
        _slave: OwnedFd,
    }

    impl PtyPair {
        pub fn open() -> Result<Self> {
            let OpenptyResult { master, slave } = openpty(None, None).context("openpty failed")?;
            // Put the slave in raw mode so newlines / echo / etc. don't
            // mangle the byte stream — the server expects a serial port.
            let mut termios = tcgetattr(&slave).context("tcgetattr")?;
            cfmakeraw(&mut termios);
            tcsetattr(&slave, SetArg::TCSANOW, &termios).context("tcsetattr")?;

            let slave_path = ttyname(&slave).context("ttyname")?;
            let master_std = std::fs::File::from(master);
            let master = File::from_std(master_std);
            Ok(PtyPair {
                slave_path,
                master,
                _slave: slave,
            })
        }

        pub async fn write_device(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            self.master.write_all(bytes).await?;
            self.master.flush().await
        }

        pub async fn read_device(&mut self, dst: &mut [u8]) -> std::io::Result<usize> {
            self.master.read(dst).await
        }

        /// Read exactly `dst.len()` bytes from the device side or error.
        pub async fn read_device_exact(&mut self, dst: &mut [u8]) -> std::io::Result<()> {
            self.master.read_exact(dst).await.map(|_| ())
        }

        /// Split the pair into its master file and slave fd so the
        /// test can move the master into a spawned emulator task whilst
        /// keeping the slave alive.
        pub fn into_parts(self) -> (File, OwnedFd) {
            (self.master, self._slave)
        }
    }
}
