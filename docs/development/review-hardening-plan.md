# Review Hardening Plan

> Status: **Phase 0–6 open; all design decisions resolved 2026-07-05** (1B:
> drop-and-count · 0A: implement P-talker split · 3B: implement TX
> auto-checksum · 5C: fix the scan). Source: branch review of
> `feature-additions` (`e4069cb`…`f45f984`) done 2026-07-05, before the PR to
> `main`. Baseline: all suites green (645+ tests), clippy clean. This plan
> addresses every review finding — one verified crash bug, one data-loss
> design flaw, a set of duplication targets inside `src/framing.rs`, parser
> polish, doc drift, and repo housekeeping. **Phases 0, 1, and 4D are
> PR-blocking; the rest are hardening and may land in follow-ups.** Line
> numbers reference `f45f984`.

## Why

The protocol expansion plan (P0–P3, now removed from this directory) shipped
7 presets with strong test discipline, but the post-hoc review found issues
the test net did not catch: the NMEA parser **panics on real-world byte
sequences** (verified by reproduction), and the decode-error path **discards
all collected frames in `read`** — with `validate: true` being the default
for the `nmea0183` / `modbus_ascii` presets, a single corrupted sentence in a
burst throws away every good frame of the call. Beyond the two bugs, the new
decoder code re-introduced local copy-paste (the frame-emission block exists
three times) that the P3b dedup pass did not cover.

## Goals / non-goals

**Goals**

- Zero panics on arbitrary RX bytes, pinned by regression tests.
- No silent data loss on decode errors: a checksum failure must not destroy
  previously collected frames.
- Collapse the remaining intra-`framing.rs` duplication so the next protocol
  (HDLC / Modbus RTU) doesn't copy the same blocks a fourth time.
- Close every stale doc/description and repo-hygiene item found in review.

**Non-goals**

- No new protocols or presets (HDLC, Modbus RTU, MIDI stay in FEATURES.md).
  The one additive schema change is `TxFramingMode::Nmea` (3B), which serves
  the already-shipped `nmea0183` preset.
- No unification of `slip_decode` / `cobs_decode` state machines beyond the
  shared emission helper — the byte-level state machines are deliberately
  distinct (AGENTS.md).
- No performance work; every refactor here is correctness/maintainability.

---

## Phase 0 — Crash fix (PR-blocking)

### 0A. NMEA parser panics on non-ASCII address part

- **Bug (verified):** `NmeaParser::parse` slices `address_part[..2]` by byte
  index without a char-boundary check (`src/framing.rs:1760-1772`). The body
  is validated UTF-8, but byte index 2 need not be a char boundary. Feeding
  `[0x61, 0xC3, 0xA9, 0x2C, 0x78, 0x0A]` (`aé,x\n`) through
  `Line(auto)` + `Nmea` panics at `framing.rs:1766`:
  *"byte index 2 is not a char boundary"*. A noisy serial line can produce
  this — it only needs valid multi-byte UTF-8 plus a comma to pass both of
  the existing guards.
- **Fix:** NMEA-0183 is ASCII by spec. Add an ASCII guard next to the
  existing UTF-8 check (`framing.rs:1747-1750`):
  `if !body_str.is_ascii() { return Ok(ParsedFrame::Raw); }` — mirrors the
  ModbusAscii parser's non-ASCII → `Raw` behavior exactly.
- **Also (decided): implement the proprietary-sentence split.** The two
  branches at `framing.rs:1760-1768` (`len() >= 5` and `len() >= 2`) are
  **character-for-character identical** — the proprietary handling the doc
  comment promises was never implemented. Replace them with:
  - address starts with `P` → `talker_id: "P"`, `sentence_type:` the rest
    (e.g. `PGRMM` → `P` + `GRMM`), per the NMEA proprietary convention.
  - otherwise → `talker_id:` first 2 chars, `sentence_type:` the rest
    (unchanged standard split), with a single `len() >= 2` guard.
  This is a behavior change for `$P...` sentences (previously `PG` + `RMM`);
  the existing `nmea_parser_proprietary_sentence` test pins the OLD split
  and must be updated, plus a CHANGELOG note. The `ParsedFrame::Nmea`
  field docs (`framing.rs:565-570`) already describe proprietary talkers —
  align the wording.
