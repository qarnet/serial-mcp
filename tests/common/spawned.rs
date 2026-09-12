//! `SpawnedServer` runs a real `serial-mcp` child process with the
//! streamable-HTTP transport on a kernel-assigned ephemeral port.
//!
//! It validates the shipped binary rather than an in-test assembly, mirrors
//! what users run (`serial-mcp --transport=http --bind=...`), and keeps the
//! in-process `TestServer` available for tests that need to inject a custom
//! `ConnectionManager` or non-default security rules before a client
//! connects.
//!
//! The child is spawned with `--bind=127.0.0.1:0`, so the kernel assigns the
//! port atomically inside the child at `TcpListener::bind` time and the
//! child announces the bound address on stdout (`SERIAL_MCP_BOUND=<addr>`).
//! There is no free-port window for this test process or the OS ephemeral
//! allocator to race: a picked-then-dropped listener can never be stolen by
//! an outbound connection between pick and bind, because no pick happens.
//!
//! Usage:
//!
//! ```ignore
//! let server = SpawnedServer::start().await;
//! let (client, _rx) = spawn_client(&server).await?;
//! // ...
//! drop(server); // kills the child
//! ```

use std::process::Stdio;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use rmcp::service::{RoleClient, RunningService};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio_util::sync::CancellationToken;

use super::binaries::{ensure_serial_mcp_built, serial_mcp_bin};

/// How long the harness waits for the child to print its bind announcement
/// (`SERIAL_MCP_BOUND=...`). The child prints immediately after binding, so
/// this only covers binary startup (exec, tracing init, profile store open).
const BIND_ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(15);

/// Real `serial-mcp` HTTP server running in a child process on
/// `127.0.0.1:<kernel-assigned>`. The child is killed on `Drop`.
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

/// Tail of the child's stderr, retained for failure messages. Shared with the
/// drain task spawned in [`spawn_serial_mcp_http`].
type StderrTail = Arc<StdMutex<String>>;

impl SpawnedServer {
    /// Build the binary if necessary, then spawn the server on a
    /// kernel-assigned port with an isolated temporary `--profiles-path`
    /// (owned by this struct for the child's lifetime). Returns the URL
    /// (`http://127.0.0.1:<port>/mcp`) and the announced port.
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
    /// child with `cwd` as its working directory for relative
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
        let (child, port, _stderr_tail) = spawn_and_await_bind(profiles_path, cwd, capture_dir)
            .await
            .unwrap_or_else(|e| panic!("spawned serial-mcp failed to start: {e:#}"));
        // Drop performs best-effort reaping.
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
            // Reap the child so the test process does not leak a zombie. Drop
            // is synchronous, so spawn a thread instead of awaiting here.
            #[allow(clippy::let_underscore_future)]
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}

/// Spawn the child with `--bind=127.0.0.1:0` and wait for its bind
/// announcement. The kernel owns port allocation inside the child, so the
/// only failure modes are a dead child or a silent one — both surface here
/// with the retained stderr tail instead of a probe loop racing the OS.
/// The returned `StderrTail` is owned by the drain task; callers may hold
/// it to include the tail in later failure messages.
async fn spawn_and_await_bind(
    profiles_path: Option<&std::path::Path>,
    cwd: Option<&std::path::Path>,
    capture_dir: Option<&std::path::Path>,
) -> Result<(Child, u16, StderrTail)> {
    let (mut child, stderr_tail) = spawn_serial_mcp_http(profiles_path, cwd, capture_dir).await?;
    let stdout = child
        .stdout
        .take()
        .expect("spawned serial-mcp stdout is piped");
    let port = tokio::time::timeout(
        BIND_ANNOUNCE_TIMEOUT,
        await_bind_announcement(BufReader::new(stdout)),
    )
    .await
    .map_err(|_| anyhow::anyhow!("timed out after {BIND_ANNOUNCE_TIMEOUT:?} waiting for the child to bind and announce its address"))?
    .with_context(|| {
        format!(
            "spawned serial-mcp closed stdout without a bind announcement; stderr tail:\n{}",
            stderr_tail.lock().unwrap()
        )
    })?;
    // The child printed its announcement, meaning it bound its listener.
    // Confirm it is still alive so a crash between announce and return is
    // reported here rather than as a mysterious client-connect failure.
    if let Some(status) = child.try_wait().context("poll child liveness")? {
        bail!("spawned serial-mcp exited immediately after announcing port {port}: {status}");
    }
    Ok((child, port, stderr_tail))
}

