//! Named serial device profiles.
//!
//! Profiles bind a device selector (VID/PID/serial/...) to default serial
//! configuration so that agents can open devices by name instead of
//! fragile port path.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::serial::PortInfo;

/// Maximum number of prior selector/defaults snapshots retained per profile.
/// Oldest snapshots are dropped first; the newest five prior states survive.
pub const MAX_PROFILE_REVISIONS: usize = 5;

/// A single named profile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub selector: ProfileSelector,
    #[serde(default)]
    pub defaults: ProfileDefaults,
    /// Bookkeeping metadata (revision, timestamps, generated flag).
    /// Phase 3 uses `last_used_at_ms`/`use_count` for selection ranking.
    #[serde(default)]
    pub metadata: ProfileMetadata,
    /// Bounded history of prior selector/defaults snapshots. Empty for
    /// profiles that were never overwritten.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub revisions: Vec<ProfileRevision>,
}

/// Per-profile bookkeeping metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ProfileMetadata {
    /// True when the profile was created by automatic session machinery
    /// (Phase 3), false for profiles created by explicit tools.
    pub generated: bool,
    /// Monotonic revision number. Legacy/unversioned profiles default to 0
    /// and become 1 on their first update.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub revision: u64,
    /// Creation timestamp (ms since Unix epoch), set on create.
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub created_at_ms: Option<u64>,
    /// Last mutation timestamp (ms since Unix epoch).
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub updated_at_ms: Option<u64>,
    /// Last open/selection timestamp (ms since Unix epoch). Phase 3 only.
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub last_used_at_ms: Option<u64>,
    /// Number of times the profile was used/selected. Phase 3 only.
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub use_count: u64,
}

/// A snapshot of a profile's selector/defaults as they were before one
/// overwrite. `revision` is the profile revision that owned this state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfileRevision {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub revision: u64,
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub saved_at_ms: u64,
    pub selector: ProfileSelector,
    pub defaults: ProfileDefaults,
}

/// Rules for matching a live serial port against this profile.
///
/// All fields are optional. A port matches when every non-empty field
/// agrees with the port's identity. An empty selector (all fields
/// `None`) matches any port — not recommended outside testing.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProfileSelector {
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub vid: Option<u16>,
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub pid: Option<u16>,
    pub serial_number: Option<String>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub interface: Option<u8>,
    /// Glob pattern matched against the port's `name` field
    /// (e.g. `/dev/ttyACM*` or `COM?`). Case-sensitive.
    pub port_pattern: Option<String>,
    /// Glob pattern matched against the port's `description` field.
    pub description_pattern: Option<String>,
    /// Transport type filter (matches `port.transport` Display string).
    /// Examples: "usb", "pci", "bluetooth", "unknown".
    pub transport: Option<String>,
    /// Exact match on the port's hardware_id field.
    pub hardware_id: Option<String>,
}

/// Default serial configuration applied when opening via this profile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfileDefaults {
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    #[serde(default = "default_baud")]
    pub baud_rate: u32,
    #[serde(default = "default_data_bits")]
    pub data_bits: String,
    #[serde(default = "default_stop_bits")]
    pub stop_bits: String,
    #[serde(default = "default_parity")]
    pub parity: String,
    #[serde(default = "default_flow_control")]
    pub flow_control: String,
    /// Connection name prefix. The actual connection name will be
    /// `{name_prefix}-{short_port_name}` when a name is provided.
    pub name: Option<String>,
    /// Default TX framing applied when `write` omits `tx_framing`.
    #[serde(default)]
    pub tx_framing: Option<crate::framing::TxFramingConfig>,
    /// Default RX framing applied when `read`/`subscribe` omit `rx_framing`.
    #[serde(default)]
    pub rx_framing: Option<crate::framing::RxFramingConfig>,
    /// Default RX parser applied when `read`/`subscribe` omit `rx_parser`.
    #[serde(default)]
    pub rx_parser: Option<crate::framing::ParserConfig>,
    /// Default protocol preset. When set, expands to fill any of the above
    /// framing/parser fields that are themselves `None`.
    #[serde(default)]
    pub protocol: Option<crate::framing::ProtocolPreset>,
    /// Default RX ring buffer size in bytes. Overridable at open time.
    #[serde(default = "default_rx_buffer_size_profile")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub rx_buffer_size: usize,
    /// Default max buffered bytes for `read` (was per-call, now profile-level).
    /// Default 32768 (32 KiB).
    #[serde(default = "default_max_buffered_bytes_profile")]
    #[schemars(schema_with = "crate::schema_helpers::read_max_buffered_bytes_schema")]
    pub max_buffered_bytes: usize,
    /// Default poll interval for `subscribe` in milliseconds (was per-call on
    /// SubscribeArgs, now profile-level). Default 200.
    #[serde(default = "default_subscribe_poll_ms_profile")]
    #[schemars(schema_with = "crate::schema_helpers::poll_interval_ms_schema")]
    pub poll_interval_ms: u64,
    /// Default reconnect policy. Open-time only; reopen to apply. Default: disabled.
    #[serde(default)]
    pub reconnect_policy: crate::serial::ReconnectPolicy,
    /// Default log buffer capacity in events. 0 disables logging. Default 1024.
    #[serde(default = "default_log_capacity_profile")]
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub log_capacity: usize,
    /// Default logging enabled flag. Default true (ignored when capacity is 0).
    #[serde(default = "default_true_profile")]
    pub log_enabled: bool,
}

