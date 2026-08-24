//! Reusable Linux real-PTY device fixture for public MCP integration tests.
//!
//! Fixture owns PTY pair, stable symlink, peer tasks, bounded queues, and
//! explicit teardown. Serial-mcp sees only normal slave path and opens it through
//! production serial code. Production-path fixture tests are Linux-only because
//! macOS `serialport` baud configuration invokes `IOSSIOSPEED`, which macOS PTYs
//! reject with `ENOTTY`; macOS uses controlled-backend coverage instead.

pub mod core;
#[cfg(target_os = "linux")]
pub mod protocol_peers;

use std::collections::VecDeque;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{Context, Result};
use core::{Action, InputAssembler, OutputQueue, QueuePolicy, ScriptLimits};
use nix::fcntl::{fcntl, FcntlArg, OFlag};
use nix::pty::{openpty, OpenptyResult};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
use nix::unistd::{read, ttyname, write};
use tokio::io::unix::AsyncFd;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const TASK_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// Stateful byte peer. Implementations remain independent of serial-mcp parsers.
pub trait DevicePeer: Send + 'static {
    fn on_start(&mut self) -> Vec<Action> {
        Vec::new()
    }

    fn on_command(&mut self, command: &[u8]) -> Vec<Action>;
}

/// Small default peer used by fixture lifecycle tests and command-parity ports.
#[derive(Debug, Default)]
pub struct PingPeer {
    sequence: u64,
    boot_banner: bool,
}

/// Finite deterministic flood peer for matcher and bounded-read parity.
///
/// `flood complete` emits a fixed payload followed by a unique completion
/// marker. `flood budget` emits only fixed payload bytes so a configured read
/// budget, rather than a matcher, owns the public stop reason.
#[derive(Debug, Default)]
pub struct FloodPeer;

impl DevicePeer for FloodPeer {
    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        match command {
            b"flood complete" => vec![
                Action::Emit(vec![b'x'; 1024]),
                Action::Emit(b"FLOOD-COMPLETE-9e7d\r\n".to_vec()),
            ],
            b"flood budget" => vec![Action::Emit(vec![b'y'; 1024])],
            _ => vec![Action::Emit(b"ERROR\r\n".to_vec())],
        }
    }
}

impl PingPeer {
    pub fn with_boot_banner() -> Self {
        Self {
            sequence: 0,
            boot_banner: true,
        }
    }
}

impl DevicePeer for PingPeer {
    fn on_start(&mut self) -> Vec<Action> {
        if self.boot_banner {
            vec![Action::Emit(b"serial-mcp test device ready\r\n".to_vec())]
        } else {
            Vec::new()
        }
    }

    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        self.sequence = self.sequence.wrapping_add(1);
        match command {
            b"ping" => vec![Action::Emit(
                format!("pong seq={}\r\n", self.sequence).into_bytes(),
            )],
            b"touch" => vec![
                Action::Emit(b"touch exit(42)\r\n".to_vec()),
                Action::Crash(42),
            ],
            _ => vec![Action::Emit(b"ERROR\r\n".to_vec())],
        }
    }
}

/// Fixture sizing and safety limits.
#[derive(Debug, Clone)]
pub struct DeviceFixtureConfig {
    pub output_capacity: usize,
    pub output_chunk_size: usize,
    pub queue_policy: QueuePolicy,
    pub max_pending_input_bytes: usize,
    pub max_actions_per_script: usize,
    pub max_script_bytes: usize,
    pub control_capacity: usize,
}

impl Default for DeviceFixtureConfig {
    fn default() -> Self {
        Self {
            output_capacity: 64 * 1024,
            output_chunk_size: 256,
            queue_policy: QueuePolicy::DropNew,
            max_pending_input_bytes: 64 * 1024,
            max_actions_per_script: 1024,
            max_script_bytes: 1024 * 1024,
            control_capacity: 32,
        }
    }
}

