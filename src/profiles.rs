//! Named serial device profiles.
//!
//! Profiles bind a device selector (VID/PID/serial/...) to default serial
//! configuration so that agents can open devices by name instead of
//! fragile port path.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;

use crate::serial::{PortInfo, PortTransport};

/// Maximum number of prior selector/defaults snapshots retained per profile.
/// Oldest snapshots are dropped first; the newest five prior states survive.
pub const MAX_PROFILE_REVISIONS: usize = 5;

/// How confidently a port's identity can be used for automatic profile
/// reuse. Automatic persistent reuse is allowed only for [`High`](IdentityConfidence::High):
///
/// - transport is USB
/// - VID exists
/// - PID exists
/// - non-empty serial number exists
/// - interface participates when available
///
/// USB VID/PID without a serial number is `Medium` and auto-ineligible.
/// Other useful identity is `Low`; path-only/unknown is `None`.
/// Medium/low/none sessions are transient and never persisted automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum IdentityConfidence {
    /// USB transport with VID, PID, and a non-empty serial number.
    High,
    /// USB VID/PID without a serial number.
    Medium,
    /// Some other useful identity (non-USB transport with identity fields).
    Low,
    /// Path-only or unknown identity.
    None,
}

/// Outcome of a write-through profile persistence attempt (Phase 3B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePersistenceState {
    /// The effective defaults were durably written to the bound profile.
    Persisted,
    /// No persistence was needed — the durable defaults already equal the
    /// connection's effective defaults (no file write happened).
    NotNeeded,
    /// The connection is not backed by a durable profile (transient,
    /// disabled, or no binding); nothing was persisted.
    Transient,
    /// Live state changed but the profile write failed or conflicted. The
    /// binding is dirty (and stale for conflicts/missing profiles).
    Failed,
}

/// What kind of durable operation triggered a persistence attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePersistenceOperation {
    /// Explicit open-field overrides learned after successful hardware open.
    OpenOverride,
    /// A durable live mutation (`reconfigure`, `set_flow_control`,
    /// connection-mode `configure`).
    Learned,
    /// Clean close snapshot/retry.
    CloseSnapshot,
    /// The `rollback_profile` tool.
    Rollback,
}

/// Additive result of one write-through persistence attempt, carried on
/// tool results whose live mutation succeeded. `state == "failed"` means
/// the live change applied but the profile write did not; the binding is
/// dirty and the error is recorded.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfilePersistenceResult {
    pub state: ProfilePersistenceState,
    pub operation: ProfilePersistenceOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_name: Option<String>,
    /// Profile revision after the attempt (`None` for transient sessions or
    /// when the attempt failed before any revision was observed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub revision: Option<u64>,
    /// Persistence/conflict error text when `state == "failed"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// How an active connection's profile session was selected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSelectionSource {
    /// Automatically selected as the unique most-recently-used
    /// high-confidence profile for the device.
    Automatic,
    /// Explicitly selected via `open_profile`.
    Explicit,
    /// Automatically created durable profile for a new high-confidence
    /// device.
    Generated,
    /// Non-persistent session (weak/ambiguous identity, disabled store
    /// interaction, or a failed profile persistence).
    Transient,
    /// Automatic selection/creation disabled via `profile_mode="none"`.
    Disabled,
}

/// Profile-session binding shape exposed on open/status/connection results.
/// `ActiveProfileBinding` (stored on `SerialConnection`) converts losslessly
/// into this wire type via `to_session_result`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProfileSessionResult {
    /// Name of the bound profile. Empty for transient/disabled sessions.
    pub profile_name: String,
    /// How the binding was selected.
    pub source: ProfileSelectionSource,
    /// Identity confidence that drove automatic behavior.
    pub confidence: IdentityConfidence,
    /// Whether the session is backed by a durable profile.
    pub persistent: bool,
    /// Whether the bound profile was auto-generated (Phase 3A).
    pub generated: bool,
    /// Bound profile's revision at selection time. `null` for
    /// transient/disabled sessions.
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub revision: Option<u64>,
    /// `true` when explicit open fields override the selected profile's
    /// defaults (3B persists the effective settings on durable operations).
    pub dirty: bool,
    /// `true` when the durable profile revision changed externally (CAS
    /// conflict or rollback) and this connection must not overwrite it.
    /// Stale bindings keep reporting the conflict until reopened.
    pub stale: bool,
    /// Candidate profile names when selection was ambiguous; empty
    /// otherwise.
    pub candidates: Vec<String>,
    /// Last profile persistence error, when a metadata write failed after
    /// hardware open succeeded. The connection stays open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_persistence_error: Option<String>,
}