/// Read lines from the child's stdout until a `SERIAL_MCP_BOUND=<addr>`
/// announcement appears. Any other output is ignored (kept forward
/// compatible), EOF without an announcement is an error.
async fn await_bind_announcement(mut reader: BufReader<ChildStdout>) -> Result<u16> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .context("read child stdout")?;
        if n == 0 {
            bail!("child exited its stdout writer before announcing a bound address");
        }
        let Some(addr) = line.trim().strip_prefix("SERIAL_MCP_BOUND=") else {
            continue;
        };
        let port: u16 = addr
            .rsplit_once(':')
            .map(|(_, p)| p)
            .unwrap_or("")
            .parse()
            .with_context(|| format!("child announced unparseable address '{addr}'"))?;
        if port == 0 {
            bail!("child announced port 0, which cannot be a usable bind result");
        }
        return Ok(port);
    }
}

/// Type alias for the child's piped stdout handle.
type ChildStdout = tokio::process::ChildStdout;

async fn spawn_serial_mcp_http(
    profiles_path: Option<&std::path::Path>,
    cwd: Option<&std::path::Path>,
    capture_dir: Option<&std::path::Path>,
) -> Result<(Child, StderrTail)> {
    let bin = serial_mcp_bin();
    let mut command = Command::new(&bin);
    command
        .args(["--transport=http", "--bind=127.0.0.1:0"])
        .env("RUST_LOG", "off")
        .stdin(Stdio::null())
        // The bind announcement (`SERIAL_MCP_BOUND=...`) is the only stdout
        // output in HTTP serving mode; all logging goes to stderr.
        .stdout(Stdio::piped())
        // Pipe stderr (drained below) so startup failures — profile-store
        // validation, bind errors — are observable instead of a silent exit.
        .stderr(Stdio::piped())
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
    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn {} for HTTP tests", bin.display()))?;
    // Drain stderr continuously so the pipe never fills and blocks the
    // child. Retain only the last 4 KiB for failure messages.
    let tail: StderrTail = Arc::new(StdMutex::new(String::new()));
    if let Some(stderr) = child.stderr.take() {
        let writer = Arc::clone(&tail);
        tokio::spawn(async move {
            let mut reader = stderr;
            let mut buf = vec![0u8; 4096];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let chunk = String::from_utf8_lossy(&buf[..n]).into_owned();
                        let mut tail = writer.lock().unwrap();
                        tail.push_str(&chunk);
                        if tail.len() > 4096 {
                            let excess = tail.len() - 4096;
                            tail.drain(..excess);
                        }
                    }
                }
            }
        });
    }
    Ok((child, tail))
}

/// Connect an `rmcp` HTTP client to a `SpawnedServer`. Returns the
/// running client service plus a unit receiver (kept for caller symmetry;
/// there are no logging-message notifications anymore).
pub async fn spawn_client(
    server: &SpawnedServer,
) -> Result<(RunningService<RoleClient, super::TestClientHandler>, ())> {
    super::connect_to_url(server.url.as_str()).await
}

/// Connect an `rmcp` HTTP client to a `SpawnedServer` using the exact
/// `2026-07-28` discover lifecycle.
pub async fn spawn_2026_07_28_client(
    server: &SpawnedServer,
) -> Result<(
    RunningService<RoleClient, super::VersionedClientHandler>,
    (),
)> {
    super::connect_2026_07_28_to_url(server.url.as_str()).await
}

/// Connect an `rmcp` HTTP client to a `SpawnedServer` using the exact
/// `2025-11-25` initialize lifecycle.
pub async fn spawn_2025_11_25_client(
    server: &SpawnedServer,
) -> Result<(
    RunningService<RoleClient, super::VersionedClientHandler>,
    (),
)> {
    super::connect_2025_11_25_to_url(server.url.as_str()).await
}
