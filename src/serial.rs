//! Serial port discovery, configuration, and a session-less connection manager.
//!
//! Public surface:
//! - [`PortInfo::list_available`] enumerates serial ports on the host.
//! - [`SerialConnection::open`] opens a single configured port.
//! - [`ConnectionManager`] holds a set of open connections indexed by id and
//!   rejects double-opens of the same port.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
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
}

// ---- Port enumeration --------------------------------------------------------

/// Information about a serial port reported by the OS.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
    pub hardware_id: Option<String>,
}

impl PortInfo {
    /// Enumerate all serial ports the operating system currently exposes.
    pub fn list_available() -> Result<Vec<PortInfo>> {
        let ports = available_ports()?;
        Ok(ports.into_iter().map(PortInfo::from_os).collect())
    }

    fn from_os(port: SerialPortInfo) -> Self {
        PortInfo {
            hardware_id: format_hardware_id(&port),
            description: describe_port(&port),
            name: port.port_name,
        }
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
    baud_rate: u32,
    flow_control: StdMutex<FlowControl>,
    io: Mutex<Option<Box<dyn SerialIo>>>,
    close_token: CancellationToken,
    closed: AtomicBool,
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
            },
            io,
        )
    }

    pub fn from_io_with_config(config: ConnectionConfig, io: Box<dyn SerialIo>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            port: config.port,
            name: config.name,
            baud_rate: config.baud_rate,
            flow_control: StdMutex::new(config.flow_control),
            io: Mutex::new(Some(io)),
            close_token: CancellationToken::new(),
            closed: AtomicBool::new(false),
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
        self.baud_rate
    }

    pub fn flow_control(&self) -> FlowControl {
        *self
            .flow_control
            .lock()
            .expect("flow_control mutex poisoned")
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
        let mut io = self.io.lock().await;
        let io = io
            .as_mut()
            .ok_or_else(|| SerialError::ConnectionClosed(self.display_name()))?;
        io.write_all(data).await?;
        io.flush().await?;
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
                tokio::select! {
                    _ = self.close_token.cancelled() => Err(SerialError::ConnectionClosed(self.display_name())),
                    res = io.read(dst) => Ok(res?),
                }
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
                            Ok(Ok(n)) if n > 0 => return Ok(n),
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
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema)]
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
