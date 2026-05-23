//! Serial port discovery, configuration, and a session-less connection manager.
//!
//! Public surface:
//! - [`PortInfo::list_available`] enumerates serial ports on the host.
//! - [`SerialConnection::open`] opens a single configured port.
//! - [`ConnectionManager`] holds a set of open connections indexed by id and
//!   rejects double-opens of the same port.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serialport::{available_ports, SerialPortInfo, SerialPortType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tokio_serial::{ClearBuffer, SerialPort, SerialPortBuilderExt, SerialStream};
use uuid::Uuid;

use crate::error::{Result, SerialError};

/// Largest baud rate accepted by [`SerialConnection::open`]. Anything higher
/// is treated as a typo or accidental overflow and rejected.
pub const MAX_BAUD_RATE: u32 = 4_000_000;

// ---- Configuration enums -----------------------------------------------------

#[derive(Debug, Clone, Copy, Deserialize)]
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

#[derive(Debug, Clone, Copy, Deserialize)]
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

#[derive(Debug, Clone, Copy, Deserialize)]
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

#[derive(Debug, Clone, Copy, Deserialize)]
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
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: DataBits,
    pub stop_bits: StopBits,
    pub parity: Parity,
    pub flow_control: FlowControl,
}

// ---- Port enumeration --------------------------------------------------------

/// Information about a serial port reported by the OS.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PortInfo {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
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

// ---- Single open connection --------------------------------------------------

/// A single open serial port. Cheap to clone via [`Arc`] because all state lives
/// behind a [`Mutex`].
#[derive(Debug)]
pub struct SerialConnection {
    id: String,
    port: String,
    stream: Mutex<SerialStream>,
}

impl SerialConnection {
    /// Open a serial port using the supplied configuration.
    pub async fn open(config: ConnectionConfig) -> Result<Self> {
        ensure_valid_baud_rate(config.baud_rate)?;
        let stream = build_stream(&config)?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            port: config.port,
            stream: Mutex::new(stream),
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn port(&self) -> &str {
        &self.port
    }

    /// Write a byte slice, flushing before returning.
    pub async fn write(&self, data: &[u8]) -> Result<usize> {
        let mut stream = self.stream.lock().await;
        let written = stream.write(data).await?;
        stream.flush().await?;
        Ok(written)
    }

    /// Read up to `dst.len()` bytes. Returns [`SerialError::ReadTimeout`] if
    /// `timeout_ms` is set and elapses before any byte arrives.
    pub async fn read(&self, dst: &mut [u8], timeout_ms: Option<u64>) -> Result<usize> {
        let mut stream = self.stream.lock().await;
        match timeout_ms {
            Some(ms) => match timeout(Duration::from_millis(ms), stream.read(dst)).await {
                Ok(io_result) => Ok(io_result?),
                Err(_elapsed) => Err(SerialError::ReadTimeout),
            },
            None => Ok(stream.read(dst).await?),
        }
    }

    /// Discard data buffered in the OS for the input, output, or both
    /// directions of this port.
    pub async fn flush_buffers(&self, target: FlushTarget) -> Result<()> {
        let stream = self.stream.lock().await;
        stream.clear(target.into()).map_err(SerialError::from)
    }

    /// Drive the DTR and RTS control lines. Common use case: pulse DTR low
    /// to soft-reset an Arduino, or hold both low to enter the ESP32
    /// bootloader.
    pub async fn set_dtr_rts(&self, dtr: bool, rts: bool) -> Result<()> {
        let mut stream = self.stream.lock().await;
        stream.write_data_terminal_ready(dtr)?;
        stream.write_request_to_send(rts)?;
        Ok(())
    }

    /// Assert the BREAK condition on the TX line for `duration_ms`
    /// milliseconds, then release it.
    pub async fn send_break(&self, duration_ms: u64) -> Result<()> {
        let stream = self.stream.lock().await;
        stream.set_break()?;
        drop(stream);
        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
        let stream = self.stream.lock().await;
        stream.clear_break()?;
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
        .map_err(|e| SerialError::ConnectionFailed(format!("{}: {}", config.port, e)))
}

// ---- Multi-connection registry ----------------------------------------------

/// Registry of currently open serial connections, indexed by an opaque
/// connection id. Rejects opening the same port twice.
#[derive(Debug, Default)]
pub struct ConnectionManager {
    connections: Mutex<HashMap<String, Arc<SerialConnection>>>,
}

impl ConnectionManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open a new connection and store it. Returns the new connection id.
    pub async fn open(&self, config: ConnectionConfig) -> Result<String> {
        let mut connections = self.connections.lock().await;
        if is_port_in_use(&connections, &config.port) {
            return Err(SerialError::ConnectionExists(config.port));
        }
        let connection = Arc::new(SerialConnection::open(config).await?);
        let id = connection.id().to_string();
        connections.insert(id.clone(), connection);
        Ok(id)
    }

    /// Remove a connection. The serial port is closed when the last [`Arc`]
    /// reference is dropped, which happens here if no caller still holds one.
    pub async fn close(&self, id: &str) -> Result<()> {
        self.connections
            .lock()
            .await
            .remove(id)
            .ok_or_else(|| SerialError::InvalidConnection(id.to_string()))?;
        Ok(())
    }

    /// Look up an existing connection by id.
    pub async fn get(&self, id: &str) -> Result<Arc<SerialConnection>> {
        self.connections
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| SerialError::InvalidConnection(id.to_string()))
    }
}

fn is_port_in_use(
    connections: &HashMap<String, Arc<SerialConnection>>,
    port: &str,
) -> bool {
    connections.values().any(|c| c.port() == port)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
