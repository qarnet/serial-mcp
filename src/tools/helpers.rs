use std::sync::Arc;

use tracing::error;

use crate::codec::Encoding;
use crate::serial::{ConnectionConfig, ConnectionManager, SerialConnection};
use crate::tools::types::*;

pub use crate::limits::{MAX_READ_BYTES, MAX_TIMEOUT_MS, MAX_WRITE_BYTES, MIN_READ_BYTES};

pub(crate) const DEFAULT_READ_TIMEOUT_MS: u64 = 1000;

pub fn clamp_or_err(name: &str, value: usize, max: usize) -> Result<usize, String> {
    if value > max {
        Err(format!("{name}={value} exceeds maximum {max}"))
    } else {
        Ok(value)
    }
}

pub fn require_min_or_err(name: &str, value: usize, min: usize) -> Result<usize, String> {
    if value < min {
        Err(format!("{name}={value} is below minimum {min}"))
    } else {
        Ok(value)
    }
}

pub fn clamp_timeout_or_err(name: &str, value: u64, max: u64) -> Result<u64, String> {
    if value > max {
        Err(format!("{name}={value}ms exceeds maximum {max}ms"))
    } else {
        Ok(value)
    }
}

// ------------------------------------------------------------------
// Budget error mapping
// ------------------------------------------------------------------

/// Map a [`crate::buffer_budget::BufferBudgetError`] to a user-facing error
/// string. `field` is the fully-qualified argument name
/// (e.g. `"read.max_buffered_bytes"`) used to prefix the limit/zero messages.
pub fn map_budget_err(field: &str, e: crate::buffer_budget::BufferBudgetError) -> String {
    use crate::buffer_budget::BufferBudgetError;
    match e {
        BufferBudgetError::OverToolLimit {
            requested,
            tool_limit,
        } => format!("{field}={requested} exceeds per-tool limit {tool_limit}"),
        BufferBudgetError::ZeroRequest => format!("{field} must be > 0"),
        BufferBudgetError::InsufficientProgramBudget {
            requested,
            available,
        } => format!(
            "insufficient program buffer budget: requested {requested}, available {available}"
        ),
    }
}

// ------------------------------------------------------------------
// Connection lookup
// ------------------------------------------------------------------

pub async fn lookup_connection(
    connections: &Arc<ConnectionManager>,
    id: &str,
) -> Result<Arc<SerialConnection>, String> {
    connections
        .get(id)
        .await
        .map_err(|_| format!("Connection ID {id} not found"))
}

// ------------------------------------------------------------------
// Parsers
// ------------------------------------------------------------------

pub fn parse_encoding(raw: &str) -> Result<Encoding, String> {
    raw.parse::<Encoding>()
        .map_err(|e| format!("Unsupported encoding - {e}"))
}

// ------------------------------------------------------------------
// Open-settings resolution
// ------------------------------------------------------------------

/// Optional open-field overlay shared by `open` and `open_profile`.
///
/// `None` fields fall through to the selected profile's defaults, then to
/// built-in defaults. `open` fills the overlay from `OpenArgs`;
/// `open_profile` fills only `name`/`log_capacity`/`log_enabled`/
/// `rx_buffer_size` from `OpenProfileArgs` (all other fields come from the
/// named profile).
#[derive(Debug, Clone, Default)]
pub struct OpenOverlay {
    pub(crate) name: Option<String>,
    pub(crate) baud_rate: Option<u32>,
    pub(crate) data_bits: Option<String>,
    pub(crate) stop_bits: Option<String>,
    pub(crate) parity: Option<String>,
    pub(crate) flow_control: Option<String>,
    pub(crate) log_capacity: Option<usize>,
    pub(crate) log_enabled: Option<bool>,
    pub(crate) reconnect_policy: Option<crate::serial::ReconnectPolicy>,
    pub(crate) tx_framing: Option<crate::framing::TxFramingConfig>,
    pub(crate) rx_framing: Option<crate::framing::RxFramingConfig>,
    pub(crate) rx_parser: Option<crate::framing::ParserConfig>,
    pub(crate) protocol: Option<crate::framing::ProtocolPreset>,
    pub(crate) rx_buffer_size: Option<usize>,
    pub(crate) max_buffered_bytes: Option<usize>,
}

