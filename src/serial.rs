//! Serial port discovery, configuration, and a session-less connection manager.
//!
//! Public surface:
//! - [`PortInfo::list_available`] enumerates serial ports on the host.
//! - [`SerialConnection::open`] opens a single configured port.
//! - [`ConnectionManager`] holds a set of open connections indexed by id and
//!   rejects double-opens of the same port.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serialport::{available_ports, SerialPortInfo, SerialPortType};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_serial::{ClearBuffer, SerialPort, SerialPortBuilderExt, SerialStream};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::error::{Result, SerialError};

/// Largest baud rate accepted by [`SerialConnection::open`]. Anything higher
/// is treated as a typo or accidental overflow and rejected.
pub const MAX_BAUD_RATE: u32 = 4_000_000;

// ---- Configuration enums -----------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub enum DataBits {
    #[serde(rename = "5")]
    Five,
    #[serde(rename = "6")]
    Six,
    #[serde(rename = "7")]
    Seven,
    #[serde(rename = "8")]
    Eight,
}

impl From<DataBits> for serialport::DataBits {
    fn from(value: DataBits) -> Self {
        match value {
            DataBits::Five => Self::Five,
            DataBits::Six => Self::Six,
            DataBits::Seven => Self::Seven,
            DataBits::Eight => Self::Eight,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
pub enum StopBits {
    #[serde(rename = "1")]
    One,
    #[serde(rename = "2")]
    Two,
}

impl From<StopBits> for serialport::StopBits {
    fn from(value: StopBits) -> Self {
        match value {
            StopBits::One => Self::One,
            StopBits::Two => Self::Two,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    None,
    Odd,
    Even,
}

impl From<Parity> for serialport::Parity {
    fn from(value: Parity) -> Self {
        match value {
            Parity::None => Self::None,
            Parity::Odd => Self::Odd,
            Parity::Even => Self::Even,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FlowControl {
    None,
    Software,
    Hardware,
}

impl From<FlowControl> for serialport::FlowControl {
    fn from(value: FlowControl) -> Self {
        match value {
            FlowControl::None => Self::None,
            FlowControl::Software => Self::Software,
            FlowControl::Hardware => Self::Hardware,
        }
    }
}

pub(crate) fn data_bits_to_str(d: DataBits) -> String {
    match d {
        DataBits::Five => "5".into(),
        DataBits::Six => "6".into(),
        DataBits::Seven => "7".into(),
        DataBits::Eight => "8".into(),
    }
}

pub(crate) fn stop_bits_to_str(s: StopBits) -> String {
    match s {
        StopBits::One => "1".into(),
        StopBits::Two => "2".into(),
    }
}

pub(crate) fn parity_to_str(p: Parity) -> String {
    match p {
        Parity::None => "none".into(),
        Parity::Odd => "odd".into(),
        Parity::Even => "even".into(),
    }
}

pub(crate) fn flow_control_to_str(f: FlowControl) -> String {
    match f {
        FlowControl::None => "none".into(),
        FlowControl::Software => "software".into(),
        FlowControl::Hardware => "hardware".into(),
    }
}

/// Concrete parameters required to open a serial port.
#[derive(Debug, Clone, JsonSchema)]
pub struct ConnectionConfig {
    pub port: String,
    pub name: Option<String>,
    pub baud_rate: u32,
    pub data_bits: DataBits,
    pub stop_bits: StopBits,
    pub parity: Parity,
    pub flow_control: FlowControl,
    /// OS-level port identity (VID, PID, serial, transport, etc.)
    /// Captured at open time for status and profile save operations.
    pub port_info: Option<PortInfo>,
    /// Log buffer capacity in events. 0 or None disables logging.
    /// Default: 1024.
    #[serde(default = "default_log_capacity")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub log_capacity: usize,
    /// Whether logging is enabled. Default: true when capacity > 0.
    #[serde(default = "default_true")]
    pub log_enabled: bool,
}

fn default_log_capacity() -> usize {
    1024
}

fn default_true() -> bool {
    true
}

// ---- Connection state -------------------------------------------------------

/// The health state of a live serial connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Open,
    Disconnected,
    Reconnecting,
    Closed,
}

impl ConnectionState {
    pub fn is_healthy(&self) -> bool {
        matches!(self, ConnectionState::Open)
    }
}

/// Reconnect policy for a connection. When enabled and the port
/// disappears, the server will try to re-establish the connection
/// automatically with exponential backoff.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReconnectPolicy {
    /// Enable automatic reconnect. Default: false.
    #[serde(default)]
    pub enabled: bool,
    /// Maximum reconnect attempts. 0 = unlimited. Default: 10.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    #[serde(default = "default_max_reconnect_attempts")]
    pub max_attempts: u32,
    /// Initial delay between reconnect attempts in milliseconds. Default: 500.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    #[serde(default = "default_initial_reconnect_delay_ms")]
    pub initial_delay_ms: u64,
    /// Maximum delay between attempts in milliseconds. Default: 30000.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    #[serde(default = "default_max_reconnect_delay_ms")]
    pub max_delay_ms: u64,
    /// Backoff multiplier. Default: 2.0.
    #[serde(default = "default_backoff_multiplier")]
    pub backoff_multiplier: f64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            max_attempts: 10,
            initial_delay_ms: 500,
            max_delay_ms: 30_000,
            backoff_multiplier: 2.0,
        }
    }
}

fn default_max_reconnect_attempts() -> u32 {
    10
}
fn default_initial_reconnect_delay_ms() -> u64 {
    500
}
fn default_max_reconnect_delay_ms() -> u64 {
    30_000
}
fn default_backoff_multiplier() -> f64 {
    2.0
}

/// Classify an I/O error as a fatal disconnect (port vanished).
pub fn is_fatal_disconnect(err: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    matches!(
        err.kind(),
        ErrorKind::NotFound
            | ErrorKind::PermissionDenied
            | ErrorKind::ConnectionReset
            | ErrorKind::ConnectionAborted
            | ErrorKind::BrokenPipe
            | ErrorKind::Interrupted
    )
}

// ---- Port enumeration --------------------------------------------------------

/// Transport type observed on the host OS for a serial port.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PortTransport {
    Usb,
    Pci,
    Bluetooth,
    Unknown,
}

impl std::fmt::Display for PortTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PortTransport::Usb => f.write_str("usb"),
            PortTransport::Pci => f.write_str("pci"),
            PortTransport::Bluetooth => f.write_str("bluetooth"),
            PortTransport::Unknown => f.write_str("unknown"),
        }
    }
}