impl DeviceFixtureConfig {
    fn validate(&self) -> Result<()> {
        let _ = OutputQueue::new(
            self.output_capacity,
            self.output_chunk_size,
            self.queue_policy,
        )?;
        let _ = InputAssembler::new(self.max_pending_input_bytes)?;
        if self.max_actions_per_script == 0 {
            anyhow::bail!("device action limit must be greater than zero");
        }
        if self.max_script_bytes == 0 {
            anyhow::bail!("device script byte limit must be greater than zero");
        }
        if self.control_capacity == 0 {
            anyhow::bail!("device control capacity must be greater than zero");
        }
        Ok(())
    }

    fn script_limits(&self) -> ScriptLimits {
        ScriptLimits {
            max_actions: self.max_actions_per_script,
            max_supplied_bytes: self.max_script_bytes,
        }
    }
}

/// Terminal state of one fixture endpoint generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixtureExit {
    Running,
    Shutdown,
    PeerClosed,
    Crashed(i32),
    IoError(String),
    PeerError(String),
}

impl FixtureExit {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// Observable readiness and queue state. Tests wait on counters, never sleeps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixtureSnapshot {
    pub generation: u64,
    pub commands_accepted: u64,
    pub output_accepted: usize,
    pub output_dropped: usize,
    pub output_drained: usize,
    pub output_pending: usize,
    pub held: bool,
    pub exit: FixtureExit,
}

/// One peer-observed complete command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedCommand {
    pub command: Vec<u8>,
}

impl FixtureSnapshot {
    fn initial(generation: u64) -> Self {
        Self {
            generation,
            commands_accepted: 0,
            output_accepted: 0,
            output_dropped: 0,
            output_drained: 0,
            output_pending: 0,
            held: false,
            exit: FixtureExit::Running,
        }
    }
}

/// Explicit teardown report for one endpoint generation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownReport {
    pub snapshot: FixtureSnapshot,
    pub task_aborted: bool,
}

enum Control {
    Script(Vec<Action>),
    SetHold(bool),
}

struct EndpointRuntime {
    cancellation: CancellationToken,
    control: mpsc::Sender<Control>,
    snapshot_rx: watch::Receiver<FixtureSnapshot>,
    observed_rx: mpsc::Receiver<ObservedCommand>,
    raw_input_rx: mpsc::Receiver<Vec<u8>>,
    task: Option<JoinHandle<()>>,
}

/// Owned test device reachable through stable real filesystem path.
pub struct DeviceFixture {
    config: DeviceFixtureConfig,
    tempdir: tempfile::TempDir,
    stable_path: PathBuf,
    physical_path: PathBuf,
    slave: Option<OwnedFd>,
    retired_slaves: Vec<OwnedFd>,
    runtime: Option<EndpointRuntime>,
    generation: u64,
}

impl DeviceFixture {
    pub async fn spawn(peer: impl DevicePeer, config: DeviceFixtureConfig) -> Result<Self> {
        config.validate()?;
        let tempdir = tempfile::tempdir().context("create device fixture temp directory")?;
        let stable_path = tempdir.path().join("serial-port");
        let boundary = PtyBoundary::open()?;
        create_initial_symlink(&boundary.slave_path, &stable_path)?;
        let physical_path = boundary.slave_path.clone();
        let (runtime, slave) = spawn_runtime(boundary, peer, config.clone(), 1)?;
        Ok(Self {
            config,
            tempdir,
            stable_path,
            physical_path,
            slave: Some(slave),
            retired_slaves: Vec::new(),
            runtime: Some(runtime),
            generation: 1,
        })
    }

    /// Stable path passed to public `open`. Replacement keeps this path.
    pub fn port_path(&self) -> &Path {
        &self.stable_path
    }

    /// Current kernel PTY slave path, useful only for diagnostics.
    pub fn physical_path(&self) -> &Path {
        &self.physical_path
    }

