//! `SpawnedServer` — a real `serial-mcp` child process running the
//! streamable-HTTP transport on a free local port.
//!
//! Why this exists:
//!
//! - Validates the actual shipped binary, not an in-test assembly.
//! - Mirrors what users run (`serial-mcp --transport=http --bind=...`).
//! - Keeps the in-process `TestServer` available for tests that need
//!   to inject a custom `ConnectionManager` or non-default security
//!   rules into the server before a client connects.
//!
//! Usage:
//!
//! ```ignore
//! let server = SpawnedServer::start().await;
//! let (client, _rx) = spawn_client(&server).await?;
//! // ...
//! drop(server); // kills the child
//! ```

use std::net::TcpListener;
use std::time::Duration;

use anyhow::{Context, Result};
use rmcp::handler::client::ClientHandler;
use rmcp::model::LoggingMessageNotificationParam;
use rmcp::service::{NotificationContext, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::binaries::{ensure_serial_mcp_built, serial_mcp_bin};

/// A real `serial-mcp` HTTP server running in a child process on
/// `127.0.0.1:<chosen>`. The child is killed on `Drop`.
pub struct SpawnedServer {
    pub url: String,
    pub port: u16,
    child: Option<Child>,
    shutdown: CancellationToken,
}

impl SpawnedServer {
    /// Build the binary if necessary, then spawn the server on a free
    /// local port. Returns the URL (`http://127.0.0.1:<port>/mcp`) and
    /// the chosen port.
    pub async fn start() -> Self {
        ensure_serial_mcp_built().expect("serial-mcp binary available for spawned server");
        let port = pick_free_port().expect("find a free local TCP port for the spawned server");
        let child = spawn_serial_mcp_http(port)
            .await
            .expect("spawn serial-mcp --transport=http");
        // Wait until the listener is actually accepting. axum binds and
        // prints nothing to stdout, so we have to probe it.
        wait_for_port(port, Duration::from_secs(15))
            .await
            .expect("spawned serial-mcp to start listening");
        // Best-effort reap on drop.
        let shutdown = CancellationToken::new();
        SpawnedServer {
            url: format!("http://127.0.0.1:{port}/mcp"),
            port,
            child: Some(child),
            shutdown,
        }
    }
}

impl Drop for SpawnedServer {
    fn drop(&mut self) {
        self.shutdown.cancel();
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            // Reap the zombie so the test process does not leak it.
            // We do not await here (Drop is sync), so spawn a thread.
            #[allow(clippy::let_underscore_future)]
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

fn pick_free_port() -> Option<u16> {
    // TcpListener::bind("127.0.0.1:0") assigns a free port. We close
    // immediately; the brief window between close and the spawned
    // server's bind is small enough to ignore for CI purposes.
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

async fn spawn_serial_mcp_http(port: u16) -> Result<Child> {
    let bin = serial_mcp_bin();
    let child = Command::new(&bin)
        .args(["--transport=http", &format!("--bind=127.0.0.1:{port}")])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("failed to spawn {} for HTTP tests", bin.display()))?;
    Ok(child)
}

async fn wait_for_port(port: u16, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    let target = format!("127.0.0.1:{port}");
    while tokio::time::Instant::now() < deadline {
        if TcpListener::bind(&target).is_err() {
            // Port already taken -> server is up.
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("timed out waiting for spawned serial-mcp to bind {target}")
}

/// Forward every received `notifications/message` onto an unbounded
/// mpsc channel so tests can await log/progress events.
#[derive(Clone)]
pub struct NotificationCollector {
    tx: mpsc::UnboundedSender<LoggingMessageNotificationParam>,
}

impl ClientHandler for NotificationCollector {
    fn on_logging_message(
        &self,
        params: LoggingMessageNotificationParam,
        _ctx: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let tx = self.tx.clone();
        async move {
            let _ = tx.send(params);
        }
    }
}

/// Connect an `rmcp` HTTP client to a `SpawnedServer`. Returns the
/// running client service plus the receiving end of the notification
/// collector.
pub async fn spawn_client(
    server: &SpawnedServer,
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
