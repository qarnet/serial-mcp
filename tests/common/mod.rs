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
#[cfg(unix)]
pub mod device_fixture;
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

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::Result;
use rmcp::handler::client::ClientHandler;
use rmcp::model::{CallToolRequestParams, ProgressNotificationParam, ProtocolVersion};
use rmcp::service::{ClientLifecycleMode, NotificationContext, RoleClient, RunningService};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::{ClientServiceExt, ServiceExt};
use serde_json::Map;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use serial_mcp::capture_store::CaptureStore;
use serial_mcp::resource_events::{PortWatcher, ResourceEventHub};
use serial_mcp::security::SecurityManager;
use serial_mcp::serial::ConnectionManager;
use serial_mcp::serial::PortProvider;
use serial_mcp::SerialHandler;

/// Static [`PortProvider`]: returns a fixed list of ports. Linux-only
/// production-path fixture tests use a `PortInfo.name` that points at a real
/// PTY slave, while identity fields describe a synthetic USB device.
/// Controlled-backend tests use the same provider without OS serial I/O.
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

/// Mutable [`PortProvider`] for hotplug-watcher tests: the test swaps the
/// snapshot and injects enumeration failures while the watcher polls.
pub struct MutexPortProvider {
    ports: StdMutex<Vec<serial_mcp::serial::PortInfo>>,
    fail: AtomicBool,
}

impl PortProvider for MutexPortProvider {
    fn list_available(&self) -> serial_mcp::error::Result<Vec<serial_mcp::serial::PortInfo>> {
        if self.fail.load(Ordering::SeqCst) {
            Err(serial_mcp::error::SerialError::IoError(
                std::io::Error::other("injected enumeration failure"),
            ))
        } else {
            Ok(self.ports.lock().expect("ports mutex poisoned").clone())
        }
    }
}

impl MutexPortProvider {
    pub fn new(ports: Vec<serial_mcp::serial::PortInfo>) -> Arc<Self> {
        Arc::new(Self {
            ports: StdMutex::new(ports),
            fail: AtomicBool::new(false),
        })
    }

    pub fn set_ports(&self, ports: Vec<serial_mcp::serial::PortInfo>) {
        *self.ports.lock().expect("ports mutex poisoned") = ports;
    }

    pub fn set_fail(&self, fail: bool) {
        self.fail.store(fail, Ordering::SeqCst);
    }
}

