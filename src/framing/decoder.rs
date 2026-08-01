//! RX frame decoding.
//!
//! The stateful `FrameDecoder` splits a byte stream into structured frames
//! (line, delimiter, length-prefixed, start/end marker, SLIP, COBS), plus
//! the `Frame`/`ParsedFrame`/`PushOutcome` result types, `FrameDecodeError`,
//! the `FrameParser` trait, and frame emission.

use crate::codec;
use crate::util::find_subsequence;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::codecs::{
    extract_delimited, hex_upper, is_blank_frame, read_length_prefix, SLIP_END, SLIP_ESC,
    SLIP_ESC_END, SLIP_ESC_ESC,
};
use super::config::{Endianness, LineEnding, ParserConfig, RxFramingConfig, RxFramingMode};
use super::parsers::build_parser;

// ---- Frame types -----------------------------------------------------------

/// A decoded frame with optional parsed content.
///
/// `Frame` is constructed by decoders and serialized to clients; it is not
/// deserialized, so no `Deserialize` derive.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Frame {
    /// Raw frame bytes (without delimiters/terminators unless include_terminators is set).
    #[schemars(schema_with = "crate::schema_helpers::byte_array_schema")]
    pub data: Vec<u8>,
    /// Frame number since decoder creation (0-based).
    #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
    pub index: usize,
    /// Boundary detection mode used (for diagnostic purposes).
    pub frame_type: &'static str,
    /// Parsed frame fields, if a parser is configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parsed: Option<ParsedFrame>,
}

/// Structured field interpretation of a frame.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "parser", rename_all = "snake_case")]
pub enum ParsedFrame {
    AtCommand {
        /// Result code, URC, data, or error.
        response_type: String,
        /// Command name (e.g. "+CGREG").
        #[serde(skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// Status: "OK", "ERROR", "CME ERROR: <code>", etc.
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        /// Response data fields (lines between echo and status).
        fields: Vec<String>,
    },
    Json(serde_json::Value),
    ShellPrompt {
        prompt: String,
        prompt_type: String,
    },
    Raw,
    Nmea {
        /// Talker ID (e.g. "GP" for GPS, "GN" for GLONASS, "AI" for AIS).
        /// Two characters for standard sentences; for proprietary sentences
        /// (`$P...`) the talker_id is `"P"` and the rest forms the sentence_type.
        talker_id: String,
        /// Sentence type (e.g. "GGA", "RMC", "GLL", "AIVDM").
        /// Three characters for standard sentences; for proprietary (`$P...`)
        /// this holds the remainder after the leading `P` (e.g. `"GRMM"`).
        sentence_type: String,
        /// Comma-separated data fields (the body after the address, before '*').
        fields: Vec<String>,
        /// Checksum status:
        /// - Some(true): checksum present and valid (or present, validate=false: not enforced but reported as valid-shape).
        /// - Some(false): checksum present and INVALID (only reachable when validate=false; when validate=true a mismatch drops the frame and increments `frames_dropped`).
        /// - None: no checksum present in the sentence.
        #[serde(skip_serializing_if = "Option::is_none")]
        checksum_valid: Option<bool>,
    },
    ModbusAscii {
        /// Slave address (1-247), decoded from the first 2 hex chars of the body.
        /// 0 = broadcast. Stored as the decoded byte value (0-255).
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        address: u8,
        /// Function code (decoded byte value). 1-127 = normal; 128+ = exception
        /// response (the high bit set indicates an exception).
        #[schemars(schema_with = "crate::schema_helpers::uint_schema")]
        function_code: u8,
        /// Decoded data bytes (the PDU payload after address + function code,
        /// excluding the LRC). For an exception response, the first byte is the
        /// exception code.
        #[schemars(schema_with = "crate::schema_helpers::byte_array_schema")]
        data: Vec<u8>,
        /// LRC status. Same semantics as `ParsedFrame::Nmea::checksum_valid`
        /// (`Some(true)` valid, `Some(false)` invalid with `validate=false`,
        /// `None` absent) — see that field's doc for the full explanation.
        /// `None` here also covers a malformed frame shorter than 2 hex chars
        /// after the data (defensive; should not occur for valid frames).
        checksum_valid: Option<bool>,
    },
}

// ---- Frame decoder ---------------------------------------------------------

/// Stateful frame boundary detector.
///
/// Push chunks via [`FrameDecoder::push`] to receive decoded
/// [`Frame`] instances. Callers accumulate frames and drain consumed
/// bytes from their accumulation buffer.
pub struct FrameDecoder {
    /// Buffer for incomplete frame data.
    buf: Vec<u8>,
    /// How frame boundaries are detected.
    mode: DecoderMode,
    /// Total frames emitted so far.
    frame_count: usize,
    /// Include terminators in frame data.
    include_terminators: bool,
    /// Skip frames whose data is empty or whitespace-only.
    skip_empty: bool,
    /// Optional parser for frame content.
    parser: Option<Box<dyn FrameParser>>,
}

enum DecoderMode {
    Line(LineState),
    Delimiter(Vec<u8>),
    LengthPrefixed {
        prefix_size: u8,
        endianness: Endianness,
        remaining_offset: usize,
        next_payload_len: Option<usize>,
    },
    StartEnd {
        start: Vec<Vec<u8>>,
        end: Vec<u8>,
        include_markers: bool,
        in_frame: bool,
    },
    Slip {
        state: SlipState,
    },
    Cobs {
        state: CobsState,
    },
}

/// Internal line-decoder state.
///
/// `Lf`, `Cr`, `Crlf` are terminal (no promotion). `AutoLf` is the starting
/// state for `ending: auto`; it can transition to `PendingCr` when a bare `\r`
/// is detected, and from there to `CrMode` when the `\r` is confirmed not to
/// be part of a CRLF sequence.
enum LineState {
    Lf,
    Cr,
    Crlf,
    /// `auto` initial state: split on `\n`, strip preceding `\r` (CRLF-aware).
    AutoLf,
    /// Saw a `\r` at end of buffer with no trailing `\n` yet. Stores the index
    /// of the pending `\r` in `self.buf` at the moment of transition. Waiting
    /// for next byte to decide if it's CRLF or bare CR.
    PendingCr(usize),
    /// Promoted: split on bare `\r` only, ignore `\n` for the rest of the call.
    CrMode,
}

/// SLIP decoder state.
#[derive(Debug, Clone)]
enum SlipState {
    /// Discard bytes until the first END marker is seen.
    BeforeFirstEnd,
    /// Inside a frame. `buf` accumulates decoded payload bytes. `escaped` is
    /// true after an `ESC` byte — the next byte is the escape code.
    InFrame { buf: Vec<u8>, escaped: bool },
}

/// COBS decoder state.
#[derive(Debug, Clone)]
enum CobsState {
    /// Discard bytes until the first 0x00 delimiter is seen (SLIP parity:
    /// skip junk before the first frame marker).
    BeforeFirstDelim,
    /// Inside a frame. `decoded` accumulates the reconstructed payload (with
    /// phantom zeros inserted by each non-0xFF code). `remaining` is data
    /// bytes still to copy for the current code block; when `remaining == 0`
    /// the next byte is a code byte (or the 0x00 delimiter ending the frame).
    /// `pending_zero` is true when the current code block (non-0xFF) has its
    /// implicit zero still to be appended after the remaining data bytes are
    /// copied.
    InFrame {
        decoded: Vec<u8>,
        remaining: u8,
        pending_zero: bool,
    },
}

pub(crate) trait FrameParser: Send + Sync {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError>;
}

/// Emit one decoded frame: apply skip_empty, run the parser with
/// Phase-1 checksum drop-and-count semantics, increment frame_count on
/// success, and return either `Some(Frame)` to emit or `None` to skip.
/// On a `ChecksumMismatch` with `validate=true`: returns `Ok(None)`,
/// increments `frames_dropped`, logs a `warn`. On a stream-fatal parser
/// error: returns `Err(e)` so the caller can set `PushOutcome.error` and
/// stop.
fn emit_frame(
    data: Vec<u8>,
    frame_type: &'static str,
    frame_count: &mut usize,
    skip_empty: bool,
    parser: &Option<Box<dyn FrameParser>>,
    frames_dropped: &mut usize,
) -> Result<Option<Frame>, FrameDecodeError> {
    if skip_empty && is_blank_frame(&data) {
        return Ok(None);
    }
    let parsed = match parser.as_ref().map(|p| p.parse(&data)) {
        Some(Ok(pf)) => Some(pf),
        Some(Err(FrameDecodeError::ChecksumMismatch { expected, received })) => {
            *frames_dropped += 1;
            tracing::warn!(
                "frame dropped: checksum mismatch (expected {}, received {})",
                hex_upper(&expected),
                hex_upper(&received)
            );
            return Ok(None);
        }
        Some(Err(e)) => return Err(e),
        None => None,
    };
    *frame_count += 1;
    Ok(Some(Frame {
        data,
        index: *frame_count - 1,
        frame_type,
        parsed,
    }))
}

