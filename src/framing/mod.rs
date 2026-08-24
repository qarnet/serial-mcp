//! Frame boundary detection and protocol parsing for RX and TX streams.
//!
//! [`FrameDecoder`] splits byte streams into line, delimiter, length-prefixed,
//! start/end marker, SLIP, or COBS frames. Optional parsers interpret frame
//! content as AT commands, JSON lines, shell prompts, raw data, NMEA-0183, or
//! Modbus ASCII. Used by `read`, `transact`, and `capture_boot`.
//!
//! [`TxFramingMode`] applies TX frame boundaries corresponding to RX framing
//! modes for `write` and `transact`.
//!
//! Modules contain configuration and presets (`config`), TX codecs and byte
//! helpers (`codecs`), decoder state (`decoder`), and frame parsers (`parsers`).
//! Public types are re-exported from `crate::framing`.

mod codecs;
mod config;
mod decoder;
mod parsers;

pub use config::{
    preset_rx_framing, preset_rx_parser, preset_tx_framing, Endianness, LineEnding, ParserConfig,
    ParserType, ProtocolPreset, RxFramingConfig, RxFramingMode, TxFramingConfig, TxFramingMode,
    TxLineEnding,
};
pub use decoder::{Frame, FrameDecodeError, FrameDecoder, ParsedFrame, PushOutcome};
