//! Frame-content parsers.
//!
//! The parser factory (`build_parser`) and the AT command, JSON lines, shell
//! prompt, raw, NMEA-0183, and Modbus ASCII parsers. Parser-only helpers
//! (trailing-newline/leading-marker stripping, checksum validation) live here
//! beside their sole consumers.

use crate::checksums::{lrc, xor_checksum};

use super::config::{ParserConfig, ParserType};
use super::decoder::{FrameDecodeError, FrameParser, ParsedFrame};

// ---- Parser implementations ------------------------------------------------

pub(crate) fn build_parser(config: &ParserConfig) -> Result<Box<dyn FrameParser>, String> {
    match config.parser_type {
        ParserType::AtCommand => Ok(Box::new(AtCommandParser)),
        ParserType::JsonLines => Ok(Box::new(JsonLinesParser)),
        ParserType::ShellPrompt => {
            let custom = config
                .custom_prompt
                .as_deref()
                .map(|s| {
                    regex::bytes::Regex::new(s).map_err(|e| format!("Invalid prompt regex: {e}"))
                })
                .transpose()?;
            Ok(Box::new(ShellPromptParser { custom }))
        }
        ParserType::Raw => Ok(Box::new(RawParser)),
        ParserType::Nmea => Ok(Box::new(NmeaParser {
            validate: config.validate,
        })),
        ParserType::ModbusAscii => Ok(Box::new(ModbusAsciiParser {
            validate: config.validate,
        })),
    }
}

// AT command parser

struct AtCommandParser;

impl FrameParser for AtCommandParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        let text = String::from_utf8_lossy(data);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Ok(ParsedFrame::Raw);
        }
        if let Some(cmd) = trimmed.strip_prefix('+') {
            if let Some(colon) = cmd.find(':') {
                let cmd_name = cmd[..colon].to_string();
                let fields: Vec<String> = cmd[colon + 1..]
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect();
                return Ok(ParsedFrame::AtCommand {
                    response_type: "response".into(),
                    command: Some(cmd_name),
                    status: None,
                    fields,
                });
            }
        }
        if trimmed == "OK" {
            return Ok(ParsedFrame::AtCommand {
                response_type: "status".into(),
                command: None,
                status: Some("OK".into()),
                fields: vec![],
            });
        }
        if trimmed == "ERROR" {
            return Ok(ParsedFrame::AtCommand {
                response_type: "error".into(),
                command: None,
                status: Some("ERROR".into()),
                fields: vec![],
            });
        }
        if trimmed.starts_with("+CME ERROR") || trimmed.starts_with("+CMS ERROR") {
            return Ok(ParsedFrame::AtCommand {
                response_type: "error".into(),
                command: None,
                status: Some(trimmed.to_string()),
                fields: vec![],
            });
        }
        Ok(ParsedFrame::AtCommand {
            response_type: "data".into(),
            command: None,
            status: None,
            fields: vec![trimmed.to_string()],
        })
    }
}

// JSON lines parser

struct JsonLinesParser;

impl FrameParser for JsonLinesParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        match serde_json::from_slice::<serde_json::Value>(data) {
            Ok(val) if val.is_object() => Ok(ParsedFrame::Json(val)),
            _ => Ok(ParsedFrame::Raw),
        }
    }
}

// Shell prompt parser

struct ShellPromptParser {
    custom: Option<regex::bytes::Regex>,
}

impl FrameParser for ShellPromptParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        let text = String::from_utf8_lossy(data);
        let trimmed = text.trim_end();
        if trimmed.is_empty() {
            return Ok(ParsedFrame::Raw);
        }
        if let Some(ref re) = self.custom {
            if re.is_match(data) {
                return Ok(ParsedFrame::ShellPrompt {
                    prompt: trimmed.to_string(),
                    prompt_type: "custom".into(),
                });
            }
        }
        if trimmed.ends_with("$ ") || trimmed.ends_with("$") {
            return Ok(ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "user".into(),
            });
        }
        if trimmed.ends_with("# ") || trimmed.ends_with("#") {
            return Ok(ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "root".into(),
            });
        }
        if trimmed.ends_with("> ") || trimmed.ends_with(">") {
            return Ok(ParsedFrame::ShellPrompt {
                prompt: trimmed.to_string(),
                prompt_type: "generic".into(),
            });
        }
        Ok(ParsedFrame::Raw)
    }
}

// Raw parser (passthrough)

struct RawParser;

impl FrameParser for RawParser {
    fn parse(&self, _data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        Ok(ParsedFrame::Raw)
    }
}

/// Strip a trailing `\r\n` or bare `\n` from `content` in place (defensive:
/// handles frames where the end marker was NOT stripped by the framing layer).
fn strip_trailing_newline(content: &mut Vec<u8>) {
    if content.ends_with(b"\r\n") {
        content.truncate(content.len() - 2);
    } else if content.ends_with(b"\n") {
        content.truncate(content.len() - 1);
    }
}