impl OpenOverlay {
    /// Overlay from the bare `open` tool's arguments.
    pub(crate) fn from_open_args(args: &OpenArgs) -> Self {
        Self {
            name: args.name.clone(),
            baud_rate: args.baud_rate,
            data_bits: args.data_bits.clone(),
            stop_bits: args.stop_bits.clone(),
            parity: args.parity.clone(),
            flow_control: args.flow_control.clone(),
            log_capacity: args.log_capacity,
            log_enabled: args.log_enabled,
            reconnect_policy: args.reconnect_policy.clone(),
            tx_framing: args.tx_framing.clone(),
            rx_framing: args.rx_framing.clone(),
            rx_parser: args.rx_parser.clone(),
            protocol: args.protocol,
            rx_buffer_size: args.rx_buffer_size,
            max_buffered_bytes: args.max_buffered_bytes,
        }
    }

    /// Overlay from the `open_profile` tool's override arguments. Only the
    /// fields `open_profile` exposes are populated; everything else falls
    /// through to the named profile's defaults.
    pub(crate) fn from_open_profile_args(args: &OpenProfileArgs) -> Self {
        Self {
            name: args.name.clone(),
            log_capacity: args.log_capacity,
            log_enabled: args.log_enabled,
            rx_buffer_size: args.rx_buffer_size,
            ..Self::default()
        }
    }
}

/// Concrete, fully-resolved open settings after merging explicit open
/// fields, a selected profile's defaults, and built-in defaults
/// (115200/8-N-1, 256 KiB ring, etc.). `ConnectionConfig` is built from
/// this; no `unwrap_or` precedence is scattered across tool logic.
#[derive(Debug, Clone)]
pub struct ResolvedOpenSettings {
    pub port: String,
    pub name: Option<String>,
    pub baud_rate: u32,
    pub data_bits: crate::serial::DataBits,
    pub stop_bits: crate::serial::StopBits,
    pub parity: crate::serial::Parity,
    pub flow_control: crate::serial::FlowControl,
    pub log_capacity: usize,
    pub log_enabled: bool,
    pub reconnect_policy: crate::serial::ReconnectPolicy,
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    pub rx_parser: Option<crate::framing::ParserConfig>,
    pub protocol: Option<crate::framing::ProtocolPreset>,
    pub rx_buffer_size: usize,
    pub max_buffered_bytes: usize,
}

impl PartialEq for ResolvedOpenSettings {
    fn eq(&self, other: &Self) -> bool {
        self.port == other.port
            && self.name == other.name
            && self.baud_rate == other.baud_rate
            && crate::serial::data_bits_to_str(self.data_bits)
                == crate::serial::data_bits_to_str(other.data_bits)
            && crate::serial::stop_bits_to_str(self.stop_bits)
                == crate::serial::stop_bits_to_str(other.stop_bits)
            && crate::serial::parity_to_str(self.parity)
                == crate::serial::parity_to_str(other.parity)
            && crate::serial::flow_control_to_str(self.flow_control)
                == crate::serial::flow_control_to_str(other.flow_control)
            && self.log_capacity == other.log_capacity
            && self.log_enabled == other.log_enabled
            && self.reconnect_policy == other.reconnect_policy
            && self.tx_framing == other.tx_framing
            && self.rx_framing == other.rx_framing
            && self.rx_parser == other.rx_parser
            && self.protocol == other.protocol
            && self.rx_buffer_size == other.rx_buffer_size
            && self.max_buffered_bytes == other.max_buffered_bytes
    }
}