- **Tests:**
  - Fixed regression test with the exact reproduction bytes → `Raw`, no
    panic.
  - `$PGRMM`-style proprietary sentence → `talker_id == "P"`,
    `sentence_type == "GRMM"`; standard sentences unchanged.
  - Strengthen `nmea_parser_never_panics_on_arbitrary_bytes`
    (`tests/proptest.rs:553`): random bytes almost never form valid
    multi-byte UTF-8 + comma, which is why the existing proptest missed
    this. Add a second strategy biased toward valid-UTF-8 strings with
    injected commas/`*` (e.g. `"\\PC*"` fragments joined with `,`).
- **Risk:** near-zero for the guard (narrows accepted input to
  spec-conformant ASCII; non-ASCII frames become `Raw` instead of a panic);
  low for the `P`-split (documented behavior change, pre-1.0).

---

## Phase 1 — Decode-error semantics: stop destroying collected data (PR-blocking)

Three layers of the same problem, fixed bottom-up. The presets default to
`validate: true`, so this is default-path behavior, not an edge case.

### 1A. `FrameDecoder::push` must not drop already-decoded frames on `Err`

- **Bug:** `push()` accumulates decoded frames in a local vec and returns
  `Err` on the first parse failure, dropping every frame already decoded
  from the same chunk (`src/framing.rs:1210`; same pattern inside
  `slip_decode` ~`:777` and `cobs_decode` ~`:901`).
- **Fix:** change the decode result so frames survive an error — e.g.
  `fn push(&mut self, chunk: &[u8]) -> (Vec<Frame>, Option<FrameDecodeError>)`
  or a small `PushOutcome { frames, error }` struct. Callers
  (`consume_frames` in `src/tools/rx_consume.rs:61`) dispatch the surviving
  frames to the sink *before* acting on the error.
- **Also:** `frame_count` is incremented before the parser runs
  (`framing.rs:1207`, and equivalents in slip/cobs), so an errored frame
  consumes an index and the next emitted frame has a gap. Increment only on
  successful emission so `Frame.index` stays contiguous. (Interacts with
  `skip_empty_preserves_frame_index_continuity` — extend that test.)

### 1B. Checksum mismatch: per-frame outcome, not stream-fatal (decided)

- **Problem:** with `validate: true` (the `nmea0183` / `modbus_ascii` preset
  default), one corrupted sentence aborts the whole read. Real NMEA streams
  (marine RS-422) routinely contain occasional corrupt sentences; the current
  semantics make the preset unusable on exactly the streams it targets.
- **Decided design — drop and count.** Demote `ChecksumMismatch` from a
  stream error to a per-frame outcome:
  - `validate: true` → **drop** the failing frame and count it in the
    existing `ReadResult.frames_dropped` field (already in the schema,
    currently always 0 from this path); emit a `warn!` log with the
    expected/received values. Dropped frames do not consume a `Frame.index`.
  - `validate: false` → unchanged (emit with `checksum_valid: Some(false)`).
  - `SlipInvalidEscape` / `CobsInvalidCode` remain stream-fatal — they mean
    the byte stream itself is corrupt, not just one frame's payload.
  - subscribe has no result object to carry a drop count — surface the drop
    count in the final stop notification (alongside `bytes_observed` etc.).
- **Docs:** update the `validate` field doc (`framing.rs:493-498`), the
  `checksum_valid` field docs (`framing.rs:573-578`, `594-598`), the
  read/subscribe tool descriptions, and CHANGELOG (behavior change, pre-1.0).

### 1C. `read` must return partial results on the remaining fatal errors

- **Bug:** `src/tools/helpers.rs:517-518` maps `FrameOutcome::DecodeError`
  to `return Err(...)` — discarding **all** `collected_frames` and
  accumulated bytes from the entire read call. `subscribe` already does this
  right (`src/tools/stream_ops.rs:536-540`): prior frames were emitted, and
  the stream stops with `stop_reason: framing_error`.