/// In-process HTTP MCP server bound to `127.0.0.1` on an OS-assigned
/// port. The shared [`ConnectionManager`] is exposed so tests can insert
/// in-memory connections before the client connects.
pub struct TestServer {
    pub url: String,
    pub manager: Arc<ConnectionManager>,
    /// The process-wide resource event hub shared by every handler instance
    /// (and the optional port watcher). Tests may publish directly through
    /// it (e.g. to force broadcast lag).
    pub hub: Arc<ResourceEventHub>,
    shutdown: CancellationToken,
    handle: tokio::task::JoinHandle<()>,
    watcher: Option<PortWatcher>,
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
            resource_hub: None,
            port_watcher_interval: None,
        }
    }

    /// Shared construction: one `Arc<ProfileStore>` per server, cloned into
    /// every session handler factory so all HTTP MCP sessions observe the
    /// same profile state. `None` selects the ephemeral store default. An
    /// injected `provider` (defaults to the system provider) is shared the
    /// same way. One `Arc<ResourceEventHub>` per server is shared by every
    /// handler instance and the optional port watcher, exactly like
    /// production `main.rs`.
    async fn start_inner(
        manager: Arc<ConnectionManager>,
        security: SecurityManager,
        profile_store: Option<Arc<serial_mcp::profile_store::ProfileStore>>,
        provider: Option<Arc<dyn PortProvider>>,
        capture_store: Option<Arc<CaptureStore>>,
        resource_hub: Option<Arc<ResourceEventHub>>,
        port_watcher_interval: Option<Duration>,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/mcp");
        let shutdown = CancellationToken::new();

        let profile_store = profile_store
            .unwrap_or_else(|| Arc::new(serial_mcp::profile_store::ProfileStore::ephemeral()));
        let provider = provider.unwrap_or_else(|| {
            Arc::new(serial_mcp::serial::SystemPortProvider) as Arc<dyn PortProvider>
        });
        let capture_store = capture_store.unwrap_or_else(|| Arc::new(CaptureStore::disabled()));
        let hub = resource_hub.unwrap_or_else(|| Arc::new(ResourceEventHub::default()));
        // ONE process-wide RX session registry per in-process server (ring +
        // pump + shared cursor): cloned into every stateless HTTP handler
        // factory so sequential modern reads share ring/cursor state.
        let rx_sessions = Arc::new(serial_mcp::rx_session::RxSessionManager::new(
            Arc::new(serial_mcp::buffer_budget::AtomicBudget::new(
                1 << 30,
                1 << 30,
            )),
            Arc::clone(&hub),
        ));
        let manager_for_service = Arc::clone(&manager);
        let profile_store_for_service = Arc::clone(&profile_store);
        let provider_for_service = Arc::clone(&provider);
        let capture_store_for_service = Arc::clone(&capture_store);
        let hub_for_service = Arc::clone(&hub);
        let rx_sessions_for_service = Arc::clone(&rx_sessions);
        let shutdown_for_service = shutdown.child_token();
        let service = StreamableHttpService::new(
            move || {
                Ok(SerialHandler::builder()
                    .connections(Arc::clone(&manager_for_service))
                    .security(security.clone())
                    .profile_store(Arc::clone(&profile_store_for_service))
                    .capture_store(Arc::clone(&capture_store_for_service))
                    .port_provider(Arc::clone(&provider_for_service))
                    .resource_events(Arc::clone(&hub_for_service))
                    .rx_sessions(Arc::clone(&rx_sessions_for_service))
                    .build())
            },
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default().with_cancellation_token(shutdown_for_service),
        );
        let router = axum::Router::new().nest_service("/mcp", service);

        // Optional proactive port watcher sharing the injected provider and
        // the SAME hub as every handler instance (short interval in tests).
        let watcher = port_watcher_interval.map(|interval| {
            PortWatcher::start(
                Arc::clone(&provider),
                Arc::clone(&hub),
                shutdown.child_token(),
                interval,
            )
        });

        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });

        TestServer {
            url,
            manager,
            hub,
            shutdown,
            handle,
            watcher,
        }
    }

    /// Stop the test server and await its task after aborting the listener.
    ///
    /// Repeat gates use this explicit path so every iteration proves fixture,
    /// client, and server teardown completes.
    pub async fn shutdown_and_join(mut self) {
        self.shutdown.cancel();
        if let Some(watcher) = self.watcher.take() {
            watcher.shutdown_and_join().await;
        }
        self.handle.abort();
        let _ = (&mut self.handle).await;
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.handle.abort();
        if let Some(watcher) = self.watcher.take() {
            watcher.shutdown();
            watcher.abort();
        }
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
    resource_hub: Option<Arc<ResourceEventHub>>,
    port_watcher_interval: Option<Duration>,
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

    /// Inject a static port provider (synthetic USB identity over
    /// a real PTY slave path) instead of the system provider default.
    pub fn port_provider(mut self, provider: Arc<dyn PortProvider>) -> Self {
        self.provider = Some(provider);
        self
    }

    /// Inject a capture store (for `export_log` tests) instead of the
    /// disabled default.
    pub fn capture_store(mut self, store: Arc<CaptureStore>) -> Self {
        self.capture_store = Some(store);
        self
    }

    /// Inject the process-wide resource event hub (for lag tests that
    /// publish directly) instead of the fresh default.
    pub fn resource_hub(mut self, hub: Arc<ResourceEventHub>) -> Self {
        self.resource_hub = Some(hub);
        self
    }

    /// Start a proactive port hotplug watcher with the given poll interval
    /// (tests use a short interval; production uses one second).
    pub fn port_watcher_interval(mut self, interval: Duration) -> Self {
        self.port_watcher_interval = Some(interval);
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
            self.resource_hub,
            self.port_watcher_interval,
        )
        .await
    }
}