impl ResolvedOpenSettings {
    /// Resolve `overlay` against `profile_defaults` (the selected profile's
    /// defaults) and the built-in defaults. Parsing failures (invalid data
    /// bits etc.) return a tool error.
    pub fn resolve(
        port: String,
        overlay: &OpenOverlay,
        profile_defaults: Option<&crate::profiles::ProfileDefaults>,
    ) -> Result<Self, String> {
        let builtin = crate::profiles::ProfileDefaults::default();
        let base = profile_defaults.unwrap_or(&builtin);

        // Connection name: explicit name, else the profile's name prefix
        // (expanded to `{prefix}-{short_port_name}`), else none.
        let name = match &overlay.name {
            Some(n) => Some(n.clone()),
            None => base.name.as_ref().map(|prefix| {
                let short = port.rsplit('/').next().unwrap_or(&port);
                format!("{prefix}-{short}")
            }),
        };

        let data_bits = overlay
            .data_bits
            .clone()
            .unwrap_or_else(|| base.data_bits.clone())
            .parse()?;
        let stop_bits = overlay
            .stop_bits
            .clone()
            .unwrap_or_else(|| base.stop_bits.clone())
            .parse()?;
        let parity = overlay
            .parity
            .clone()
            .unwrap_or_else(|| base.parity.clone())
            .parse()?;
        let flow_control = overlay
            .flow_control
            .clone()
            .unwrap_or_else(|| base.flow_control.clone())
            .parse()?;

        let rx_buffer_size = overlay.rx_buffer_size.unwrap_or(base.rx_buffer_size);
        let rx_buffer_size = validate_open_rx_buffer_size(rx_buffer_size)?;

        Ok(Self {
            port,
            name,
            baud_rate: overlay.baud_rate.unwrap_or(base.baud_rate),
            data_bits,
            stop_bits,
            parity,
            flow_control,
            log_capacity: overlay.log_capacity.unwrap_or(base.log_capacity),
            log_enabled: overlay.log_enabled.unwrap_or(base.log_enabled),
            reconnect_policy: overlay
                .reconnect_policy
                .clone()
                .unwrap_or_else(|| base.reconnect_policy.clone()),
            tx_framing: overlay
                .tx_framing
                .clone()
                .or_else(|| base.tx_framing.clone()),
            rx_framing: overlay
                .rx_framing
                .clone()
                .or_else(|| base.rx_framing.clone()),
            rx_parser: overlay.rx_parser.clone().or_else(|| base.rx_parser.clone()),
            protocol: overlay.protocol.or(base.protocol),
            rx_buffer_size,
            max_buffered_bytes: overlay
                .max_buffered_bytes
                .unwrap_or(base.max_buffered_bytes),
        })
    }

    /// The settings a profile alone would produce (no explicit overlay),
    /// used to detect whether explicit fields override the profile
    /// (`dirty`).
    pub fn from_profile(port: String, profile: &crate::profiles::Profile) -> Result<Self, String> {
        Self::resolve(port, &OpenOverlay::default(), Some(&profile.defaults))
    }

    /// Build the concrete `ConnectionConfig` for hardware open.
    pub fn into_connection_config(
        self,
        port_info: Option<crate::serial::PortInfo>,
    ) -> ConnectionConfig {
        ConnectionConfig {
            port: self.port,
            name: self.name,
            baud_rate: self.baud_rate,
            data_bits: self.data_bits,
            stop_bits: self.stop_bits,
            parity: self.parity,
            flow_control: self.flow_control,
            port_info,
            log_capacity: self.log_capacity,
            log_enabled: self.log_enabled,
            tx_framing: self.tx_framing,
            rx_framing: self.rx_framing,
            rx_parser: self.rx_parser,
            protocol: self.protocol,
            rx_buffer_size: self.rx_buffer_size,
            max_buffered_bytes: self.max_buffered_bytes,
        }
    }

    /// The effective settings as profile defaults (used for generated
    /// profiles, whose defaults equal the effective live open settings).
    pub fn as_profile_defaults(&self) -> crate::profiles::ProfileDefaults {
        crate::profiles::ProfileDefaults {
            baud_rate: self.baud_rate,
            data_bits: crate::serial::data_bits_to_str(self.data_bits),
            stop_bits: crate::serial::stop_bits_to_str(self.stop_bits),
            parity: crate::serial::parity_to_str(self.parity),
            flow_control: crate::serial::flow_control_to_str(self.flow_control),
            name: self.name.clone(),
            tx_framing: self.tx_framing.clone(),
            rx_framing: self.rx_framing.clone(),
            rx_parser: self.rx_parser.clone(),
            protocol: self.protocol,
            rx_buffer_size: self.rx_buffer_size,
            max_buffered_bytes: self.max_buffered_bytes,
            reconnect_policy: self.reconnect_policy.clone(),
            log_capacity: self.log_capacity,
            log_enabled: self.log_enabled,
        }
    }
}

/// Validate a resolved open `rx_buffer_size` (min 1, max 16 MiB ceiling).
fn validate_open_rx_buffer_size(size: usize) -> Result<usize, String> {
    use crate::limits::MAX_RX_BUFFER_SIZE;
    let size = require_min_or_err("open.rx_buffer_size", size, 1)?;
    clamp_or_err("open.rx_buffer_size", size, MAX_RX_BUFFER_SIZE)
}

