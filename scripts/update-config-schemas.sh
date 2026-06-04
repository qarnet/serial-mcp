#!/usr/bin/env bash
set -euo pipefail

mkdir -p schemas

curl -fsSL \
  https://json.schemastore.org/claude-code-settings.json \
  -o schemas/claude-code-settings.schema.json

curl -fsSL \
  https://developers.openai.com/codex/config-schema.json \
  -o schemas/codex-config.schema.json

curl -fsSL \
  https://opencode.ai/config.json \
  -o schemas/opencode.schema.json

cargo test --locked --test config_schema_validation