/// Information about a single serial port on the system.
///
/// Fields are populated from OS-level enumeration. USB ports carry
/// the richest identity; other transports provide more limited metadata.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct PortInfo {
    /// OS-level path, e.g. `/dev/ttyUSB0` or `COM3`.
    pub name: String,
    /// Short platform-local name, e.g. `ttyUSB0`.
    pub display_name: String,
    /// Human-readable description (manufacturer + product when available).
    pub description: String,
    /// Formatted hardware identifier string.
    pub hardware_id: Option<String>,
    /// Transport type — `usb`, `pci`, `bluetooth`, or `unknown`.
    pub transport: PortTransport,
    /// USB Vendor ID. `None` for non-USB ports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vid: Option<u16>,
    /// USB Product ID. `None` for non-USB ports.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u16>,
    /// USB serial number string from the device descriptor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial_number: Option<String>,
    /// USB manufacturer string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    /// USB product string.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product: Option<String>,
    /// USB interface index. `None` when not available or not a USB port.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface: Option<u8>,
}

impl PortInfo {
    /// Enumerate all serial ports the operating system currently exposes.
    pub fn list_available() -> Result<Vec<PortInfo>> {
        let ports = available_ports()?;
        Ok(ports.into_iter().map(PortInfo::from_os).collect())
    }

    fn from_os(port: SerialPortInfo) -> Self {
        let transport = transport_from_os(&port.port_type);
        let (vid, pid, serial_number, manufacturer, product, interface) =
            usb_fields(&port.port_type);
        let description = describe_port(&port);
        let hardware_id = format_hardware_id(&port);
        let display_name = short_display_name(&port.port_name);

        PortInfo {
            display_name,
            name: port.port_name,
            description,
            hardware_id,
            transport,
            vid,
            pid,
            serial_number,
            manufacturer,
            product,
            interface,
        }
    }
}