/// SLIP decoder: consume `buf_outer` byte-by-byte according to current
/// [`SlipState`] in `mode`. Returns decoded frames, or `Err` for a
/// malformed escape sequence. Updates `mode` state in-place.
fn slip_decode(
    buf_outer: &mut Vec<u8>,
    frame_count: &mut usize,
    parser: &Option<Box<dyn FrameParser>>,
    mode: &mut DecoderMode,
    skip_empty: bool,
) -> PushOutcome {
    let mut frames = Vec::new();
    let mut frames_dropped: usize = 0;
    let mut error: Option<FrameDecodeError> = None;
    let state = match mode {
        DecoderMode::Slip { ref mut state } => state,
        _ => {
            return PushOutcome {
                frames,
                frames_dropped,
                error,
            }
        }
    };

    loop {
        match state {
            SlipState::BeforeFirstEnd => {
                if let Some(pos) = buf_outer.iter().position(|&b| b == SLIP_END) {
                    buf_outer.drain(..=pos);
                    *state = SlipState::InFrame {
                        buf: Vec::new(),
                        escaped: false,
                    };
                    continue;
                }
                buf_outer.clear();
                return PushOutcome {
                    frames,
                    frames_dropped,
                    error,
                };
            }
            SlipState::InFrame {
                ref mut buf,
                ref mut escaped,
            } => {
                let mut read_pos: usize = 0;
                while read_pos < buf_outer.len() {
                    let b = buf_outer[read_pos];
                    read_pos += 1;

                    if *escaped {
                        match b {
                            SLIP_ESC_END => {
                                buf.push(SLIP_END);
                                *escaped = false;
                            }
                            SLIP_ESC_ESC => {
                                buf.push(SLIP_ESC);
                                *escaped = false;
                            }
                            _ => {
                                // Malformed escape: clear in-progress frame,
                                // reset escaped flag, and resync on next END.
                                // Drain only the consumed bytes (up to and
                                // including the malformed byte via read_pos);
                                // leave the remainder of buf_outer intact so
                                // BeforeFirstEnd can scan/discard it on the
                                // next push.
                                buf_outer.drain(..read_pos);
                                buf.clear();
                                *escaped = false;
                                *state = SlipState::BeforeFirstEnd;
                                error = Some(FrameDecodeError::SlipInvalidEscape(b));
                                return PushOutcome {
                                    frames,
                                    frames_dropped,
                                    error,
                                };
                            }
                        }
                    } else {
                        match b {
                            SLIP_END => {
                                let data = std::mem::take(buf);
                                match emit_frame(
                                    data,
                                    "slip",
                                    frame_count,
                                    skip_empty,
                                    parser,
                                    &mut frames_dropped,
                                ) {
                                    Ok(Some(frame)) => frames.push(frame),
                                    Ok(None) => (),
                                    Err(e) => {
                                        buf_outer.drain(..read_pos);
                                        *state = SlipState::BeforeFirstEnd;
                                        error = Some(e);
                                        return PushOutcome {
                                            frames,
                                            frames_dropped,
                                            error,
                                        };
                                    }
                                }
                            }
                            SLIP_ESC => {
                                *escaped = true;
                            }
                            _ => {
                                buf.push(b);
                            }
                        }
                    }
                }
                // Consumed the whole buffer without hitting a terminal
                // return: drain everything read and fall through to the
                // outer loop.
                buf_outer.drain(..read_pos);
                return PushOutcome {
                    frames,
                    frames_dropped,
                    error,
                };
            }
        }
    }
}

/// COBS decoder: consume `buf_outer` according to current [`CobsState`].
/// Returns decoded frames. A code byte of 0x00 (the delimiter) terminates the
/// frame; the trailing phantom zero (inserted by the final non-0xFF code) is
/// dropped before emitting the frame. Malformed code bytes surface as
/// [`FrameDecodeError::CobsInvalidCode`].
fn cobs_decode(
    buf_outer: &mut Vec<u8>,
    frame_count: &mut usize,
    parser: &Option<Box<dyn FrameParser>>,
    mode: &mut DecoderMode,
    skip_empty: bool,
) -> PushOutcome {
    let mut frames = Vec::new();
    let mut frames_dropped: usize = 0;
    let mut error: Option<FrameDecodeError> = None;
    let state = match mode {
        DecoderMode::Cobs { ref mut state } => state,
        _ => {
            return PushOutcome {
                frames,
                frames_dropped,
                error,
            }
        }
    };

    loop {
        match state {
            CobsState::BeforeFirstDelim => {
                if let Some(pos) = buf_outer.iter().position(|&b| b == 0x00) {
                    buf_outer.drain(..=pos);
                    *state = CobsState::InFrame {
                        decoded: Vec::new(),
                        remaining: 0,
                        pending_zero: false,
                    };
                    continue;
                }
                buf_outer.clear();
                return PushOutcome {
                    frames,
                    frames_dropped,
                    error,
                };
            }
            CobsState::InFrame {
                ref mut decoded,
                ref mut remaining,
                ref mut pending_zero,
            } => {
                let mut read_pos: usize = 0;
                let len = buf_outer.len();
                loop {
                    // 1. Flush pending zero first (does NOT consume a
                    //    byte). This handles the code==1 case (zero data
                    //    bytes) where the implicit zero is due immediately
                    //    before the next code byte. For code>1, this flush
                    //    happens on the iteration after the last data byte
                    //    was copied (when remaining just hit 0), and the
                    //    current b is the next code byte.
                    if *remaining == 0 && *pending_zero {
                        decoded.push(0x00);
                        *pending_zero = false;
                        // Fall through to code-byte handling (or the next
                        // iteration will read the next byte). Do NOT
                        // consume a byte from buf_outer for this flush.
                    }
                    // 2. If we have consumed all bytes in buf_outer, break.
                    if read_pos >= len {
                        break;
                    }
                    let b = buf_outer[read_pos];
                    read_pos += 1;

                    // 3. Copy data bytes for the current code block.
                    if *remaining > 0 {
                        decoded.push(b);
                        *remaining -= 1;
                        // (Pending-zero flush happens at top of next
                        //  iteration when remaining hits 0.)
                        continue;
                    }

                    // 4. Expecting a code byte (or the 0x00 delimiter).
                    if b == 0x00 {
                        // Frame delimiter: the final code's implicit zero
                        // is the phantom. Drop it and emit the frame.
                        let mut data = std::mem::take(decoded);
                        // The final code is always non-0xFF (the encoder
                        // always writes a final code for the phantom,
                        // which is 0x01-0xFE). Pop the trailing zero that
                        // the final code inserted. Defensive: only pop if
                        // the last byte is 0x00.
                        if data.last() == Some(&0x00) {
                            data.pop();
                        }
                        match emit_frame(
                            data,
                            "cobs",
                            frame_count,
                            skip_empty,
                            parser,
                            &mut frames_dropped,
                        ) {
                            Ok(Some(frame)) => frames.push(frame),
                            Ok(None) => (),
                            Err(e) => {
                                buf_outer.drain(..read_pos);
                                *decoded = Vec::new();
                                *remaining = 0;
                                *pending_zero = false;
                                *state = CobsState::BeforeFirstDelim;
                                error = Some(e);
                                return PushOutcome {
                                    frames,
                                    frames_dropped,
                                    error,
                                };
                            }
                        }
                        *state = CobsState::BeforeFirstDelim;
                        buf_outer.drain(..read_pos);
                        // Continue the outer loop to look for the next
                        // frame's leading delimiter (or run out of bytes).
                        break;
                    }

                    // 5. Code byte: 0x01-0xFE => (code-1) data bytes then
                    //    a zero; 0xFF => 254 data bytes, no zero.
                    if b == 0xFF {
                        *remaining = 254;
                        *pending_zero = false;
                    } else {
                        // b in 0x01-0xFE: set (code-1) data bytes,
                        // then insert a zero.
                        *remaining = b - 1;
                        *pending_zero = true;
                    }
                }
                // If we broke out of the inner loop via the delimiter,
                // the outer loop continues with BeforeFirstDelim.
                // Otherwise we consumed all bytes in buf_outer without
                // hitting a delimiter — drain and wait for more bytes.
                if !matches!(state, CobsState::BeforeFirstDelim) {
                    buf_outer.drain(..read_pos);
                    return PushOutcome {
                        frames,
                        frames_dropped,
                        error,
                    };
                }
            }
        }
    }
}

// ---- Frame decoder implementation ------------------------------------------

impl FrameDecoder {
    /// Create a new frame decoder from an RX framing configuration and
    /// an optional parser configuration.
    pub fn new(
        config: &RxFramingConfig,
        parser_config: Option<&ParserConfig>,
    ) -> Result<Self, String> {
        let mode = match &config.mode {
            RxFramingMode::Line { ending } => {
                let state = match ending {
                    LineEnding::Auto => LineState::AutoLf,
                    LineEnding::Lf => LineState::Lf,
                    LineEnding::Cr => LineState::Cr,
                    LineEnding::Crlf => LineState::Crlf,
                };
                DecoderMode::Line(state)
            }
            RxFramingMode::Delimiter {
                delimiter,
                delimiter_encoding,
            } => {
                let bytes = codec::decode((*delimiter_encoding).into(), delimiter)
                    .map_err(|e| format!("Invalid delimiter encoding: {e}"))?;
                if bytes.is_empty() {
                    return Err("Delimiter must not be empty".into());
                }
                DecoderMode::Delimiter(bytes)
            }
            RxFramingMode::LengthPrefixed {
                prefix_size,
                endianness,
                initial_offset,
            } => {
                if !matches!(prefix_size, 1 | 2 | 4) {
                    return Err("prefix_size must be 1, 2, or 4".into());
                }
                DecoderMode::LengthPrefixed {
                    prefix_size: *prefix_size,
                    endianness: *endianness,
                    remaining_offset: initial_offset.unwrap_or(0),
                    next_payload_len: None,
                }
            }
            RxFramingMode::StartEnd {
                start,
                end,
                marker_encoding,
                include_markers,
            } => {
                if start.is_empty() {
                    return Err("Start markers must not be empty".into());
                }
                let mut start_bytes_vec: Vec<Vec<u8>> = Vec::with_capacity(start.len());
                for s in start {
                    let sb = codec::decode((*marker_encoding).into(), s)
                        .map_err(|e| format!("Invalid start marker encoding: {e}"))?;
                    if sb.is_empty() {
                        return Err("Start and end markers must not be empty".into());
                    }
                    start_bytes_vec.push(sb);
                }
                let end_bytes = codec::decode((*marker_encoding).into(), end)
                    .map_err(|e| format!("Invalid end marker encoding: {e}"))?;
                if end_bytes.is_empty() {
                    return Err("Start and end markers must not be empty".into());
                }
                DecoderMode::StartEnd {
                    start: start_bytes_vec,
                    end: end_bytes,
                    include_markers: *include_markers,
                    in_frame: false,
                }
            }
            RxFramingMode::Slip => DecoderMode::Slip {
                state: SlipState::BeforeFirstEnd,
            },
            RxFramingMode::Cobs => DecoderMode::Cobs {
                state: CobsState::BeforeFirstDelim,
            },
        };

        let parser: Option<Box<dyn FrameParser>> = match parser_config {
            Some(pc) => Some(build_parser(pc)?),
            None => None,
        };

        let buf = Vec::new();

        Ok(Self {
            buf,
            mode,
            frame_count: 0,
            include_terminators: config.include_terminators,
            skip_empty: config.skip_empty,
            parser,
        })
    }

