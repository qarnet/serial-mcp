//! The single open serial connection: the `SerialIo` backend trait and its
//! production `SerialStream` implementation, the `SerialConnection` struct
//! with all methods, stream construction and baud validation, serialport
//! error conversion, and connection/I/O/close/disconnect-state tests.
//! Config types come from the `config` sibling; `PortInfo` from `port_info`.

use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_serial::{SerialPort, SerialPortBuilderExt, SerialStream};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::{Result, SerialError};

use super::config::{
    data_bits_to_str, flow_control_to_str, parity_to_str, stop_bits_to_str, ActiveProfileBinding,
    ConnectionConfig, ConnectionState, ConnectionStatus, ConnectionSummary, DataBits, FlowControl,
    FlushTarget, Parity, ReconnectPolicy, StopBits, MAX_BAUD_RATE,
};
use super::port_info::PortInfo;

// ---- I/O backend trait -------------------------------------------------------

/// Abstraction over the underlying byte stream plus the modem-control lines
/// of a serial port.
///
/// The production backend ([`SerialStream`]) talks to a real OS-level
/// serial port. Tests substitute an in-memory implementation backed by
/// [`tokio::io::duplex`] so that read/write/transact can be exercised
/// without any hardware.
///
/// Control-line operations (`clear_os_buffers`, `set_dtr_rts`,
/// `set_flow_control`, `set_break_state`) are required methods so the trait can stay
/// object-safe even when the backend doesn't have a real port behind it;
/// in-memory backends typically implement them as no-ops.
pub trait SerialIo: AsyncRead + AsyncWrite + Send + Unpin {
    fn clear_os_buffers(&self, target: FlushTarget) -> std::io::Result<()>;
    fn set_dtr_rts(&mut self, dtr: bool, rts: bool) -> std::io::Result<()>;
    fn set_flow_control(&mut self, flow_control: FlowControl) -> std::io::Result<()>;
    fn set_break_state(&self, asserted: bool) -> std::io::Result<()>;

    /// Reconfigure baud rate on an already-open port. Default is no-op
    /// for backends that don't support hardware reconfiguration.
    fn reconfigure_baud_rate(&mut self, _baud_rate: u32) -> std::io::Result<()> {
        Ok(())
    }

    /// Reconfigure data bits on an already-open port.
    fn reconfigure_data_bits(&mut self, _data_bits: serialport::DataBits) -> std::io::Result<()> {
        Ok(())
    }

    /// Reconfigure stop bits on an already-open port.
    fn reconfigure_stop_bits(&mut self, _stop_bits: serialport::StopBits) -> std::io::Result<()> {
        Ok(())
    }

    /// Reconfigure parity on an already-open port.
    fn reconfigure_parity(&mut self, _parity: serialport::Parity) -> std::io::Result<()> {
        Ok(())
    }
}

impl SerialIo for SerialStream {
    fn clear_os_buffers(&self, target: FlushTarget) -> std::io::Result<()> {
        self.clear(target.into()).map_err(io_error_from_serialport)
    }

    fn set_dtr_rts(&mut self, dtr: bool, rts: bool) -> std::io::Result<()> {
        self.write_data_terminal_ready(dtr)
            .map_err(io_error_from_serialport)?;
        self.write_request_to_send(rts)
            .map_err(io_error_from_serialport)
    }

    fn set_flow_control(&mut self, flow_control: FlowControl) -> std::io::Result<()> {
        SerialPort::set_flow_control(self, flow_control.into()).map_err(io_error_from_serialport)
    }

    fn set_break_state(&self, asserted: bool) -> std::io::Result<()> {
        if asserted {
            self.set_break().map_err(io_error_from_serialport)
        } else {
            self.clear_break().map_err(io_error_from_serialport)
        }
    }

    fn reconfigure_baud_rate(&mut self, baud_rate: u32) -> std::io::Result<()> {
        SerialPort::set_baud_rate(self, baud_rate).map_err(io_error_from_serialport)
    }

    fn reconfigure_data_bits(&mut self, data_bits: serialport::DataBits) -> std::io::Result<()> {
        SerialPort::set_data_bits(self, data_bits).map_err(io_error_from_serialport)
    }

    fn reconfigure_stop_bits(&mut self, stop_bits: serialport::StopBits) -> std::io::Result<()> {
        SerialPort::set_stop_bits(self, stop_bits).map_err(io_error_from_serialport)
    }

    fn reconfigure_parity(&mut self, parity: serialport::Parity) -> std::io::Result<()> {
        SerialPort::set_parity(self, parity).map_err(io_error_from_serialport)
    }
}

fn io_error_from_serialport(err: serialport::Error) -> std::io::Error {
    std::io::Error::other(err.to_string())
}

// ---- Single open connection --------------------------------------------------