/// No-op [`ClientHandler`] used by the standard test client. Old
/// logging-message collection is gone with MCP logging removal; progress
/// notifications are collected only through the dedicated
/// [`ProgressNotificationCollector`] client.
#[derive(Clone, Default)]
pub struct TestClientHandler;

impl ClientHandler for TestClientHandler {}

/// One cloneable client handler that explicitly advertises one exact MCP
/// protocol version (instead of relying on rmcp's default,
/// `ProtocolVersion::LATEST`). One type serves both lifecycle modes so the
/// common connect helpers share a single return type. `get_info()` returns
/// default client info tagged with `self.protocol.version()` for every case.
#[derive(Clone)]
pub struct VersionedClientHandler {
    protocol: TestProtocol,
}

impl VersionedClientHandler {
    /// Build a handler advertising the given explicit protocol version.
    pub fn new(protocol: TestProtocol) -> Self {
        Self { protocol }
    }
}

impl ClientHandler for VersionedClientHandler {
    fn get_info(&self) -> rmcp::model::ClientInfo {
        rmcp::model::ClientInfo::default().with_protocol_version(self.protocol.version())
    }
}

/// Which exact MCP protocol version a typed test client negotiates.
///
/// Variant names carry the exact version date so a future protocol revision
/// cannot silently reclassify a case as "modern"/"legacy".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestProtocol {
    /// MCP `2026-07-28` discovery / stateless requests.
    V2026_07_28,
    /// MCP `2025-11-25` initialize / session requests.
    V2025_11_25,
}

impl TestProtocol {
    /// Every advertised version, in product-preferred order. The coverage
    /// lock in `tests/protocol_compatibility.rs` compares this list against
    /// the raw `server/discover` `supportedVersions` so a future production
    /// policy row requires an explicit test case.
    pub const ALL: [Self; 2] = [Self::V2026_07_28, Self::V2025_11_25];

    /// The exact rmcp protocol version constant for this case.
    pub fn version(self) -> ProtocolVersion {
        match self {
            TestProtocol::V2026_07_28 => ProtocolVersion::V_2026_07_28,
            TestProtocol::V2025_11_25 => ProtocolVersion::V_2025_11_25,
        }
    }

    /// The rmcp client lifecycle mode for this exact version:
    /// `2026-07-28` uses discovery with only that preferred version;
    /// `2025-11-25` uses the legacy initialize handshake.
    pub fn lifecycle(self) -> ClientLifecycleMode {
        match self {
            TestProtocol::V2026_07_28 => ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            },
            TestProtocol::V2025_11_25 => ClientLifecycleMode::Initialize,
        }
    }
}

/// Connect an `rmcp` HTTP client to the given test server. Returns the
/// running client service plus a unit receiver (kept for caller symmetry;
/// there are no logging-message notifications anymore).
pub async fn connect_client(
    server: &TestServer,
) -> Result<(RunningService<RoleClient, TestClientHandler>, ())> {
    connect_to_url(server.url.as_str()).await
}

/// Connect an `rmcp` HTTP client to a server URL (in-process or
/// spawned-binary). Returns the running client service plus a unit
/// receiver (kept for caller symmetry).
pub async fn connect_to_url(
    url: &str,
) -> Result<(RunningService<RoleClient, TestClientHandler>, ())> {
    let handler = TestClientHandler;
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = handler.serve(transport).await?;
    Ok((client, ()))
}

/// Connect an `rmcp` HTTP client to the given test server using the
/// explicit lifecycle for one exact protocol version. Returns the running
/// client service plus a unit receiver (kept for caller symmetry). One
/// return type covers every case: [`VersionedClientHandler`] advertises
/// `protocol.version()` for both lifecycle modes.
pub async fn connect_protocol_client(
    server: &TestServer,
    protocol: TestProtocol,
) -> Result<(RunningService<RoleClient, VersionedClientHandler>, ())> {
    connect_protocol_to_url(server.url.as_str(), protocol).await
}