/// Extract the last path component or the full name when no separator exists.
fn short_display_name(port_name: &str) -> String {
    port_name
        .rsplit(&['/', '\\'][..])
        .next()
        .unwrap_or(port_name)
        .to_string()
}

fn transport_from_os(port_type: &SerialPortType) -> PortTransport {
    match port_type {
        SerialPortType::UsbPort(_) => PortTransport::Usb,
        SerialPortType::PciPort => PortTransport::Pci,
        SerialPortType::BluetoothPort => PortTransport::Bluetooth,
        SerialPortType::Unknown => PortTransport::Unknown,
    }
}

#[allow(clippy::type_complexity)]
fn usb_fields(
    port_type: &SerialPortType,
) -> (
    Option<u16>,
    Option<u16>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<u8>,
) {
    if let SerialPortType::UsbPort(info) = port_type {
        (
            Some(info.vid),
            Some(info.pid),
            info.serial_number.clone(),
            info.manufacturer.clone(),
            info.product.clone(),
            info.interface,
        )
    } else {
        (None, None, None, None, None, None)
    }
}

fn format_hardware_id(port: &SerialPortInfo) -> Option<String> {
    match &port.port_type {
        SerialPortType::UsbPort(info) => {
            Some(format!("USB VID:{:04X} PID:{:04X}", info.vid, info.pid))
        }
        SerialPortType::PciPort => Some("PCI".to_string()),
        SerialPortType::BluetoothPort => Some("Bluetooth".to_string()),
        SerialPortType::Unknown => None,
    }
}

fn describe_port(port: &SerialPortInfo) -> String {
    match &port.port_type {
        SerialPortType::UsbPort(info) => format!(
            "{} {}",
            info.manufacturer.as_deref().unwrap_or("Unknown"),
            info.product.as_deref().unwrap_or("USB Serial Device")
        ),
        SerialPortType::PciPort => "PCI Serial Port".to_string(),
        SerialPortType::BluetoothPort => "Bluetooth Serial Port".to_string(),
        SerialPortType::Unknown => "Serial Port".to_string(),
    }
}

// ---- I/O backend trait -------------------------------------------------------

