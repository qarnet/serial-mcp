//! Serial-line and lifecycle data/config declarations: the configuration
//! enums (`DataBits`/`StopBits`/`Parity`/`FlowControl`) with serialport
//! conversions, string parsing, and to-string helpers; `ConnectionConfig`
//! and its defaults; connection state types (`ConnectionState`,
//! `ReconnectPolicy`, `is_fatal_disconnect`); the active profile-session
//! binding; `FlushTarget`; and the connection summary/status snapshot
//! shapes. `PortInfo` is imported from the sibling `port_info` module.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_serial::ClearBuffer;

use super::port_info::PortInfo;

/// Largest baud rate accepted by [`crate::serial::SerialConnection::open`].
/// Anything higher is treated as a typo or accidental overflow and rejected.
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

// ---- String parsing (single source of truth) --------------------------------

impl std::str::FromStr for DataBits {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "5" => Ok(DataBits::Five),
            "6" => Ok(DataBits::Six),
            "7" => Ok(DataBits::Seven),
            "8" => Ok(DataBits::Eight),
            other => Err(format!("Invalid data_bits {other:?} (expected 5/6/7/8)")),
        }
    }
}

impl std::str::FromStr for StopBits {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "1" => Ok(StopBits::One),
            "2" => Ok(StopBits::Two),
            other => Err(format!("Invalid stop_bits {other:?} (expected 1/2)")),
        }
    }
}

impl std::str::FromStr for Parity {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(Parity::None),
            "odd" => Ok(Parity::Odd),
            "even" => Ok(Parity::Even),
            other => Err(format!("Invalid parity {other:?} (expected none/odd/even)")),
        }
    }
}

impl std::str::FromStr for FlowControl {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(FlowControl::None),
            "software" => Ok(FlowControl::Software),
            "hardware" => Ok(FlowControl::Hardware),
            other => Err(format!(
                "Invalid flow_control {other:?} (expected none/software/hardware)"
            )),
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
    /// Default TX framing applied when `write` omits `tx_framing`.
    #[serde(default)]
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    /// Default RX framing applied when `read`/`subscribe` omit `rx_framing`.
    #[serde(default)]
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    /// Default RX parser applied when `read`/`subscribe` omit `rx_parser`.
    #[serde(default)]
    pub rx_parser: Option<crate::framing::ParserConfig>,
    /// Default protocol preset. Expands to fill framing/parser gaps.
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
    /// RX ring buffer size in bytes for the always-on pump.
    #[serde(default = "default_rx_buffer_size")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_buffer_size: usize,
    /// Default max buffered bytes for `read` operations.
    #[serde(default = "default_max_buffered_bytes")]
    #[schemars(schema_with = "crate::schema_helpers::read_max_buffered_bytes_schema")]
    pub max_buffered_bytes: usize,
    /// Default poll interval for `subscribe` in milliseconds.
    #[serde(default = "default_poll_interval_ms")]
    #[schemars(schema_with = "crate::schema_helpers::poll_interval_ms_schema")]
    pub poll_interval_ms: u64,
}

fn default_rx_buffer_size() -> usize {
    crate::limits::DEFAULT_RX_BUFFER_SIZE
}
fn default_log_capacity() -> usize {
    1024
}

fn default_true() -> bool {
    true
}

fn default_max_buffered_bytes() -> usize {
    32768
}