fn default_baud() -> u32 {
    115200
}
fn default_data_bits() -> String {
    "8".into()
}
fn default_stop_bits() -> String {
    "1".into()
}
fn default_parity() -> String {
    "none".into()
}
fn default_flow_control() -> String {
    "none".into()
}
fn default_rx_buffer_size_profile() -> usize {
    crate::limits::DEFAULT_RX_BUFFER_SIZE
}
fn default_max_buffered_bytes_profile() -> usize {
    32768
}
fn default_subscribe_poll_ms_profile() -> u64 {
    200
}
fn default_log_capacity_profile() -> usize {
    1024
}
fn default_true_profile() -> bool {
    true
}

impl Default for ProfileDefaults {
    fn default() -> Self {
        Self {
            baud_rate: default_baud(),
            data_bits: default_data_bits(),
            stop_bits: default_stop_bits(),
            parity: default_parity(),
            flow_control: default_flow_control(),
            name: None,
            tx_framing: None,
            rx_framing: None,
            rx_parser: None,
            protocol: None,
            rx_buffer_size: default_rx_buffer_size_profile(),
            max_buffered_bytes: default_max_buffered_bytes_profile(),
            poll_interval_ms: default_subscribe_poll_ms_profile(),
            reconnect_policy: crate::serial::ReconnectPolicy::default(),
            log_capacity: default_log_capacity_profile(),
            log_enabled: default_true_profile(),
        }
    }
}

impl Profile {
    /// Check whether `port` matches this profile's selector. Returns
    /// `true` when every non-empty field in the selector agrees with
    /// the port's identity.
    pub fn matches(&self, port: &PortInfo) -> bool {
        let s = &self.selector;

        if let Some(vid) = s.vid {
            if port.vid != Some(vid) {
                return false;
            }
        }
        if let Some(pid) = s.pid {
            if port.pid != Some(pid) {
                return false;
            }
        }
        if let Some(ref want) = s.serial_number {
            if port.serial_number.as_deref() != Some(want.as_str()) {
                return false;
            }
        }
        if let Some(ref want) = s.manufacturer {
            if port.manufacturer.as_deref() != Some(want.as_str()) {
                return false;
            }
        }
        if let Some(ref want) = s.product {
            if port.product.as_deref() != Some(want.as_str()) {
                return false;
            }
        }
        if let Some(iface) = s.interface {
            if port.interface != Some(iface) {
                return false;
            }
        }
        if let Some(ref pattern) = s.port_pattern {
            if !glob::Pattern::new(pattern)
                .map(|p| p.matches(&port.name))
                .unwrap_or(false)
            {
                return false;
            }
        }
        if let Some(ref pattern) = s.description_pattern {
            if !glob::Pattern::new(pattern)
                .map(|p| p.matches(&port.description))
                .unwrap_or(false)
            {
                return false;
            }
        }
        if let Some(ref want) = s.transport {
            if port.transport.to_string().as_str() != want.as_str() {
                return false;
            }
        }
        if let Some(ref want) = s.hardware_id {
            if port.hardware_id.as_deref() != Some(want.as_str()) {
                return false;
            }
        }

        true
    }
}