    pub fn snapshot(&self) -> FixtureSnapshot {
        self.runtime
            .as_ref()
            .expect("device fixture endpoint not running")
            .snapshot_rx
            .borrow()
            .clone()
    }

    pub async fn wait_for<F>(&mut self, timeout: Duration, predicate: F) -> Result<FixtureSnapshot>
    where
        F: Fn(&FixtureSnapshot) -> bool,
    {
        let runtime = self
            .runtime
            .as_mut()
            .context("device fixture endpoint not running")?;
        let wait = async {
            loop {
                let snapshot = runtime.snapshot_rx.borrow_and_update().clone();
                if predicate(&snapshot) {
                    return Ok(snapshot);
                }
                runtime
                    .snapshot_rx
                    .changed()
                    .await
                    .context("device fixture readiness channel closed")?;
            }
        };
        match tokio::time::timeout(timeout, wait).await {
            Ok(result) => result,
            Err(_) => {
                let latest = runtime.snapshot_rx.borrow().clone();
                anyhow::bail!("timed out waiting for device fixture readiness; latest={latest:?}")
            }
        }
    }

    pub async fn run_script(&self, actions: Vec<Action>) -> Result<()> {
        self.config.script_limits().validate(&actions)?;
        self.runtime
            .as_ref()
            .context("device fixture endpoint not running")?
            .control
            .send(Control::Script(actions))
            .await
            .context("device fixture writer stopped")
    }

    pub async fn next_observed_command(&mut self, timeout: Duration) -> Result<ObservedCommand> {
        let runtime = self
            .runtime
            .as_mut()
            .context("device fixture endpoint not running")?;
        tokio::time::timeout(timeout, runtime.observed_rx.recv())
            .await
            .context("timed out waiting for peer-observed command")?
            .context("device fixture observation channel closed")
    }

    /// Return next exact OS read observed on peer side.
    ///
    /// Callers comparing a complete TX sequence must concatenate chunks because
    /// PTY read boundaries are intentionally not part of serial contract.
    pub async fn next_raw_input(&mut self, timeout: Duration) -> Result<Vec<u8>> {
        let runtime = self
            .runtime
            .as_mut()
            .context("device fixture endpoint not running")?;
        tokio::time::timeout(timeout, runtime.raw_input_rx.recv())
            .await
            .context("timed out waiting for raw device input")?
            .context("device fixture raw-input channel closed")
    }

    pub async fn set_hold(&self, held: bool) -> Result<()> {
        self.runtime
            .as_ref()
            .context("device fixture endpoint not running")?
            .control
            .send(Control::SetHold(held))
            .await
            .context("device fixture writer stopped")
    }

    /// Close current peer and slave, making public serial connection disappear.
    pub async fn disconnect_peer(&mut self) -> Result<ShutdownReport> {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime
                .control
                .send(Control::Script(vec![Action::Close]))
                .await
                .context("device fixture writer stopped before close")?;
        }
        let report = stop_runtime(&mut self.runtime, false).await?;
        if let Some(slave) = self.slave.take() {
            // Keep old PTY number reserved until replacement is allocated.
            // Linux can otherwise immediately reuse `/dev/pts/N`, hiding
            // whether stable-path retargeting reached a distinct endpoint.
            self.retired_slaves.push(slave);
        }
        Ok(report)
    }

    /// Allocate a fresh PTY and atomically retarget stable fixture path.
    pub async fn replace_endpoint(&mut self, peer: impl DevicePeer) -> Result<()> {
        if self.runtime.is_some() {
            anyhow::bail!("disconnect current device endpoint before replacement");
        }
        let boundary = PtyBoundary::open()?;
        atomic_retarget_symlink(
            self.tempdir.path(),
            &self.stable_path,
            &boundary.slave_path,
            self.generation.saturating_add(1),
        )?;
        self.generation = self.generation.saturating_add(1);
        self.physical_path = boundary.slave_path.clone();
        let (runtime, slave) = spawn_runtime(boundary, peer, self.config.clone(), self.generation)?;
        self.runtime = Some(runtime);
        self.slave = Some(slave);
        self.retired_slaves.clear();
        Ok(())
    }

    pub async fn shutdown(mut self) -> Result<ShutdownReport> {
        let report = stop_runtime(&mut self.runtime, true).await?;
        self.slave.take();
        self.retired_slaves.clear();
        Ok(report)
    }
}

