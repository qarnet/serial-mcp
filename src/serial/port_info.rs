//! OS-level serial port enumeration: `PortTransport`, `PortInfo`, the
//! `PortProvider` abstraction, the production `SystemPortProvider`, and the
//! private OS enumeration/conversion and display/hardware-ID helpers.
//! This module has no dependency on the `config` sibling.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serialport::{available_ports, SerialPortInfo, SerialPortType};

use crate::error::Result;

// Port enumeration.

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
    /// Transport type: `usb`, `pci`, `bluetooth`, or `unknown`.
    pub transport: PortTransport,
    /// USB vendor ID. Omitted for non-USB ports.
    ///
    /// `schema_with` makes schemars emit
    /// `{"type": ["integer", "null"], "minimum": 0}` rather than the
    /// non-standard `"format": "uint16"` keyword. Validators can drop that
    /// format and warn. Use this override on every `uN` or `Option<uN>` field
    /// deriving `JsonSchema`.
    ///
    /// `#[serde(default)]` is required with `skip_serializing_if` here because
    /// schemars 1.2.2 cannot infer the `Option` through `schema_with`. Without
    /// it, this omitted field becomes required in the generated schema. The
    /// default is `None`, so the schema emits no `"default"` value.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
    pub vid: Option<u16>,
    /// USB product ID. Omitted for non-USB ports.
    ///
    /// See `vid` for why the `#[schemars(schema_with = ...)]` override is
    /// required on every unsigned-integer field that derives `JsonSchema`,
    /// and why `#[serde(default)]` must accompany `skip_serializing_if`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
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
    /// USB interface index. Omitted when unavailable or for non-USB ports.
    ///
    /// See `vid` for why the `#[schemars(schema_with = ...)]` override is
    /// required on every unsigned-integer field that derives `JsonSchema`,
    /// and why `#[serde(default)]` must accompany `skip_serializing_if`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    #[schemars(schema_with = "crate::schema_helpers::option_uint_schema")]
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

// Port enumeration provider.

/// Abstraction over OS port enumeration, so tools, resources, and the
/// automatic profile-session machinery share one consistent view of live
/// ports. Production uses [`SystemPortProvider`]; tests inject a static
/// provider whose `PortInfo.name` points at a real PTY slave while identity
/// fields describe a synthetic USB device.
pub trait PortProvider: Send + Sync {
    fn list_available(&self) -> crate::error::Result<Vec<PortInfo>>;
}

/// Production port provider: delegates to OS-level enumeration.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPortProvider;

impl PortProvider for SystemPortProvider {
    fn list_available(&self) -> crate::error::Result<Vec<PortInfo>> {
        PortInfo::list_available()
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
