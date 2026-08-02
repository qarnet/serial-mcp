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
use rmcp::model::LoggingMessageNotificationParam;
use rmcp::service::{RoleClient, RunningService};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::binaries::{ensure_serial_mcp_built, serial_mcp_bin};

/// A real `serial-mcp` HTTP server running in a child process on
/// `127.0.0.1:<chosen>`. The child is killed on `Drop`.
///
/// Unless the caller supplies an explicit profiles path, the server runs
/// against its own temporary profile directory (owned here and retained
/// for the child's lifetime) so real-server tests never touch the user's
/// actual default profile config.
pub struct SpawnedServer {
    pub url: String,
    pub port: u16,
    child: Option<Child>,
    shutdown: CancellationToken,
    /// Owned isolated profile directory when `start()` created one.
    _profiles_dir: Option<tempfile::TempDir>,
}

/// Serializes the pick-port → spawn → wait-listening window across
/// concurrently running tests in this process. `pick_free_port` cannot
/// return a port that is currently bound, so once `wait_for_port`
/// confirms the child owns its port, later picks cannot collide.
/// Without this, two tests could pick the same port between listener
/// drop and child bind; the losing child exits silently (stderr is
/// null) and both clients talk to the winning test's server — which
/// dies mid-flight when that test completes.
static SPAWN_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl SpawnedServer {
    /// Build the binary if necessary, then spawn the server on a free
    /// local port with an isolated temporary `--profiles-path` (owned by
    /// this struct for the child's lifetime). Returns the URL
    /// (`http://127.0.0.1:<port>/mcp`) and the chosen port.
    pub async fn start() -> Self {
        let dir = tempfile::TempDir::new().expect("temp dir for isolated profile store");
        let path = dir.path().join("profiles.toml");
        let mut server = Self::start_with_profiles_path(Some(&path)).await;
        server._profiles_dir = Some(dir);
        server
    }

    /// Like [`SpawnedServer::start`], but passes `--profiles-path <path>`
    /// so the child uses the caller's isolated persistent profile store.
    /// `None` leaves the default path resolution untouched (use only when
    /// the caller explicitly wants the OS user-config default). The caller
    /// owns the path's lifetime.
    pub async fn start_with_profiles_path(profiles_path: Option<&std::path::Path>) -> Self {
        Self::start_inner(profiles_path, None, None).await
    }

    /// Like [`SpawnedServer::start_with_profiles_path`], but runs the
    /// child with `cwd` as its working directory — for relative
    /// `--profiles-path` resolution tests.
    pub async fn start_with_cwd(
        cwd: &std::path::Path,
        profiles_path: Option<&std::path::Path>,
    ) -> Self {
        Self::start_inner(profiles_path, Some(cwd), None).await
    }

    /// Like [`SpawnedServer::start`], but passes `--capture-dir <path>` so
    /// the child enables persistent capture into the caller's directory.
    /// The caller owns the directory's lifetime.
    pub async fn start_with_capture_dir(capture_dir: &std::path::Path) -> Self {
        Self::start_inner(None, None, Some(capture_dir)).await
    }

    async fn start_inner(
        profiles_path: Option<&std::path::Path>,
        cwd: Option<&std::path::Path>,
        capture_dir: Option<&std::path::Path>,
    ) -> Self {
        ensure_serial_mcp_built().expect("serial-mcp binary available for spawned server");
        let _guard = SPAWN_LOCK.lock().await;
        let port = pick_free_port().expect("find a free local TCP port for the spawned server");
        let child = spawn_serial_mcp_http(port, profiles_path, cwd, capture_dir)
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
            _profiles_dir: None,
        }
    }

    /// Kill the child process and await its exit (reap the zombie).
    /// After this the server is shut down; use it when a test must prove
    /// that a fresh process can continue the same profile store.
    pub async fn stop(&mut self) -> anyhow::Result<()> {
        self.shutdown.cancel();
        if let Some(mut child) = self.child.take() {
            child
                .start_kill()
                .context("kill spawned serial-mcp process")?;
            let _ = child.wait().await;
        }
        Ok(())
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
    // immediately; the window between close and the spawned server's
    // bind is guarded by SPAWN_LOCK against intra-process collisions.
    let listener = TcpListener::bind("127.0.0.1:0").ok()?;
    let port = listener.local_addr().ok()?.port();
    drop(listener);
    Some(port)
}

async fn spawn_serial_mcp_http(
    port: u16,
    profiles_path: Option<&std::path::Path>,
    cwd: Option<&std::path::Path>,
    capture_dir: Option<&std::path::Path>,
) -> Result<Child> {
    let bin = serial_mcp_bin();
    let mut command = Command::new(&bin);
    command
        .args(["--transport=http", &format!("--bind=127.0.0.1:{port}")])
        .env("RUST_LOG", "off")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    if let Some(path) = profiles_path {
        command.arg("--profiles-path").arg(path);
    }
    if let Some(dir) = capture_dir {
        command.arg("--capture-dir").arg(dir);
    }
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let child = command
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

/// Connect an `rmcp` HTTP client to a `SpawnedServer`. Returns the
/// running client service plus the receiving end of the shared
/// notification collector.
pub async fn spawn_client(
    server: &SpawnedServer,
) -> Result<(
    RunningService<RoleClient, super::NotificationCollector>,
    mpsc::UnboundedReceiver<LoggingMessageNotificationParam>,
)> {
    super::connect_to_url(server.url.as_str()).await
}
