//! Four-layer framing/parser precedence resolution.
//!
//! Single source of truth for the precedence rule:
//!
//! ```text
//! explicit call field > call-time protocol preset > connection default > connection protocol preset
//! ```
//!
//! Shared by `io_ops::write` (tx_framing), `io_ops::read` (rx_framing +
//! rx_parser), and `stream_ops::subscribe` (rx_framing + rx_parser). Keeping
//! the ladder in one place prevents the three hand-written copies from
//! drifting and makes every precedence boundary directly unit-testable.

use crate::framing::ProtocolPreset;

/// Resolve a single framing/parser field using the four-layer precedence
/// ladder.
///
/// Precedence (first non-`None` source wins):
/// 1. `explicit` — the per-call field.
/// 2. `call_protocol` — the per-call `protocol` preset, mapped through
///    `apply_preset`.
/// 3. `conn_default` — the connection default for this field.
/// 4. `conn_protocol` — the connection `protocol` preset, mapped through
///    `apply_preset`.
///
/// `apply_preset` is the matching `preset_*` function from `crate::framing`
/// for the field being resolved. `conn_default` is borrowed and cloned only
/// if reached.
pub(crate) fn resolve_field<T: Clone>(
    explicit: Option<T>,
    call_protocol: Option<ProtocolPreset>,
    apply_preset: impl Fn(ProtocolPreset) -> T,
    conn_default: Option<&T>,
    conn_protocol: Option<ProtocolPreset>,
) -> Option<T> {
    if let Some(explicit) = explicit {
        return Some(explicit);
    }
    if let Some(p) = call_protocol {
        return Some(apply_preset(p));
    }
    if let Some(def) = conn_default {
        return Some(def.clone());
    }
    conn_protocol.map(apply_preset)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::{
        preset_rx_framing, preset_rx_parser, preset_tx_framing, LineEnding, ParserConfig,
        ParserType, ProtocolPreset, RxFramingConfig, RxFramingMode, TxFramingConfig, TxFramingMode,
        TxLineEnding,
    };

    const PROTO: Option<ProtocolPreset> = Some(ProtocolPreset::AtCommand);

    fn explicit_tx() -> TxFramingConfig {
        // A TX config that differs from the AtCommand preset (which uses Cr).
        TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Lf,
            },
        }
    }

    fn conn_default_tx() -> TxFramingConfig {
        // A TX config that differs from the AtCommand preset but is not the
        // same as `explicit_tx`.
        TxFramingConfig {
            mode: TxFramingMode::Line {
                ending: TxLineEnding::Crlf,
            },
        }
    }

    fn explicit_rx() -> RxFramingConfig {
        // An RX config that differs from the AtCommand preset (which uses Auto).
        RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Lf,
            },
            max_frames: None,
            include_terminators: false,
        }
    }

    fn conn_default_rx() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Cr,
            },
            max_frames: None,
            include_terminators: false,
        }
    }

    fn conn_default_parser() -> ParserConfig {
        // A parser config that differs from the AtCommand preset.
        ParserConfig {
            parser_type: ParserType::Raw,
            custom_prompt: None,
        }
    }

    // --- TxFramingConfig tests ---

    #[test]
    fn explicit_beats_call_protocol() {
        let explicit = explicit_tx();
        let result = resolve_field::<TxFramingConfig>(
            Some(explicit.clone()),
            PROTO,
            preset_tx_framing,
            None,
            None,
        );
        assert!(result.is_some());
        assert_ne!(result, Some(preset_tx_framing(ProtocolPreset::AtCommand)));
        assert_eq!(result, Some(explicit));
    }

    #[test]
    fn call_protocol_beats_conn_default() {
        let conn_def = conn_default_tx();
        let result =
            resolve_field::<TxFramingConfig>(None, PROTO, preset_tx_framing, Some(&conn_def), None);
        assert_eq!(result, Some(preset_tx_framing(ProtocolPreset::AtCommand)));
        assert_ne!(result, Some(conn_def));
    }

    #[test]
    fn conn_default_beats_conn_protocol() {
        let conn_def = conn_default_tx();
        let result =
            resolve_field::<TxFramingConfig>(None, None, preset_tx_framing, Some(&conn_def), PROTO);
        assert_eq!(result, Some(conn_def));
        assert_ne!(result, Some(preset_tx_framing(ProtocolPreset::AtCommand)));
    }

    #[test]
    fn conn_protocol_is_fallback() {
        let result = resolve_field::<TxFramingConfig>(None, None, preset_tx_framing, None, PROTO);
        assert_eq!(result, Some(preset_tx_framing(ProtocolPreset::AtCommand)));
    }

    #[test]
    fn all_none_returns_none() {
        let result = resolve_field::<TxFramingConfig>(None, None, preset_tx_framing, None, None);
        assert_eq!(result, None);
    }

    #[test]
    fn mixed_gap_fill_rx_framing_explicit_plus_conn_protocol_parser() {
        // Explicit rx_framing wins; rx_parser falls through to conn_protocol.
        let explicit = explicit_rx();
        let rx_framing = resolve_field::<RxFramingConfig>(
            Some(explicit.clone()),
            None,
            preset_rx_framing,
            None,
            PROTO,
        );
        assert_eq!(rx_framing, Some(explicit));
        assert_ne!(
            rx_framing,
            Some(preset_rx_framing(ProtocolPreset::AtCommand))
        );

        let rx_parser = resolve_field::<ParserConfig>(None, None, preset_rx_parser, None, PROTO);
        assert_eq!(rx_parser, Some(preset_rx_parser(ProtocolPreset::AtCommand)));
    }

    // --- RxFramingConfig generic boundary test ---

    #[test]
    fn rx_framing_call_protocol_beats_conn_default() {
        let conn_def = conn_default_rx();
        let result =
            resolve_field::<RxFramingConfig>(None, PROTO, preset_rx_framing, Some(&conn_def), None);
        assert_eq!(result, Some(preset_rx_framing(ProtocolPreset::AtCommand)));
        assert_ne!(result, Some(conn_def));
    }

    // --- ParserConfig generic boundary test ---

    #[test]
    fn parser_call_protocol_beats_conn_default() {
        let conn_def = conn_default_parser();
        let result =
            resolve_field::<ParserConfig>(None, PROTO, preset_rx_parser, Some(&conn_def), None);
        assert_eq!(result, Some(preset_rx_parser(ProtocolPreset::AtCommand)));
        assert_ne!(result, Some(conn_def));
    }
}
