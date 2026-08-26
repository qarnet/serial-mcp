# Protocol support matrix

This matrix lists every serial protocol with a specification in the gitignored
`resources/` symlink. Longer-horizon items are tracked in
[FEATURES.md](FEATURES.md).

Specs are cited, never committed. See the License column. Source files are
named relative to `resources/`.

## Legend

- Status values are `shipped`, `deferred` (FEATURES.md), and `reference` (not a
  serial-framing target).
- Effort is the relative implementation cost for the current frame pipeline
  (`src/framing/`).
- License values are `free` for redistributable RFCs, IETF drafts, or explicitly
  free specifications, and `©` for copyrighted, cite-only specifications.

## Serial protocols

| Protocol | Spec source (`resources/`) | Standard | What it is | Implementation approach | New primitive | Checksum | Status | Effort | License |
|---|---|---|---|---|---|---|---|---|---|
| **SLIP** | `rfc1055.txt` | RFC 1055 | Byte-stuffed IP-over-serial framing | `Slip` framing mode + `slip` preset | `Slip` preset (wiring) | none | shipped (0.7.0; preset `2e89343`) | trivial | free |
| **AT commands** | `AT-Command-Specification-v2.0.0.pdf` | 3GPP/V.250 family | Modem command/response + URCs | `at_command` preset (Line CR + AT parser) | — (exists) | none | shipped (0.7.0) | — | © |
| **JSON lines** | `rfc8259.txt` | RFC 8259 | One JSON value per frame | `JsonLines` parser + `json_lines` preset | `JsonLines` preset (wiring) | none | shipped (0.6.0; preset `2e89343`) | trivial | free |
| **NDJSON** | `README.md` | ndjson 1.0 | Newline-delimited JSON, skips blank lines | `Line(auto)` + `JsonLines` + `skip_empty`; `ndjson` preset | `skip_empty` flag | none | shipped (`111a757`) | trivial | free |
| **COBS** | `draft-ietf-pppext-cobs-00.txt` | IETF draft | Consistent Overhead Byte Stuffing, `0x00` delimiter | new framing mode modeled on `Slip`; `cobs` preset | `Cobs` TX/RX mode | none | shipped (`f85c1ab`+fixes) | low | free |
| **NMEA-0183** | `NMEA0183.pdf` | NMEA 0183 | `$`/`!` sentences, comma fields, `*XX` checksum | `StartEnd` (`$`/`!` multi-marker) + `Nmea` parser; `nmea0183` preset | `Nmea` parser, XOR checksum | `*XX` XOR | shipped (`dd23f61`) | medium | © |
| **Modbus ASCII** | `modbusoverserial.pdf` | Modbus | `:`…`\r\n` hex frames, LRC | `StartEnd` + `ModbusAscii` parser; `modbus_ascii` preset | `ModbusAscii` parser, LRC | LRC | shipped (`f738a60`) | medium | © |
| **PPP / HDLC framing** | `rfc1662.txt` | RFC 1662 | `0x7E`-flag framing, `0x7D` escape, FCS-16 | new framing mode + FCS validation | `HdlcLike` mode, `fcs16` | FCS-16 | deferred | medium-high | free |
| **Modbus RTU** | `modbusoverserial.pdf`, `modbusprotocolspecification.pdf` | Modbus | Binary, **silence-delimited** frames, CRC-16 | needs a *new framing concept* (inter-frame gap timing) + PDU parser | gap-delimited framing, `crc16_modbus` | CRC-16 | deferred *(needs design)* | high | © |
| **MIDI** | `M1_v4-2-1_MIDI_1-0_Detailed_Specification_96-1-4.pdf` | MIDI 1.0 | Status/data bytes, running status, variable length | new stateful parser | `Midi` parser | none | deferred | high | © |
| **Firmata** | `protocol-master/` | Firmata | MIDI-message-based + SysEx; large feature surface (i2c/spi/onewire/stepper/servo/…) | own epic atop MIDI framing | many | none | deferred | very high | free |

## Reference-only specs (not serial-framing targets)

These specifications are present in the local library for context. They are
not protocol-preset candidates. They are listed so the matrix is complete.

| Spec | Source (`resources/`) | Standard | Why it's here, not a target |
|---|---|---|---|
| **WebSocket** | `rfc6455.txt` | RFC 6455 | Network transport; relevant to the HTTP MCP transport, not serial framing. |
| **Media types** | `rfc6838.txt` | RFC 6838 | Reference for content-type / MIME handling (e.g. resource blobs), not a wire protocol. |
| **HDLC over L2TP** | `rfc4349.txt` | RFC 4349 | HDLC-family reference. The HDLC framing we'd actually implement is RFC 1662; this tunneling variant is out of scope. |

## Notes

- The `checksums` module (`src/checksums.rs`) is the shared home for the XOR
  checksum, LRC, and future `crc16_modbus` and FCS-16 primitives. Checksum
  mismatches with `validate: true` are dropped and counted per frame. The
  `stream-fatal` framing errors use the `FramingError` stop reason.
- Add new framing modes (`Cobs` shipped, `HdlcLike` later) symmetrically to
  `RxFramingMode` and `TxFramingMode`, then wire them into
  `preset_tx_framing` and `preset_rx_framing`. New parsers (`Nmea` and
  `ModbusAscii` shipped, `Midi` later) extend `ParserType` and
  `preset_rx_parser`.
- The `ShellPrompt` parser shipped in 0.6.0 has no formal spec. It is a
  heuristic and is intentionally omitted from this matrix.
- The `protocol:` knob is uniform. All 7 advertised protocols (`at_command`,
  `slip`, `json_lines`, `cobs`, `ndjson`, `nmea0183`, `modbus_ascii`) are
  selectable as first-class presets.
- Every addition is additive to the JSON schema through new enum variants or
  optional fields, preserving the path to a stable 1.0.