/// Automatic profile-session mode for bare `open`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileMode {
    /// Automatically reuse the most recently used high-confidence profile
    /// or create a durable generated profile for a new high-confidence
    /// device. Weak/ambiguous identity gets a transient session.
    Auto,
    /// Disable automatic profile selection/creation for deliberate
    /// troubleshooting. Every open gets a non-persistent disabled binding.
    None,
}

/// Canonical high-confidence device identity used for automatic profile
/// matching and duplicate detection.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct HighIdentity {
    /// Transport as displayed by `PortTransport` (`"usb"` for USB ports).
    pub transport: String,
    pub vid: u16,
    pub pid: u16,
    pub serial_number: String,
    pub interface: Option<u8>,
}

/// Compute the canonical high-confidence identity of a port, or `None` when
/// the port is not a uniquely identified USB device.
pub fn high_identity(port: &PortInfo) -> Option<HighIdentity> {
    if port.transport != PortTransport::Usb {
        return None;
    }
    let vid = port.vid?;
    let pid = port.pid?;
    let serial_number = port
        .serial_number
        .as_deref()
        .filter(|s| !s.is_empty())?
        .to_string();
    Some(HighIdentity {
        transport: port.transport.to_string(),
        vid,
        pid,
        serial_number,
        interface: port.interface,
    })
}

/// Classify a port's identity confidence for automatic profile behavior.
pub fn identity_confidence(port: &PortInfo) -> IdentityConfidence {
    if port.transport == PortTransport::Usb
        && port.vid.is_some()
        && port.pid.is_some()
        && port.serial_number.as_deref().is_some_and(|s| !s.is_empty())
    {
        return IdentityConfidence::High;
    }
    if port.transport == PortTransport::Usb && port.vid.is_some() && port.pid.is_some() {
        return IdentityConfidence::Medium;
    }
    let has_any_identity = port.vid.is_some()
        || port.pid.is_some()
        || port.serial_number.as_deref().is_some_and(|s| !s.is_empty())
        || port.manufacturer.is_some()
        || port.product.is_some()
        || port.interface.is_some()
        || port.hardware_id.is_some();
    if has_any_identity {
        IdentityConfidence::Low
    } else {
        IdentityConfidence::None
    }
}

/// Canonical generated selector for a high-confidence port: only transport,
/// VID, PID, serial number, and optional interface. Path, description,
/// manufacturer, product, and formatted hardware ID are deliberately
/// excluded so the profile survives port path changes.
pub fn canonical_high_selector(port: &PortInfo) -> Option<ProfileSelector> {
    let id = high_identity(port)?;
    Some(ProfileSelector {
        vid: Some(id.vid),
        pid: Some(id.pid),
        serial_number: Some(id.serial_number),
        manufacturer: None,
        product: None,
        interface: id.interface,
        port_pattern: None,
        description_pattern: None,
        transport: Some(id.transport),
        hardware_id: None,
    })
}

/// Whether a profile selector carries the same high identity fields as the
/// target device (transport, VID, PID, serial, and interface when present).
/// Used together with [`Profile::matches`] to filter automatic candidates.
pub fn selector_matches_high_identity(selector: &ProfileSelector, id: &HighIdentity) -> bool {
    selector.transport.as_deref() == Some(id.transport.as_str())
        && selector.vid == Some(id.vid)
        && selector.pid == Some(id.pid)
        && selector.serial_number.as_deref() == Some(id.serial_number.as_str())
        && id
            .interface
            .is_none_or(|iface| selector.interface == Some(iface))
}

