//! MCP server for serial port communication.
//!
//! The primary interface is [`SerialHandler`], built via
//! [`SerialHandler::builder`] (or [`SerialHandler::new`] for defaults): an MCP
//! server surface exposing the 25 serial tools, resources, and prompts over
//! stdio or streamable HTTP. The `serial-mcp` binary wires the full
//! configuration surface (`--allowlist`, `--profiles-path`, `--capture-dir`,
//! buffer budgets). For agent-facing contracts — tool semantics, the RX
//! cursor model, framing/parser presets, device profiles, and persistent
//! capture — see the README and the guides under `docs/`.
//!
//! The library is published for programmatic server embedding; it is not a
//! client API.

pub mod buffer_budget;
pub mod capture_store;
pub(crate) mod checksums;
pub mod codec;
pub mod error;
pub mod framing;
pub(crate) mod learning;
pub mod limits;
pub mod log_buffer;
pub mod match_config;
pub(crate) mod mcp_protocol;
pub(crate) mod precedence;
pub mod profile_store;
pub mod profiles;
pub mod prompts;
pub mod resource_events;
pub mod resources;
pub mod rx_metadata;
pub mod rx_ring;
pub mod rx_session;
pub mod schema_helpers;
pub mod security;
pub mod serial;
pub mod server;
pub mod stop_controller;
pub mod tools;
pub mod tx_session;
pub(crate) mod util;

pub use error::{Result, SerialError};
pub use server::SerialHandler;
