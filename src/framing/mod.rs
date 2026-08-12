//! Frame boundary detection and protocol parsing for RX and TX streams.
//!
//! Provides a [`FrameDecoder`] that splits a byte stream into structured
//! frames using line, delimiter, length-prefixed, start/end marker, SLIP, or
//! COBS framing. Optional parsers interpret frame content as AT commands,
//! JSON lines, shell prompts, raw data, NMEA-0183, or Modbus ASCII. Used by
//! `read`, the `transact` read half, and `capture_boot`.
//!
//! Also provides TX framing via [`TxFramingMode`] which encodes payloads
//! with frame boundaries matching the RX modes. Used on `write` and the
//! `transact` write half.
//!
//! The implementation is split into focused submodules: configuration types
//! and preset expansion (`config`), TX codecs and byte helpers (`codecs`),
//! the RX decoder state machine (`decoder`), and frame-content parsers
//! (`parsers`). Public types are re-exported at `crate::framing::*`.

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