- **Fix:** mirror subscribe — finish the read with the frames/bytes
  collected so far and `stop_reason: framing_error` (the `finish!` macro +
  `RxStopController::framing_error` plumbing already exist), instead of a
  bare tool error.
- **Tests (pin the new semantics end-to-end):**
  - read: burst of N good NMEA sentences + 1 corrupted + M good in one chunk
    → all N+M good frames returned, `frames_dropped == 1`, contiguous
    `Frame.index`, no framing_error (per 1B).
  - read: SLIP invalid escape mid-stream → partial result with
    `stop_reason: framing_error`, frames before the error present.
  - decoder-level: multi-frame chunk where frame 2 fails → frame 1 survives
    in the returned vec; `Frame.index` contiguous across the error.

---

## Phase 2 — Deduplication inside `framing.rs`

Follow-up to the shipped P3b pass, targeting the blocks it did not cover.
Each item is independently shippable; existing tests (roundtrips, proptests,
preset-equivalence) guard all of them. Do Phase 1 first — 2A touches the same
code and is easier on top of the new push signature.

### 2A. Extract the frame-emission block (3 copies)

The `skip_empty`/`is_blank_frame` check + `frame_count` increment + parser
dispatch with error handling + `Frame` construction exists three times:
`push()` main loop (~`framing.rs:1206-1221`), `slip_decode` (~`:769-787`),
`cobs_decode` (~`:891-911`). Extract one helper (free function, since the
slip/cobs paths borrow `mode` mutably alongside):

```rust
fn emit_frame(
    data: Vec<u8>, frame_type: &'static str,
    frame_count: &mut usize, skip_empty: bool,
    parser: &Option<Box<dyn FrameParser>>,
) -> Result<Option<Frame>, FrameDecodeError>
```

This makes the Phase 1 error semantics (index contiguity, frame survival)
correct-by-construction in all three modes.

### 2B. Unify the three `flush_partial` blocks

`framing.rs:1404-1458` has three near-identical emit blocks differing only in
which buffer they drain (SLIP in-frame buf, COBS decoded buf, `self.buf`).
Resolve the buffer reference + frame type first, then one shared emission
tail.

### 2C. Line-matcher helper + merge duplicate matchers

- `match_line_lf` (`:1350`) and `match_line_cr` (`:1362`) are identical
  except for the split byte — merge into `match_line_byte(b: u8)`.
- The `if include_terminators { ..pos+n } else { ..pos }` + `drain(..pos+n)`
  pattern appears in all six matchers (`:1242-1252`, `:1262-1268`,
  `:1274-1283`, `:1323-1332`, `:1337-1346`, and the lf/cr/crlf trio) —
  extract `fn take_frame(&mut self, split_pos: usize, term_len: usize) ->
  Vec<u8>`.

### 2D. TX `LengthPrefixed`: collapse duplicated overflow checks

`framing.rs:398-430` duplicates the `len > 65535` check per endianness.
Match on `prefix_size` first for the range check, then on `endianness` only
for `to_be_bytes`/`to_le_bytes`.

### 2E. NMEA/Modbus checksum-validate ladder

- The `ChecksumMismatch { expected, received }` construction is repeated 3×
  in `NmeaParser` (`:1703`, `:1720`, `:1734`).
- The `if validate { Err(...) } else { Some(computed == received) }` ladder
  is structurally duplicated between `NmeaParser` (`:1718-1728`) and
  `ModbusAsciiParser` (`:1843-1853`). One shared helper
  (`fn check(validate: bool, computed: u8, received: u8) -> Result<Option<bool>, FrameDecodeError>`)
  covers both. **Coordinate with 1B** — if checksum failure becomes
  per-frame, this helper is where the semantics live.

### 2F. `checksums.rs`: drop the dead trait metadata

`Checksum::width()` and `validate()` are `#[allow(dead_code)]`, and
`compute()` allocates a `Vec<u8>` per frame for a single byte
(`src/checksums.rs:16-31`). Replace the trait with two free functions
returning `u8` (`xor_checksum`, `lrc`) — the shape the original expansion
plan specified. Reintroduce a trait when CRC-16/FCS-16 actually land and
there is a second checksum width to abstract over. Update the two consumers
and the module docs.

