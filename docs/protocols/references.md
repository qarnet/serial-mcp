# Protocol References

Normative references for the framing modes, parsers, and presets
implemented in `serial-mcp`. Cite-only — no content from these specs is
committed to this repository. The local `resources/` symlink (gitignored)
points at a private collection of the source documents for offline
reference; it is not part of the distribution.

For the user-facing protocol guide (presets, precedence, checksum
behavior), see [../protocols.md](../protocols.md).

## Framing modes

- **Line (`line`)** — newline framing. No single RFC; the `auto` RX mode
  recognizes LF, CRLF, and bare-CR line endings per common terminal
  conventions. The bare-CR mid-stream promotion is a serial-mcp behavior,
  not a spec requirement.
- **Delimiter (`delimiter`)** — generic byte-sequence delimiter framing.
  No spec; agent-supplied delimiter.
- **Length-prefixed (`length_prefixed`)** — length prefix framing. No
  single RFC; the 1/2/4-byte prefix with big/little endianness matches
  common embedded framing conventions.
- **Start/End marker (`start_end`)** — start and end marker framing. No
  spec; agent-supplied markers. The NMEA-0183 and Modbus ASCII presets
  use this mode with protocol-specific markers (see below).
- **SLIP (`slip`)** — RFC 1055: *A Nonstandard for Transmission of IP
  Datagrams over Serial Lines: SLIP*. J. Romkey. January 1988.
  <https://www.rfc-editor.org/rfc/rfc1055>. Byte-stuffed framing with
  `END` (0xC0) and `ESC` (0xDB) delimiters.
- **COBS (`cobs`)** — *Consistent Overhead Byte Stuffing*. Stu Cheshire,
  Mary Baker. IEEE Network, 1999. The plain 0x00-delimited variant
  (Cheshire/Baker) is implemented; the PPP-COBS draft variant (0x7E) is
  not supported and is tracked in FEATURES.md.
  <https://stuartcheshire.org/papers/COBSforToN.pdf>. See also
  draft-ietf-pppext-cobs-00 (expired I-D).

## Parsers

- **`at_command`** — *AT Commands Reference Manual*. 3GPP TS 27.007 and
  the ITU-T V.250 / V.25ter lineage (Hayes AT command set). serial-mcp
  parses the common `OK` / `ERROR` / `+CME ERROR: <n>` / `+CMS ERROR: <n>`
  result codes and `+<cmd>: …` URCs. No single canonical doc; the parser
  targets the de-facto modem dialect.
- **`json_lines` / `ndjson`** — *JSON Lines* and *NDJSON* conventions.
  JSON Lines: <https://jsonlines.org/>. NDJSON (newline-delimited JSON):
  <https://github.com/ndjson/ndjson-spec>. The `ndjson` preset sets
  `skip_empty: true` per the NDJSON spec (skip blank/whitespace-only
  lines); `json_lines` does not. The JSON values themselves are per
  RFC 8259 (*The JavaScript Object Notation (JSON) Data Interchange
  Format*, T. Bray, December 2017, <https://www.rfc-editor.org/rfc/rfc8259>).
- **`shell_prompt`** — generic shell prompt detection. No spec;
  serial-mcp matches common prompt patterns (`$`, `#`, `>` plus a
  configurable custom regex).
- **`raw`** — no parsing; frames returned as raw bytes.

## Protocol presets

- **`at_command`** — TX appends `\r`, RX line framing (auto), `at_command`
  parser. Combines the AT command spec above with line framing.
- **`slip`** — SLIP TX + SLIP RX + `raw` parser (RFC 1055).
- **`json_lines`** / **`ndjson`** — line TX (`\n`) + line RX (auto) +
  `json_lines` parser; `ndjson` adds `skip_empty`. RFC 8259 + the
  JSON Lines / NDJSON conventions above.
- **`cobs`** — COBS TX + COBS RX + `raw` parser (Cheshire/Baker).
- **`nmea0183`** — *NMEA 0183* marine sentence protocol. Start/End framing
  (`$`/`!` … `\r\n`) + `nmea` parser with `validate: true`. The `*XX` XOR
  checksum is computed over the bytes between `$`/`!` and `*`. NMEA 0183
  is a © NMEA standard; the spec is not freely redistributable. See the
  National Marine Electronics Association
  (<https://www.nmea.org/content/STANDARDS/NMEA_0183_Standard>) for the
  official document. The proprietary-sentence convention (`$P<vendor>...`)
  splits as `talker_id: "P"` + `sentence_type:` the rest.
- **`modbus_ascii`** — *Modbus over Serial Line: Specification and
  Implementation Guide* (Modbus Organization, formerly Modbus-IDA /
  Schneider Electric). ASCII mode: `:` start, `\r\n` end, hex-encoded body,
  trailing LRC (Longitudinal Redundancy Check) over the PDU. The `modbus_ascii`
  preset uses Start/End framing + `modbus_ascii` parser with `validate: true`.
  <https://www.modbus.org/specs.php>. Modbus is a © Modbus Organization
  specification; the spec is freely downloadable from the link above.

## Related standards (referenced by features not yet implemented)

- **HDLC** — ISO/IEC 13239 / RFC 1662 (*PPP in HDLC Framing*,
  <https://www.rfc-editor.org/rfc/rfc1662>). Tracked in FEATURES.md as a
  future framing mode with FCS-16 checksum.
- **Modbus RTU** — Modbus over Serial Line, RTU mode (binary frames with
  CRC-16). Tracked in FEATURES.md; the `checksums.rs` module will gain a
  CRC-16 function and the `Checksum` trait abstraction will return when
  there is a second checksum width to abstract over.

## Attribution

This references document cites spec sources for the implemented protocols.
No spec text is reproduced here. Trademarks and copyrights belong to their
respective owners (NMEA, Modbus Organization, IETF, IEEE, Stu Cheshire and
Mary Baker, the JSON Lines / NDJSON community).
