//! Frame boundary detection and protocol parsing for RX and TX streams.
//!
//! Provides a [`FrameDecoder`] that splits a byte stream into structured
//! frames using one of four boundary modes (line, delimiter, length-prefixed,
//! start/end marker). Optional parsers interpret frame content (AT commands,
//! JSON lines, shell prompts). Used as an option on `read` and `subscribe`.
//!
//! Also provides TX framing via [`TxFramingMode`] which encodes payloads
//! with frame boundaries matching the RX modes. Used on `write`.
//!
//! The implementation is split into focused submodules: configuration types
//! and preset expansion (`config`), TX codecs and byte helpers (`codecs`),
//! the RX decoder state machine (`decoder`), and frame-content parsers
//! (`parsers`). Everything formerly public at `crate::framing::*` is
//! re-exported here at the flat original path.

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