#[cfg(test)]
mod strip_tests {
    use super::*;

    #[test]
    fn strip_trailing_newline_removes_crlf() {
        let mut content = b"GPGLL,1,2\r\n".to_vec();
        strip_trailing_newline(&mut content);
        assert_eq!(content, b"GPGLL,1,2");
    }

    #[test]
    fn strip_trailing_newline_removes_bare_lf() {
        // Pins the bare-`\n` branch: a mutation turning `len() - 1` into
        // `len() + 1` (or `/ 1`) must fail this test.
        let mut content = b"GPGLL,1,2\n".to_vec();
        strip_trailing_newline(&mut content);
        assert_eq!(content, b"GPGLL,1,2");
    }

    #[test]
    fn strip_trailing_newline_noop_without_newline() {
        let mut content = b"GPGLL,1,2".to_vec();
        strip_trailing_newline(&mut content);
        assert_eq!(content, b"GPGLL,1,2");
    }
}

/// Strip a leading byte if it matches any of `markers` (defensive: handles
/// frames where the start marker was NOT stripped by the framing layer).
fn strip_leading_if_any(content: &mut Vec<u8>, markers: &[u8]) {
    if let Some(&first) = content.first() {
        if markers.contains(&first) {
            content.remove(0);
        }
    }
}

/// Shared checksum-validate ladder for NMEA XOR and Modbus ASCII LRC.
///
/// Returns `Ok(Some(true))` if computed == received, `Ok(Some(false))` if
/// validate is false and they differ, or `Err(ChecksumMismatch)` if validate is
/// true and they differ. `received` is `None` when the received checksum is
/// unparseable or too short (guaranteed mismatch). The error carries the 1-byte
/// `computed_byte` as `expected` and the raw received bytes as `received` to
/// preserve the [`FrameDecodeError`] shape.
fn check_checksum(
    validate: bool,
    computed_byte: u8,
    received: Option<u8>,
    received_raw: Vec<u8>,
) -> Result<Option<bool>, FrameDecodeError> {
    match received {
        Some(r) if r == computed_byte => Ok(Some(true)),
        Some(_) if validate => Err(FrameDecodeError::ChecksumMismatch {
            expected: vec![computed_byte],
            received: received_raw,
        }),
        Some(_) => Ok(Some(false)),
        None if validate => Err(FrameDecodeError::ChecksumMismatch {
            expected: vec![computed_byte],
            received: received_raw,
        }),
        None => Ok(Some(false)),
    }
}

// NMEA-0183 parser

struct NmeaParser {
    validate: bool,
}