/// A single open serial port. Cheap to clone via [`Arc`] because all state lives
/// behind a [`Mutex`].
pub struct SerialConnection {
    id: String,
    port: String,
    name: Option<String>,
    baud_rate: StdMutex<u32>,
    data_bits: StdMutex<DataBits>,
    stop_bits: StdMutex<StopBits>,
    parity: StdMutex<Parity>,
    flow_control: StdMutex<FlowControl>,
    io: Mutex<Option<Box<dyn SerialIo>>>,
    close_token: CancellationToken,
    closed: AtomicBool,
    /// Total bytes written to the device via the `write` tool.
    tx_bytes: AtomicU64,
    /// Total bytes read from the device and delivered through any RX path.
    rx_bytes: AtomicU64,
    /// Wall-clock time of the last rx or tx byte operation.
    last_activity: StdMutex<Option<std::time::SystemTime>>,
    /// Number of successful read-pipeline operations (`read`, `transact` read
    /// half, `capture_boot`).
    read_ops: AtomicU64,
    /// Number of successful `write` operations.
    write_ops: AtomicU64,
    /// Number of RX operations where data was truncated
    /// (bytes_returned < bytes_observed).
    truncation_count: AtomicU64,
    /// OS-level port identity captured at open time.
    port_info: Option<PortInfo>,
    /// Per-connection event log buffer.
    log: Arc<crate::log_buffer::LogBuffer>,
    /// Current connection health state.
    state: StdMutex<ConnectionState>,
    /// Reconnect policy for this connection.
    pub(crate) reconnect_policy: StdMutex<ReconnectPolicy>,
    /// Count of reconnect attempts since the last disconnect.
    reconnect_attempts: AtomicU64,
    /// Last fatal I/O error message and timestamp.
    last_error: StdMutex<Option<(std::time::SystemTime, String)>>,
    /// Default TX framing from profile/open call.
    tx_framing_default: StdMutex<Option<crate::framing::TxFramingConfig>>,
    /// Default RX framing from profile/open call.
    rx_framing_default: StdMutex<Option<crate::framing::RxFramingConfig>>,
    /// Default RX parser from profile/open call.
    rx_parser_default: StdMutex<Option<crate::framing::ParserConfig>>,
    /// Default protocol preset from profile/open call.
    protocol_default: StdMutex<Option<crate::framing::ProtocolPreset>>,
    /// Default max buffered bytes for `read` (from profile/open, mutable live).
    max_buffered_bytes_default: AtomicUsize,
    /// Active profile-session binding. `None` for connections
    /// inserted directly by low-level tests.
    active_profile: StdMutex<Option<ActiveProfileBinding>>,
    /// Serializes durable write-through-learning sequences on this
    /// connection (live mutation → effective snapshot → CAS persistence →
    /// binding update). Held across `reconfigure`/`set_flow_control`/
    /// connection-mode `configure`/clean close so concurrent requests
    /// cannot snapshot each other's half-applied state.
    learning_lock: tokio::sync::Mutex<()>,
    /// Serializes line-control operations: `set_dtr_rts` and the
    /// `capture_boot` reset pulse both hold this so a concurrent line-control
    /// request cannot interleave inside an assert/hold/release sequence.
    control_lock: tokio::sync::Mutex<()>,
    /// Effective RX ring buffer size from the open config, so profile
    /// snapshots never depend on whichever handler-local `RxSessionManager`
    /// receives a later request.
    rx_buffer_size: usize,
}

impl fmt::Debug for SerialConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SerialConnection")
            .field("id", &self.id)
            .field("port", &self.port)
            .field("name", &self.name)
            .finish()
    }
}

impl SerialConnection {
    /// Open a serial port using the supplied configuration.
    pub async fn open(config: ConnectionConfig) -> Result<Self> {
        ensure_valid_baud_rate(config.baud_rate)?;
        let stream = build_stream(&config)?;
        Ok(Self::from_io_with_config(config, Box::new(stream)))
    }

    /// Build a connection from an arbitrary [`SerialIo`] backend. Used by
    /// tests to inject an in-memory duplex stream.
    pub fn from_io(port: String, io: Box<dyn SerialIo>) -> Self {
        Self::from_io_with_config(
            ConnectionConfig {
                port,
                name: None,
                baud_rate: 115200,
                data_bits: DataBits::Eight,
                stop_bits: StopBits::One,
                parity: Parity::None,
                flow_control: FlowControl::None,
                port_info: None,
                log_capacity: 1024,
                log_enabled: true,
                tx_framing: None,
                rx_framing: None,
                rx_parser: None,
                protocol: None,
                rx_buffer_size: crate::limits::DEFAULT_RX_BUFFER_SIZE,
                max_buffered_bytes: 32768,
            },
            io,
        )
    }

