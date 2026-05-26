//! Shared test harness for HTTP integration tests.
//!
//! Spins up the real `SerialHandler` behind an `axum` server on an
//! auto-assigned port and connects to it with the real `rmcp` HTTP client
//! transport. The harness optionally pre-populates the
//! [`ConnectionManager`] with an in-memory loopback connection so the
//! HTTP surface can be exercised end-to-end without an OS-level serial
//! port.

#![allow(dead_code)]

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

use serial_mcp_server::security::SecurityManager;
use serial_mcp_server::serial::ConnectionManager;
use serial_mcp_server::server::StreamRegistry;
use serial_mcp_server::SerialHandler;

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
        Self::start_with(Arc::new(ConnectionManager::new())).await
    }

    /// Start a server reusing a caller-supplied [`ConnectionManager`].
    /// Useful when the test wants to insert a loopback connection before
    /// the server is up.
    pub async fn start_with(manager: Arc<ConnectionManager>) -> Self {
        Self::start_with_and_security(manager, SecurityManager::default()).await
    }

    /// Start a server with a custom [`ConnectionManager`] and [`SecurityManager`].
    /// The security manager's allowlist will govern `open` calls during the test.
    pub async fn start_with_and_security(
        manager: Arc<ConnectionManager>,
        security: SecurityManager,
    ) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}/mcp");
        let shutdown = CancellationToken::new();

        let streams: StreamRegistry = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        let manager_for_service = Arc::clone(&manager);
        let streams_for_service = Arc::clone(&streams);
        let shutdown_for_service = shutdown.child_token();
        let service = StreamableHttpService::new(
            move || {
                Ok(SerialHandler::with_manager_security_and_streams(
                    Arc::clone(&manager_for_service),
                    security.clone(),
                    Arc::clone(&streams_for_service),
                ))
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
    let (tx, rx) = mpsc::unbounded_channel();
    let handler = NotificationCollector { tx };
    let transport = StreamableHttpClientTransport::from_uri(server.url.as_str());
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
    }
}
