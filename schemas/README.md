# Schema provenance

Every file in this directory is a verbatim copy of an upstream JSON Schema
document. No downloaded blob is ever edited: when upstream changes, re-fetch
the exact bytes with the refresh script and record the new checksum here.

| File | Source URI | Vendored (UTC) | SHA-256 |
| --- | --- | --- | --- |
| `claude-code-settings.schema.json` | https://json.schemastore.org/claude-code-settings.json | 2026-06-04 | `22ffdfc7013b40b9fefdfd9df4af889a096ed0c5bf23607484526f291439b8f8` |
| `codex-config.schema.json` | https://developers.openai.com/codex/config-schema.json | 2026-06-04 | `a974e094f94f09cb58c5da4b0789d7f39aa1a4faaf3837a00e3b41944efdb6b8` |
| `opencode.schema.json` | https://opencode.ai/config.json | 2026-06-04 | `32454121862ca939365f5a1dfaafd359b9e2416882fd07493b750f4457536591` |
| `models-dev-model.schema.json` | https://models.dev/model-schema.json | 2026-08-01 | `13e4a274b069f3640bfe2880d621db188f552c49d546bfaf47d15534f0fcd095` |

`models-dev-model.schema.json` backs the four
`https://models.dev/model-schema.json#/$defs/Model` refs in
`opencode.schema.json`. The test harness registers it in memory under its
original URI, so the opencode schema is never rewritten to a local path.

## Update procedure

1. Run `scripts/update-config-schemas.sh`. It re-fetches all four schemas with
   fail-fast `curl`, prints the new SHA-256 of the models.dev schema, and runs
   the focused validation test suite.
2. Copy the printed models.dev checksum into the table above (this is the one
   manual step — the script cannot edit this README for you).
3. Review the diff; every downloaded blob must remain byte-identical to
   upstream (whitespace included). Reject and re-fetch any reformatting.
4. Run `cargo test --locked --test config_schema_validation` (and
   `-- --ignored` if you want the networked latest-upstream check).

## Validation contract

- `tests/config_schema_validation.rs::example_configs_match_vendored_schemas`
  validates exactly three fixture configs (Claude Code, Codex, opencode)
  against the vendored schemas, fully offline, and **fails** if any schema or
  fixture file is missing or malformed. There is no skip path.
- The ignored `example_configs_match_latest_upstream_schemas` test is the only
  networked check and intentionally stays that way.
