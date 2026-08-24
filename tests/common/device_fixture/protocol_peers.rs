//! Independent protocol peers and wire builders for Linux PTY parity tests.
//!
//! These helpers deliberately do not use `serial_mcp::framing`. They model
//! device-side behavior or derive wire bytes from protocol rules and the pinned
//! test-only codec/server dependencies.

use anyhow::{Context, Result};
use rmodbus::{
    generate_ascii_frame, parse_ascii_frame,
    server::{storage::ModbusStorage, ModbusFrame},
    ModbusFrameBuf, ModbusProto,
};

use super::core::Action;
use super::DevicePeer;

/// Wrap a protocol peer with an explicit device-side response cadence.
///
/// PTY parity tests use this to let a public `transact` arm its post-write
/// read before the peer emits a response. The delay is part of fixture script
/// behavior, not a caller-side sleep.
pub struct DelayedPeer<P> {
    peer: P,
    delay: std::time::Duration,
}

impl<P> DelayedPeer<P> {
    pub fn new(peer: P, delay: std::time::Duration) -> Self {
        Self { peer, delay }
    }
}

impl<P: DevicePeer> DevicePeer for DelayedPeer<P> {
    fn on_start(&mut self) -> Vec<Action> {
        self.peer.on_start()
    }

    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        let mut actions = vec![Action::Delay(self.delay)];
        actions.extend(self.peer.on_command(command));
        actions
    }
}

/// Peer that observes TX through the fixture but intentionally emits nothing.
#[derive(Debug, Default)]
pub struct SilentPeer;

impl DevicePeer for SilentPeer {
    fn on_command(&mut self, _command: &[u8]) -> Vec<Action> {
        Vec::new()
    }
}

/// Small stateful AT DCE peer for command/default-parity tests.
#[derive(Debug, Default)]
pub struct AtPeer {
    echo: bool,
    signal: u8,
    urc_sequence: u32,
}

impl AtPeer {
    /// Generate response actions for one complete AT command without relying
    /// on serial-mcp's AT parser.
    pub fn handle_command(&mut self, command: &[u8]) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.echo {
            let mut echoed = command.to_vec();
            echoed.extend_from_slice(b"\r\n");
            actions.push(Action::Emit(echoed));
        }

        match command {
            b"ATE0" => {
                self.echo = false;
                actions.push(Action::Emit(b"OK\r\n".to_vec()));
            }
            b"ATE1" => {
                self.echo = true;
                actions.push(Action::Emit(b"OK\r\n".to_vec()));
            }
            b"AT+CSQ" => {
                self.signal = self.signal.saturating_add(1).max(1);
                actions.push(Action::Emit(
                    format!("+CEREG: {}\r\n", self.urc_sequence % 2).into_bytes(),
                ));
                actions.push(Action::Emit(
                    format!("+CSQ: {},99\r\n", self.signal).into_bytes(),
                ));
                actions.push(Action::Emit(b"OK\r\n".to_vec()));
                self.urc_sequence = self.urc_sequence.wrapping_add(1);
            }
            b"AT+CME" => actions.push(Action::Emit(b"+CME ERROR: 10\r\n".to_vec())),
            b"AT+CMS" => actions.push(Action::Emit(b"+CMS ERROR: 515\r\n".to_vec())),
            b"AT+NORESPONSE" => {}
            _ => actions.push(Action::Emit(b"ERROR\r\n".to_vec())),
        }
        actions
    }
}

impl DevicePeer for AtPeer {
    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        self.handle_command(command)
    }
}

/// RFC 1055 encoding from the pinned `slip-codec` dependency.
pub fn slip_encode(payload: &[u8]) -> Result<Vec<u8>> {
    let mut encoded = Vec::new();
    slip_codec::SlipEncoder::default().encode(payload, &mut encoded)?;
    Ok(encoded)
}