/// Abstraction over the underlying byte stream plus the modem-control lines
/// of a serial port.
///
/// The production backend ([`SerialStream`]) talks to a real OS-level
/// serial port. Tests substitute an in-memory implementation backed by
/// [`tokio::io::duplex`] so that read/write/wait_for can be exercised
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
    /// Number of successful `read` or `subscribe` operations.
    read_ops: AtomicU64,
    /// Number of successful `write` operations.
    write_ops: AtomicU64,
    /// Number of RX operations where data was truncated
    /// (bytes_returned < bytes_observed).
    truncation_count: AtomicU64,
    /// Number of notification drops (encoding errors or disconnected peers).
    notification_drop_count: AtomicU64,
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
            notification_drop_count: AtomicU64::new(0),
            port_info: config.port_info,
            log: crate::log_buffer::LogBuffer::new_shared(config.log_capacity, config.log_enabled),
            state: StdMutex::new(ConnectionState::Open),
            reconnect_policy: StdMutex::new(ReconnectPolicy::default()),
            reconnect_attempts: AtomicU64::new(0),
            last_error: StdMutex::new(None),
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

    /// Record one successful read or subscribe operation.
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

    /// Record one notification drop (encoding error or disconnected peer).
    pub fn record_notification_drop(&self) {
        self.notification_drop_count.fetch_add(1, Ordering::SeqCst);
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
            notification_drop_count: self.notification_drop_count.load(Ordering::SeqCst),
            port_info: self.port_info.clone(),
            state: self.state(),
            reconnect_attempts: self.reconnect_attempts.load(Ordering::SeqCst),
            last_error: self
                .last_error
                .lock()
                .expect("poisoned")
                .as_ref()
                .map(|(_, msg)| msg.clone()),
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
    /// request/response pattern (`wait_for` + `write`) on CDC-ACM devices.
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
    pub async fn set_dtr_rts(&self, dtr: bool, rts: bool) -> Result<()> {
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

/// Which OS-side buffer(s) a flush should clear.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FlushTarget {
    /// Bytes the OS has received from the device but the app has not yet read.
    Input,
    /// Bytes the app has queued but the OS has not yet sent to the device.
    Output,
    /// Both input and output buffers.
    Both,
}

impl From<FlushTarget> for ClearBuffer {
    fn from(value: FlushTarget) -> Self {
        match value {
            FlushTarget::Input => ClearBuffer::Input,
            FlushTarget::Output => ClearBuffer::Output,
            FlushTarget::Both => ClearBuffer::All,
        }
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

// ---- Multi-connection registry ----------------------------------------------

/// Registry of currently open serial connections, indexed by an opaque
/// connection id. Rejects opening the same port twice.
#[derive(Debug, Default)]
pub struct ConnectionManager {
    state: Mutex<ConnectionRegistry>,
}

#[derive(Debug, Default)]
struct ConnectionRegistry {
    connections: HashMap<String, Arc<SerialConnection>>,
    opening_ports: HashSet<String>,
    closing_ports: HashSet<String>,
    reconnect_tasks: HashMap<String, tokio::task::JoinHandle<()>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new connection and store it. Returns the new connection id.
    pub async fn open(&self, config: ConnectionConfig) -> Result<String> {
        let port = config.port.clone();
        {
            let mut state = self.state.lock().await;
            if let Some(connection) = find_connection_by_port(&state.connections, &port) {
                return Err(SerialError::PortAlreadyOpen {
                    port,
                    connection_id: Some(connection.id().to_string()),
                    name: connection.name().map(str::to_string),
                });
            }
            if state.opening_ports.contains(&port) || state.closing_ports.contains(&port) {
                return Err(SerialError::PortAlreadyOpening(port));
            }
            state.opening_ports.insert(port.clone());
        }

        let opened = SerialConnection::open(config).await;
        let mut state = self.state.lock().await;
        state.opening_ports.remove(&port);
        let connection = Arc::new(opened?);
        let id = connection.id().to_string();
        state.connections.insert(id.clone(), connection);
        Ok(id)
    }

    /// Insert an already-built [`SerialConnection`] (typically one backed
    /// by an in-memory loopback) into the registry. Honours the same
    /// port-uniqueness invariant as [`Self::open`].
    ///
    /// Exposed for integration tests that want to drive the MCP surface
    /// against a fake connection without going through the OS serial layer.
    pub async fn insert(&self, connection: SerialConnection) -> Result<String> {
        let mut state = self.state.lock().await;
        if let Some(existing) = find_connection_by_port(&state.connections, connection.port()) {
            return Err(SerialError::PortAlreadyOpen {
                port: connection.port().to_string(),
                connection_id: Some(existing.id().to_string()),
                name: existing.name().map(str::to_string),
            });
        }
        if state.opening_ports.contains(connection.port())
            || state.closing_ports.contains(connection.port())
        {
            return Err(SerialError::PortAlreadyOpening(
                connection.port().to_string(),
            ));
        }
        let id = connection.id().to_string();
        state.connections.insert(id.clone(), Arc::new(connection));
        Ok(id)
    }

    /// Remove a connection, cancel in-flight operations, flush RX, and close
    /// the underlying port before allowing a reopen.
    pub async fn close(&self, id: &str) -> Result<()> {
        let connection = {
            let mut state = self.state.lock().await;
            let connection = state
                .connections
                .remove(id)
                .ok_or_else(|| SerialError::ConnectionNotFound(id.to_string()))?;
            state.closing_ports.insert(connection.port().to_string());
            connection
        };

        let port = connection.port().to_string();
        // Abort any running reconnect task for this connection.
        {
            let mut state = self.state.lock().await;
            if let Some(handle) = state.reconnect_tasks.remove(id) {
                handle.abort();
            }
        }
        connection.log().closed();
        let result = connection.close().await;

        self.state.lock().await.closing_ports.remove(&port);
        result
    }

    /// Look up an existing connection by id.
    pub async fn get(&self, id: &str) -> Result<Arc<SerialConnection>> {
        self.state
            .lock()
            .await
            .connections
            .get(id)
            .cloned()
            .ok_or_else(|| SerialError::ConnectionNotFound(id.to_string()))
    }

    /// Return all currently-registered connections with their ids.
    pub async fn list_all(&self) -> Vec<(String, Arc<SerialConnection>)> {
        self.state
            .lock()
            .await
            .connections
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    }

    /// Start a background reconnect task for the given connection.
    /// The task retries `reconnect()` with exponential backoff,
    /// respecting the connection's `ReconnectPolicy`. On success,
    /// restarts the RX pump via `rx_sessions`.
    pub async fn start_reconnect(
        &self,
        id: &str,
        rx_sessions: Arc<crate::rx_session::RxSessionManager>,
    ) {
        let conn = match self.get(id).await {
            Ok(c) => c,
            Err(_) => return,
        };
        let policy = conn.reconnect_policy.lock().expect("poisoned").clone();
        if !policy.enabled {
            return;
        }
        // Avoid spawning a duplicate task. Prune finished handles first.
        {
            let mut state = self.state.lock().await;
            state.reconnect_tasks.retain(|_, h| !h.is_finished());
            if state.reconnect_tasks.contains_key(id) {
                return;
            }
        }

        let id_owned = id.to_string();
        let conn_clone = Arc::clone(&conn);
        let handle = tokio::spawn(async move {
            let mut delay_ms = policy.initial_delay_ms;
            let mut attempts: u32 = 0;
            loop {
                // Check if still disconnected / not cancelled.
                let state = conn_clone.state();
                if state == ConnectionState::Open || state == ConnectionState::Closed {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                match conn_clone.reconnect().await {
                    Ok(()) => {
                        // Reset attempt counter after successful reconnect.
                        conn_clone.reconnect_attempts.store(0, Ordering::SeqCst);
                        // Restart the RX pump so data flows again.
                        if let Some(session) = rx_sessions.get(&id_owned).await {
                            session.ensure_pump_running();
                        }
                        break;
                    }
                    Err(_e) => {
                        attempts += 1;
                        if policy.max_attempts > 0 && attempts >= policy.max_attempts {
                            conn_clone
                                .log()
                                .record(None, crate::log_buffer::LogEvent::ReconnectExhausted);
                            break;
                        }
                        // Exponential backoff with cap.
                        delay_ms = ((delay_ms as f64) * policy.backoff_multiplier)
                            .min(policy.max_delay_ms as f64)
                            as u64;
                    }
                }
            }
            // Task completes: handle stays in reconnect_tasks; supervisor
            // prunes finished handles on its next poll.
        });

        let mut state = self.state.lock().await;
        state.reconnect_tasks.insert(id.to_string(), handle);
    }

    /// Cancel a running reconnect task for the given connection.
    pub async fn cancel_reconnect(&self, id: &str) {
        let mut state = self.state.lock().await;
        if let Some(handle) = state.reconnect_tasks.remove(id) {
            handle.abort();
        }
    }

    /// Number of currently open connections.
    pub async fn count(&self) -> usize {
        self.state.lock().await.connections.len()
    }

    /// Lightweight snapshot of all currently-open connections. Cheap because
    /// it only clones the id + port pair, not the underlying IO.
    pub async fn list_open(&self) -> Vec<ConnectionSummary> {
        self.state
            .lock()
            .await
            .connections
            .values()
            .map(|c| c.summary())
            .collect()
    }
}

/// Public-facing summary of an open connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionSummary {
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
    pub flow_control: FlowControl,
}

/// Full status snapshot of a connection used by the `get_status` tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionStatus {
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
    pub data_bits: String,
    pub stop_bits: String,
    pub parity: String,
    pub flow_control: String,
    pub is_closed: bool,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub tx_bytes: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_bytes: u64,
    /// Last I/O activity as milliseconds since Unix epoch, or null.
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub last_activity_ms: Option<u64>,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub read_ops: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub write_ops: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub truncation_count: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub notification_drop_count: u64,
    /// OS-level port identity captured at open time (vid, pid, serial,
    /// manufacturer, etc.). `null` for connections opened without
    /// identity data (e.g. loopback tests).
    pub port_info: Option<PortInfo>,
    /// Current connection health state.
    pub state: ConnectionState,
    /// Number of reconnect attempts since last disconnect.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub reconnect_attempts: u64,
    /// Last fatal error message, or null.
    pub last_error: Option<String>,
}

fn find_connection_by_port<'a>(
    connections: &'a HashMap<String, Arc<SerialConnection>>,
    port: &str,
) -> Option<&'a Arc<SerialConnection>> {
    connections.values().find(|c| c.port() == port)
}