impl Drop for DeviceFixture {
    fn drop(&mut self) {
        if let Some(runtime) = self.runtime.as_ref() {
            runtime.cancellation.cancel();
        }
        if let Some(runtime) = self.runtime.as_mut() {
            if let Some(task) = runtime.task.take() {
                task.abort();
            }
        }
    }
}

struct PtyBoundary {
    slave_path: PathBuf,
    master: AsyncFd<OwnedFd>,
    slave: OwnedFd,
}

impl PtyBoundary {
    fn open() -> Result<Self> {
        let OpenptyResult { master, slave } = openpty(None, None).context("openpty failed")?;
        let mut termios = tcgetattr(&slave).context("read PTY termios")?;
        cfmakeraw(&mut termios);
        tcsetattr(&slave, SetArg::TCSANOW, &termios).context("set PTY raw mode")?;
        let slave_path = ttyname(&slave).context("resolve PTY slave path")?;
        let flags = OFlag::from_bits_truncate(fcntl(&master, FcntlArg::F_GETFL)?);
        fcntl(&master, FcntlArg::F_SETFL(flags | OFlag::O_NONBLOCK))
            .context("set PTY master nonblocking")?;
        Ok(Self {
            slave_path,
            master: AsyncFd::new(master).context("register PTY master")?,
            slave,
        })
    }
}

fn create_initial_symlink(target: &Path, stable_path: &Path) -> Result<()> {
    std::os::unix::fs::symlink(target, stable_path).with_context(|| {
        format!(
            "create device fixture symlink {} -> {}",
            stable_path.display(),
            target.display()
        )
    })
}

fn atomic_retarget_symlink(
    owned_dir: &Path,
    stable_path: &Path,
    target: &Path,
    generation: u64,
) -> Result<()> {
    let temporary = owned_dir.join(format!(".serial-port-next-{generation}"));
    std::os::unix::fs::symlink(target, &temporary).with_context(|| {
        format!(
            "create replacement symlink {} -> {}",
            temporary.display(),
            target.display()
        )
    })?;
    if let Err(error) = std::fs::rename(&temporary, stable_path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).with_context(|| {
            format!(
                "atomically retarget device fixture symlink {}",
                stable_path.display()
            )
        });
    }
    Ok(())
}

fn spawn_runtime(
    boundary: PtyBoundary,
    mut peer: impl DevicePeer,
    config: DeviceFixtureConfig,
    generation: u64,
) -> Result<(EndpointRuntime, OwnedFd)> {
    let cancellation = CancellationToken::new();
    let (control_tx, control_rx) = mpsc::channel(config.control_capacity);
    let (observed_tx, observed_rx) = mpsc::channel(config.control_capacity);
    let (raw_input_tx, raw_input_rx) = mpsc::channel(config.control_capacity);
    let initial = FixtureSnapshot::initial(generation);
    let snapshot = Arc::new(StdMutex::new(initial.clone()));
    let (snapshot_tx, snapshot_rx) = watch::channel(initial);

    let start_actions = peer.on_start();
    config.script_limits().validate(&start_actions)?;

    let task = tokio::spawn(run_device(
        boundary.master,
        peer,
        control_rx,
        observed_tx,
        raw_input_tx,
        DeviceTaskState {
            cancellation: cancellation.child_token(),
            snapshot,
            snapshot_tx,
            config,
            start_actions,
        },
    ));

    Ok((
        EndpointRuntime {
            cancellation,
            control: control_tx,
            snapshot_rx,
            observed_rx,
            raw_input_rx,
            task: Some(task),
        },
        boundary.slave,
    ))
}

