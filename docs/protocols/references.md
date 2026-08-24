# Protocol references

This page lists normative references for the framing modes, parsers, and
presets implemented in `serial-mcp`. These are citations only. No specification
content is committed to this repository.

The local `resources/` symlink is ignored by Git. It points to a private
collection of source documents for offline reference. It is not part of the
distribution.

For the user-facing protocol guide covering presets, precedence, and checksum
behavior, see [../protocols.md](../protocols.md).

## Framing modes

- The `line` mode uses newline framing. No single RFC defines it. RX `auto`
  mode recognizes LF, CRLF, and bare-CR line endings according to common
  terminal conventions. Bare-CR promotion during a stream is a serial-mcp
  behavior, not a specification requirement.
- The `delimiter` mode uses a generic byte-sequence delimiter. The caller
  supplies the delimiter.
- The `length_prefixed` mode uses length-prefix framing. No single RFC defines
  it. The 1-, 2-, and 4-byte prefixes with big- or little-endian order follow
  common embedded framing conventions.
- The `start_end` mode uses start and end markers. The caller supplies the
  markers. The NMEA-0183 and Modbus ASCII presets use protocol-specific markers
  with this mode.
- The `slip` mode follows RFC 1055, *A Nonstandard for Transmission of IP
  Datagrams over Serial Lines*, J. Romkey, January 1988.
  <https://www.rfc-editor.org/rfc/rfc1055>. The implementation uses byte-stuffed
  framing with `END` (0xC0) and `ESC` (0xDB) delimiters.
- The `cobs` mode implements *Consistent Overhead Byte Stuffing*, Stu Cheshire
  and Mary Baker, IEEE Network, 1999. The implementation uses the plain
  0x00-delimited Cheshire/Baker variant. The PPP-COBS draft variant with 0x7E is
  not supported and is tracked in `FEATURES.md`.
  <https://stuartcheshire.org/papers/COBSforToN.pdf>. See also
  draft-ietf-pppext-cobs-00, an expired Internet-Draft.

## Parsers

- The `at_command` parser follows the *AT Commands Reference Manual*, 3GPP TS
  27.007, and the ITU-T V.250 / V.25ter lineage of the Hayes AT command set.
  serial-mcp parses common `OK`, `ERROR`, `+CME ERROR: <n>`, and
  `+CMS ERROR: <n>` result codes. It also parses `+<cmd>: …` URCs. No single
  canonical document defines the de facto modem dialect targeted by the parser.
- The `json_lines` and `ndjson` parsers follow the JSON Lines and NDJSON
  conventions. JSON Lines is documented at <https://jsonlines.org/>. NDJSON,
  or newline-delimited JSON, is documented at
  <https://github.com/ndjson/ndjson-spec>. The `ndjson` preset sets `skip_empty`
  to true for blank or whitespace-only lines. `json_lines` does not. JSON values
  follow RFC 8259, *The JavaScript Object Notation (JSON) Data Interchange
  Format*, T. Bray, December 2017, <https://www.rfc-editor.org/rfc/rfc8259>.
- The `shell_prompt` parser detects generic shell prompts. No specification
  defines it. serial-mcp matches common prompt patterns such as `$`, `#`, and
  `>`, plus a configurable custom regex.
- The `raw` parser performs no parsing. It returns frames as raw bytes.

## Protocol presets

- The `at_command` preset appends `\r` on TX. It uses line framing in `auto`
  mode on RX and uses the `at_command` parser. It combines the AT command
  references above with line framing.
- The `slip` preset uses SLIP TX, SLIP RX, and the `raw` parser. It is based on
  RFC 1055.
- The `json_lines` and `ndjson` presets use line TX with `\n`, line RX in `auto`
  mode, and the `json_lines` parser. `ndjson` also sets `skip_empty`. These
  presets use RFC 8259 and the JSON Lines and NDJSON conventions above.
- The `cobs` preset uses COBS TX, COBS RX, and the `raw` parser. It is based on
  the Cheshire/Baker variant.
- The `nmea0183` preset implements the *NMEA 0183* marine sentence protocol. It
  uses start/end framing with `$` or `!` and `\r\n`, plus the `nmea` parser with
  `validate` enabled. The `*XX` XOR checksum covers the bytes between `$` or `!`
  and `*`. NMEA 0183 is a © NMEA standard, and its specification is not freely
  redistributable. See the [National Marine Electronics Association](https://www.nmea.org/content/STANDARDS/NMEA_0183_Standard)
  for the official document. The proprietary-sentence convention,
  `$P<vendor>...`, sets `talker_id` to `"P"` and uses the remaining text as
  `sentence_type`.
- The `modbus_ascii` preset implements *Modbus over Serial Line: Specification
  and Implementation Guide*, published by the Modbus Organization, formerly
  Modbus-IDA / Schneider Electric. ASCII mode uses `:` as the start marker and
  `\r\n` as the end marker. It uses a hex-encoded body and a trailing LRC over
  the PDU. The preset uses start/end framing and the `modbus_ascii` parser with
  `validate` enabled. <https://www.modbus.org/specs.php>. Modbus is a © Modbus
  Organization specification. The specification is freely downloadable from
  that link.

## Related standards for unimplemented features

- HDLC follows ISO/IEC 13239 and RFC 1662, *PPP in HDLC Framing*.
  <https://www.rfc-editor.org/rfc/rfc1662>. `FEATURES.md` tracks it as a future
  framing mode with FCS-16 checksum.
- Modbus RTU uses Modbus over Serial Line, RTU mode, with binary frames and
  CRC-16. `FEATURES.md` tracks it as a future feature. The `checksums.rs` module
  would gain a CRC-16 function. The `Checksum` trait abstraction would return
  when a second checksum width needs abstraction.

## Attribution

This document cites sources for the implemented protocols and reproduces no
specification text. Trademarks and copyrights belong to their respective owners.
They include NMEA, Modbus Organization, IETF, IEEE, Stu Cheshire and Mary Baker,
and the JSON Lines and NDJSON community.