/// Rank candidate profiles by `last_used_at_ms`, newest first. `None`
/// timestamps sort oldest (ranked as 0).
pub fn rank_candidates(mut candidates: Vec<Profile>) -> Vec<Profile> {
    candidates.sort_by(|a, b| {
        let a_ts = a.metadata.last_used_at_ms.unwrap_or(0);
        let b_ts = b.metadata.last_used_at_ms.unwrap_or(0);
        b_ts.cmp(&a_ts)
    });
    candidates
}

/// Normalize a product/manufacturer label for generated profile names:
/// lowercase ASCII, each non-alphanumeric run becomes `-`, trimmed of
/// leading/trailing `-`, capped at 32 chars, `serial-device` fallback when
/// nothing usable remains.
pub fn normalize_generated_label(label: &str) -> String {
    let mut out = String::new();
    let mut pending_sep = false;
    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_sep && !out.is_empty() {
                out.push('-');
            }
            pending_sep = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_sep = true;
        }
    }
    let out = out.trim_matches('-');
    let capped: String = out.chars().take(32).collect();
    let capped = capped.trim_end_matches('-');
    if capped.is_empty() {
        "serial-device".to_string()
    } else {
        capped.to_string()
    }
}

/// Choose the first free generated profile name for `base`: `base`, then
/// `base-2`, `base-3`, ... — never overwriting an existing profile.
pub fn allocate_generated_name(existing: &[Profile], base: &str) -> String {
    let taken: HashSet<&str> = existing.iter().map(|p| p.name.as_str()).collect();
    if !taken.contains(base) {
        return base.to_string();
    }
    let mut suffix = 2u32;
    loop {
        let candidate = format!("{base}-{suffix}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
        suffix += 1;
    }
}

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

impl ProfileSelector {
    /// Whether every selector field is `None` — an empty selector matches
    /// ANY port and must never appear as a weak candidate in `list_ports`
    /// profile previews. Empty selectors remain valid for
    /// `Profile::matches` (used by tests and the `configure` profile mode
    /// creation path), but they carry no discoverable device knowledge.
    pub fn is_empty(&self) -> bool {
        self.vid.is_none()
            && self.pid.is_none()
            && self.serial_number.is_none()
            && self.manufacturer.is_none()
            && self.product.is_none()
            && self.interface.is_none()
            && self.port_pattern.is_none()
            && self.description_pattern.is_none()
            && self.transport.is_none()
            && self.hardware_id.is_none()
    }
}

/// Default serial configuration applied when opening via this profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    fn empty_selector_is_detected() {
        assert!(ProfileSelector::default().is_empty());
        assert!(!ProfileSelector {
            vid: Some(0x1234),
            ..Default::default()
        }
        .is_empty());
        assert!(!ProfileSelector {
            port_pattern: Some("/dev/ttyACM*".into()),
            ..Default::default()
        }
        .is_empty());
        assert!(!ProfileSelector {
            transport: Some("usb".into()),
            ..Default::default()
        }
        .is_empty());
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

    // ── Identity confidence / canonical selector ─────────────────────────

    fn usb_port(name: &str, vid: Option<u16>, pid: Option<u16>, serial: Option<&str>) -> PortInfo {
        let mut port = make_port(name, vid, pid, serial);
        port.transport = crate::serial::PortTransport::Usb;
        port
    }

    #[test]
    fn confidence_tiers() {
        let high = usb_port("/dev/ttyACM0", Some(0x1234), Some(0x5678), Some("SN-1"));
        assert_eq!(identity_confidence(&high), IdentityConfidence::High);

        let empty_serial = usb_port("/dev/ttyACM0", Some(0x1234), Some(0x5678), Some(""));
        assert_eq!(
            identity_confidence(&empty_serial),
            IdentityConfidence::Medium
        );

        let no_serial = usb_port("/dev/ttyACM0", Some(0x1234), Some(0x5678), None);
        assert_eq!(identity_confidence(&no_serial), IdentityConfidence::Medium);

        // USB without VID/PID: only description-ish identity → Low.
        let partial = usb_port("/dev/ttyACM0", None, None, Some("SN-1"));
        assert_eq!(identity_confidence(&partial), IdentityConfidence::Low);

        // PCI with hardware id → Low.
        let mut pci = make_port("/dev/ttyS0", None, None, None);
        pci.transport = crate::serial::PortTransport::Pci;
        pci.hardware_id = Some("PCI".into());
        assert_eq!(identity_confidence(&pci), IdentityConfidence::Low);

        // Path-only/unknown → None.
        let unknown = make_port("/dev/pts/3", None, None, None);
        assert_eq!(identity_confidence(&unknown), IdentityConfidence::None);
    }

    #[test]
    fn high_identity_requires_full_usb_identity() {
        let high = usb_port("/dev/ttyACM0", Some(0x1234), Some(0x5678), Some("SN-1"));
        assert!(high_identity(&high).is_some());

        assert!(
            high_identity(&usb_port("/dev/ttyACM0", Some(0x1234), Some(0x5678), None)).is_none()
        );
        assert!(
            high_identity(&usb_port("/dev/ttyACM0", Some(0x1234), None, Some("SN-1"))).is_none()
        );
        assert!(high_identity(&make_port("/dev/ttyS0", None, None, None)).is_none());
    }

    #[test]
    fn canonical_high_selector_survives_path_change() {
        let mut port = usb_port("/dev/ttyACM0", Some(0x1234), Some(0x5678), Some("SN-1"));
        port.interface = Some(2);
        port.product = Some("Widget".into());
        port.manufacturer = Some("Acme".into());
        port.hardware_id = Some("USB VID:1234 PID:5678".into());

        let selector = canonical_high_selector(&port).expect("high identity selector");

        // Path/description/manufacturer/product/hardware_id must NOT be in
        // the canonical selector.
        assert_eq!(selector.port_pattern, None);
        assert_eq!(selector.description_pattern, None);
        assert_eq!(selector.manufacturer, None);
        assert_eq!(selector.product, None);
        assert_eq!(selector.hardware_id, None);

        // Identity fields must be present.
        assert_eq!(selector.vid, Some(0x1234));
        assert_eq!(selector.pid, Some(0x5678));
        assert_eq!(selector.serial_number.as_deref(), Some("SN-1"));
        assert_eq!(selector.interface, Some(2));
        assert_eq!(selector.transport.as_deref(), Some("usb"));

        // A different OS path with the same identity still matches.
        let mut same_device = port.clone();
        same_device.name = "/dev/ttyACM7".into();
        same_device.display_name = "ttyACM7".into();
        let selector = canonical_high_selector(&same_device).unwrap();
        let id = high_identity(&same_device).unwrap();
        assert!(selector_matches_high_identity(&selector, &id));
        assert!(Profile {
            name: "x".into(),
            selector,
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata::default(),
            revisions: Vec::new(),
        }
        .matches(&port));
    }

    #[test]
    fn selector_matches_high_identity_rejects_weak_and_foreign_selectors() {
        let id = HighIdentity {
            transport: "usb".into(),
            vid: 0x1234,
            pid: 0x5678,
            serial_number: "SN-1".into(),
            interface: Some(2),
        };

        // Exact canonical selector matches (port carries the interface).
        let mut port = usb_port("/dev/ttyACM0", Some(0x1234), Some(0x5678), Some("SN-1"));
        port.interface = Some(2);
        assert!(selector_matches_high_identity(
            &canonical_high_selector(&port).unwrap(),
            &id
        ));

        // Empty selector (matches any port) must NOT count as high identity.
        assert!(!selector_matches_high_identity(
            &ProfileSelector::default(),
            &id
        ));

        // Different serial / pid / transport all fail.
        let wrong_serial = ProfileSelector {
            vid: Some(0x1234),
            pid: Some(0x5678),
            transport: Some("usb".into()),
            serial_number: Some("OTHER".into()),
            ..Default::default()
        };
        assert!(!selector_matches_high_identity(&wrong_serial, &id));

        // Missing interface while the device has one fails.
        let no_interface = ProfileSelector {
            vid: Some(0x1234),
            pid: Some(0x5678),
            transport: Some("usb".into()),
            serial_number: Some("SN-1".into()),
            ..Default::default()
        };
        assert!(!selector_matches_high_identity(&no_interface, &id));

        // Device without interface: interface does not participate.
        let no_iface_id = HighIdentity {
            interface: None,
            ..id.clone()
        };
        assert!(selector_matches_high_identity(&no_interface, &no_iface_id));
    }

    // ── Candidate ranking ─────────────────────────────────────────────────

    fn ranked_profile(name: &str, last_used: Option<u64>) -> Profile {
        Profile {
            name: name.into(),
            selector: ProfileSelector::default(),
            defaults: ProfileDefaults::default(),
            metadata: ProfileMetadata {
                last_used_at_ms: last_used,
                ..Default::default()
            },
            revisions: Vec::new(),
        }
    }

    #[test]
    fn ranking_picks_unique_newest_first_and_none_sorts_oldest() {
        let ranked = rank_candidates(vec![
            ranked_profile("old", Some(100)),
            ranked_profile("never", None),
            ranked_profile("new", Some(200)),
        ]);
        let names: Vec<&str> = ranked.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["new", "old", "never"]);
    }

    #[test]
    fn ranking_equal_timestamps_tie() {
        let ranked = rank_candidates(vec![
            ranked_profile("a", Some(500)),
            ranked_profile("b", Some(500)),
        ]);
        // Both candidates have the same top timestamp: no unique winner.
        assert_eq!(ranked.len(), 2);
        let top_ts = ranked[0].metadata.last_used_at_ms;
        let next_ts = ranked[1].metadata.last_used_at_ms;
        assert_eq!(top_ts, next_ts, "equal top timestamps must tie");
    }

    // ── Generated name normalization / allocation ────────────────────────

    #[test]
    fn generated_label_normalization() {
        assert_eq!(
            normalize_generated_label("Fake USB Serial"),
            "fake-usb-serial"
        );
        assert_eq!(normalize_generated_label("  Acme  Widget  "), "acme-widget");
        assert_eq!(normalize_generated_label("USB__Port_1"), "usb-port-1");
        assert_eq!(normalize_generated_label("---"), "serial-device");
        assert_eq!(normalize_generated_label(""), "serial-device");
        // Non-ASCII runs collapse to a single dash between ASCII runs.
        assert_eq!(normalize_generated_label("Ünïcode"), "n-code");
        assert_eq!(normalize_generated_label("A###B"), "a-b");
    }

    #[test]
    fn generated_label_truncation_caps_at_32_and_trims_dash() {
        let long = normalize_generated_label(&"x".repeat(60));
        assert_eq!(long.len(), 32, "label capped at 32 chars: {long:?}");
        let cut_dash = normalize_generated_label(&format!("{}!!!", "a".repeat(30)));
        assert_eq!(cut_dash.len(), 30, "cap must not leave a trailing dash");
        assert!(!cut_dash.ends_with('-'));
    }

    #[test]
    fn generated_label_usb_vid_pid_fallback() {
        // The usb-{vid:04x}-{pid:04x} label path lives in the tool layer;
        // normalization must keep the hex digits and dashes.
        assert_eq!(normalize_generated_label("usb-1234-5678"), "usb-1234-5678");
    }

    #[test]
    fn generated_name_allocation_never_overwrites() {
        let existing = vec![
            ranked_profile("auto-device", None),
            ranked_profile("auto-device-2", None),
            ranked_profile("auto-device-3", None),
        ];
        assert_eq!(
            allocate_generated_name(&existing, "auto-device"),
            "auto-device-4"
        );
        assert_eq!(allocate_generated_name(&[], "auto-fresh"), "auto-fresh");
        // An existing suffix does not block a higher one.
        let with_gap = vec![
            ranked_profile("auto-device", None),
            ranked_profile("auto-device-2", None),
        ];
        assert_eq!(
            allocate_generated_name(&with_gap, "auto-device"),
            "auto-device-3"
        );
    }
}