fn default_poll_interval_ms() -> u64 {
    200
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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

// ---- Active profile-session binding -----------------------------------------

/// Active profile-session binding stored on a connection.
///
/// Every connection opened through the public `open`/`open_profile` tools
/// carries one. Connections inserted directly by low-level tests may have
/// `None`. Converts losslessly into the wire type
/// [`crate::profiles::ProfileSessionResult`] for open/status/connection
/// summaries.
#[derive(Debug, Clone)]
pub struct ActiveProfileBinding {
    pub profile_name: String,
    pub source: crate::profiles::ProfileSelectionSource,
    pub confidence: crate::profiles::IdentityConfidence,
    pub persistent: bool,
    pub generated: bool,
    pub revision: Option<u64>,
    pub dirty: bool,
    /// `true` when the durable profile revision changed externally (CAS
    /// conflict or rollback) and this connection must not overwrite it.
    pub stale: bool,
    pub candidates: Vec<String>,
    pub last_persistence_error: Option<String>,
}

impl ActiveProfileBinding {
    /// Lossless conversion to the serializable session-result shape.
    pub fn to_session_result(&self) -> crate::profiles::ProfileSessionResult {
        crate::profiles::ProfileSessionResult {
            profile_name: self.profile_name.clone(),
            source: self.source,
            confidence: self.confidence,
            persistent: self.persistent,
            generated: self.generated,
            revision: self.revision,
            dirty: self.dirty,
            stale: self.stale,
            candidates: self.candidates.clone(),
            last_persistence_error: self.last_persistence_error.clone(),
        }
    }
}

// ---- Flush target -----------------------------------------------------------

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

// ---- Connection summary / status --------------------------------------------

/// Public-facing summary of an open connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectionSummary {
    pub connection_id: String,
    pub name: Option<String>,
    pub port: String,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub baud_rate: u32,
    pub flow_control: FlowControl,
    /// Active profile-session binding. `null` for connections inserted
    /// directly by low-level tests.
    pub profile: Option<crate::profiles::ProfileSessionResult>,
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
    /// Active profile-session binding. `null` for connections inserted
    /// directly by low-level tests.
    pub profile: Option<crate::profiles::ProfileSessionResult>,
}

// ---- Config parsing tests ---------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_str_parses_config_enums() {
        use std::str::FromStr;
        assert!(matches!("5".parse::<DataBits>(), Ok(DataBits::Five)));
        assert!(matches!("8".parse::<DataBits>(), Ok(DataBits::Eight)));
        assert!("9".parse::<DataBits>().is_err());

        assert!(matches!("1".parse::<StopBits>(), Ok(StopBits::One)));
        assert!(matches!("2".parse::<StopBits>(), Ok(StopBits::Two)));
        assert!("3".parse::<StopBits>().is_err());

        // Parity / flow_control are case-insensitive (the intended shared behavior).
        assert!(matches!(Parity::from_str("none"), Ok(Parity::None)));
        assert!(matches!(Parity::from_str("Even"), Ok(Parity::Even)));
        assert!(matches!("ODD".parse::<Parity>(), Ok(Parity::Odd)));
        assert!("weird".parse::<Parity>().is_err());

        assert!(matches!(
            "NONE".parse::<FlowControl>(),
            Ok(FlowControl::None)
        ));
        assert!(matches!(
            "Software".parse::<FlowControl>(),
            Ok(FlowControl::Software)
        ));
        assert!(matches!(
            "hardware".parse::<FlowControl>(),
            Ok(FlowControl::Hardware)
        ));
        assert!("xon".parse::<FlowControl>().is_err());
    }

    #[test]
    fn from_str_round_trips_with_to_str() {
        for d in [
            DataBits::Five,
            DataBits::Six,
            DataBits::Seven,
            DataBits::Eight,
        ] {
            let s = data_bits_to_str(d);
            assert_eq!(data_bits_to_str(s.parse::<DataBits>().unwrap()), s);
        }
        for sb in [StopBits::One, StopBits::Two] {
            let s = stop_bits_to_str(sb);
            assert_eq!(stop_bits_to_str(s.parse::<StopBits>().unwrap()), s);
        }
        for p in [Parity::None, Parity::Odd, Parity::Even] {
            let s = parity_to_str(p);
            assert_eq!(parity_to_str(s.parse::<Parity>().unwrap()), s);
        }
        for f in [
            FlowControl::None,
            FlowControl::Software,
            FlowControl::Hardware,
        ] {
            let s = flow_control_to_str(f);
            assert_eq!(flow_control_to_str(s.parse::<FlowControl>().unwrap()), s);
        }
    }

    #[test]
    fn from_str_error_messages_are_descriptive() {
        assert!("9"
            .parse::<DataBits>()
            .unwrap_err()
            .contains("expected 5/6/7/8"));
        assert!("3"
            .parse::<StopBits>()
            .unwrap_err()
            .contains("expected 1/2"));
        assert!("weird"
            .parse::<Parity>()
            .unwrap_err()
            .contains("expected none/odd/even"));
        assert!("xon"
            .parse::<FlowControl>()
            .unwrap_err()
            .contains("expected none/software/hardware"));
    }
}