---

## Phase 3 — Parser polish (minor correctness)

### 3A. NMEA malformed-checksum branches: keep the parsed body

- **Issue:** a checksum with invalid hex chars and `validate: false` returns
  an `Nmea` variant with **empty** talker/type/fields (`framing.rs:1708-1713`)
  even when the body parses fine — inconsistent with the wrong-value case,
  which returns the full parse + `checksum_valid: Some(false)`.
- **Fix:** restructure so checksum evaluation yields only
  `checksum_valid` / an error, and body parsing always runs (parse body
  first, evaluate checksum after). The `< 2 hex chars` branch (`:1730-1740`)
  gets the same treatment.
- **Tests (both branches are currently untested):**
  - invalid hex chars in checksum, `validate: false` → full body parse,
    `checksum_valid: Some(false)`.
  - 1-hex-char checksum, `validate: false` → same.
  - both shapes with `validate: true` → per-1B semantics.

### 3B. NMEA TX checksum: implement auto-append (decided)

The `nmea0183` TX preset only wraps `$…\r\n` (`framing.rs:208-214`) — it
does not compute/append `*XX`, while the RX side *enforces* checksums by
default. **Decided: implement auto-append now.** The generic `StartEnd` mode
is the wrong home for protocol logic, so:

- **Design:** add a dedicated `TxFramingMode::Nmea` variant (parameterless,
  additive schema change; obeys the `check_schema!` guard). `encode()`
  produces `$<payload>*XX\r\n` with `XX` = XOR over the payload bytes
  (Phase 2F's `xor_checksum`).
  - If the payload already ends in `*HH` (two hex chars after a `*`), do
    **not** append a second checksum — validate the existing one and error
    on mismatch (`"TX NMEA checksum mismatch: payload declares HH, computed
    XX"`), so an agent supplying its own checksum can't silently send a bad
    frame.
  - If the payload already starts with `$` or `!`, don't double the start
    marker — use the payload's own leading char (AIS `!` sentences).
  - Reject payloads containing `\r`/`\n` (embedded terminators) and bytes
    outside printable ASCII, mirroring RX-side spec strictness.
- **Wire-up:** `preset_tx_framing(Nmea0183)` returns `TxFramingMode::Nmea`
  instead of the generic `StartEnd`. Explicit `tx_framing` still wins via
  the precedence ladder — an agent can always fall back to raw `StartEnd`.
- **Out of scope:** Modbus ASCII TX (LRC + hex-encoding of a binary PDU) is
  a bigger feature — already tracked as "Modbus ASCII TX auto-LRC" in
  FEATURES.md § Near-term (added 2026-07-05); nothing to do here.
- **Tests:** roundtrip through the preset (TX encode → RX decode with
  `validate: true` → `checksum_valid: Some(true)`); payload with correct
  existing `*XX` passes through un-doubled; payload with wrong existing
  `*XX` errors; `!`-payload keeps `!`; embedded `\r\n` rejected; update
  `preset_tx_framing_nmea0183_returns_start_end_dollar` and the
  `nmea0183_preset_equivalent_to_bare_config` TX half; tool-description +
  CHANGELOG.
- **Docs:** `write` tool description gains "the nmea0183 preset appends the
  `*XX` checksum automatically".

---

## Phase 4 — Docs and tool-description drift

### 4A. `subscribe` tool description is stale

`src/server.rs:367` still says framing is "(line, delimiter,
length-prefixed, SLIP, start/end marker)" — no COBS — mentions only SLIP in
the framing_error sentence, and (unlike the updated read/write descriptions)
never mentions the `protocol` presets at all, although `SubscribeArgs` has
the field. Bring it in line with the read description (`server.rs:282`),
including whatever Phase 1B changes.

### 4B. Protocol references doc (carried over from the expansion plan)