    /// Feed a chunk of bytes. Returns any complete frames found, or a
    /// [`FrameDecodeError`] for protocol violations (SLIP malformed escape).
    /// The caller is responsible for draining consumed bytes from their
    /// accumulation buffer.
    /// Feed a chunk of bytes. Returns any complete frames found in a
    /// [`PushOutcome`] that carries both the decoded frames and any
    /// stream-fatal error (SLIP malformed escape, COBS invalid code).
    /// Per-frame checksum mismatches are counted in `frames_dropped`
    /// and do NOT set `error`.
    /// The caller is responsible for draining consumed bytes from their
    /// accumulation buffer.
    pub fn push(&mut self, chunk: &[u8]) -> PushOutcome {
        self.buf.extend_from_slice(chunk);
        // SLIP and COBS are handled separately via free functions to avoid
        // borrow conflicts between the mutable mode borrow and self.
        if matches!(self.mode, DecoderMode::Slip { .. }) {
            let outcome = slip_decode(
                &mut self.buf,
                &mut self.frame_count,
                &self.parser,
                &mut self.mode,
                self.skip_empty,
            );
            return outcome;
        }
        if matches!(self.mode, DecoderMode::Cobs { .. }) {
            let outcome = cobs_decode(
                &mut self.buf,
                &mut self.frame_count,
                &self.parser,
                &mut self.mode,
                self.skip_empty,
            );
            return outcome;
        }
        let mut frames = Vec::new();
        let mut frames_dropped: usize = 0;
        let mut error: Option<FrameDecodeError> = None;
        'outer: loop {
            let consumed = match &mut self.mode {
                DecoderMode::Line(state) => match state {
                    LineState::Lf => self.match_line_byte(b'\n'),
                    LineState::Cr => self.match_line_byte(b'\r'),
                    LineState::Crlf => self.match_line_crlf(),
                    LineState::AutoLf => self.match_auto_lf(),
                    LineState::PendingCr(_) => self.match_pending_cr(),
                    LineState::CrMode => self.match_line_byte(b'\r'),
                },
                DecoderMode::Delimiter(delim) => {
                    extract_delimited(&mut self.buf, delim, self.include_terminators)
                }
                DecoderMode::LengthPrefixed {
                    prefix_size,
                    endianness,
                    remaining_offset,
                    next_payload_len,
                } => {
                    if *remaining_offset > 0 {
                        let drain = (*remaining_offset).min(self.buf.len());
                        self.buf.drain(..drain);
                        *remaining_offset -= drain;
                    }
                    // Determine next_payload_len if not yet known
                    if next_payload_len.is_none() {
                        let needed = *prefix_size as usize;
                        if self.buf.len() < needed {
                            break;
                        }
                        let len =
                            read_length_prefix(&self.buf[..needed], *prefix_size, *endianness);
                        *next_payload_len = Some(len);
                    }
                    let payload_len = match *next_payload_len {
                        Some(len) => len,
                        None => break,
                    };
                    let header_len = *prefix_size as usize;
                    let total_needed = header_len + payload_len;
                    if self.buf.len() < total_needed {
                        break;
                    }
                    let frame_bytes = if self.include_terminators {
                        self.buf[..total_needed].to_vec()
                    } else {
                        self.buf[header_len..total_needed].to_vec()
                    };
                    self.buf.drain(..total_needed);
                    *next_payload_len = None;
                    Some(frame_bytes)
                }
                DecoderMode::StartEnd {
                    start,
                    end,
                    include_markers,
                    in_frame,
                } => {
                    let start = start.clone();
                    let end = end.clone();
                    let include = *include_markers;
                    if !*in_frame {
                        // Find the earliest start marker among all candidates.
                        let mut best_pos: Option<(usize, usize)> = None; // (pos, marker_len)
                        for marker in &start {
                            if let Some(pos) = find_subsequence(&self.buf, marker) {
                                match best_pos {
                                    Some((bp, _)) if pos < bp => {
                                        best_pos = Some((pos, marker.len()));
                                    }
                                    None => {
                                        best_pos = Some((pos, marker.len()));
                                    }
                                    _ => {}
                                }
                            }
                        }
                        if let Some((pos, marker_len)) = best_pos {
                            self.buf.drain(..pos);
                            if !include {
                                self.buf.drain(..marker_len);
                            }
                            *in_frame = true;
                        } else {
                            // Keep enough trailing bytes to allow a partial
                            // match of the longest start marker.
                            let keep = start
                                .iter()
                                .map(|m| m.len().saturating_sub(1))
                                .max()
                                .unwrap_or(0);
                            if self.buf.len() > keep {
                                self.buf.drain(..(self.buf.len() - keep));
                            }
                            break;
                        }
                    }
                    if *in_frame {
                        if let Some(pos) = find_subsequence(&self.buf, &end) {
                            let frame_bytes = if include {
                                let fb = self.buf[..pos + end.len()].to_vec();
                                self.buf.drain(..pos + end.len());
                                fb
                            } else {
                                let fb = self.buf[..pos].to_vec();
                                self.buf.drain(..pos + end.len());
                                fb
                            };
                            *in_frame = false;
                            Some(frame_bytes)
                        } else {
                            break;
                        }
                    } else {
                        None
                    }
                }
                DecoderMode::Slip { .. } => unreachable!("SLIP handled before match"),
                DecoderMode::Cobs { .. } => unreachable!("COBS handled before match"),
            };

            match consumed {
                None => break,
                Some(frame_bytes) => {
                    match emit_frame(
                        frame_bytes,
                        self.frame_type_str(),
                        &mut self.frame_count,
                        self.skip_empty,
                        &self.parser,
                        &mut frames_dropped,
                    ) {
                        Ok(Some(frame)) => frames.push(frame),
                        Ok(None) => continue,
                        Err(e) => {
                            error = Some(e);
                            break 'outer;
                        }
                    }
                }
            }
        }
        PushOutcome {
            frames,
            frames_dropped,
            error,
        }
    }

    /// Split the frame at `split_pos` with terminator length `term_len`.
    /// Returns the frame bytes (including terminators if
    /// `include_terminators`), and drains consumed bytes from the buffer.
    fn take_frame(&mut self, split_pos: usize, term_len: usize) -> Vec<u8> {
        let fb = if self.include_terminators {
            self.buf[..split_pos + term_len].to_vec()
        } else {
            self.buf[..split_pos].to_vec()
        };
        self.buf.drain(..split_pos + term_len);
        fb
    }

    /// `auto` initial state: scan for `\n` (CRLF-aware), detect bare `\r`.
    ///
    /// If a `\r` is found at the end of the buffer with no `\n` following it in
    /// the same chunk, transitions to [`LineState::PendingCr`] and returns
    /// `None` to wait for more data. If a `\r` is immediately followed by a
    /// non-`\n` byte (bare CR confirmed in same chunk), transitions directly to
    /// [`LineState::CrMode`] and returns the frame before the `\r`.
    fn match_auto_lf(&mut self) -> Option<Vec<u8>> {
        // Scan for \n first — preserves existing eager-LF behavior.
        if let Some(lf_pos) = self.buf.iter().position(|&b| b == b'\n') {
            // Check if there's a bare \r before this \n that hasn't been
            // consumed yet. If no \r precedes this \n, or only the \r
            // immediately before \n, handle normally.
            // Walk backwards from lf_pos to check for any earlier \r that
            // isn't part of a CRLF.
            let (split_pos, term_len) =
                if !self.include_terminators && lf_pos > 0 && self.buf[lf_pos - 1] == b'\r' {
                    (lf_pos - 1, 2)
                } else {
                    (lf_pos, 1)
                };
            return Some(self.take_frame(split_pos, term_len));
        }

        // No \n found. Look for a bare \r.
        if let Some(cr_pos) = self.buf.iter().position(|&b| b == b'\r') {
            let next_is_lf = self.buf.get(cr_pos + 1) == Some(&b'\n');
            if next_is_lf {
                // CRLF: \n is in the buffer right after \r. This means \n was
                // found above. Unreachable in practice, but safe.
                return Some(self.take_frame(cr_pos, 2));
            }
            // \r found, no \n follows in the buffer.
            if cr_pos + 1 < self.buf.len() {
                // Bytes after \r exist in this chunk → bare CR confirmed immediately.
                // Emit the line before \r, drain through \r, transition to CrMode.
                let fb = self.take_frame(cr_pos, 1);
                if let DecoderMode::Line(ref mut state) = self.mode {
                    *state = LineState::CrMode;
                }
                return Some(fb);
            }
            // \r is the last byte in buffer → transition to PendingCr.
            if let DecoderMode::Line(ref mut state) = self.mode {
                *state = LineState::PendingCr(cr_pos);
            }
            return None;
        }

        // No \n, no \r. Wait for more data.
        None
    }

    /// `PendingCr` state: buffer has a `\r` at a known position. The next byte
    /// after that `\r` decides.
    ///
    /// On next non-`\n` byte → bare CR confirmed, emit frame before `\r`,
    /// drain through `\r`, promote to [`LineState::CrMode`].
    /// On `\n` → CRLF, emit frame before `\r\n`, drain through `\r\n`, return
    /// to [`LineState::AutoLf`].
    fn match_pending_cr(&mut self) -> Option<Vec<u8>> {
        let cr_pos = match self.mode {
            DecoderMode::Line(LineState::PendingCr(pos)) => pos,
            _ => {
                // Shouldn't happen. Reset to AutoLf.
                if let DecoderMode::Line(ref mut state) = self.mode {
                    *state = LineState::AutoLf;
                }
                return None;
            }
        };

        if cr_pos + 1 >= self.buf.len() {
            // \r is still the last byte. Wait for more data.
            return None;
        }

        let next_byte = self.buf[cr_pos + 1];
        if next_byte == b'\n' {
            // CRLF confirmed. Emit frame (strip \r\n unless include_terminators).
            let fb = self.take_frame(cr_pos, 2);
            if let DecoderMode::Line(ref mut state) = self.mode {
                *state = LineState::AutoLf;
            }
            return Some(fb);
        }

        // Non-\n byte after \r → bare CR confirmed.
        // Emit frame before \r, drain through \r, promote to CrMode.
        let fb = self.take_frame(cr_pos, 1);
        if let DecoderMode::Line(ref mut state) = self.mode {
            *state = LineState::CrMode;
        }
        Some(fb)
    }

    /// Match a line ending on a single byte: split on `b`, drain through it.
    fn match_line_byte(&mut self, b: u8) -> Option<Vec<u8>> {
        let pos = self.buf.iter().position(|&x| x == b)?;
        Some(self.take_frame(pos, 1))
    }

    /// Match a line with `crlf` ending: split on exact `\r\n`.
    fn match_line_crlf(&mut self) -> Option<Vec<u8>> {
        let pos = find_subsequence(&self.buf, b"\r\n")?;
        Some(self.take_frame(pos, 2))
    }

    fn frame_type_str(&self) -> &'static str {
        match &self.mode {
            DecoderMode::Line(_) => "line",
            DecoderMode::Delimiter(_) => "delimiter",
            DecoderMode::LengthPrefixed { .. } => "length_prefixed",
            DecoderMode::StartEnd { .. } => "start_end",
            DecoderMode::Slip { .. } => "slip",
            DecoderMode::Cobs { .. } => "cobs",
        }
    }

    /// Bytes pending in the incomplete frame buffer.
    pub fn pending_len(&self) -> usize {
        self.buf.len()
    }

    /// Flush any remaining bytes as a partial frame. For SLIP and COBS, this
    /// drains the in-frame buffer; pending escaped/partial state is emitted as
    /// raw bytes.
    pub fn flush_partial(&mut self) -> Option<Frame> {
        let (data, frame_type) = match &mut self.mode {
            DecoderMode::Slip {
                state: SlipState::InFrame { ref mut buf, .. },
            } => {
                if buf.is_empty() {
                    return None;
                }
                (std::mem::take(buf), "slip")
            }
            DecoderMode::Cobs {
                state:
                    CobsState::InFrame {
                        ref mut decoded, ..
                    },
            } => {
                if decoded.is_empty() {
                    return None;
                }
                (std::mem::take(decoded), "cobs")
            }
            _ => {
                if self.buf.is_empty() {
                    return None;
                }
                (std::mem::take(&mut self.buf), self.frame_type_str())
            }
        };
        self.frame_count += 1;
        Some(Frame {
            data,
            index: self.frame_count - 1,
            frame_type,
            parsed: None,
        })
    }
}