/// Plain COBS block followed by its 0x00 delimiter, encoded by the pinned
/// `cobs` dependency. Callers add a leading 0x00 when feeding an RX stream,
/// matching the fixture's packet-boundary convention.
pub fn cobs_encode(payload: &[u8]) -> Vec<u8> {
    let mut encoded = vec![0; cobs::max_encoding_length(payload.len())];
    let len = cobs::encode(payload, &mut encoded);
    encoded.truncate(len);
    encoded.push(0);
    encoded
}

/// A complete plain-COBS RX packet with an explicit leading boundary marker.
pub fn cobs_frame(payload: &[u8]) -> Vec<u8> {
    let mut framed = vec![0];
    framed.extend_from_slice(&cobs_encode(payload));
    framed
}

/// Build an NMEA sentence with a spec-derived XOR checksum.
pub fn nmea_sentence(marker: u8, body: &str, checksum: bool) -> Vec<u8> {
    let mut out = vec![marker];
    out.extend_from_slice(body.as_bytes());
    if checksum {
        let xor = body.as_bytes().iter().fold(0u8, |acc, byte| acc ^ byte);
        out.extend_from_slice(format!("*{xor:02X}").as_bytes());
    }
    out.extend_from_slice(b"\r\n");
    out
}

/// Modbus ASCII LRC: two's complement of the PDU byte sum.
pub fn modbus_lrc(pdu: &[u8]) -> u8 {
    pdu.iter()
        .fold(0u8, |sum, byte| sum.wrapping_add(*byte))
        .wrapping_neg()
}

/// Encode a Modbus ASCII PDU as uppercase hex, including an independently
/// calculated LRC but excluding ':' and CRLF transport markers.
pub fn modbus_ascii_payload(pdu: &[u8]) -> String {
    let mut out = String::with_capacity((pdu.len() + 1) * 2);
    for byte in pdu.iter().copied().chain(std::iter::once(modbus_lrc(pdu))) {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    out
}

/// Encode a complete Modbus ASCII transport frame from a PDU.
pub fn modbus_ascii_frame(pdu: &[u8]) -> Vec<u8> {
    format!(":{}\r\n", modbus_ascii_payload(pdu)).into_bytes()
}

/// Stateful Modbus ASCII device peer backed by pinned `rmodbus` server logic.
pub struct ModbusAsciiPeer {
    unit: u8,
    storage: ModbusStorage<128, 16, 16, 128>,
}

impl ModbusAsciiPeer {
    pub fn new(unit: u8) -> Self {
        Self {
            unit,
            storage: ModbusStorage::new(),
        }
    }

    /// Handle one complete framed Modbus ASCII request independently of the
    /// production framing/parser implementation.
    pub fn handle(&mut self, ascii: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut request: ModbusFrameBuf = [0; 256];
        let parsed = parse_ascii_frame(ascii, ascii.len(), &mut request, 0)
            .context("parse Modbus ASCII request")?;
        let mut response = Vec::new();
        let mut frame = ModbusFrame::new(
            self.unit,
            &request[..usize::from(parsed)],
            ModbusProto::Ascii,
            &mut response,
        );
        frame.parse().context("parse Modbus PDU")?;
        if frame.processing_required {
            if frame.readonly {
                frame.process_read(&self.storage)?;
            } else {
                frame.process_write(&mut self.storage)?;
            }
        }
        if !frame.response_required {
            return Ok(None);
        }
        frame.finalize_response()?;
        let mut ascii_response = Vec::new();
        generate_ascii_frame(&response, &mut ascii_response)?;
        Ok(Some(ascii_response))
    }
}

impl DevicePeer for ModbusAsciiPeer {
    fn on_command(&mut self, command: &[u8]) -> Vec<Action> {
        let mut framed = command.to_vec();
        framed.extend_from_slice(b"\r\n");
        match self.handle(&framed) {
            Ok(Some(response)) => vec![Action::Emit(response)],
            Ok(None) | Err(_) => Vec::new(),
        }
    }
}