The one unfinished item of the shipped plan: add
`docs/protocols/references.md` citing each implemented/deferred spec (RFC
1055, RFC 8259, ndjson, `draft-ietf-pppext-cobs-00`, NMEA-0183 ©, Modbus ©,
RFC 1662, AT v2.0.0). Cite-only; nothing committed from `resources/`.

### 4C. CHANGELOG

Entries for 0A (panic fix), 1B/1C (decode-error behavior change), 3A/3B.

### 4D. User-facing protocol guide (PR-blocking)

The framing/preset/parser system is now the product's most differentiating
feature, and its only documentation is tool descriptions and doc comments.
Add `docs/protocols.md`:

- one section per preset (`at_command`, `slip`, `json_lines`, `cobs`,
  `ndjson`, `nmea0183`, `modbus_ascii`): what it wires up (TX framing / RX
  framing / parser), a concrete write/read example with the `protocol` field,
  and a sample decoded frame (`data` + `parsed` shape);
- the precedence ladder in one paragraph (explicit field > call protocol >
  connection default > connection protocol);
- checksum behavior per the Phase 1B/3A semantics (`validate`,
  `checksum_valid`, `frames_dropped`) — **write this after Phase 1 lands**
  so it documents the shipped behavior, not the old one;
- link it from the README § Documentation list.

Sits next to the planned `docs/protocols/references.md` (4B). **This gates
the feature-additions PR** together with Phases 0 and 1 — the branch should
not ship its headline feature undocumented.

---

## Phase 5 — Repo housekeeping

### 5A. Gitignore the `resources/` symlink

`resources` → `~/Nextcloud/Development-Resources/Serial-Protocols/` shows as
untracked; the expansion plan's premise was "gitignored symlink". Add
`/resources` to `.gitignore`. Never commit it.

### 5B. Commit the living development docs

`docs/development/protocol-matrix.md`, `FEATURES.md` updates, and this plan
are meant to be tracked — commit them with the cleanup (the shipped
handoffs and the completed expansion plan were removed 2026-07-05).

### 5C. `--version` arg scanning strictness (decided: fix)

`src/main.rs::parse_args` scans **all** argv for `-V`/`--version`, so
`serial-mcp --bind --version` prints the version instead of erroring.
**Decided: fix the scan** — stop at a `--` separator and don't treat a token
as the version flag when it is the value position of a preceding
value-taking option (`--bind`, `--transport`, `--max-*-buffered-bytes`).
Pin with `stdio_integration`-style tests: `--version` → version + exit 0;
`--bind --version` → argument error, not a version print.

---

## Phase 6 — Test consolidation (optional, lowest priority)

### 6A. Table-driven preset tests

The per-preset test blocks in `framing.rs` are heavy copy-paste: for each of
the 7 presets there are 3 preset-mapping tests + a tagged-object roundtrip +
an equivalence test (~35 near-identical functions). A table-driven test
(`[(preset, expected_tx, expected_rx, expected_parser); 7]`) collapses them
to three functions and makes adding preset #8 a one-line diff. Keep the
`assert_preset_roundtrip` helper (already extracted in P3b). Test-only; do
last, after Phases 0–3 have settled what the expected values are.

---

## Suggested PR sequence

1. **Phase 0** — NMEA panic fix + `P`-talker split + regression tests.
   Smallest, ships first.
2. **Phase 1** — decode-error semantics (1A → 1B → 1C) + tests + CHANGELOG.
3. **Phase 2** — dedup (2A–2F), one PR, pure refactor on green tests. 2F
   (checksum free functions) before 3B, which consumes `xor_checksum`.
4. **Phase 3A + 4** — checksum-branch polish + doc drift + the 4D protocol
   guide (4D after Phase 1, so it documents the shipped semantics), can
   share a PR.
5. **Phase 3B** — `TxFramingMode::Nmea` auto-checksum. Own PR: additive
   schema variant + preset TX change + its own test block.
6. **Phase 5** — housekeeping, rides with any of the above.
7. **Phase 6** — optional test consolidation, whenever convenient.

Phases 0, 1, and 4D gate the feature-additions PR; the rest may land as
follow-ups on `main` afterwards.