// ---- Utility ---------------------------------------------------------------

/// SLIP framing error: a malformed escape sequence was encountered during
/// RX decode. Construction errors (e.g. invalid delimiter) are synchronous
/// and configurable by the agent; runtime decode errors like this indicate
/// stream corruption and are not recoverable by retrying the same bytes.
#[derive(Debug, Clone)]
pub enum FrameDecodeError {
    /// SLIP `ESC` (0xDB) followed by an invalid byte (not `ESC_END` 0xDC
    /// or `ESC_ESC` 0xDD).
    SlipInvalidEscape(u8),
    /// COBS invalid code byte (impossible run length or truncated frame).
    CobsInvalidCode(u8),
    /// A protocol checksum (e.g. NMEA *XX XOR) did not match the recomputed value.
    /// `expected` is the recomputed checksum bytes; `received` is the frame's value.
    ChecksumMismatch {
        expected: Vec<u8>,
        received: Vec<u8>,
    },
}

impl std::fmt::Display for FrameDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameDecodeError::SlipInvalidEscape(b) => {
                write!(f, "SLIP framing error: invalid escape byte 0x{b:02X}")
            }
            FrameDecodeError::CobsInvalidCode(b) => {
                write!(f, "COBS framing error: invalid code byte 0x{b:02X}")
            }
            FrameDecodeError::ChecksumMismatch { expected, received } => {
                let exp = expected
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join("");
                let rcv = received
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect::<Vec<_>>()
                    .join("");
                write!(f, "checksum mismatch: expected {exp}, received {rcv}")
            }
        }
    }
}

impl std::error::Error for FrameDecodeError {}

/// Result of pushing a chunk through a [`FrameDecoder`].
///
/// Always carries the frames decoded before any error — stream-fatal errors
/// (SLIP malformed escape, COBS invalid code) no longer discard frames that
/// were successfully decoded from the same chunk. Per-frame checksum mismatches
/// are counted in `frames_dropped` and do NOT set `error`.
#[derive(Debug)]
pub struct PushOutcome {
    /// Frames successfully decoded from this chunk.
    pub frames: Vec<Frame>,
    /// Per-frame drops (currently only checksum mismatches with `validate: true`).
    /// Does NOT include frames skipped by `skip_empty` (those never consume
    /// an index by design).
    pub frames_dropped: usize,
    /// Stream-fatal error (SLIP malformed escape, COBS invalid code).
    /// `None` for per-frame checksum drops — those are counted in
    /// `frames_dropped` and the decoder keeps going.
    pub error: Option<FrameDecodeError>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checksums::lrc;
    use crate::framing::codecs::slip_stuff;
    use crate::framing::config::{
        preset_rx_framing, preset_rx_parser, Endianness, LineEnding, ParserConfig, ParserType,
        ProtocolPreset, RxFramingConfig, RxFramingMode, TxFramingMode,
    };
    use crate::match_config::PatternEncoding;

    // ── Line decoder (auto — original behavior) ──────────────────────────