struct DeviceTaskState {
    cancellation: CancellationToken,
    snapshot: Arc<StdMutex<FixtureSnapshot>>,
    snapshot_tx: watch::Sender<FixtureSnapshot>,
    config: DeviceFixtureConfig,
    start_actions: Vec<Action>,
}

async fn run_device(
    master: AsyncFd<OwnedFd>,
    mut peer: impl DevicePeer,
    mut control_rx: mpsc::Receiver<Control>,
    observed_tx: mpsc::Sender<ObservedCommand>,
    raw_input_tx: mpsc::Sender<Vec<u8>>,
    state: DeviceTaskState,
) {
    let DeviceTaskState {
        cancellation,
        snapshot,
        snapshot_tx,
        config,
        start_actions,
    } = state;
    let mut assembler = match InputAssembler::new(config.max_pending_input_bytes) {
        Ok(assembler) => assembler,
        Err(error) => {
            set_exit(
                &snapshot,
                &snapshot_tx,
                FixtureExit::PeerError(error.to_string()),
            );
            return;
        }
    };
    let mut buffer = [0u8; 4096];
    let mut queue = match OutputQueue::new(
        config.output_capacity,
        config.output_chunk_size,
        config.queue_policy,
    ) {
        Ok(queue) => queue,
        Err(error) => {
            set_exit(
                &snapshot,
                &snapshot_tx,
                FixtureExit::PeerError(error.to_string()),
            );
            return;
        }
    };
    let mut actions = VecDeque::from(start_actions);
    let mut emission: Option<PendingEmission> = None;
    let mut delay: Option<tokio::time::Instant> = None;

    loop {
        if cancellation.is_cancelled() {
            set_exit(&snapshot, &snapshot_tx, FixtureExit::Shutdown);
            break;
        }

        if delay.is_none() {
            process_actions(
                &mut actions,
                &mut emission,
                &mut delay,
                &mut queue,
                &cancellation,
                &snapshot,
                &snapshot_tx,
            );
            if cancellation.is_cancelled() {
                break;
            }
        }

        if let Some(current) = emission.as_mut() {
            if current.offset < current.bytes.len() {
                let report = queue.enqueue(&current.bytes[current.offset..]);
                current.offset = current
                    .offset
                    .saturating_add(report.accepted + report.dropped);
                publish_queue(&snapshot, &snapshot_tx, &queue);
            }
            if current.offset == current.bytes.len()
                && (!current.wait_until_drained || queue.len() == 0)
            {
                emission = None;
                continue;
            }
        }

        let chunk = queue.next_chunk();
        tokio::select! {
            () = cancellation.cancelled() => {
                set_exit(&snapshot, &snapshot_tx, FixtureExit::Shutdown);
                break;
            }
            Some(control) = control_rx.recv() => {
                apply_control(control, &mut actions, &mut queue, &snapshot, &snapshot_tx);
            }
            () = wait_for_deadline(delay), if delay.is_some() => {
                delay = None;
            }
            result = async_fd_write(&master, &chunk), if !chunk.is_empty() => {
                match result {
                    Ok(0) => {
                        set_exit(
                            &snapshot,
                            &snapshot_tx,
                            FixtureExit::IoError("PTY master write returned zero bytes".to_owned()),
                        );
                        cancellation.cancel();
                        break;
                    }
                    Ok(count) => {
                        if let Err(error) = queue.commit_drain(count) {
                            set_exit(
                                &snapshot,
                                &snapshot_tx,
                                FixtureExit::PeerError(error.to_string()),
                            );
                            cancellation.cancel();
                            break;
                        }
                        publish_queue(&snapshot, &snapshot_tx, &queue);
                    }
                    Err(error) => {
                        set_exit(
                            &snapshot,
                            &snapshot_tx,
                            FixtureExit::IoError(error.to_string()),
                        );
                        cancellation.cancel();
                        break;
                    }
                }
            }
            result = async_fd_read(&master, &mut buffer) => {
                let count = match result {
                    Ok(0) => {
                        set_exit(
                            &snapshot,
                            &snapshot_tx,
                            FixtureExit::IoError("PTY master reached EOF".to_owned()),
                        );
                        cancellation.cancel();
                        break;
                    }
                    Ok(count) => count,
                    Err(error) => {
                        set_exit(
                            &snapshot,
                            &snapshot_tx,
                            FixtureExit::IoError(error.to_string()),
                        );
                        cancellation.cancel();
                        break;
                    }
                };
                if raw_input_tx.send(buffer[..count].to_vec()).await.is_err() {
                    cancellation.cancel();
                    break;
                }
                let commands = match assembler.push(&buffer[..count]) {
                    Ok(commands) => commands,
                    Err(error) => {
                        set_exit(
                            &snapshot,
                            &snapshot_tx,
                            FixtureExit::PeerError(error.to_string()),
                        );
                        cancellation.cancel();
                        break;
                    }
                };
                for command in commands {
                    if observed_tx
                        .send(ObservedCommand {
                            command: command.clone(),
                        })
                        .await
                        .is_err()
                    {
                        cancellation.cancel();
                        break;
                    }
                    update_snapshot(&snapshot, &snapshot_tx, |state| {
                        state.commands_accepted = state.commands_accepted.saturating_add(1);
                    });
                    let peer_actions = peer.on_command(&command);
                    if let Err(error) = config.script_limits().validate(&peer_actions) {
                        set_exit(
                            &snapshot,
                            &snapshot_tx,
                            FixtureExit::PeerError(error.to_string()),
                        );
                        cancellation.cancel();
                        break;
                    }
                    actions.extend(peer_actions);
                }
            }
            else => tokio::task::yield_now().await,
        }
    }
}

