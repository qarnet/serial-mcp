//! Named serial device profiles.
//!
//! Profiles bind a device selector (VID/PID/serial/...) to default serial
//! configuration so that agents can open devices by name instead of
//! fragile port path.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::serial::PortInfo;

/// A single named profile.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub selector: ProfileSelector,
    #[serde(default)]
    pub defaults: ProfileDefaults,
}

/// Rules for matching a live serial port against this profile.
///
/// All fields are optional. A port matches when every non-empty field
/// agrees with the port's identity. An empty selector (all fields
/// `None`) matches any port — not recommended outside testing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
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
    /// Reserved for future reconnect policy. Not yet enforced.
    #[serde(default)]
    pub reconnect_policy: Option<String>,
    /// Reserved for future decoder selection. Not yet enforced.
    #[serde(default)]
    pub decoder: Option<String>,
    /// Reserved for future safety policy hints. Not yet enforced.
    #[serde(default)]
    pub safety_policy: Option<String>,
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

impl Default for ProfileDefaults {
    fn default() -> Self {
        Self {
            baud_rate: default_baud(),
            data_bits: default_data_bits(),
            stop_bits: default_stop_bits(),
            parity: default_parity(),
            flow_control: default_flow_control(),
            name: None,
            reconnect_policy: None,
            decoder: None,
            safety_policy: None,
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

/// Load profiles from a TOML file. Returns an empty vec when the file
/// does not exist or cannot be read.
pub fn load_profiles(path: &PathBuf) -> Vec<Profile> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Cannot read profiles file {}: {e}", path.display());
            return vec![];
        }
    };

    match toml::from_str::<ProfilesFile>(&content) {
        Ok(f) => f.profile,
        Err(e) => {
            tracing::warn!("Failed to parse profiles file {}: {e}", path.display());
            vec![]
        }
    }
}

/// Default location for the profiles configuration file.
pub fn default_profiles_path() -> PathBuf {
    let mut p = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    p.push("serial-mcp");
    p.push("profiles.toml");
    p
}

/// Write a profile to the TOML file atomically.
///
/// Returns `true` if the profile was newly created, `false` if it
/// replaced an existing profile with the same name.
///
/// Writes to a temporary file first, then renames for atomicity.
pub fn save_profile(path: &PathBuf, profile: &Profile) -> Result<bool, String> {
    let mut profiles = load_profiles(path);
    let existing_idx = profiles.iter().position(|p| p.name == profile.name);

    match existing_idx {
        Some(idx) => {
            profiles[idx] = profile.clone();
        }
        None => {
            profiles.push(profile.clone());
        }
    }
    let created = existing_idx.is_none();

    let toml = toml::to_string_pretty(&ProfilesFile { profile: profiles })
        .map_err(|e| format!("Failed to serialize profiles: {e}"))?;

    // Atomic write: temp file + rename.
    let dir = path
        .parent()
        .ok_or_else(|| "Profiles path has no parent directory".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create profile dir: {e}"))?;

    let mut tmp = path.clone();
    tmp.set_extension("tmp");
    std::fs::write(&tmp, toml).map_err(|e| format!("Failed to write profiles: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to commit profiles: {e}"))?;

    tracing::info!(
        "Profile '{}' {} (path: {})",
        profile.name,
        if created { "created" } else { "updated" },
        path.display()
    );
    Ok(created)
}

/// Delete a profile by name from the TOML file. Returns an error if
/// the profile does not exist.
pub fn delete_profile(path: &PathBuf, name: &str) -> Result<(), String> {
    let mut profiles = load_profiles(path);
    let len_before = profiles.len();
    profiles.retain(|p| p.name != name);

    if profiles.len() == len_before {
        return Err(format!("Profile '{name}' not found"));
    }

    let toml = toml::to_string_pretty(&ProfilesFile { profile: profiles })
        .map_err(|e| format!("Failed to serialize profiles: {e}"))?;

    let dir = path
        .parent()
        .ok_or_else(|| "Profiles path has no parent directory".to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create profile dir: {e}"))?;

    let mut tmp = path.clone();
    tmp.set_extension("tmp");
    std::fs::write(&tmp, toml).map_err(|e| format!("Failed to write profiles: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to commit profiles: {e}"))?;

    tracing::info!("Profile '{}' deleted (path: {})", name, path.display());
    Ok(())
}

/// TOML root structure for the profiles file.
#[derive(Debug, Deserialize, Serialize)]
struct ProfilesFile {
    profile: Vec<Profile>,
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
    fn load_profiles_from_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profiles.toml");
        std::fs::write(
            &path,
            r#"
[[profile]]
name = "nrf-dk"
[profile.selector]
vid = 0x1366
pid = 0x0105
[profile.defaults]
baud_rate = 115200
name = "nrf"

[[profile]]
name = "arduino"
[profile.selector]
port_pattern = "/dev/ttyACM*"
[profile.defaults]
baud_rate = 9600
"#,
        )
        .unwrap();

        let profiles = load_profiles(&path);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "nrf-dk");
        assert_eq!(profiles[0].selector.vid, Some(0x1366));
        assert_eq!(profiles[0].defaults.name.as_deref(), Some("nrf"));
        assert_eq!(profiles[1].name, "arduino");
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
        };
        assert!(!p.matches(&make_port("/dev/ttyUSB0", None, None, None)));
    }

    #[test]
    fn load_profiles_missing_file_returns_empty() {
        let profiles = load_profiles(&PathBuf::from("/nonexistent/path/profiles.toml"));
        assert!(profiles.is_empty());
    }

    #[test]
    fn load_profiles_invalid_toml_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.toml");
        std::fs::write(&path, "not valid toml {{{").unwrap();
        let profiles = load_profiles(&path);
        assert!(profiles.is_empty());
    }
}