pub fn parse_open_args(args: OpenArgs) -> Result<ConnectionConfig, String> {
    let overlay = OpenOverlay::from_open_args(&args);
    let port = args.port;
    let resolved = ResolvedOpenSettings::resolve(port, &overlay, None)?;
    Ok(resolved.into_connection_config(None))
}

// ------------------------------------------------------------------
// Error helper
// ------------------------------------------------------------------

pub fn log_tool_err<E: std::fmt::Display>(op: &str, context: &str, err: E) -> String {
    error!("{op} failed: {err}");
    format!("{context} - {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_args_parsed_strictly() {
        let args = OpenArgs {
            port: "/dev/ttyUSB0".into(),
            name: Some("console".into()),
            baud_rate: Some(115200),
            data_bits: Some("8".into()),
            stop_bits: Some("1".into()),
            parity: Some("none".into()),
            flow_control: Some("none".into()),
            log_capacity: Some(1024),
            log_enabled: Some(true),
            reconnect_policy: Some(Default::default()),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: Some(crate::limits::DEFAULT_RX_BUFFER_SIZE),
            max_buffered_bytes: Some(32768),
            profile_mode: None,
        };
        let config = parse_open_args(args).unwrap();
        assert_eq!(config.port, "/dev/ttyUSB0");
        assert_eq!(config.name.as_deref(), Some("console"));
        assert_eq!(config.baud_rate, 115200);
    }

    #[test]
    fn open_args_reject_invalid_data_bits() {
        let args = OpenArgs {
            port: "X".into(),
            name: None,
            baud_rate: Some(9600),
            data_bits: Some("9".into()),
            stop_bits: Some("1".into()),
            parity: Some("none".into()),
            flow_control: Some("none".into()),
            log_capacity: Some(1024),
            log_enabled: Some(true),
            reconnect_policy: Some(Default::default()),
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: Some(crate::limits::DEFAULT_RX_BUFFER_SIZE),
            max_buffered_bytes: Some(32768),
            profile_mode: None,
        };
        let err = parse_open_args(args).unwrap_err();
        assert!(err.contains("data_bits"));
    }

    // ── Open-settings resolution precedence ───────────────────────────────

    #[test]
    fn omitted_open_fields_fall_back_to_builtin_defaults() {
        let args = OpenArgs {
            port: "/dev/ttyACM0".into(),
            name: None,
            baud_rate: None,
            data_bits: None,
            stop_bits: None,
            parity: None,
            flow_control: None,
            log_capacity: None,
            log_enabled: None,
            reconnect_policy: None,
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: None,
            max_buffered_bytes: None,
            profile_mode: None,
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            None,
        )
        .unwrap();
        assert_eq!(resolved.baud_rate, 115200, "built-in 115200 fallback");
        assert_eq!(resolved.name, None);
        assert_eq!(resolved.log_capacity, 1024);
        assert_eq!(
            resolved.rx_buffer_size,
            crate::limits::DEFAULT_RX_BUFFER_SIZE
        );
        assert_eq!(resolved.max_buffered_bytes, 32768);
        assert!(!resolved.reconnect_policy.enabled);
        assert_eq!(
            resolved.into_connection_config(None).baud_rate,
            115200,
            "config carries the resolved baud"
        );
    }

    #[test]
    fn explicit_open_field_overrides_profile_default() {
        let profile = crate::profiles::Profile {
            name: "p".into(),
            selector: Default::default(),
            defaults: crate::profiles::ProfileDefaults {
                baud_rate: 9600,
                rx_buffer_size: 8192,
                name: Some("console".into()),
                ..Default::default()
            },
            metadata: Default::default(),
            revisions: Vec::new(),
        };
        let args = OpenArgs {
            port: "/dev/ttyACM0".into(),
            baud_rate: Some(115200),
            name: None,
            data_bits: None,
            stop_bits: None,
            parity: None,
            flow_control: None,
            log_capacity: None,
            log_enabled: None,
            reconnect_policy: None,
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: None,
            max_buffered_bytes: None,
            profile_mode: None,
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            Some(&profile.defaults),
        )
        .unwrap();
        assert_eq!(resolved.baud_rate, 115200, "explicit wins over profile");
        assert_eq!(resolved.rx_buffer_size, 8192, "omitted uses profile");
        assert_eq!(
            resolved.name.as_deref(),
            Some("console-ttyACM0"),
            "profile name prefix expanded"
        );
    }

    #[test]
    fn profile_only_settings_detect_dirty_overrides() {
        let profile = crate::profiles::Profile {
            name: "p".into(),
            selector: Default::default(),
            defaults: crate::profiles::ProfileDefaults {
                baud_rate: 9600,
                ..Default::default()
            },
            metadata: Default::default(),
            revisions: Vec::new(),
        };

        // Same effective settings as the profile → clean.
        let args = OpenArgs {
            port: "/dev/ttyACM0".into(),
            baud_rate: Some(9600),
            ..OpenArgs {
                port: "/dev/ttyACM0".into(),
                name: None,
                baud_rate: None,
                data_bits: None,
                stop_bits: None,
                parity: None,
                flow_control: None,
                log_capacity: None,
                log_enabled: None,
                reconnect_policy: None,
                tx_framing: None,
                rx_framing: None,
                rx_parser: None,
                protocol: None,
                rx_buffer_size: None,
                max_buffered_bytes: None,
                profile_mode: None,
            }
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            Some(&profile.defaults),
        )
        .unwrap();
        let profile_only =
            ResolvedOpenSettings::from_profile("/dev/ttyACM0".into(), &profile).unwrap();
        assert_eq!(
            resolved, profile_only,
            "explicit value equal to profile is not dirty"
        );

        // Different baud → dirty.
        let args = OpenArgs {
            baud_rate: Some(19200),
            ..args
        };
        let resolved = ResolvedOpenSettings::resolve(
            "/dev/ttyACM0".into(),
            &OpenOverlay::from_open_args(&args),
            Some(&profile.defaults),
        )
        .unwrap();
        assert_ne!(resolved, profile_only, "explicit override differs → dirty");
    }

    #[test]
    fn open_args_reject_invalid_parity() {
        use crate::serial::Parity;
        assert!("weird".parse::<Parity>().is_err());
        assert!("none".parse::<Parity>().is_ok());
        assert!("Even".parse::<Parity>().is_ok());
    }

    #[test]
    fn parse_encoding_rejects_garbage() {
        assert!(parse_encoding("rot13").is_err());
        assert!(parse_encoding("utf-8").is_ok());
    }

    #[test]
    fn clamp_or_err_rejects_oversized_values() {
        assert!(clamp_or_err("test.max_bytes", 1024 * 1024, MAX_READ_BYTES).is_ok());
        assert!(clamp_or_err("test.max_bytes", 1024 * 1024 + 1, MAX_READ_BYTES).is_err());
        assert!(clamp_or_err("test.max_bytes", usize::MAX, MAX_WRITE_BYTES).is_err());
    }

    #[test]
    fn require_min_or_err_rejects_undersized_values() {
        assert!(require_min_or_err("test.max_bytes", 1, MIN_READ_BYTES).is_ok());
        assert!(require_min_or_err("test.max_bytes", 0, MIN_READ_BYTES).is_err());
    }

    #[test]
    fn clamp_timeout_or_err_rejects_oversized_timeout() {
        assert!(clamp_timeout_or_err("test.timeout_ms", 1000, MAX_TIMEOUT_MS).is_ok());
        assert!(
            clamp_timeout_or_err("test.timeout_ms", MAX_TIMEOUT_MS + 1, MAX_TIMEOUT_MS).is_err()
        );
    }

    #[test]
    fn shape_match_context_at_offset_zero_with_context() {
        let shaped = crate::match_config::shape_match_context(b"OK>rest", 0, 3, Some(128));
        assert_eq!(shaped.data, b"OK>");
        assert_eq!(shaped.match_index, 0);
    }

    #[test]
    fn shape_match_context_larger_than_pre_match() {
        let shaped = crate::match_config::shape_match_context(b"ABOK>x", 2, 3, Some(100));
        assert_eq!(shaped.data, b"ABOK>");
        assert_eq!(shaped.match_index, 2);
    }

    #[test]
    fn shape_match_context_exact_pre_match() {
        let shaped = crate::match_config::shape_match_context(b"XXOK>", 2, 3, Some(2));
        assert_eq!(shaped.data, b"XXOK>");
        assert_eq!(shaped.match_index, 2);
    }

    #[test]
    fn shape_match_context_truncates_post_match() {
        let shaped = crate::match_config::shape_match_context(b"preOK>post123", 3, 3, Some(3));
        // pre_start=0, match_end=6, shaped="preOK>" (6 bytes)
        assert_eq!(shaped.data, b"preOK>");
        assert_eq!(shaped.match_index, 3);
    }
}