impl FrameParser for NmeaParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        // 1. Strip optional leading $ or ! and trailing \r\n.
        //    Defensive: when include_markers is true, data includes the start/end
        //    markers. The nmea0183 preset uses include_markers=false, but bare
        //    callers might set true. Handle both.
        let mut content = data.to_vec();
        strip_leading_if_any(&mut content, b"$!");
        strip_trailing_newline(&mut content);

        // 2. Validate that this looks like a NMEA sentence. The preset strips
        //    $/! via StartEnd framing, so the data may start directly with the
        //    talker ID (e.g. "GPGLL,..."). If include_markers were true, $/!
        //    was stripped by step 1 above. In either case, a valid NMEA
        //    sentence always contains at least one comma (separating address
        //    from fields). Non-NMEA frames (e.g. "hello world", "AT+CGMI")
        //    have no comma and return Raw — the parser is opt-in; bare callers
        //    mixing data types see Raw for non-matching frames.
        if !content.contains(&b',') {
            return Ok(ParsedFrame::Raw);
        }

        // 3. Split at '*': content before = sentence body, after = checksum hex.
        let (body, checksum_hex) = if let Some(star_pos) = content.iter().position(|&b| b == b'*') {
            let body = &content[..star_pos];
            let checksum_hex = &content[star_pos + 1..];
            (body.to_vec(), Some(checksum_hex.to_vec()))
        } else {
            (content, None)
        };

        // 4. Evaluate checksum validity. Yields None (no checksum present),
        // Some(true) (present + correct), Some(false) (present + incorrect),
        // or Err (present + incorrect + validate=true). Body parsing always
        // runs afterward so malformed-checksum frames carry the full parsed
        // body with checksum_valid: Some(false) when validate is false.
        let checksum_valid = match &checksum_hex {
            Some(hex) if hex.len() >= 2 => {
                let computed_byte = xor_checksum(&body);
                match std::str::from_utf8(&hex[..2])
                    .ok()
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
                {
                    Some(received_val) => {
                        // Valid hex — compare against computed checksum.
                        check_checksum(
                            self.validate,
                            computed_byte,
                            Some(received_val),
                            vec![received_val],
                        )?
                    }
                    None => {
                        // Invalid hex chars — guaranteed mismatch (None).
                        check_checksum(self.validate, computed_byte, None, hex.clone())?
                    }
                }
            }
            Some(hex) => {
                // Checksum present but too short (<2 hex chars). Guaranteed mismatch.
                let computed_byte = xor_checksum(&body);
                check_checksum(self.validate, computed_byte, None, hex.clone())?
            }
            None => None,
        };

        // 5. Parse the sentence body: split into address + comma fields.
        // NMEA spec is ASCII; invalid UTF-8 is non-NMEA input → Raw
        // (mirrors ModbusAsciiParser).
        let body_str = match std::str::from_utf8(&body) {
            Ok(s) => s,
            Err(_) => return Ok(ParsedFrame::Raw),
        };
        if !body_str.is_ascii() {
            return Ok(ParsedFrame::Raw);
        }

        let (address_part, fields_part) = match body_str.find(',') {
            Some(comma_pos) => (
                body_str[..comma_pos].to_string(),
                body_str[comma_pos + 1..].to_string(),
            ),
            None => (body_str.to_string(), String::new()),
        };

        let (talker_id, sentence_type) = if address_part.len() >= 2 && address_part.starts_with('P')
        {
            // Proprietary NMEA: talker = "P", type = rest of address
            let tid = "P".to_string();
            let stype = address_part[1..].to_string();
            (tid, stype)
        } else if address_part.len() >= 2 {
            // Standard NMEA: talker = first 2, type = rest
            let tid = address_part[..2].to_string();
            let stype = address_part[2..].to_string();
            (tid, stype)
        } else {
            // < 2 chars: use whole as talker, no type
            (address_part, String::new())
        };

        let fields: Vec<String> = if fields_part.is_empty() {
            vec![]
        } else {
            fields_part.split(',').map(|s| s.to_string()).collect()
        };

        Ok(ParsedFrame::Nmea {
            talker_id,
            sentence_type,
            fields,
            checksum_valid,
        })
    }
}

// Modbus ASCII parser

struct ModbusAsciiParser {
    validate: bool,
}

impl FrameParser for ModbusAsciiParser {
    fn parse(&self, data: &[u8]) -> Result<ParsedFrame, FrameDecodeError> {
        // 1. Strip optional leading ':' and trailing \r\n (defensive — the
        //    preset uses include_markers=false so data is the body between
        //    ':' and \r\n; a bare caller with include_markers=true would
        //    include them).
        let mut content = data.to_vec();
        strip_leading_if_any(&mut content, b":");
        strip_trailing_newline(&mut content);

        // 2. Modbus ASCII body is all hex chars (0-9, A-F, a-f). Validate:
        //    if any non-hex byte is present, return Ok(ParsedFrame::Raw) —
        //    non-Modbus frames routed through this parser see Raw (mirror
        //    Nmea's non-NMEA → Raw behavior). The body MUST have an even
        //    number of hex chars (each decoded byte = 2 hex chars). Odd
        //    length → Raw (malformed).
        let body_str = match std::str::from_utf8(&content) {
            Ok(s) => s,
            Err(_) => return Ok(ParsedFrame::Raw),
        };
        if body_str.is_empty() || body_str.bytes().any(|b| !b.is_ascii_hexdigit()) {
            return Ok(ParsedFrame::Raw);
        }
        if body_str.len() % 2 != 0 {
            return Ok(ParsedFrame::Raw);
        }

        // 3. Hex-decode the body into bytes.
        let decoded: Vec<u8> = (0..body_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&body_str[i..i + 2], 16).unwrap_or(0))
            .collect();
        // decoded = [address, function_code, data..., lrc]

        // 4. Split off the LRC (last byte). Need at least 3 bytes
        //    (address + function + lrc) for a minimal frame; fewer → Raw.
        if decoded.len() < 3 {
            return Ok(ParsedFrame::Raw);
        }
        let lrc_received = decoded[decoded.len() - 1];
        let pdu = &decoded[..decoded.len() - 1]; // address + function + data
        let address = pdu[0];
        let function_code = pdu[1];
        let data = pdu[2..].to_vec();

        // 5. Validate LRC over the PDU (address + function + data — NOT the LRC byte).
        let computed_val = lrc(pdu);
        let checksum_valid = check_checksum(
            self.validate,
            computed_val,
            Some(lrc_received),
            vec![lrc_received],
        )?;