    pub fn from_io_with_config(config: ConnectionConfig, io: Box<dyn SerialIo>) -> Self {
        let log = crate::log_buffer::LogBuffer::new_shared(config.log_capacity, config.log_enabled);
        log.opened();
        Self {
            id: Uuid::new_v4().to_string(),
            port: config.port,
            name: config.name,
            baud_rate: StdMutex::new(config.baud_rate),
            data_bits: StdMutex::new(config.data_bits),
            stop_bits: StdMutex::new(config.stop_bits),
            parity: StdMutex::new(config.parity),
            flow_control: StdMutex::new(config.flow_control),
            io: Mutex::new(Some(io)),
            close_token: CancellationToken::new(),
            closed: AtomicBool::new(false),
            tx_bytes: AtomicU64::new(0),
            rx_bytes: AtomicU64::new(0),
            last_activity: StdMutex::new(None),
            read_ops: AtomicU64::new(0),
            write_ops: AtomicU64::new(0),
            truncation_count: AtomicU64::new(0),
            port_info: config.port_info,
            log,
            state: StdMutex::new(ConnectionState::Open),
            reconnect_policy: StdMutex::new(ReconnectPolicy::default()),
            reconnect_attempts: AtomicU64::new(0),
            last_error: StdMutex::new(None),
            tx_framing_default: StdMutex::new(config.tx_framing),
            rx_framing_default: StdMutex::new(config.rx_framing),
            rx_parser_default: StdMutex::new(config.rx_parser),
            protocol_default: StdMutex::new(config.protocol),
            max_buffered_bytes_default: AtomicUsize::new(config.max_buffered_bytes),
            active_profile: StdMutex::new(None),
            learning_lock: tokio::sync::Mutex::new(()),
            control_lock: tokio::sync::Mutex::new(()),
            rx_buffer_size: config.rx_buffer_size,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn port(&self) -> &str {
        &self.port
    }

    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn baud_rate(&self) -> u32 {
        *self.baud_rate.lock().expect("baud_rate mutex poisoned")
    }

    pub fn flow_control(&self) -> FlowControl {
        *self
            .flow_control
            .lock()
            .expect("flow_control mutex poisoned")
    }

    pub fn data_bits(&self) -> DataBits {
        *self.data_bits.lock().expect("data_bits mutex poisoned")
    }

    pub fn stop_bits(&self) -> StopBits {
        *self.stop_bits.lock().expect("stop_bits mutex poisoned")
    }

    pub fn parity(&self) -> Parity {
        *self.parity.lock().expect("parity mutex poisoned")
    }

    /// Return the OS-level port identity captured at open time.
    pub fn port_info(&self) -> Option<&PortInfo> {
        self.port_info.as_ref()
    }

    /// Return the per-connection event log buffer.
    pub fn log(&self) -> &Arc<crate::log_buffer::LogBuffer> {
        &self.log
    }

    /// Default TX framing stored on the connection (from profile or `open`).
    /// Returns by value (cloned from interior mutex for live mutation).
    pub fn tx_framing_default(&self) -> Option<crate::framing::TxFramingConfig> {
        self.tx_framing_default.lock().expect("poisoned").clone()
    }

    /// Default RX framing stored on the connection.
    pub fn rx_framing_default(&self) -> Option<crate::framing::RxFramingConfig> {
        self.rx_framing_default.lock().expect("poisoned").clone()
    }

    /// Default RX parser stored on the connection.
    pub fn rx_parser_default(&self) -> Option<crate::framing::ParserConfig> {
        self.rx_parser_default.lock().expect("poisoned").clone()
    }

    /// Default protocol preset stored on the connection. `Copy`, so returned by value.
    /// Lock internal mutex for live mutation via `configure`.
    pub fn protocol_default(&self) -> Option<crate::framing::ProtocolPreset> {
        *self
            .protocol_default
            .lock()
            .expect("protocol_default mutex poisoned")
    }

    /// Default max buffered bytes for `read` operations. Mutable live.
    pub fn max_buffered_bytes_default(&self) -> usize {
        self.max_buffered_bytes_default
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// The active profile-session binding, if this connection was opened
    /// through a public open path.
    pub fn active_profile_binding(&self) -> Option<ActiveProfileBinding> {
        self.active_profile.lock().expect("poisoned").clone()
    }

    /// Attach (or clear) the active profile-session binding.
    pub(crate) fn set_active_profile_binding(&self, binding: Option<ActiveProfileBinding>) {
        *self.active_profile.lock().expect("poisoned") = binding;
    }

    /// Mutate the active profile-session binding in place
    /// (dirty/stale/error/revision updates from write-through learning).
    pub(crate) fn update_active_profile_binding(
        &self,
        update: impl FnOnce(&mut ActiveProfileBinding),
    ) {
        let mut guard = self.active_profile.lock().expect("poisoned");
        if let Some(binding) = guard.as_mut() {
            update(binding);
        }
    }

    /// The per-connection learning lock. Durable operations hold this across
    /// the live mutation, the effective snapshot, the CAS persistence
    /// attempt, and the binding update.
    pub(crate) fn learning_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.learning_lock
    }

    /// The per-connection line-control lock. `capture_boot` holds this
    /// across its whole assert/hold/release sequence so a concurrent
    /// `set_dtr_rts` cannot interleave inside the pulse. Callers acquiring
    /// it must use the unlocked line setter
    /// ([`Self::set_dtr_rts_unlocked`]).
    pub(crate) fn control_lock(&self) -> &tokio::sync::Mutex<()> {
        &self.control_lock
    }

    /// Snapshot the connection's full effective `ProfileDefaults`: current
    /// serial parameters, framing/parser/protocol defaults, the stored RX
    /// buffer size, read defaults, reconnect policy, log
    /// configuration, and the connection name. Used by write-through
    /// learning, close retry, and explicit `save_profile` — never consults
    /// a handler-local `RxSessionManager`.
    pub(crate) fn effective_defaults(&self) -> crate::profiles::ProfileDefaults {
        crate::profiles::ProfileDefaults {
            baud_rate: self.baud_rate(),
            data_bits: data_bits_to_str(self.data_bits()),
            stop_bits: stop_bits_to_str(self.stop_bits()),
            parity: parity_to_str(self.parity()),
            flow_control: flow_control_to_str(self.flow_control()),
            name: self.name.clone(),
            tx_framing: self.tx_framing_default(),
            rx_framing: self.rx_framing_default(),
            rx_parser: self.rx_parser_default(),
            protocol: self.protocol_default(),
            rx_buffer_size: self.rx_buffer_size(),
            max_buffered_bytes: self.max_buffered_bytes_default(),
            reconnect_policy: self.reconnect_policy.lock().expect("poisoned").clone(),
            log_capacity: self.log.capacity(),
            log_enabled: self.log.is_enabled(),
        }
    }

    /// The effective RX ring buffer size configured at open time.
    pub fn rx_buffer_size(&self) -> usize {
        self.rx_buffer_size
    }

    // ── Live mutators (pub(crate); exposed via `configure` tool) ──────────

    /// Set the default TX framing on a live connection.
    pub(crate) fn set_tx_framing_default(&self, v: Option<crate::framing::TxFramingConfig>) {
        *self.tx_framing_default.lock().expect("poisoned") = v;
    }

    /// Set the default RX framing on a live connection.
    pub(crate) fn set_rx_framing_default(&self, v: Option<crate::framing::RxFramingConfig>) {
        *self.rx_framing_default.lock().expect("poisoned") = v;
    }

    /// Set the default RX parser on a live connection.
    pub(crate) fn set_rx_parser_default(&self, v: Option<crate::framing::ParserConfig>) {
        *self.rx_parser_default.lock().expect("poisoned") = v;
    }

    /// Set the default protocol preset on a live connection.
    pub(crate) fn set_protocol_default(&self, v: Option<crate::framing::ProtocolPreset>) {
        *self.protocol_default.lock().expect("poisoned") = v;
    }

    /// Set the default max buffered bytes on a live connection.
    pub(crate) fn set_max_buffered_bytes_default(&self, v: usize) {
        self.max_buffered_bytes_default
            .store(v, std::sync::atomic::Ordering::SeqCst);
    }

    /// Return the current connection health state.
    pub fn state(&self) -> ConnectionState {
        *self.state.lock().expect("state mutex poisoned")
    }

    /// Set the connection state and log the transition.
    fn set_state(&self, new_state: ConnectionState) {
        *self.state.lock().expect("state mutex poisoned") = new_state;
    }

    /// Mark the connection as disconnected due to a fatal I/O error.
    /// Takes the io handle out (sets to None), cancels in-flight operations,
    /// and clears RX buffers.
    pub async fn mark_disconnected(&self, error_message: String) {
        let was_healthy = self.state().is_healthy();
        self.set_state(ConnectionState::Disconnected);
        self.last_error
            .lock()
            .expect("poisoned")
            .replace((std::time::SystemTime::now(), error_message.clone()));
        // We do NOT cancel close_token here — that is reserved for explicit
        // close(). The pump and in-flight reads will time out naturally and
        // retry when the port is reconnected.
        self.log.rx_data(0); // dummy to trigger log
        self.log.record(
            None,
            crate::log_buffer::LogEvent::Disconnect {
                error: error_message,
            },
        );
        // Take the io handle out so subsequent I/O calls get ConnectionClosed
        let mut io_lock = self.io.lock().await;
        if let Some(mut io) = io_lock.take() {
            // Best-effort: clear OS buffers and shutdown
            let _ = io.clear_os_buffers(FlushTarget::Input);
            let _ = io.shutdown().await;
        }
        if was_healthy {
            tracing::warn!("Connection {} disconnected", self.display_name());
        }
    }

    /// Attempt to re-establish the serial port connection.
    ///
    /// Rebuilds a `SerialStream` from the stored config and replaces the
    /// current `io` handle in place. Preserves all counters, id, name,
    /// and log buffer. Called by auto-reconnect tasks and the reconnect
    /// tool.
    pub async fn reconnect(&self) -> Result<()> {
        let state = self.state();
        if state == ConnectionState::Open {
            return Ok(()); // already connected
        }
        if state == ConnectionState::Closed {
            return Err(SerialError::ConnectionClosed(self.display_name()));
        }

        self.set_state(ConnectionState::Reconnecting);
        self.reconnect_attempts.fetch_add(1, Ordering::SeqCst);
        let attempt = self.reconnect_attempts.load(Ordering::SeqCst) as u32;
        self.log.record(
            None,
            crate::log_buffer::LogEvent::ReconnectStart { attempt },
        );

        let config = self.build_config();
        match build_stream(&config) {
            Ok(stream) => {
                let mut io_lock = self.io.lock().await;
                *io_lock = Some(Box::new(stream));
                self.closed.store(false, Ordering::SeqCst);
                self.set_state(ConnectionState::Open);
                self.log
                    .record(None, crate::log_buffer::LogEvent::ReconnectSuccess);
                tracing::info!("Connection {} reconnected", self.display_name());
                Ok(())
            }
            Err(e) => {
                self.set_state(ConnectionState::Disconnected);
                let msg = e.to_string();
                self.log.record(
                    None,
                    crate::log_buffer::LogEvent::ReconnectFailed {
                        attempt,
                        error: msg,
                    },
                );
                Err(e)
            }
        }
    }

    /// Reset the reconnect-attempt counter after a successful reconnect.
    pub(super) fn reset_reconnect_attempts(&self) {
        self.reconnect_attempts.store(0, Ordering::SeqCst);
    }

    /// Build a `ConnectionConfig` from the current connection state,
    /// for use in reconnect.
    fn build_config(&self) -> ConnectionConfig {
        ConnectionConfig {
            port: self.port.clone(),
            name: self.name.clone(),
            baud_rate: self.baud_rate(),
            data_bits: self.data_bits(),
            stop_bits: self.stop_bits(),
            parity: self.parity(),
            flow_control: self.flow_control(),
            port_info: self.port_info.clone(),
            log_capacity: 1024, // preserve log config
            log_enabled: self.log.is_enabled(),
            tx_framing: self.tx_framing_default(),
            rx_framing: self.rx_framing_default(),
            rx_parser: self.rx_parser_default(),
            protocol: self.protocol_default(),
            rx_buffer_size: self.rx_buffer_size(),
            max_buffered_bytes: self.max_buffered_bytes_default(),
        }
    }

    /// Record `n` bytes written to the device.
    pub fn record_tx_bytes(&self, n: usize) {
        self.tx_bytes.fetch_add(n as u64, Ordering::SeqCst);
        *self.last_activity.lock().expect("poisoned") = Some(std::time::SystemTime::now());
    }

    /// Record `n` bytes read from the device.
    pub fn record_rx_bytes(&self, n: usize) {
        self.rx_bytes.fetch_add(n as u64, Ordering::SeqCst);
        *self.last_activity.lock().expect("poisoned") = Some(std::time::SystemTime::now());
    }

    /// Return the last I/O activity as milliseconds since Unix epoch.
    pub fn last_activity_ms(&self) -> Option<u64> {
        self.last_activity.lock().expect("poisoned").and_then(|t| {
            t.duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .ok()
        })
    }

    /// Record one successful read-pipeline operation (`read`, `transact` read
    /// half, or `capture_boot`).
    pub fn record_read_op(&self) {
        self.read_ops.fetch_add(1, Ordering::SeqCst);
    }

    /// Record one successful write operation.
    pub fn record_write_op(&self) {
        self.write_ops.fetch_add(1, Ordering::SeqCst);
    }

    /// Record one RX truncation (bytes_returned < bytes_observed).
    pub fn record_truncation(&self) {
        self.truncation_count.fetch_add(1, Ordering::SeqCst);
    }

    /// Build a snapshot of the current status of this connection.
    pub fn status_snapshot(&self) -> ConnectionStatus {
        ConnectionStatus {
            connection_id: self.id().to_string(),
            name: self.name().map(str::to_string),
            port: self.port().to_string(),
            baud_rate: self.baud_rate(),
            data_bits: data_bits_to_str(self.data_bits()),
            stop_bits: stop_bits_to_str(self.stop_bits()),
            parity: parity_to_str(self.parity()),
            flow_control: flow_control_to_str(self.flow_control()),
            is_closed: self.closed.load(Ordering::SeqCst),
            tx_bytes: self.tx_bytes.load(Ordering::SeqCst),
            rx_bytes: self.rx_bytes.load(Ordering::SeqCst),
            last_activity_ms: self.last_activity_ms(),
            read_ops: self.read_ops.load(Ordering::SeqCst),
            write_ops: self.write_ops.load(Ordering::SeqCst),
            truncation_count: self.truncation_count.load(Ordering::SeqCst),
            port_info: self.port_info.clone(),
            state: self.state(),
            reconnect_attempts: self.reconnect_attempts.load(Ordering::SeqCst),
            last_error: self
                .last_error
                .lock()
                .expect("poisoned")
                .as_ref()
                .map(|(_, msg)| msg.clone()),
            profile: self.active_profile_binding().map(|b| b.to_session_result()),
        }
    }

    pub fn cancel_token(&self) -> CancellationToken {
        self.close_token.clone()
    }

    pub fn summary(&self) -> ConnectionSummary {
        ConnectionSummary {
            connection_id: self.id().to_string(),
            name: self.name().map(str::to_string),
            port: self.port().to_string(),
            baud_rate: self.baud_rate(),
            flow_control: self.flow_control(),
            profile: self.active_profile_binding().map(|b| b.to_session_result()),
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(SerialError::ConnectionClosed(self.display_name()));
        }
        Ok(())
    }

    fn display_name(&self) -> String {
        self.name.clone().unwrap_or_else(|| self.id.clone())
    }

    /// Write a byte slice, flushing before returning.
    pub async fn write(&self, data: &[u8]) -> Result<usize> {
        self.ensure_open()?;
        self.log.tx_data(data.len());
        let mut io = self.io.lock().await;
        let io = io
            .as_mut()
            .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?;
        io.write_all(data).await?;
        io.flush().await?;
        self.record_tx_bytes(data.len());
        Ok(data.len())
    }

    /// Read up to `dst.len()` bytes. Returns [`SerialError::ReadTimeout`] if
    /// `timeout_ms` is set and elapses before any byte arrives.
    ///
    /// When a timeout is given, the lock on the underlying IO is held for at
    /// most `POLL_MS` milliseconds at a time and released between polls.  This
    /// lets concurrent `write` calls on the same connection proceed without
    /// waiting for the full read timeout — which is essential for the
    /// request/response pattern (`transact`) on CDC-ACM devices.
    pub async fn read(&self, dst: &mut [u8], timeout_ms: Option<u64>) -> Result<usize> {
        const POLL_MS: u64 = 50;
        self.ensure_open()?;
        match timeout_ms {
            None => {
                let mut io = self.io.lock().await;
                let io = io
                    .as_mut()
                    .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?;
                let n = tokio::select! {
                    _ = self.close_token.cancelled() => Err(SerialError::ConnectionClosed(self.display_name())),
                    res = io.read(dst) => Ok(res?),
                }?;
                self.record_rx_bytes(n);
                Ok(n)
            }
            Some(ms) => {
                let deadline = Instant::now() + Duration::from_millis(ms);
                loop {
                    if self.close_token.is_cancelled() {
                        return Err(SerialError::ConnectionClosed(self.display_name()));
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(SerialError::ReadTimeout);
                    }
                    let poll_dur = remaining.min(Duration::from_millis(POLL_MS));
                    {
                        let mut io = self.io.lock().await;
                        let io = io
                            .as_mut()
                            .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?;
                        match tokio::select! {
                            _ = self.close_token.cancelled() => return Err(SerialError::ConnectionClosed(self.display_name())),
                            res = timeout(poll_dur, io.read(dst)) => res,
                        } {
                            Ok(Ok(n)) if n > 0 => {
                                self.record_rx_bytes(n);
                                return Ok(n);
                            }
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => return Err(SerialError::from(e)),
                            Err(_elapsed) => {}
                        }
                    }
                    // Yield to allow the I/O driver time to process epoll events
                    // before re-acquiring the mutex for the next poll.
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            }
        }
    }

    /// Read up to `max_bytes` with a brief timeout (100ms) to capture any
    /// immediately available data without blocking long.
    pub async fn read_latest(&self, max_bytes: usize) -> Result<Vec<u8>> {
        let mut buf = vec![0u8; max_bytes];
        let n = self.read(&mut buf, Some(100)).await?;
        buf.truncate(n);
        Ok(buf)
    }

    /// Discard data buffered in the OS for the input, output, or both
    /// directions of this port.
    pub async fn flush_buffers(&self, target: FlushTarget) -> Result<()> {
        self.ensure_open()?;
        let io = self.io.lock().await;
        io.as_ref()
            .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?
            .clear_os_buffers(target)
            .map_err(SerialError::from)
    }

    /// Drive the DTR and RTS control lines. Common use case: pulse DTR low
    /// to soft-reset an Arduino, or hold both low to enter the ESP32
    /// bootloader.
    ///
    /// Takes the per-connection line-control lock so the change cannot
    /// interleave with a `capture_boot` reset pulse.
    pub async fn set_dtr_rts(&self, dtr: bool, rts: bool) -> Result<()> {
        let _control = self.control_lock().lock().await;
        self.set_dtr_rts_unlocked(dtr, rts).await
    }

    /// Set DTR/RTS without taking the per-connection line-control lock.
    /// Callers must already hold the lock (see [`Self::control_lock`]);
    /// `capture_boot` uses this for its assert/hold/release sequence.
    pub(crate) async fn set_dtr_rts_unlocked(&self, dtr: bool, rts: bool) -> Result<()> {
        self.ensure_open()?;
        let mut io = self.io.lock().await;
        io.as_mut()
            .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?
            .set_dtr_rts(dtr, rts)
            .map_err(SerialError::from)
    }

    pub async fn set_flow_control(&self, flow_control: FlowControl) -> Result<()> {
        self.ensure_open()?;
        let mut io = self.io.lock().await;
        io.as_mut()
            .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?
            .set_flow_control(flow_control)
            .map_err(SerialError::from)?;
        *self
            .flow_control
            .lock()
            .expect("flow_control mutex poisoned") = flow_control;
        Ok(())
    }

    /// Set the BREAK condition on the TX line.
    pub async fn set_break_state(&self, enabled: bool) -> Result<()> {
        self.ensure_open()?;
        let io = self.io.lock().await;
        io.as_ref()
            .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?
            .set_break_state(enabled)
            .map_err(SerialError::from)
    }

    /// Assert the BREAK condition on the TX line for `duration_ms`
    /// milliseconds, then release it.
    pub async fn send_break(&self, duration_ms: u64) -> Result<()> {
        self.set_break_state(true).await?;
        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        self.set_break_state(false).await
    }

    /// Reconfigure serial parameters on a live connection. Parameters passed
    /// as `None` are left unchanged. Returns the effective config after the
    /// operation completes.
    pub async fn reconfigure(
        &self,
        baud_rate: Option<u32>,
        data_bits: Option<DataBits>,
        stop_bits: Option<StopBits>,
        parity: Option<Parity>,
        flow_control: Option<FlowControl>,
    ) -> Result<ConnectionStatus> {
        self.ensure_open()?;

        if let Some(rate) = baud_rate {
            ensure_valid_baud_rate(rate)?;
        }

        // Apply requested changes to the underlying serial port hardware.
        {
            let mut io = self.io.lock().await;
            let io = io
                .as_mut()
                .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?;

            if let Some(rate) = baud_rate {
                io.reconfigure_baud_rate(rate).map_err(SerialError::from)?;
            }
            if let Some(db) = data_bits {
                io.reconfigure_data_bits(db.into())
                    .map_err(SerialError::from)?;
            }
            if let Some(sb) = stop_bits {
                io.reconfigure_stop_bits(sb.into())
                    .map_err(SerialError::from)?;
            }
            if let Some(p) = parity {
                io.reconfigure_parity(p.into()).map_err(SerialError::from)?;
            }
            if let Some(fc) = flow_control {
                io.set_flow_control(fc).map_err(SerialError::from)?;
            }
        }

        // Update stored configuration.
        if let Some(rate) = baud_rate {
            *self.baud_rate.lock().expect("poisoned") = rate;
        }
        if let Some(db) = data_bits {
            *self.data_bits.lock().expect("poisoned") = db;
        }
        if let Some(sb) = stop_bits {
            *self.stop_bits.lock().expect("poisoned") = sb;
        }
        if let Some(p) = parity {
            *self.parity.lock().expect("poisoned") = p;
        }
        if let Some(fc) = flow_control {
            *self.flow_control.lock().expect("poisoned") = fc;
        }

        Ok(self.status_snapshot())
    }

    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        self.set_state(ConnectionState::Closed);
        self.close_token.cancel();

        let mut io = self.io.lock().await;
        if let Some(mut io) = io.take() {
            io.clear_os_buffers(FlushTarget::Input)
                .map_err(SerialError::from)?;
            io.shutdown().await?;
        }
        Ok(())
    }
}

fn ensure_valid_baud_rate(baud_rate: u32) -> Result<()> {
    if baud_rate == 0 || baud_rate > MAX_BAUD_RATE {
        Err(SerialError::InvalidBaudRate(baud_rate))
    } else {
        Ok(())
    }
}

fn build_stream(config: &ConnectionConfig) -> Result<SerialStream> {
    tokio_serial::new(&config.port, config.baud_rate)
        .data_bits(config.data_bits.into())
        .stop_bits(config.stop_bits.into())
        .parity(config.parity.into())
        .flow_control(config.flow_control.into())
        .open_native_async()
        .map_err(|e| SerialError::OpenFailed(format!("{}: {}", config.port, e)))
}

/// Test-only helpers for `SerialConnection`.
#[cfg(test)]
impl SerialConnection {
    /// Set the connection state directly. Only available in tests so unit
    /// tests can exercise `disconnect_state` classification without
    /// driving the full reconnect state machine.
    pub(crate) fn set_state_for_test(&mut self, state: ConnectionState) {
        *self.state.lock().unwrap() = state;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::serial::manager::ConnectionManager;
    use crate::serial::test_support::loopback_connection;
    use crate::stop_controller::RxStopController;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn baud_rate_zero_rejected() {
        assert!(matches!(
            ensure_valid_baud_rate(0),
            Err(SerialError::InvalidBaudRate(0))
        ));
    }

    #[test]
    fn baud_rate_over_max_rejected() {
        assert!(matches!(
            ensure_valid_baud_rate(MAX_BAUD_RATE + 1),
            Err(SerialError::InvalidBaudRate(_))
        ));
    }

    #[test]
    fn baud_rate_within_range_accepted() {
        assert!(ensure_valid_baud_rate(115200).is_ok());
        assert!(ensure_valid_baud_rate(1).is_ok());
        assert!(ensure_valid_baud_rate(MAX_BAUD_RATE).is_ok());
    }

    #[tokio::test]
    async fn write_pushes_bytes_to_peer() {
        let (conn, mut peer) = loopback_connection("test");
        let n = conn.write(b"hello").await.unwrap();
        assert_eq!(n, 5);
        let mut buf = [0u8; 5];
        peer.read_exact(&mut buf).await.unwrap();
        assert_eq!(&buf, b"hello");
    }

    #[tokio::test]
    async fn read_returns_peer_bytes() {
        let (conn, mut peer) = loopback_connection("test");
        peer.write_all(b"world").await.unwrap();
        let mut buf = [0u8; 5];
        let n = conn.read(&mut buf, Some(500)).await.unwrap();
        assert_eq!(n, 5);
        assert_eq!(&buf, b"world");
    }

    #[tokio::test]
    async fn read_times_out_when_no_data() {
        let (conn, _peer) = loopback_connection("test");
        let mut buf = [0u8; 16];
        let result = conn.read(&mut buf, Some(40)).await;
        assert!(matches!(result, Err(SerialError::ReadTimeout)));
    }

    #[tokio::test]
    async fn flush_set_dtr_rts_send_break_are_noops_on_loopback() {
        let (conn, _peer) = loopback_connection("test");
        conn.flush_buffers(FlushTarget::Both).await.unwrap();
        conn.set_dtr_rts(true, false).await.unwrap();
        conn.send_break(15).await.unwrap();
    }

    #[tokio::test]
    async fn close_cancels_inflight_read() {
        let mgr = ConnectionManager::new();
        let (conn, _peer) = loopback_connection("port-read-close");
        let id = mgr.insert(conn).await.unwrap();
        let connection = mgr.get(&id).await.unwrap();

        let reader = tokio::spawn(async move {
            let mut buf = [0u8; 16];
            connection.read(&mut buf, Some(2_000)).await
        });

        tokio::time::sleep(Duration::from_millis(100)).await;
        mgr.close(&id).await.unwrap();

        let err = reader.await.unwrap().unwrap_err();
        assert!(matches!(err, SerialError::ConnectionClosed(_)));
    }

    #[test]
    fn disconnect_state_maps_connection_state_correctly() {
        use crate::tools::rx_consume::{disconnect_state, DisconnectState};
        use std::time::Instant;

        let (mut conn, _peer) = loopback_connection("test");
        let mut ctrl = RxStopController::new(Instant::now(), Some(5000), 0, None);

        // Open → Active.
        assert!(matches!(
            disconnect_state(&conn, &mut ctrl),
            DisconnectState::Active
        ));

        // Set state to Closed → Closed.
        conn.set_state_for_test(ConnectionState::Closed);
        assert!(matches!(
            disconnect_state(&conn, &mut ctrl),
            DisconnectState::Closed
        ));

        // Set state to Reconnecting with reconnect enabled → Reconnecting.
        // Note: constructing a Reconnecting state requires a real serial port
        // (the reconnect state machine drives it), so this test directly sets
        // the internal state and enable flag.
        conn.set_state_for_test(ConnectionState::Reconnecting);
        conn.reconnect_policy.lock().unwrap().enabled = true;
        assert!(matches!(
            disconnect_state(&conn, &mut ctrl),
            DisconnectState::Reconnecting
        ));

        // Reconnecting with reconnect disabled → Closed.
        conn.reconnect_policy.lock().unwrap().enabled = false;
        assert!(matches!(
            disconnect_state(&conn, &mut ctrl),
            DisconnectState::Closed
        ));
    }
}