#[derive(Debug)]
struct PendingEmission {
    bytes: Vec<u8>,
    offset: usize,
    wait_until_drained: bool,
}

async fn async_fd_read(fd: &AsyncFd<OwnedFd>, buffer: &mut [u8]) -> std::io::Result<usize> {
    loop {
        let mut ready = fd.readable().await?;
        match ready.try_io(|inner| read(inner.get_ref(), buffer).map_err(std::io::Error::from)) {
            Ok(result) => return result,
            Err(_) => continue,
        }
    }
}

async fn async_fd_write(fd: &AsyncFd<OwnedFd>, bytes: &[u8]) -> std::io::Result<usize> {
    loop {
        let mut ready = fd.writable().await?;
        match ready.try_io(|inner| write(inner.get_ref(), bytes).map_err(std::io::Error::from)) {
            Ok(result) => return result,
            Err(_) => continue,
        }
    }
}

fn process_actions(
    actions: &mut VecDeque<Action>,
    emission: &mut Option<PendingEmission>,
    delay: &mut Option<tokio::time::Instant>,
    queue: &mut OutputQueue,
    cancellation: &CancellationToken,
    snapshot: &Arc<StdMutex<FixtureSnapshot>>,
    snapshot_tx: &watch::Sender<FixtureSnapshot>,
) {
    if emission.is_some() {
        return;
    }
    while let Some(action) = actions.pop_front() {
        match action {
            Action::Emit(bytes) | Action::Malformed(bytes) => {
                *emission = Some(PendingEmission {
                    bytes,
                    offset: 0,
                    wait_until_drained: false,
                });
                return;
            }
            Action::EmitChunks(chunks) => {
                for chunk in chunks.into_iter().rev() {
                    actions.push_front(Action::EmitChunks(vec![chunk]));
                }
                if let Some(Action::EmitChunks(mut one)) = actions.pop_front() {
                    if let Some(bytes) = one.pop() {
                        *emission = Some(PendingEmission {
                            bytes,
                            offset: 0,
                            wait_until_drained: true,
                        });
                    }
                }
                return;
            }
            Action::Delay(duration) | Action::Silence(duration) => {
                *delay = Some(tokio::time::Instant::now() + duration);
                return;
            }
            Action::Saturate(pattern) => {
                if pattern.is_empty() {
                    continue;
                }
                let repetitions = queue.capacity() / pattern.len() + 2;
                let mut bytes = Vec::with_capacity(pattern.len().saturating_mul(repetitions));
                for _ in 0..repetitions {
                    bytes.extend_from_slice(&pattern);
                }
                *emission = Some(PendingEmission {
                    bytes,
                    offset: 0,
                    wait_until_drained: false,
                });
                return;
            }
            Action::Close => {
                set_exit(snapshot, snapshot_tx, FixtureExit::PeerClosed);
                cancellation.cancel();
                return;
            }
            Action::Crash(code) => {
                set_exit(snapshot, snapshot_tx, FixtureExit::Crashed(code));
                cancellation.cancel();
                return;
            }
        }
    }
}