        // 6. Return ParsedFrame::ModbusAscii.
        Ok(ParsedFrame::ModbusAscii {
            address,
            function_code,
            data,
            checksum_valid,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::framing::config::{LineEnding, RxFramingConfig, RxFramingMode};
    use crate::framing::decoder::FrameDecoder;

    // ── Parser tests ────────────────────────────────────────────────────

    #[test]
    fn at_parser_ok() {
        let p = AtCommandParser;
        let result = p.parse(b"OK").unwrap();
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                status: Some(s),
                ..
            } if response_type == "status" && s == "OK"
        ));
    }

    #[test]
    fn at_parser_error() {
        let p = AtCommandParser;
        let result = p.parse(b"ERROR").unwrap();
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                status: Some(s),
                ..
            } if response_type == "error" && s == "ERROR"
        ));
    }

    #[test]
    fn at_parser_command_response() {
        let p = AtCommandParser;
        let result = p.parse(b"+CGREG: 0,1").unwrap();
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                command: Some(ref c),
                fields,
                ..
            } if response_type == "response" && c == "CGREG" && fields == ["0", "1"]
        ));
    }

    #[test]
    fn json_parser_valid() {
        let p = JsonLinesParser;
        let result = p.parse(b"{\"key\":\"value\"}").unwrap();
        assert!(matches!(result, ParsedFrame::Json(_)));
    }

    #[test]
    fn json_parser_invalid() {
        let p = JsonLinesParser;
        let result = p.parse(b"not json").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn json_parser_non_object_is_raw() {
        let p = JsonLinesParser;
        assert!(matches!(p.parse(b"[1,2,3]").unwrap(), ParsedFrame::Raw));
        assert!(matches!(p.parse(b"42").unwrap(), ParsedFrame::Raw));
        assert!(matches!(p.parse(b"\"hi\"").unwrap(), ParsedFrame::Raw));
        assert!(matches!(
            p.parse(b"{\"k\":1}").unwrap(),
            ParsedFrame::Json(_)
        ));
    }

    #[test]
    fn shell_prompt_user() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"$ ").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "user")
        );
    }

    #[test]
    fn shell_prompt_root() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"# ").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "root")
        );
    }

    #[test]
    fn shell_prompt_host() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"root@host:~# ").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "root")
        );
    }

    #[test]
    fn shell_prompt_bare_generic_arrow() {
        // Bare `>` (no trailing space) must classify as a generic prompt; a
        // mutation turning the `||` into `&&` would make this fall through
        // to Raw.
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b">").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "generic")
        );
    }

    #[test]
    fn at_parser_empty_input() {
        let p = AtCommandParser;
        let result = p.parse(b"").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn at_parser_cme_error() {
        let p = AtCommandParser;
        let result = p.parse(b"+CME ERROR: 100").unwrap();
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                command: Some(ref c),
                ..
            } if response_type == "response" && c == "CME ERROR"
        ));
    }

    #[test]
    fn at_parser_cms_error() {
        let p = AtCommandParser;
        let result = p.parse(b"+CMS ERROR: 500").unwrap();
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                command: Some(ref c),
                ..
            } if response_type == "response" && c == "CMS ERROR"
        ));
    }

    #[test]
    fn at_parser_cme_error_fields_are_split() {
        // Pins the +CME ERROR branch (bare form without a colon, which does
        // NOT hit the `+`-prefix response branch): a mutation turning the
        // `||` into `&&` must fail this test.
        let p = AtCommandParser;
        let result = p.parse(b"+CME ERROR").unwrap();
        assert!(matches!(
            result,
            ParsedFrame::AtCommand {
                response_type,
                command: None,
                status: Some(ref s),
                ..
            } if response_type == "error" && s == "+CME ERROR"
        ));
    }

    #[test]
    fn json_parser_empty_input() {
        let p = JsonLinesParser;
        let result = p.parse(b"").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn shell_prompt_empty_input() {
        let p = ShellPromptParser { custom: None };
        let result = p.parse(b"").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn shell_prompt_custom_regex_invalid() {
        let config = RxFramingConfig {
            mode: RxFramingMode::Line {
                ending: LineEnding::Auto,
            },
            ..Default::default()
        };
        let parser = ParserConfig {
            parser_type: ParserType::ShellPrompt,
            custom_prompt: Some("[invalid".to_string()),
            validate: false,
        };
        match FrameDecoder::new(&config, Some(&parser)) {
            Ok(_) => panic!("invalid regex should be rejected"),
            Err(err) => assert!(err.contains("Invalid prompt regex"), "got: {err}"),
        }
    }

    #[test]
    fn shell_prompt_custom_regex_match() {
        let p = ShellPromptParser {
            custom: Some(regex::bytes::Regex::new("^>>> $").unwrap()),
        };
        let result = p.parse(b">>> ").unwrap();
        assert!(
            matches!(result, ParsedFrame::ShellPrompt { prompt_type, .. } if prompt_type == "custom")
        );
    }

    #[test]
    fn raw_parser_passthrough() {
        let p = RawParser;
        let result = p.parse(b"anything").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    // ── NMEA parser tests ──────────────────────────────────────────────

    /// Helper: build a NMEA sentence body with a computed XOR checksum.
    fn nmea_checksum_body(body: &[u8]) -> String {
        format!("{:02X}", xor_checksum(body))
    }

    #[test]
    fn nmea_parser_valid_gga_sentence() {
        let body = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
        let cs_hex = nmea_checksum_body(body);
        let sentence =
            format!("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GGA");
                assert_eq!(
                    fields,
                    vec![
                        "123519",
                        "4807.038",
                        "N",
                        "01131.000",
                        "E",
                        "1",
                        "08",
                        "0.9",
                        "545.4",
                        "M",
                        "46.9",
                        "M",
                        "",
                        ""
                    ]
                );
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_valid_gll_sentence() {
        // Known-good sentence from checksums::tests::xor_checksum_known_nmea_sentence
        // Body: "GPGLL,3751.65,N,12226.54,W" → XOR checksum 0x7E
        let sentence = b"GPGLL,3751.65,N,12226.54,W*7E";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(fields, vec!["3751.65", "N", "12226.54", "W"]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_ais_sentence_starts_with_bang() {
        // Simple AIS sentence body.
        let body = b"AIVDM,1,1,,B,15M67FC000G?ufbE`H9P<In,0";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("AIVDM,1,1,,B,15M67FC000G?ufbE`H9P<In,0*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                checksum_valid,
                ..
            } => {
                assert_eq!(talker_id, "AI");
                assert_eq!(sentence_type, "VDM");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_bad_checksum_returns_error_when_validate_true() {
        // GLL sentence with correct checksum 0x7E, but we pass *00 (wrong).
        let sentence = b"GPGLL,3751.65,N,12226.54,W*00";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence);
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected, vec![0x7E]);
                assert_eq!(received, vec![0x00]);
            }
            other => panic!("expected ChecksumMismatch error, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_bad_checksum_returns_some_false_when_validate_false() {
        let sentence = b"GPGLL,3751.65,N,12226.54,W*00";
        let p = NmeaParser { validate: false };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea { checksum_valid, .. } => {
                assert_eq!(checksum_valid, Some(false));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_no_checksum_accepted() {
        let sentence = b"GPGLL,3751.65,N,12226.54,W";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea { checksum_valid, .. } => {
                assert_eq!(checksum_valid, None);
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
        // Also test with validate: false
        let p2 = NmeaParser { validate: false };
        let result2 = p2.parse(sentence).unwrap();
        match result2 {
            ParsedFrame::Nmea { checksum_valid, .. } => {
                assert_eq!(checksum_valid, None);
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_non_nmea_frame_returns_raw() {
        let p = NmeaParser { validate: true };
        let result = p.parse(b"hello world").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));

        let result2 = p.parse(b"AT+CGMI").unwrap();
        assert!(matches!(result2, ParsedFrame::Raw));
    }

    #[test]
    fn nmea_parser_invalid_utf8_body_returns_raw() {
        // NMEA spec is ASCII; invalid UTF-8 is non-NMEA → Raw (mirrors Modbus).
        // The NmeaParser now uses from_utf8 (not from_utf8_lossy) for the body.
        let parser = build_parser(&ParserConfig {
            parser_type: ParserType::Nmea,
            custom_prompt: None,
            validate: false,
        })
        .unwrap();
        // Body with a comma (passes the NMEA-shape check) but invalid UTF-8.
        let body: &[u8] = b"GPGLL,\xFF,";
        let result = parser.parse(body).unwrap();
        assert!(
            matches!(result, ParsedFrame::Raw),
            "invalid UTF-8 → Raw, got {result:?}"
        );
    }

    #[test]
    fn nmea_parser_strips_leading_start_char_if_present() {
        // Simulate include_markers: true — the $ is included in the data.
        let body = b"GPGLL,3751.65,N,12226.54,W";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("$GPGLL,3751.65,N,12226.54,W*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                checksum_valid,
                ..
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_strips_trailing_crlf_if_present() {
        // Simulate include_markers: true — the trailing \r\n is included.
        let body = b"GPGLL,3751.65,N,12226.54,W";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("GPGLL,3751.65,N,12226.54,W*{cs_hex}\r\n");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                checksum_valid,
                ..
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_proprietary_sentence() {
        // $PGRMZ proprietary sentence without the $.
        let body = b"PGRMZ,2010,f,3";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("PGRMZ,2010,f,3*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                // Proprietary $PGRMZ: talker = "P", sentence_type = rest = "GRMZ"
                assert_eq!(talker_id, "P");
                assert_eq!(sentence_type, "GRMZ");
                assert_eq!(fields, vec!["2010", "f", "3"]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_non_ascii_body_returns_raw() {
        // Two branches: valid-UTF-8-but-non-ASCII and invalid-UTF-8.
        //
        // Case 1: valid UTF-8 but non-ASCII → is_ascii fails, returns Raw.
        // b"a\xC3\xA9,x" = "aé,x" where é is U+00E9 (2-byte UTF-8, non-ASCII).
        // This passes the contains(',') guard and from_utf8 but hits the ASCII
        // guard — byte index 2 is not a char boundary, so slicing panicked
        // before the ASCII guard was added.
        let p = NmeaParser { validate: true };
        let body1 = b"a\xC3\xA9,x";
        let result1 = p.parse(body1).unwrap();
        assert!(
            matches!(result1, ParsedFrame::Raw),
            "valid-UTF-8 non-ASCII body should return Raw, got {result1:?}"
        );

        // validate: false variant — same behavior.
        let p2 = NmeaParser { validate: false };
        let result1b = p2.parse(body1).unwrap();
        assert!(
            matches!(result1b, ParsedFrame::Raw),
            "valid-UTF-8 non-ASCII body (validate=false) should return Raw, got {result1b:?}"
        );

        // Case 2: invalid UTF-8 → from_utf8 returns Err → Raw.
        // b"\xFFx,y" — 0xFF is a lone byte >0x7F and not a valid UTF-8
        // lead byte. from_utf8 returns Err immediately.
        let body2 = b"\xFFx,y";
        let result2 = p.parse(body2).unwrap();
        assert!(
            matches!(result2, ParsedFrame::Raw),
            "invalid-UTF-8 body should return Raw, got {result2:?}"
        );

        // Sanity: a valid NMEA sentence with the same parser DOES parse to Nmea.
        let sentence = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
        let cs_hex = nmea_checksum_body(sentence);
        let valid_sentence =
            format!("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*{cs_hex}");
        let result_valid = p.parse(valid_sentence.as_bytes()).unwrap();
        assert!(
            matches!(result_valid, ParsedFrame::Nmea { .. }),
            "valid ASCII sentence should parse to Nmea, got {result_valid:?}"
        );
    }

    #[test]
    fn nmea_parser_proprietary_p_prefix_split() {
        // $PGRMM proprietary sentence: talker = "P", sentence_type = rest = "GRMM"
        let body = b"PGRMM,W";
        let cs_hex = nmea_checksum_body(body);
        let sentence = format!("PGRMM,W*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "P", "proprietary talker should be 'P'");
                assert_eq!(
                    sentence_type, "GRMM",
                    "proprietary sentence_type should be rest after P"
                );
                assert_eq!(fields, vec!["W"]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_standard_5char_address_unchanged() {
        // GPGGA standard sentence (5-char address): confirms standard split
        // is not accidentally swallowed by the P-branch.
        let body = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,";
        let cs_hex = nmea_checksum_body(body);
        let sentence =
            format!("GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*{cs_hex}");
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP", "standard talker should be first 2 chars");
                assert_eq!(
                    sentence_type, "GGA",
                    "standard sentence_type should be rest"
                );
                assert_eq!(
                    fields,
                    vec![
                        "123519",
                        "4807.038",
                        "N",
                        "01131.000",
                        "E",
                        "1",
                        "08",
                        "0.9",
                        "545.4",
                        "M",
                        "46.9",
                        "M",
                        "",
                        ""
                    ]
                );
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_invalid_hex_checksum_validate_false_returns_full_body() {
        // Sentence with non-hex checksum chars (*GG), validate: false →
        // full body parse + checksum_valid: Some(false).
        let sentence = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*GG";
        let p = NmeaParser { validate: false };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GGA");
                assert_eq!(
                    fields,
                    vec![
                        "123519",
                        "4807.038",
                        "N",
                        "01131.000",
                        "E",
                        "1",
                        "08",
                        "0.9",
                        "545.4",
                        "M",
                        "46.9",
                        "M",
                        "",
                        ""
                    ]
                );
                assert_eq!(checksum_valid, Some(false));
            }
            other => panic!("expected Nmea frame with full body, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_too_short_checksum_validate_false_returns_full_body() {
        // Sentence with 1-hex-char checksum (*7), validate: false →
        // full body parse + checksum_valid: Some(false).
        let sentence = b"GPGLL,3751.65,N,12226.54,W*7";
        let p = NmeaParser { validate: false };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "GLL");
                assert_eq!(fields, vec!["3751.65", "N", "12226.54", "W"]);
                assert_eq!(checksum_valid, Some(false));
            }
            other => panic!("expected Nmea frame with full body, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_invalid_hex_checksum_validate_true_returns_err() {
        // Sentence with non-hex checksum chars (*GG), validate: true →
        // Err(ChecksumMismatch). The received vec contains the raw hex bytes.
        let sentence = b"GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*GG";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence);
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected.len(), 1);
                assert_eq!(received, b"GG".to_vec());
            }
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_too_short_checksum_validate_true_returns_err() {
        // Sentence with 1-hex-char checksum (*7), validate: true →
        // Err(ChecksumMismatch). The received vec contains the raw hex bytes.
        let sentence = b"GPGLL,3751.65,N,12226.54,W*7";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence);
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected.len(), 1);
                assert_eq!(received, b"7".to_vec());
            }
            other => panic!("expected ChecksumMismatch, got {other:?}"),
        }
    }

    // ── Modbus ASCII parser unit tests ───────────────────────────────────

    // Helper: compute LRC over a PDU byte slice and return the 1-byte hex string.
    fn modbus_lrc(pdu: &[u8]) -> String {
        format!("{:02X}", lrc(pdu))
    }

    // Helper: build a Modbus ASCII hex body from PDU and append LRC.
    fn modbus_body(hex_pdu: &str) -> String {
        let pdu_bytes: Vec<u8> = (0..hex_pdu.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_pdu[i..i + 2], 16).unwrap())
            .collect();
        let lrc_hex = modbus_lrc(&pdu_bytes);
        format!("{hex_pdu}{lrc_hex}")
    }

    #[test]
    fn modbus_ascii_parser_valid_read_holding_registers() {
        // :010300000001FB\r\n — address=1, fc=3, data=[0,0,0,1], LRC=FB
        let body = modbus_body("010300000001");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                data,
                checksum_valid,
            } => {
                assert_eq!(address, 1);
                assert_eq!(function_code, 3);
                assert_eq!(data, vec![0, 0, 0, 1]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_valid_write_single_register() {
        // Address 0x01, FC 0x06, register 0x000A, value 0x0001
        // PDU = [0x01, 0x06, 0x00, 0x0A, 0x00, 0x01]
        // LRC = compute in test
        let pdu = [0x01u8, 0x06, 0x00, 0x0A, 0x00, 0x01];
        let lrc_hex = modbus_lrc(&pdu);
        let body = format!("0106000A0001{lrc_hex}");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                data,
                checksum_valid,
            } => {
                assert_eq!(address, 1);
                assert_eq!(function_code, 6);
                assert_eq!(data, vec![0x00, 0x0A, 0x00, 0x01]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_exception_response() {
        // Exception response: address 0x01, FC 0x83 (0x03 | 0x80), exception code 0x02
        let pdu = [0x01u8, 0x83, 0x02];
        let lrc_hex = modbus_lrc(&pdu);
        let body = format!("018302{lrc_hex}");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                data,
                checksum_valid,
            } => {
                assert_eq!(address, 1);
                assert_eq!(function_code, 131); // 0x83
                assert_eq!(data, vec![2]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_broadcast_address_zero() {
        let pdu = [0x00u8, 0x03, 0x00, 0x00, 0x00, 0x01];
        let lrc_hex = modbus_lrc(&pdu);
        let body = format!("000300000001{lrc_hex}");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address,
                function_code,
                ..
            } => {
                assert_eq!(address, 0);
                assert_eq!(function_code, 3);
            }
            other => panic!("expected ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_bad_lrc_returns_error_when_validate_true() {
        // Correct LRC is 0xFB, corrupt to 0x00
        let body = b"01030000000100";
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body);
        match result {
            Err(FrameDecodeError::ChecksumMismatch { expected, received }) => {
                assert_eq!(expected, vec![0xFB]);
                assert_eq!(received, vec![0x00]);
            }
            other => panic!("expected ChecksumMismatch error, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_bad_lrc_returns_some_false_when_validate_false() {
        let body = b"01030000000100";
        let p = ModbusAsciiParser { validate: false };
        let result = p.parse(body).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                checksum_valid: Some(false),
                ..
            } => {}
            other => panic!("expected ModbusAscii with checksum_valid: false, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_lowercase_hex_accepted() {
        let body = b"010300000001fb";
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                checksum_valid: Some(true),
                ..
            } => {}
            other => panic!("expected valid ModbusAscii, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_non_hex_returns_raw() {
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"hello world").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_odd_length_returns_raw() {
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"010").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_too_short_returns_raw() {
        // "01" → 1 decoded byte, < 3
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"01").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_strips_leading_colon_and_trailing_crlf() {
        // Body with markers included — defensive stripping should still work.
        let pdu = [0x01u8, 0x03, 0x00, 0x00, 0x00, 0x01];
        let lrc_hex = modbus_lrc(&pdu);
        let frame = format!(":010300000001{lrc_hex}\r\n");
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(frame.as_bytes()).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 3,
                checksum_valid: Some(true),
                ..
            } => {}
            other => panic!("expected valid ModbusAscii, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_empty_body_returns_raw() {
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(b"").unwrap();
        assert!(matches!(result, ParsedFrame::Raw));
    }

    #[test]
    fn modbus_ascii_parser_minimal_frame_3_bytes() {
        // address 0x01, function 0xFF, LRC: sum=0x01+0xFF=0x00 (wrap), LRC=0x00
        let body = b"01FF00";
        let p = ModbusAsciiParser { validate: true };
        let result = p.parse(body).unwrap();
        match result {
            ParsedFrame::ModbusAscii {
                address: 1,
                function_code: 255,
                data,
                checksum_valid: Some(true),
            } if data.is_empty() => {}
            other => panic!("expected minimal ModbusAscii frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_multi_fragment_ais_first_sentence() {
        use crate::checksums::xor_checksum;

        // AIS multi-fragment messages are NOT reassembled by the parser —
        // each fragment is its own frame. This test pins the first-fragment
        // parse and documents the no-reassembly behavior.
        //
        // Verify the XOR checksum of the body between ! and * (exclusive).
        let checksum_body = b"AIVDM,2,1,3,B,55?MbV02;H0000,0";
        let computed = xor_checksum(checksum_body);
        // Correct XOR is 0x22 (an earlier draft claimed 0x5C).
        assert_eq!(computed, 0x22, "AIS sentence checksum is 0x22, not 0x5C");

        let sentence = b"!AIVDM,2,1,3,B,55?MbV02;H0000,0*22";
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "AI");
                assert_eq!(sentence_type, "VDM");
                assert_eq!(fields, vec!["2", "1", "3", "B", "55?MbV02;H0000", "0"]);
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn nmea_parser_preserves_whitespace_in_fields() {
        use crate::checksums::xor_checksum;

        // The parser uses split(',') without per-field trimming; this test
        // pins that whitespace inside a field is preserved. A future
        // "helpful" refactor adding .trim() per field would break this.
        let body_without_star = b"GPVTG,  48.7  ,T,,,N,,K,N";
        let cs = xor_checksum(body_without_star);
        let cs_hex = format!("{cs:02X}");
        // cs_hex = "58" for this body.
        let sentence = format!(
            "${}*{}",
            std::str::from_utf8(body_without_star).unwrap(),
            cs_hex
        );
        let p = NmeaParser { validate: true };
        let result = p.parse(sentence.as_bytes()).unwrap();
        match result {
            ParsedFrame::Nmea {
                talker_id,
                sentence_type,
                fields,
                checksum_valid,
            } => {
                assert_eq!(talker_id, "GP");
                assert_eq!(sentence_type, "VTG");
                // The first data field (fields[0]) has leading + trailing
                // spaces — they are NOT trimmed.
                assert_eq!(fields[0], "  48.7  ");
                assert_eq!(checksum_valid, Some(true));
            }
            other => panic!("expected Nmea frame, got {other:?}"),
        }
    }

    #[test]
    fn modbus_ascii_parser_non_ascii_body_returns_raw() {
        // Two branches in ModbusAsciiParser::parse that lead to Raw:
        //
        // Case 1: valid UTF-8 but non-ASCII → is_ascii_hexdigit fails.
        // b"01\xC3\xA900" = "01é00" where é is U+00E9 (valid UTF-8, non-
        // hex-digit byte 0xC3). The parser calls from_utf8 (Ok), then
        // is_ascii_hexdigit fails → Raw.
        let p = ModbusAsciiParser { validate: true };
        let body1 = b"01\xC3\xA900";
        let result1 = p.parse(body1).unwrap();
        assert!(
            matches!(result1, ParsedFrame::Raw),
            "non-ASCII valid-UTF-8 body should return Raw, got {result1:?}"
        );

        // Case 2: invalid UTF-8 → from_utf8 returns Err → Raw.
        // b"01\xFF00" — 0xFF is a lone byte >0x7F and not a valid UTF-8
        // continuation. from_utf8 returns Err immediately.
        let body2 = b"01\xFF00";
        let result2 = p.parse(body2).unwrap();
        assert!(
            matches!(result2, ParsedFrame::Raw),
            "invalid-UTF-8 body should return Raw, got {result2:?}"
        );

        // Sanity: a valid hex body with the same parser DOES parse to ModbusAscii.
        let pdu = [0x01u8, 0x03, 0x00, 0x00, 0x00, 0x01];
        let lrc_hex = {
            let cs = lrc(&pdu);
            format!("{cs:02X}")
        };
        let body_valid = format!("010300000001{lrc_hex}");
        let result_valid = p.parse(body_valid.as_bytes()).unwrap();
        assert!(
            matches!(result_valid, ParsedFrame::ModbusAscii { .. }),
            "valid hex body should parse to ModbusAscii, got {result_valid:?}"
        );
    }
}