/// Like [`connect_protocol_client`], but for an arbitrary server URL
/// (in-process or spawned-binary).
pub async fn connect_protocol_to_url(
    url: &str,
    protocol: TestProtocol,
) -> Result<(RunningService<RoleClient, VersionedClientHandler>, ())> {
    let handler = VersionedClientHandler::new(protocol);
    let transport = StreamableHttpClientTransport::from_uri(url);
    let client = handler
        .serve_with_lifecycle(transport, protocol.lifecycle())
        .await?;
    Ok((client, ()))
}

/// Connect an `rmcp` HTTP client using the exact `2026-07-28` discover
/// lifecycle (`server/discover` + self-contained per-request `_meta`).
pub async fn connect_2026_07_28_client(
    server: &TestServer,
) -> Result<(RunningService<RoleClient, VersionedClientHandler>, ())> {
    connect_protocol_client(server, TestProtocol::V2026_07_28).await
}

/// Like [`connect_2026_07_28_client`], but for an arbitrary server URL
/// (in-process or spawned-binary).
pub async fn connect_2026_07_28_to_url(
    url: &str,
) -> Result<(RunningService<RoleClient, VersionedClientHandler>, ())> {
    connect_protocol_to_url(url, TestProtocol::V2026_07_28).await
}

/// Connect an `rmcp` HTTP client using the exact `2025-11-25` initialize
/// lifecycle. The handler's `ClientInfo.protocol_version` is exactly
/// `2025-11-25`.
pub async fn connect_2025_11_25_client(
    server: &TestServer,
) -> Result<(RunningService<RoleClient, VersionedClientHandler>, ())> {
    connect_protocol_client(server, TestProtocol::V2025_11_25).await
}

/// Like [`connect_2025_11_25_client`], but for an arbitrary server URL
/// (in-process or spawned-binary).
pub async fn connect_2025_11_25_to_url(
    url: &str,
) -> Result<(RunningService<RoleClient, VersionedClientHandler>, ())> {
    connect_protocol_to_url(url, TestProtocol::V2025_11_25).await
}

#[derive(Clone)]
pub struct ProgressNotificationCollector {
    progress_tx: mpsc::UnboundedSender<ProgressNotificationParam>,
}

impl ClientHandler for ProgressNotificationCollector {
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
    mpsc::UnboundedReceiver<ProgressNotificationParam>,
)> {
    let (progress_tx, progress_rx) = mpsc::unbounded_channel();
    let handler = ProgressNotificationCollector { progress_tx };
    let transport = StreamableHttpClientTransport::from_uri(server.url.as_str());
    let client = handler.serve(transport).await?;
    Ok((client, progress_rx))
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

// ---- Linux production-path PTY pair -----------------------------------------
//
// `openpty` gives back a master fd and a slave fd whose device path
// (`/dev/pts/N`) can be opened by `tokio_serial::SerialStream` on Linux
// exactly like a real serial port. Production-path PTY tests stay Linux-only:
// macOS `serialport` baud configuration invokes `IOSSIOSPEED`, which macOS
// PTYs reject with `ENOTTY`. macOS coverage uses controlled backends instead.

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
        master: Option<File>,
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
                master: Some(master),
                _slave: slave,
            })
        }

        pub async fn write_device(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            let master = self.master.as_mut().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY master closed")
            })?;
            master.write_all(bytes).await?;
            master.flush().await
        }

        pub async fn read_device(&mut self, dst: &mut [u8]) -> std::io::Result<usize> {
            self.master
                .as_mut()
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY master closed")
                })?
                .read(dst)
                .await
        }

        /// Read exactly `dst.len()` bytes from the device side or error.
        pub async fn read_device_exact(&mut self, dst: &mut [u8]) -> std::io::Result<()> {
            self.master
                .as_mut()
                .ok_or_else(|| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "PTY master closed")
                })?
                .read_exact(dst)
                .await
                .map(|_| ())
        }

        /// Close the device side while retaining the slave descriptor and path.
        pub fn close_master(&mut self) {
            self.master.take();
        }

        /// Split the pair into its master file and slave fd so the
        /// test can move the master into a spawned emulator task whilst
        /// keeping the slave alive.
        pub fn into_parts(mut self) -> (File, OwnedFd) {
            (
                self.master.take().expect("PTY master already closed"),
                self._slave,
            )
        }
    }
}