fn apply_control(
    control: Control,
    actions: &mut VecDeque<Action>,
    queue: &mut OutputQueue,
    snapshot: &Arc<StdMutex<FixtureSnapshot>>,
    snapshot_tx: &watch::Sender<FixtureSnapshot>,
) {
    match control {
        Control::Script(script) => actions.extend(script),
        Control::SetHold(held) => {
            queue.set_held(held);
            publish_queue(snapshot, snapshot_tx, queue);
        }
    }
}

async fn wait_for_deadline(deadline: Option<tokio::time::Instant>) {
    if let Some(deadline) = deadline {
        tokio::time::sleep_until(deadline).await;
    }
}

fn publish_queue(
    snapshot: &Arc<StdMutex<FixtureSnapshot>>,
    snapshot_tx: &watch::Sender<FixtureSnapshot>,
    queue: &OutputQueue,
) {
    let stats = queue.stats();
    update_snapshot(snapshot, snapshot_tx, |state| {
        state.output_accepted = stats.accepted;
        state.output_dropped = stats.dropped;
        state.output_drained = stats.drained;
        state.output_pending = queue.len();
        state.held = queue.held();
    });
}

fn set_exit(
    snapshot: &Arc<StdMutex<FixtureSnapshot>>,
    snapshot_tx: &watch::Sender<FixtureSnapshot>,
    exit: FixtureExit,
) {
    update_snapshot(snapshot, snapshot_tx, |state| {
        if !state.exit.is_terminal() {
            state.exit = exit;
        }
    });
}

fn update_snapshot<F>(
    snapshot: &Arc<StdMutex<FixtureSnapshot>>,
    snapshot_tx: &watch::Sender<FixtureSnapshot>,
    update: F,
) where
    F: FnOnce(&mut FixtureSnapshot),
{
    let next = {
        let mut state = snapshot
            .lock()
            .expect("device fixture snapshot mutex poisoned");
        update(&mut state);
        state.clone()
    };
    snapshot_tx.send_replace(next);
}

async fn stop_runtime(
    runtime: &mut Option<EndpointRuntime>,
    request_shutdown: bool,
) -> Result<ShutdownReport> {
    let mut runtime = runtime
        .take()
        .context("device fixture endpoint not running")?;
    if request_shutdown {
        runtime.cancellation.cancel();
    }
    let task_aborted = join_bounded(&mut runtime.task).await?;
    let snapshot = runtime.snapshot_rx.borrow().clone();
    Ok(ShutdownReport {
        snapshot,
        task_aborted,
    })
}

async fn join_bounded(handle: &mut Option<JoinHandle<()>>) -> Result<bool> {
    let mut handle = handle
        .take()
        .context("device fixture task already joined")?;
    match tokio::time::timeout(TASK_SHUTDOWN_TIMEOUT, &mut handle).await {
        Ok(result) => {
            result.context("device fixture task panicked")?;
            Ok(false)
        }
        Err(_) => {
            handle.abort();
            let _ = handle.await;
            Ok(true)
        }
    }
}