// ---- Test support ----------------------------------------------------------

/// In-memory implementations of [`SerialIo`] used by unit and integration
/// tests that need a fake connection. The module is `pub` so that
/// integration tests in `tests/` can build a [`SerialConnection`] backed
/// by a [`tokio::io::DuplexStream`] without going through the OS serial
/// layer.
pub mod test_support {
    use std::pin::Pin;
    use std::task::{Context, Poll};

    use tokio::io::{DuplexStream, ReadBuf};

    use super::*;

    /// Wraps a [`DuplexStream`] half so it satisfies the [`SerialIo`] trait.
    /// All control-line operations are no-ops.
    pub struct LoopbackIo(DuplexStream);

    impl AsyncRead for LoopbackIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for LoopbackIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.0).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.0).poll_shutdown(cx)
        }
    }

    impl SerialIo for LoopbackIo {
        fn clear_os_buffers(&self, _target: FlushTarget) -> std::io::Result<()> {
            Ok(())
        }
        fn set_dtr_rts(&mut self, _dtr: bool, _rts: bool) -> std::io::Result<()> {
            Ok(())
        }
        fn set_flow_control(&mut self, _flow_control: FlowControl) -> std::io::Result<()> {
            Ok(())
        }
        fn set_break_state(&self, _asserted: bool) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Build an in-memory connection plus the peer end of the duplex.
    /// The peer can be driven directly by the test to push bytes into the
    /// connection's read side or to consume bytes the connection writes.
    pub fn loopback_connection(port: &str) -> (SerialConnection, DuplexStream) {
        let (a, b) = tokio::io::duplex(4096);
        let conn = SerialConnection::from_io(port.to_string(), Box::new(LoopbackIo(a)));
        (conn, b)
    }

    pub fn loopback_connection_with_config(
        config: ConnectionConfig,
    ) -> (SerialConnection, DuplexStream) {
        let (a, b) = tokio::io::duplex(4096);
        let conn = SerialConnection::from_io_with_config(config, Box::new(LoopbackIo(a)));
        (conn, b)
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::{loopback_connection, loopback_connection_with_config};
    use super::*;
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
    async fn manager_rejects_duplicate_port() {
        let mgr = ConnectionManager::new();
        let (c1, _p1) = loopback_connection("port-a");
        mgr.insert(c1).await.unwrap();
        let (c2, _p2) = loopback_connection("port-a");
        let err = mgr.insert(c2).await.unwrap_err();
        assert!(matches!(err, SerialError::PortAlreadyOpen { .. }));
    }

    #[tokio::test]
    async fn manager_duplicate_port_error_includes_owner_metadata() {
        let mgr = ConnectionManager::new();
        let (c1, _peer_a) = loopback_connection_with_config(ConnectionConfig {
            port: "port-owner".into(),
            name: Some("console".into()),
            baud_rate: 115200,
            data_bits: DataBits::Eight,
            stop_bits: StopBits::One,
            parity: Parity::None,
            flow_control: FlowControl::None,
            port_info: None,
            log_capacity: 1024,
            log_enabled: true,
        });
        let owner_id = mgr.insert(c1).await.unwrap();

        let (c2, _p2) = loopback_connection("port-owner");
        let err = mgr.insert(c2).await.unwrap_err();
        match err {
            SerialError::PortAlreadyOpen {
                port,
                connection_id,
                name,
            } => {
                assert_eq!(port, "port-owner");
                assert_eq!(connection_id.as_deref(), Some(owner_id.as_str()));
                assert_eq!(name.as_deref(), Some("console"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn manager_close_then_get_returns_connection_not_found() {
        let mgr = ConnectionManager::new();
        let (c, _p) = loopback_connection("port-z");
        let id = mgr.insert(c).await.unwrap();
        mgr.close(&id).await.unwrap();
        let err = mgr.get(&id).await.unwrap_err();
        assert!(matches!(err, SerialError::ConnectionNotFound(_)));
    }

    #[tokio::test]
    async fn manager_get_unknown_id_returns_connection_not_found() {
        let mgr = ConnectionManager::new();
        let err = mgr.get("does-not-exist").await.unwrap_err();
        assert!(matches!(err, SerialError::ConnectionNotFound(_)));
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
}