/// Default location for the profiles configuration file: the OS user
/// config directory plus `serial-mcp/profiles.toml`.
///
/// Returns an error when the OS config directory is unavailable. There is
/// deliberately no silent fallback to the current working directory — an
/// agent's `profiles.toml` must never accidentally land in a repository.
pub fn default_profiles_path() -> Result<PathBuf, String> {
    let dir = dirs::config_dir().ok_or_else(|| {
        "OS user config directory is unavailable; pass --profiles-path".to_string()
    })?;
    Ok(dir.join("serial-mcp").join("profiles.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_port(name: &str, vid: Option<u16>, pid: Option<u16>, serial: Option<&str>) -> PortInfo {
        PortInfo {
            name: name.into(),
            display_name: name.rsplit('/').next().unwrap_or(name).into(),
            description: "Test Port".into(),
            hardware_id: None,
            transport: crate::serial::PortTransport::Usb,
            vid,
            pid,
            serial_number: serial.map(str::to_string),
            manufacturer: None,
            product: None,
            interface: None,
        }
    }

    #[test]
    fn empty_selector_matches_any_port() {
        let p = Profile {
            name: "any".into(),
            selector: ProfileSelector::default(),
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        assert!(p.matches(&make_port("/dev/ttyUSB0", Some(0x1234), Some(0x5678), None)));
        assert!(p.matches(&make_port("/dev/ttyACM0", None, None, None)));
    }

    #[test]
    fn exact_vid_pid_match() {
        let p = Profile {
            name: "my-device".into(),
            selector: ProfileSelector {
                vid: Some(0x1234),
                pid: Some(0x5678),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        assert!(p.matches(&make_port("/dev/ttyUSB0", Some(0x1234), Some(0x5678), None)));
        assert!(!p.matches(&make_port("/dev/ttyUSB0", Some(0xAAAA), Some(0x5678), None)));
        assert!(!p.matches(&make_port("/dev/ttyUSB0", None, None, None)));
    }

    #[test]
    fn serial_number_match() {
        let p = Profile {
            name: "by-serial".into(),
            selector: ProfileSelector {
                serial_number: Some("0001".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        assert!(p.matches(&make_port("/dev/ttyUSB0", None, None, Some("0001"))));
        assert!(!p.matches(&make_port("/dev/ttyUSB0", None, None, Some("0002"))));
        assert!(!p.matches(&make_port("/dev/ttyUSB0", None, None, None)));
    }

    #[test]
    fn port_pattern_glob_match() {
        let p = Profile {
            name: "acm-only".into(),
            selector: ProfileSelector {
                port_pattern: Some("/dev/ttyACM*".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        assert!(p.matches(&make_port("/dev/ttyACM0", None, None, None)));
        assert!(!p.matches(&make_port("/dev/ttyUSB0", None, None, None)));
    }

    #[test]
    fn multiple_fields_all_must_match() {
        let p = Profile {
            name: "specific".into(),
            selector: ProfileSelector {
                vid: Some(0x1234),
                serial_number: Some("0001".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        assert!(p.matches(&make_port(
            "/dev/ttyUSB0",
            Some(0x1234),
            Some(0x9999),
            Some("0001")
        )));
        // Wrong serial
        assert!(!p.matches(&make_port(
            "/dev/ttyUSB0",
            Some(0x1234),
            Some(0x9999),
            Some("0002")
        )));
        // Wrong VID
        assert!(!p.matches(&make_port(
            "/dev/ttyUSB0",
            Some(0xAAAA),
            Some(0x9999),
            Some("0001")
        )));
    }

    #[test]
    fn manufacturer_match() {
        let p = Profile {
            name: "by-mfg".into(),
            selector: ProfileSelector {
                manufacturer: Some("STMicro".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        let mut port = make_port("/dev/ttyACM0", None, None, None);
        port.manufacturer = Some("STMicro".into());
        assert!(p.matches(&port));
        port.manufacturer = Some("Other".into());
        assert!(!p.matches(&port));
    }

    #[test]
    fn product_match() {
        let p = Profile {
            name: "by-prod".into(),
            selector: ProfileSelector {
                product: Some("VirtualCom".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        let mut port = make_port("/dev/ttyACM0", None, None, None);
        port.product = Some("VirtualCom".into());
        assert!(p.matches(&port));
        port.product = Some("Other".into());
        assert!(!p.matches(&port));
    }

    #[test]
    fn interface_match() {
        let p = Profile {
            name: "by-iface".into(),
            selector: ProfileSelector {
                interface: Some(2),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        let mut port = make_port("/dev/ttyACM0", None, None, None);
        port.interface = Some(2);
        assert!(p.matches(&port));
        port.interface = Some(0);
        assert!(!p.matches(&port));
    }

    #[test]
    fn description_pattern_match() {
        let p = Profile {
            name: "by-desc".into(),
            selector: ProfileSelector {
                description_pattern: Some("*CP210*".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        let port = PortInfo {
            description: "Silicon Labs CP2102 USB to UART Bridge Controller".into(),
            ..make_port("/dev/ttyUSB0", None, None, None)
        };
        assert!(p.matches(&port));
        assert!(!p.matches(&make_port("/dev/ttyUSB0", None, None, None)));
    }

    #[test]
    fn invalid_glob_pattern_returns_false() {
        let p = Profile {
            name: "bad-glob".into(),
            selector: ProfileSelector {
                port_pattern: Some("[unclosed".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        assert!(!p.matches(&make_port("/dev/ttyUSB0", None, None, None)));
    }

    // ── transport / hardware_id selector ───────────────────────────────

    #[test]
    fn transport_match_usb() {
        let p = Profile {
            name: "usb-only".into(),
            selector: ProfileSelector {
                transport: Some("usb".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        assert!(p.matches(&make_port("/dev/ttyACM0", None, None, None)));
        let mut pci_port = make_port("/dev/ttyS0", None, None, None);
        pci_port.transport = crate::serial::PortTransport::Pci;
        assert!(!p.matches(&pci_port));
    }

    #[test]
    fn transport_match_unknown() {
        let p = Profile {
            name: "unknown-only".into(),
            selector: ProfileSelector {
                transport: Some("unknown".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        let mut unknown_port = make_port("/dev/ttyX", None, None, None);
        unknown_port.transport = crate::serial::PortTransport::Unknown;
        assert!(p.matches(&unknown_port));
    }

    #[test]
    fn transport_match_case_sensitive() {
        let p = Profile {
            name: "usb-exact".into(),
            selector: ProfileSelector {
                transport: Some("USB".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        // Display output is lowercase — "USB" does not match "usb"
        assert!(!p.matches(&make_port("/dev/ttyACM0", None, None, None)));
    }

    #[test]
    fn hardware_id_match() {
        let p = Profile {
            name: "by-hwid".into(),
            selector: ProfileSelector {
                hardware_id: Some("USB\\VID_1234&PID_5678".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        let mut port = make_port("/dev/ttyACM0", None, None, None);
        port.hardware_id = Some("USB\\VID_1234&PID_5678".into());
        assert!(p.matches(&port));

        port.hardware_id = Some("USB\\VID_AAAA&PID_BBBB".into());
        assert!(!p.matches(&port));
    }

    #[test]
    fn hardware_id_no_match_when_port_has_none() {
        let p = Profile {
            name: "hwid-required".into(),
            selector: ProfileSelector {
                hardware_id: Some("USB\\VID_1234&PID_5678".into()),
                ..Default::default()
            },
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        };
        // Port with no hardware_id should not match
        assert!(!p.matches(&make_port("/dev/ttyACM0", None, None, None)));
    }

    #[test]
    fn profile_defaults_has_no_dead_fields() {
        // Deserializing a JSON object with ONLY the two removed field names
        // must succeed (fields silently ignored — no deny_unknown_fields).
        let json = serde_json::json!({
            "decoder": "y",
            "safety_policy": "z"
        });
        let _: ProfileDefaults =
            serde_json::from_value(json).expect("dead fields must be silently ignored");

        // Serializing ProfileDefaults::default() must NOT contain the
        // removed field names. (reconnect_policy is now a real field in
        // v0.8.1 so it IS expected to appear.)
        let value = serde_json::to_value(ProfileDefaults::default())
            .expect("ProfileDefaults must serialize");
        let obj = value
            .as_object()
            .expect("serialized ProfileDefaults must be an object");
        assert!(!obj.contains_key("decoder"));
        assert!(!obj.contains_key("safety_policy"));
    }
}