    #[test]
    fn line_decoder_single_line() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[0].frame_type, "line");
    }

    #[test]
    fn line_decoder_crlf() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\nworld\n").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[1].data, b"world");
    }

    #[test]
    fn line_decoder_partial_across_chunks() {
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hel").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"lo\nwor").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
        let frames = dec.push(b"ld\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"world");
    }

    #[test]
    fn line_decoder_empty_lines() {
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\n\n\n").frames;
        assert_eq!(frames.len(), 3);
        for f in &frames {
            assert!(f.data.is_empty());
        }
    }

    #[test]
    fn line_decoder_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello\r\n");
    }

    // ── Line decoder: new ending modes ────────────────────────────────────

    #[test]
    fn line_decoder_lf_preserves_cr() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Lf,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello\r");
        assert_eq!(frames[0].frame_type, "line");
    }

    #[test]
    fn line_decoder_cr_splits_on_bare_cr() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Cr,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\rb\r").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
    }

    #[test]
    fn line_decoder_cr_with_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Cr,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\r").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a\r");
    }

    #[test]
    fn line_decoder_crlf_exact_only() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        // CRLF split across chunks: "\r" in first, "\n" in second.
        let frames = dec.push(b"a\r").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"\nb").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
    }

    #[test]
    fn line_decoder_crlf_no_split_on_bare_cr() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\rb\n").frames;
        assert_eq!(frames.len(), 0, "bare \\r should not split in crlf mode");
    }

    #[test]
    fn line_decoder_crlf_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Crlf,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\r\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello\r\n");
    }

    // ── Line decoder: auto promotion (bare-CR detection) ──────────────────

    fn auto_config() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        }
    }

    fn auto_config_include_terms() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            include_terminators: true,
            ..Default::default()
        }
    }

    #[test]
    fn auto_does_not_promote_on_crlf() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        let frames = dec.push(b"a\r\nb\r\n").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
        // After CRLF, decoder stays in AutoLf — next bare \r still triggers promotion.
        let frames = dec.push(b"c\r").frames;
        assert!(frames.is_empty(), "pending CR after CRLF");
        // Push "d" — confirmation byte stays buffered as start of next line.
        let frames = dec.push(b"d").frames;
        assert_eq!(frames.len(), 1, "bare CR confirmed on next non-\\n byte");
        assert_eq!(frames[0].data, b"c");
        // Now in CrMode. Buffer has "d". Push "e\r" → "de\r" → frame "de".
        let frames = dec.push(b"e\r").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"de");
    }

    #[test]
    fn auto_does_not_promote_on_lf() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        let frames = dec.push(b"a\nb\n").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
    }

    #[test]
    fn auto_promotes_on_next_non_lf_byte() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Push "line1\r" → \r at end, no frame emitted, enters PendingCr.
        let frames = dec.push(b"line1\r").frames;
        assert!(frames.is_empty());
        // Push "x" → non-\n byte confirms bare CR. Emit "line1", enter CrMode.
        // The "x" stays buffered as the start of the next line.
        let frames = dec.push(b"x").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"line1");
        // In CrMode: push "more\r" → split on \r. Buffer had "x" + "more\r".
        let frames = dec.push(b"more\r").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"xmore");
    }

    #[test]
    fn auto_crlf_after_pending_cr_cancels_promotion() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Push "a\r" → pending CR.
        let frames = dec.push(b"a\r").frames;
        assert!(frames.is_empty());
        // Push "\nb" → \n arrives, CRLF recognized. "b" stays buffered.
        let frames = dec.push(b"\nb").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
        // Back to AutoLf. Buffer has "b". Push "c\n" → "bc\n" → frame "bc".
        let frames = dec.push(b"c\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"bc");
    }

    #[test]
    fn auto_flush_partial_emits_pending_cr() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        let frames = dec.push(b"tail\r").frames;
        assert!(frames.is_empty());
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.data, b"tail\r");
        assert_eq!(partial.frame_type, "line");
    }

    #[test]
    fn auto_flush_partial_emits_pending_cr_include_terminators() {
        let mut dec = FrameDecoder::new(&auto_config_include_terms(), None).unwrap();
        let frames = dec.push(b"tail\r").frames;
        assert!(frames.is_empty());
        let partial = dec.flush_partial().expect("partial frame");
        // include_terminators=true → the \r is included (already in buffer).
        assert_eq!(partial.data, b"tail\r");
    }

    #[test]
    fn auto_promotes_and_stays_in_cr_mode() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Promote to CrMode.
        dec.push(b"a\r");
        dec.push(b"b");
        // In CrMode: \n is NOT a terminator, \r is.
        // Buffer has "b" from confirmation, then "x\ny\r" → "bx\ny\r" → split on \r.
        let frames = dec.push(b"x\ny\r").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"bx\ny");
    }

    #[test]
    fn auto_promotion_include_terminators() {
        let mut dec = FrameDecoder::new(&auto_config_include_terms(), None).unwrap();
        let frames = dec.push(b"line1\r").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"x").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"line1\r");
    }

    #[test]
    fn auto_pending_cr_then_flush_keeps_frame_index_monotonic() {
        let mut dec = FrameDecoder::new(&auto_config(), None).unwrap();
        // Two LF lines.
        let frames = dec.push(b"a\nb\n").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[1].index, 1);
        // Pending CR, then flush.
        dec.push(b"c\r");
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.index, 2);
    }

    fn slip_rx_config() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Slip,
            ..Default::default()
        }
    }

    #[test]
    fn rx_slip_skips_to_first_end() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"junk\xC0hi\xC0").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");
    }

    #[test]
    fn rx_slip_decodes_basic_frame() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0hello\xC0").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn rx_slip_decodes_esc_end() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0\xDB\xDC\xC0").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"\xC0");
    }

    #[test]
    fn rx_slip_decodes_esc_esc() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0\xDB\xDD\xC0").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"\xDB");
    }

    #[test]
    fn rx_slip_malformed_escape_returns_err() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let result = dec.push(b"\xC0\xDB\x41\xC0");
        assert!(result.error.is_some(), "expected decode error");
        match result.error {
            Some(FrameDecodeError::SlipInvalidEscape(b)) => assert_eq!(b, 0x41),
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[test]
    fn rx_slip_resyncs_after_malformed_escape() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        // Malformed escape.
        let result = dec.push(b"\xC0\xDB\x41\xC0");
        assert!(result.error.is_some());
        // After resync, decoder is in BeforeFirstEnd. The trailing END from
        // the malformed chunk remains in buf_outer. Push a valid frame —
        // two consecutive ENDs produce one empty frame then "ok".
        let frames = dec.push(b"\xC0ok\xC0").frames;
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert_eq!(frames[1].data, b"ok");
    }

    #[test]
    fn rx_slip_resync_clears_stale_in_progress_buf() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        // Partial frame "hello", then malformed escape.
        let result = dec.push(b"\xC0hello\xDB\x41");
        assert!(result.error.is_some());
        // After resync, "hello" must be cleared. Push a new frame.
        let frames = dec.push(b"\xC0world\xC0").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"world");
    }

    #[test]
    fn push_survives_frames_decoded_before_slip_invalid_escape() {
        // Two good SLIP frames, then a malformed escape → 2 frames survive.
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        // Build: C0 aa C0 bb C0 DB 41 (two valid frames, then malformed).
        let mut chunk = vec![SLIP_END];
        chunk.extend_from_slice(b"aa");
        chunk.push(SLIP_END);
        // One SLIP_END separates frames — no empty frame between them.
        chunk.extend_from_slice(b"bb");
        chunk.push(SLIP_END);
        // Now append a malformed escape: ESC followed by invalid byte.
        chunk.push(SLIP_ESC);
        chunk.push(0x41); // Invalid escape byte
        let result = dec.push(&chunk);
        assert_eq!(result.frames.len(), 2, "frames before error should survive");
        assert_eq!(result.frames[0].data, b"aa");
        assert_eq!(result.frames[1].data, b"bb");
        assert_eq!(result.frames_dropped, 0);
        assert!(matches!(
            result.error,
            Some(FrameDecodeError::SlipInvalidEscape(0x41))
        ));
    }

    #[test]
    fn rx_slip_two_frames_in_one_chunk() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0aa\xC0bb\xC0").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"aa");
        assert_eq!(frames[1].data, b"bb");
    }

    #[test]
    fn rx_slip_cross_chunk_frame() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0hel").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"lo\xC0").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn rx_slip_truncated_escape_holds_pending() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0\xDB").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"\xDC\xC0").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"\xC0");
    }

    #[test]
    fn rx_slip_flush_partial_emits_pending() {
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(b"\xC0hel").frames;
        assert!(frames.is_empty());
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.data, b"hel");
        assert_eq!(partial.frame_type, "slip");
    }

    #[test]
    fn roundtrip_slip_arbitrary_binary() {
        let payload: &[u8] = &[SLIP_END, SLIP_ESC, 0x41, SLIP_ESC, SLIP_ESC_END, SLIP_END];
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_slip_empty_payload() {
        let mode = TxFramingMode::Slip;
        let framed = mode.encode(b"").unwrap();
        assert_eq!(framed, &[SLIP_END, SLIP_END]);
        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.is_empty());
    }

    #[test]
    fn rx_slip_decodes_large_payload_preserves_bytes() {
        // Push a 4096-byte SLIP frame with a known repeating pattern
        // (0x00..=0xFF cycling) stuffed via slip_stuff, wrapped in END
        // markers. Assert exactly one decoded frame whose payload matches
        // the original byte-for-byte, and frame_type == "slip".
        let payload: Vec<u8> = (0..4096).map(|i| (i % 256) as u8).collect();
        let stuffed = slip_stuff(&payload);
        let mut framed = vec![SLIP_END];
        framed.extend_from_slice(&stuffed);
        framed.push(SLIP_END);

        let mut dec = FrameDecoder::new(&slip_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1, "expected exactly one frame");
        assert_eq!(
            frames[0].data, payload,
            "decoded payload must match original"
        );
        assert_eq!(frames[0].frame_type, "slip");
    }

    #[test]
    fn slip_parser_error_propagates_and_resets_state() {
        // SLIP-frame a NMEA sentence with a BAD checksum. The NmeaParser with
        // validate:true returns Err(ChecksumMismatch) when it sees the bad *XX.
        // This exercises slip_decode's parser-Err path (src/framing/decoder.rs):
        // drain consumed bytes, reset state to BeforeFirstEnd, return Err.
        // Then push a well-formed SLIP-framed NMEA sentence and confirm the
        // decoder recovers (state was reset — no stale escaped/buf corruption).
        use crate::checksums::xor_checksum;

        let bad_body = b"GPGLL,3751.65,N,12226.54,W*00"; // wrong checksum (correct is 7E)
        let good_body = b"GPGLL,3751.65,N,12226.54,W";
        let good_cs = xor_checksum(good_body);
        let good_sentence = format!("GPGLL,3751.65,N,12226.54,W*{good_cs:02X}");

        let framing = RxFramingConfig {
            mode: RxFramingMode::Slip,
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&framing, Some(&parser)).unwrap();

        // Push 1: bad-checksum frame → dropped (frames_dropped=1, no error).
        let mut push1 = vec![SLIP_END];
        push1.extend_from_slice(bad_body);
        push1.push(SLIP_END);
        let result = dec.push(&push1);
        assert!(
            result.frames.is_empty(),
            "bad-checksum frame should be dropped, not emitted"
        );
        assert_eq!(result.frames_dropped, 1, "checksum drop should be counted");
        assert!(result.error.is_none(), "checksum drop should not set error");

        // Push 2: good-checksum frame → decoder was NOT reset by the checksum
        // drop (state stayed InFrame with empty buf). The leading SLIP_END
        // produces an empty frame; the good sentence produces a parsed Nmea frame.
        let mut push2 = vec![SLIP_END];
        push2.extend_from_slice(good_sentence.as_bytes());
        push2.push(SLIP_END);
        let result2 = dec.push(&push2);
        // First frame is empty (leading SLIP_END with empty buf), second has our NMEA.
        assert!(
            result2.frames.len() >= 2,
            "expected at least 2 frames, got {}",
            result2.frames.len()
        );
        // Find the Nmea frame (skip the empty one).
        let nmea_frame = result2
            .frames
            .iter()
            .find(|f| matches!(f.parsed, Some(ParsedFrame::Nmea { .. })))
            .expect("expected a clean Nmea frame after recovery");
        assert!(
            matches!(
                &nmea_frame.parsed,
                Some(ParsedFrame::Nmea {
                    checksum_valid: Some(true),
                    ..
                })
            ),
            "expected a clean Nmea frame after recovery, got {:?}",
            nmea_frame.parsed
        );
    }

    // ── Delimiter decoder ───────────────────────────────────────────────

    #[test]
    fn delimiter_decoder_basic() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a|b|c|").frames;
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");
        assert_eq!(frames[2].data, b"c");
    }

    #[test]
    fn delimiter_decoder_multi_byte() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "AA".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"xAAyAAz").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"x");
        assert_eq!(frames[1].data, b"y");
    }

    #[test]
    fn delimiter_decoder_partial_delimiter() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "AB".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"xA").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"By").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"x");
    }

    // ── Length-prefixed decoder ─────────────────────────────────────────

    #[test]
    fn length_prefixed_basic() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x05hello\x02wo\x02rb").frames;
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"hello");
        assert_eq!(frames[1].data, b"wo");
        assert_eq!(frames[2].data, b"rb");
    }

    #[test]
    fn length_prefixed_u16_big_endian() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let mut buf = vec![0x00, 0x05];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    // ── Start/end marker decoder ────────────────────────────────────────

    #[test]
    fn start_end_basic() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["STX".into()],
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"noiseSTXdataETXjunkSTXmoreETX").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"data");
        assert_eq!(frames[1].data, b"more");
    }

    #[test]
    fn start_end_include_markers() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["<".into()],
                end: ">".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: true,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"<data>").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"<data>");
    }

    #[test]
    fn combined_line_at_parser() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::AtCommand,
            custom_prompt: None,
            validate: false,
        };
        let mut dec = FrameDecoder::new(&config, Some(&parser)).unwrap();
        let frames = dec.push(b"OK\nERROR\n+CGREG: 0,1\n").frames;
        assert_eq!(frames.len(), 3);
        assert!(
            matches!(frames[0].parsed, Some(ParsedFrame::AtCommand { ref status, .. }) if status.as_deref() == Some("OK"))
        );
        assert!(
            matches!(frames[1].parsed, Some(ParsedFrame::AtCommand { ref status, .. }) if status.as_deref() == Some("ERROR"))
        );
        assert!(
            matches!(frames[2].parsed, Some(ParsedFrame::AtCommand { ref command, .. }) if command.as_deref() == Some("CGREG"))
        );
    }

    // ── Negative / edge-case tests ───────────────────────────────────────

    #[test]
    fn line_decoder_no_terminator_then_flush() {
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello").frames;
        assert!(frames.is_empty());
        assert_eq!(dec.pending_len(), 5);
        let partial = dec.flush_partial().expect("partial frame");
        assert_eq!(partial.data, b"hello");
        assert_eq!(partial.index, 0);
        assert_eq!(partial.frame_type, "line");
        assert!(partial.parsed.is_none());
    }

    #[test]
    fn delimiter_decoder_empty_rejected() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("empty delimiter should be rejected"),
            Err(err) => assert!(err.contains("Delimiter must not be empty"), "got: {err}"),
        }
    }

    #[test]
    fn length_prefixed_zero_payload() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x00\x05hello").frames;
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert_eq!(frames[1].data, b"hello");
    }

    #[test]
    fn length_prefixed_incomplete_payload() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x0aABC").frames;
        assert!(frames.is_empty());
        assert!(dec.pending_len() >= 3);
    }

    #[test]
    fn length_prefixed_invalid_prefix_size() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 3,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("prefix_size=3 should be rejected"),
            Err(err) => assert!(err.contains("prefix_size must be 1, 2, or 4"), "got: {err}"),
        }
    }

    #[test]
    fn length_prefixed_u32_big_endian() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 4,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let mut buf = vec![0x00, 0x00, 0x00, 0x05];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn length_prefixed_u32_little_endian() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 4,
                endianness: Endianness::Little,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let mut buf = vec![0x05, 0x00, 0x00, 0x00];
        buf.extend_from_slice(b"hello");
        let frames = dec.push(&buf).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn start_end_no_start_marker() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["STX".into()],
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"noise_without_markers").frames;
        assert!(frames.is_empty());
        assert!(dec.pending_len() <= 2);
    }

    #[test]
    fn start_end_start_no_end_then_flush() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["<".into()],
                end: ">".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"<data_without_end").frames;
        assert!(frames.is_empty(), "no end marker yet");
        let partial = dec.flush_partial().expect("partial frame after flush");
        assert_eq!(partial.data, b"data_without_end");
    }

    #[test]
    fn start_end_empty_markers_rejected() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["".into()],
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("empty markers should be rejected"),
            Err(err) => assert!(
                err.contains("Start and end markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    #[test]
    fn start_end_start_split_across_chunks() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["ABC".into()],
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"AB").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"CdX").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"d");
    }

    #[test]
    fn delimiter_invalid_encoding_rejected() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "!!!".into(),
                delimiter_encoding: crate::match_config::PatternEncoding::Base64,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("expected error for invalid delimiter encoding"),
            Err(err) => assert!(err.contains("Invalid delimiter encoding"), "got: {err}"),
        }
    }

    #[test]
    fn start_end_invalid_encoding_rejected() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["!!!".into()],
                end: "X".into(),
                marker_encoding: crate::match_config::PatternEncoding::Base64,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("expected error for invalid marker encoding"),
            Err(err) => assert!(err.contains("Invalid start marker encoding"), "got: {err}"),
        }
    }

    #[test]
    fn max_frames_zero_edge() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: Some(0),
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"hello\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn length_prefixed_initial_offset() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 1,
                endianness: Endianness::Big,
                initial_offset: Some(4),
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"XXXX\x05hello").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn delimiter_include_terminators() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            include_terminators: true,
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a|b|").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a|");
        assert_eq!(frames[1].data, b"b|");
    }

    #[test]
    fn flush_partial_empty_buffer() {
        let config = RxFramingConfig::default();
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        assert!(dec.flush_partial().is_none(), "empty buf => no frame");
    }

    #[test]
    fn combined_line_json_parser() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::JsonLines,
            custom_prompt: None,
            validate: false,
        };
        let mut dec = FrameDecoder::new(&config, Some(&parser)).unwrap();
        let frames = dec.push(b"{\"a\":1}\n{\"b\":2}\n").frames;
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0].parsed, Some(ParsedFrame::Json(_))));
        assert!(matches!(frames[1].parsed, Some(ParsedFrame::Json(_))));
    }

    // ── Coverage gap tests ──────────────────────────────────────────────

    #[test]
    fn delimiter_decoder_empty_segments() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"||").frames;
        assert_eq!(frames.len(), 2);
        assert!(frames[0].data.is_empty());
        assert!(frames[1].data.is_empty());

        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a||b|").frames;
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[0].data, b"a");
        assert!(frames[1].data.is_empty());
        assert_eq!(frames[2].data, b"b");
    }

    #[test]
    fn length_prefixed_prefix_split_across_chunks() {
        let config = RxFramingConfig {
            mode: RxFramingMode::LengthPrefixed {
                prefix_size: 2,
                endianness: Endianness::Big,
                initial_offset: None,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\x00").frames;
        assert!(frames.is_empty());
        let frames = dec.push(b"\x05hello").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn start_end_end_marker_split_across_chunks() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["STX".into()],
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"STXdataET").frames;
        assert!(frames.is_empty(), "end marker ETX not yet complete");
        let frames = dec.push(b"X").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"data");
    }

    // ── COBS framing tests ───────────────────────────────────────────────

    fn cobs_rx_config() -> RxFramingConfig {
        RxFramingConfig {
            mode: RxFramingMode::Cobs,
            ..Default::default()
        }
    }

    #[test]
    fn rx_cobs_skips_to_first_delimiter() {
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Prepend junk to a valid COBS-encoded frame.
        let mode = TxFramingMode::Cobs;
        let good_frame = mode.encode(b"hi").unwrap();
        let mut input = b"junk".to_vec();
        input.extend_from_slice(&good_frame);
        let frames = dec.push(&input).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");
    }

    #[test]
    fn rx_cobs_decodes_basic_frame() {
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"hello").unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn rx_cobs_decodes_delimiter_byte_in_payload() {
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Payload: [0x41, 0x00, 0x42]. Encoded: code 0x02 (dist to first 0x00
        // is 2: 1 data byte 'A' + the zero), then 'A', then code 0x02 (dist
        // to phantom zero: 'B' + phantom = 2), then 'B'.
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&[0x41, 0x00, 0x42]).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, &[0x41, 0x00, 0x42]);
    }

    #[test]
    fn roundtrip_cobs_arbitrary_binary() {
        let payload: &[u8] = &[0x00, 0xFF, 0x41, 0x00, 0x00, 0xFF, 0x7E];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_empty_payload() {
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"").unwrap();
        // Empty payload: [delim][code 0x01][delim] → [0x00, 0x01, 0x00].
        assert_eq!(framed, &[0x00, 0x01, 0x00]);
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.is_empty());
    }

    #[test]
    fn roundtrip_cobs_max_overhead() {
        // 254-byte all-zero payload (all delimiter bytes). This is the
        // worst-case overhead: every input byte is a delimiter, so each
        // code block encodes 0 data bytes with code 0x01.
        let payload = vec![0x00u8; 254];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_long_zero_run_300() {
        // 300 zero bytes: exercises multiple COBS code blocks and the
        // phantom-zero reinsertion. Regression guard for the 0x7E bug class
        // (the bug inserted the delimiter as the phantom; with 0x00 the
        // phantom is 0x00 and this round-trips correctly).
        let payload = vec![0u8; 300];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn rx_cobs_consecutive_delimiters_emit_empty_frame() {
        // With the canonical plain-COBS decoder, every byte value 0x01-0xFF
        // is a valid code and 0x00 is the frame delimiter, so CobsInvalidCode
        // is unreachable in practice. This test verifies that consecutive
        // delimiter bytes are handled correctly: the first is consumed as a
        // leading delimiter, the second terminates the (empty) frame.
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Two delimiter bytes: first consumed by BeforeFirstDelim, second
        // emits an empty frame.
        let frames = dec.push(b"\x00\x00").frames;
        assert_eq!(
            frames.len(),
            1,
            "2 consecutive delimiters yield 1 empty frame"
        );
        assert!(frames[0].data.is_empty());
    }

    #[test]
    fn push_survives_frames_decoded_before_cobs_invalid_code() {
        // COBS decoder does not currently produce CobsInvalidCode errors
        // (all code bytes are valid). Verify PushOutcome structure:
        // good COBS frames decode without error.
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"hello").unwrap();
        let result = dec.push(&framed);
        assert_eq!(result.frames.len(), 1, "should decode COBS frame");
        assert_eq!(result.frames[0].data, b"hello");
        assert!(result.error.is_none(), "no error");
        assert_eq!(result.frames_dropped, 0);
    }

    #[test]
    fn rx_cobs_handles_between_frame_delimiters() {
        // After emitting a frame on a delimiter, the decoder transitions to
        // BeforeFirstDelim and skips any additional delimiters until the next
        // frame's stuffed block. This test verifies that a stream of
        // delimiters + a valid frame + more delimiters is handled correctly.
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        // Two consecutive delimiter bytes → one empty frame, then sync for
        // next frame.
        let frames = dec.push(b"\x00\x00").frames;
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.is_empty());
        // Subsequent valid frame decodes correctly.
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(b"ok").unwrap();
        let frames = dec.push(&framed).frames;
        assert!(!frames.is_empty());
        // The last frame should be "ok".
        assert_eq!(frames.last().unwrap().data, b"ok");
    }

    #[test]
    fn cobs_parser_error_propagates_and_resets_state() {
        // COBS-frame a NMEA sentence with a BAD checksum. The NmeaParser with
        // validate:true returns Err(ChecksumMismatch) when it sees the bad *XX.
        // This exercises cobs_decode's parser-Err path (src/framing/decoder.rs):
        // drain consumed bytes, clear decoded/remaining/pending_zero, reset
        // state to BeforeFirstDelim, return Err.
        // Then push a well-formed COBS-framed NMEA sentence and confirm the
        // decoder recovers (state was reset — no stale decoded/remaining/
        // pending_zero corruption).
        use crate::checksums::xor_checksum;

        let bad_body = b"GPGLL,3751.65,N,12226.54,W*00"; // wrong checksum (correct is 7E)
        let good_body = b"GPGLL,3751.65,N,12226.54,W";
        let good_cs = xor_checksum(good_body);
        let good_sentence = format!("GPGLL,3751.65,N,12226.54,W*{good_cs:02X}");

        // COBS-encode each sentence via TxFramingMode::Cobs (which produces
        // [0x00][cobs-stuffed block][0x00]).
        let bad_framed = TxFramingMode::Cobs.encode(bad_body).unwrap();
        let good_framed = TxFramingMode::Cobs
            .encode(good_sentence.as_bytes())
            .unwrap();

        let framing = RxFramingConfig {
            mode: RxFramingMode::Cobs,
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: true,
        };
        let mut dec = FrameDecoder::new(&framing, Some(&parser)).unwrap();

        // Push 1: bad-checksum frame → dropped (frames_dropped=1, no error).
        let result = dec.push(&bad_framed);
        assert!(
            result.frames.is_empty(),
            "bad-checksum frame should be dropped, not emitted"
        );
        assert_eq!(result.frames_dropped, 1, "checksum drop should be counted");
        assert!(result.error.is_none(), "checksum drop should not set error");

        // Push 2: good-checksum frame → one clean Nmea frame (state was reset
        // to BeforeFirstDelim by the error path; no stale decoded/remaining/
        // pending_zero corruption).
        let frames = dec.push(&good_framed).frames;
        assert_eq!(
            frames.len(),
            1,
            "decoder must recover after the parser error"
        );
        assert!(
            matches!(
                &frames[0].parsed,
                Some(ParsedFrame::Nmea {
                    checksum_valid: Some(true),
                    ..
                })
            ),
            "expected a clean Nmea frame after recovery, got {:?}",
            frames[0].parsed
        );
    }

    #[test]
    fn roundtrip_cobs_255_ones() {
        let payload = vec![1u8; 255];
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_254_ones_then_zero() {
        // Regression: 254 non-zero bytes then a trailing zero. This crossed
        // the continuation boundary and dropped the trailing zero in the
        // pre-canonical implementation.
        let mut payload = vec![1u8; 254];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_255_ones_then_zero() {
        let mut payload = vec![1u8; 255];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_256_ones_then_zero() {
        // Crosses two continuation blocks.
        let mut payload = vec![1u8; 256];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_510_ones_then_zero() {
        // Two full continuation blocks then a zero.
        let mut payload = vec![1u8; 510];
        payload.push(0);
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_alternating_ones_zeros_300() {
        // 300 bytes alternating 0x01 / 0x00 — heavy zero density across the
        // continuation boundary.
        let payload: Vec<u8> = (0..300).map(|i| if i % 2 == 0 { 1 } else { 0 }).collect();
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    #[test]
    fn roundtrip_cobs_all_byte_values_256() {
        // Every byte value 0x00-0xFF once. Exercises embedded 0x00 and 0xFF
        // (which is a non-zero data byte, NOT a code, when it appears in the
        // data portion).
        let payload: Vec<u8> = (0..=255u8).collect();
        let mode = TxFramingMode::Cobs;
        let framed = mode.encode(&payload).unwrap();
        let mut dec = FrameDecoder::new(&cobs_rx_config(), None).unwrap();
        let frames = dec.push(&framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, payload);
    }

    // ── StartEnd multi-marker tests ─────────────────────────────────────

    #[test]
    fn start_end_matches_either_of_multiple_start_markers() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$".into(), "!".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        // Input with $ marker
        let frames = dec.push(b"junk$hi\r\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");
        // Input with ! marker
        let frames = dec.push(b"junk!ok\r\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"ok");
    }

    #[test]
    fn start_end_empty_start_vec_rejected_at_construction() {
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec![],
                end: "X".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        match FrameDecoder::new(&config, None) {
            Ok(_) => panic!("empty start vec should be rejected"),
            Err(err) => assert!(
                err.contains("Start markers must not be empty"),
                "got: {err}"
            ),
        }
    }

    #[test]
    fn start_end_existing_single_marker_still_works() {
        // Re-test start_end_basic but with start wrapped in vec![].
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["STX".into()],
                end: "ETX".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"noiseSTXdataETXjunkSTXmoreETX").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"data");
        assert_eq!(frames[1].data, b"more");
    }

    // ── skip_empty framing option ──────────────────────────────────────────

    #[test]
    fn skip_empty_drops_empty_line_frames() {
        // Line framing (auto), input b"a\n\nb\n", skip_empty: true → 2 frames.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\n\nb\n").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");

        // skip_empty: false → 3 frames (a, empty, b).
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"a\n\nb\n").frames;
        assert_eq!(frames_off.len(), 3);
        assert_eq!(frames_off[0].data, b"a");
        assert!(frames_off[1].data.is_empty());
        assert_eq!(frames_off[2].data, b"b");
    }

    #[test]
    fn skip_empty_drops_whitespace_only_frames() {
        let input = b"a\n   \nb\n";
        // skip_empty: true → middle line (3 spaces) skipped.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(input).frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");

        // skip_empty: false → 3 frames.
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(input).frames;
        assert_eq!(frames_off.len(), 3);
        assert_eq!(frames_off[0].data, b"a");
        assert_eq!(frames_off[1].data, b"   ");
        assert_eq!(frames_off[2].data, b"b");
    }

    #[test]
    fn skip_empty_drops_leading_and_trailing_blank_lines() {
        // Input has leading, inner, and trailing blank lines.
        let input = b"\n{\"k\":1}\n\n{\"k\":2}\n\n";
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let parser = ParserConfig {
            parser_type: ParserType::JsonLines,
            custom_prompt: None,
            validate: false,
        };
        let mut dec = FrameDecoder::new(&config, Some(&parser)).unwrap();
        let frames = dec.push(input).frames;
        // skip_empty: true → 2 Json frames, blank lines skipped.
        assert_eq!(frames.len(), 2);
        assert!(matches!(frames[0].parsed, Some(ParsedFrame::Json(_))));
        assert!(matches!(frames[1].parsed, Some(ParsedFrame::Json(_))));
    }

    #[test]
    fn skip_empty_does_not_count_skipped_frames_toward_max_frames() {
        // line framing, input b"\na\n\nb\n", skip_empty: true →
        // the empty lines do not appear in push output and do not increment
        // frame_count. Only the 2 non-blank frames (a, b) are emitted.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"\na\n\nb\n").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[1].data, b"b");
        assert_eq!(frames[1].index, 1);
        // With skip_empty: false, the empty lines appear and consume
        // indices: frame 0 empty (leading \n), frame 1 'a', frame 2 empty
        // (inner \n), frame 3 'b'.
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"\na\n\nb\n").frames;
        assert_eq!(frames_off.len(), 4);
        assert!(frames_off[0].data.is_empty());
        assert_eq!(frames_off[0].index, 0);
        assert_eq!(frames_off[1].data, b"a");
        assert_eq!(frames_off[1].index, 1);
        assert!(frames_off[2].data.is_empty());
        assert_eq!(frames_off[2].index, 2);
        assert_eq!(frames_off[3].data, b"b");
        assert_eq!(frames_off[3].index, 3);
    }

    #[test]
    fn skip_empty_preserves_frame_index_continuity() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"a\n\nb\n").frames;
        assert_eq!(frames.len(), 2);
        // The skipped blank line does NOT consume an index.
        assert_eq!(frames[0].index, 0);
        assert_eq!(frames[1].index, 1);
    }

    #[test]
    fn push_frame_index_stays_contiguous_across_checksum_drop() {
        // Feed two good NMEA sentences with a bad-checksum sentence between them.
        // The surviving frames must have contiguous indices (0 and 1, no gap at 2).
        let rx_config = preset_rx_framing(ProtocolPreset::Nmea0183);
        let parser_config = preset_rx_parser(ProtocolPreset::Nmea0183);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();

        let good = b"$GPGLL,3751.65,N,12226.54,W*7E\r\n";
        let bad = b"$GPGLL,3751.65,N,12226.54,W*00\r\n";
        let mut chunk = Vec::new();
        chunk.extend_from_slice(good);
        chunk.extend_from_slice(bad);
        chunk.extend_from_slice(good);

        let result = dec.push(&chunk);
        assert_eq!(result.frames.len(), 2, "two good frames should survive");
        // Frame indices must be contiguous — the dropped frame consumes no index.
        assert_eq!(result.frames[0].index, 0);
        assert_eq!(result.frames[1].index, 1);
        assert_eq!(result.frames_dropped, 1);
    }

    #[test]
    fn skip_empty_applies_to_slip() {
        // Two consecutive END bytes produce one empty frame, then 'a'.
        // skip_empty: true → empty skipped, only 'a' emitted.
        let framed: &[u8] = &[SLIP_END, SLIP_END, b'a', SLIP_END];
        let config = RxFramingConfig {
            mode: RxFramingMode::Slip,
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[0].index, 0);
    }

    #[test]
    fn skip_empty_applies_to_cobs() {
        // Leading 0x00 sync, then back-to-back 0x00 delimiters for an empty
        // frame, then another 0x00 sync + COBS-stuffed 'a' + 0x00 delimiter.
        // skip_empty: true → empty skipped, only 'a' emitted.
        let framed: &[u8] = &[0x00, 0x00, 0x00, 0x02, 0x61, 0x00];
        let config = RxFramingConfig {
            mode: RxFramingMode::Cobs,
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(framed).frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[0].index, 0);
    }

    #[test]
    fn flush_partial_does_not_apply_skip_empty() {
        // Line framing, skip_empty: true. Push partial content, then flush.
        // The partial frame is emitted even though it's a single character
        // (not blank), so this test verifies flush_partial emits frames
        // regardless of skip_empty. The bypass is structural: flush_partial
        // does not check skip_empty at all.
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        // Push "b" without a newline — no frame emitted by push.
        let frames = dec.push(b"b").frames;
        assert!(frames.is_empty());
        // flush_partial emits the pending content regardless of skip_empty.
        let partial = dec.flush_partial();
        assert!(partial.is_some());
        assert_eq!(partial.unwrap().data, b"b");
    }

    // ── Checksum-failure-surfacing via push ────────────────────────────

    #[test]
    fn nmea_checksum_failure_drops_frame_and_counts() {
        // Build a FrameDecoder with nmea0183 preset framing + parser.
        let rx_config = preset_rx_framing(ProtocolPreset::Nmea0183);
        let parser_config = preset_rx_parser(ProtocolPreset::Nmea0183);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        // Push a full NMEA sentence with bad checksum ($...*00\r\n)
        let result = dec.push(b"$GPGLL,3751.65,N,12226.54,W*00\r\n");
        assert!(
            result.frames.is_empty(),
            "bad-checksum frame should be dropped"
        );
        assert_eq!(result.frames_dropped, 1, "checksum drop should be counted");
        assert!(
            result.error.is_none(),
            "checksum drop should not be an error"
        );
    }

    #[test]
    fn push_checksum_mismatch_keeps_other_frames_in_chunk() {
        // One chunk: good bad good NMEA sentences → 2 good frames, 1 dropped.
        let rx_config = preset_rx_framing(ProtocolPreset::Nmea0183);
        let parser_config = preset_rx_parser(ProtocolPreset::Nmea0183);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();

        let good = b"$GPGLL,3751.65,N,12226.54,W*7E\r\n";
        let bad = b"$GPGLL,3751.65,N,12226.54,W*00\r\n";
        let mut chunk = Vec::new();
        chunk.extend_from_slice(good);
        chunk.extend_from_slice(bad);
        chunk.extend_from_slice(good);

        let result = dec.push(&chunk);
        assert_eq!(result.frames.len(), 2, "two good frames should survive");
        assert_eq!(result.frames[0].index, 0);
        assert_eq!(result.frames[1].index, 1);
        assert_eq!(result.frames_dropped, 1, "one bad frame dropped");
        assert!(result.error.is_none(), "no stream-fatal error");
    }

    #[test]
    fn nmea_valid_sentence_decodes_to_frame_with_parsed_nmea() {
        // GLL with correct checksum *7E
        let rx_config = preset_rx_framing(ProtocolPreset::Nmea0183);
        let parser_config = preset_rx_parser(ProtocolPreset::Nmea0183);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        let frames = dec.push(b"$GPGLL,3751.65,N,12226.54,W*7E\r\n").frames;
        assert_eq!(frames.len(), 1);
        match &frames[0].parsed {
            Some(ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            }) => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(
                    fields,
                    &vec![
                        "3751.65".to_string(),
                        "N".to_string(),
                        "12226.54".to_string(),
                        "W".to_string()
                    ]
                );
                assert_eq!(*checksum_valid, Some(true));
            }
            other => panic!("expected Nmea parsed frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_checksum_failure_drops_frame_and_counts() {
        let rx_config = preset_rx_framing(ProtocolPreset::ModbusAscii);
        let parser_config = preset_rx_parser(ProtocolPreset::ModbusAscii);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        // Push a full frame with bad LRC: :01030000000100\r\n  (correct LRC is FB, here 00)
        let result = dec.push(b":01030000000100\r\n");
        assert!(result.frames.is_empty(), "bad-LRC frame should be dropped");
        assert_eq!(result.frames_dropped, 1, "LRC drop should be counted");
        assert!(result.error.is_none(), "LRC drop should not be an error");
    }

    #[test]
    fn modbus_ascii_valid_frame_decodes_to_frame_with_parsed_modbus_ascii() {
        let rx_config = preset_rx_framing(ProtocolPreset::ModbusAscii);
        let parser_config = preset_rx_parser(ProtocolPreset::ModbusAscii);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        // :010300000001FB\r\n  (valid read holding registers)
        let frames = dec.push(b":010300000001FB\r\n").frames;
        assert_eq!(frames.len(), 1);
        match &frames[0].parsed {
            Some(ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                data,
                checksum_valid: Some(true),
            }) => {
                assert_eq!(data, &vec![0, 0, 0, 1]);
            }
            other => panic!("expected ModbusAscii parsed frame, got {other:?}"),
        }
    }

    // ── Medium-risk gap tests (P3d) ───────────────────────────────────────

    #[test]
    fn skip_empty_applies_to_delimiter() {
        // Delimiter mode + skip_empty: empty frames between consecutive
        // delimiters are dropped. Mirror skip_empty_drops_empty_line_frames.
        let config_on = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config_on, None).unwrap();
        let frames = dec.push(b"a||b|").frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].data, b"a");
        assert_eq!(frames[1].data, b"b");

        // skip_empty: false → 3 frames (a, empty, b).
        let config_off = RxFramingConfig {
            mode: RxFramingMode::Delimiter {
                delimiter: "|".into(),
                delimiter_encoding: PatternEncoding::Utf8,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"a||b|").frames;
        assert_eq!(frames_off.len(), 3);
        assert_eq!(frames_off[0].data, b"a");
        assert!(frames_off[1].data.is_empty());
        assert_eq!(frames_off[2].data, b"b");
    }

    #[test]
    fn skip_empty_applies_to_start_end() {
        // StartEnd mode + skip_empty: the empty frame between a bare
        // "$" .. "\r\n" is dropped. Input b"$\r\n$hi\r\n" → two frames
        // (empty, "hi"). With skip_empty:true only the second frame emits.
        let config_on = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: true,
        };
        let mut dec = FrameDecoder::new(&config_on, None).unwrap();
        let frames = dec.push(b"$\r\n$hi\r\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");

        // skip_empty: false → 2 frames (empty, "hi").
        let config_off = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            max_frames: None,
            include_terminators: false,
            skip_empty: false,
        };
        let mut dec_off = FrameDecoder::new(&config_off, None).unwrap();
        let frames_off = dec_off.push(b"$\r\n$hi\r\n").frames;
        assert_eq!(frames_off.len(), 2);
        assert!(frames_off[0].data.is_empty());
        assert_eq!(frames_off[1].data, b"hi");
    }

    #[test]
    fn start_end_picks_earliest_of_overlapping_markers() {
        // Two start markers that share a prefix: "$G" and "$GPGGA".
        // Input: b"$GPGGA,x\r\n". Both markers match at position 0.
        // Tie-break: the first-listed marker in the start vec at the
        // earliest position wins. With start: ["$G", "$GPGGA"], "$G"
        // wins because it appears first in the list and both start at
        // offset 0. After draining the 2-byte "$G" marker, the body is
        // "PGGA,x" (the "$G" wins, not "$GPGGA").
        let config = RxFramingConfig {
            mode: RxFramingMode::StartEnd {
                start: vec!["$G".into(), "$GPGGA".into()],
                end: "\r\n".into(),
                marker_encoding: PatternEncoding::Utf8,
                include_markers: false,
            },
            ..Default::default()
        };
        let mut dec = FrameDecoder::new(&config, None).unwrap();
        let frames = dec.push(b"$GPGGA,x\r\n").frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"PGGA,x");
    }

    #[test]
    fn modbus_ascii_parser_max_adu_length() {
        // A 253-byte data payload near the Modbus ADU max. Pins no-length-cap
        // behavior. The frame is routed through a full FrameDecoder (StartEnd
        // framing with ':' start and "\r\n" end) + ModbusAscii parser.
        let address = 0x01u8;
        let function = 0x03u8;
        let data: Vec<u8> = (0..253u8).collect();
        let mut pdu = vec![address, function];
        pdu.extend_from_slice(&data);
        let lrc_byte = lrc(&pdu);

        let mut all_bytes = pdu.clone();
        all_bytes.push(lrc_byte);
        let hex_body: String = all_bytes.iter().map(|b| format!("{b:02X}")).collect();
        let frame = format!(":{hex_body}\r\n");

        let rx_config = preset_rx_framing(ProtocolPreset::ModbusAscii);
        let parser_config = preset_rx_parser(ProtocolPreset::ModbusAscii);
        let mut dec = FrameDecoder::new(&rx_config, Some(&parser_config)).unwrap();
        let frames = dec.push(frame.as_bytes()).frames;
        assert_eq!(frames.len(), 1);
        match &frames[0].parsed {
            Some(ParsedFrame::ModbusAscii {
                address: a,
                function_code: fc,
                data: d,
                checksum_valid: Some(true),
            }) => {
                assert_eq!(*a, 1);
                assert_eq!(*fc, 3);
                assert_eq!(d.len(), 253);
            }
            other => panic!("expected ModbusAscii parsed frame, got {other:?}"),
        }
    }
}
