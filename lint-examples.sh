#!/usr/bin/env bash
set -euo pipefail

ROOT="$(dirname "$(realpath "$0")")"
DIR="$ROOT/example-configs"
CACHE_DIR="$ROOT/.schemas"
FAIL=0

# Centralized schema URLs — add new schemas here.
declare -A SCHEMAS=(
    [claude_code_settings]="https://json.schemastore.org/claude-code-settings.json"
    [opencode_config]="https://opencode.ai/config.json"
    [codex_config]="https://developers.openai.com/codex/config-schema.json"
)

declare -A VALIDATES=(
    [claude_code_settings]="$DIR/claude_code.json"
    [opencode_config]="$DIR/opencode.json"
    [codex_config]="$DIR/codex.json"
)

JS="$HOME/.cargo/bin/jsonschema-cli"
if ! command -v "$JS" &>/dev/null; then
    echo "jsonschema-cli not found. Install: cargo install jsonschema-cli"
    exit 1
fi

fetch_schema() {
    local url="$1"
    local name="$2"
    mkdir -p "$CACHE_DIR"
    local dest="$CACHE_DIR/$name.json"
    if [ ! -f "$dest" ]; then
        echo "    fetching $url" >&2
        curl -sSL "$url" -o "$dest" || {
            echo "    WARNING: could not fetch schema, skipping"
            return 1
        }
    fi
    echo "$dest"
}

validate() {
    local schema="$1"
    local file="$2"
    echo "  $file"
    if "$JS" validate "$schema" -i "$file" --errors-only 2>&1; then
        echo "    valid"
    else
        echo "    INVALID"
        FAIL=1
    fi
}

echo "=== Example config validation ==="

for name in "${!SCHEMAS[@]}"; do
    url="${SCHEMAS[$name]}"
    files="${VALIDATES[$name]}"

    echo ""
    echo "$name ($url):"
    SCHEMA_FILE=$(fetch_schema "$url" "$name" || true)
    if [ -z "$SCHEMA_FILE" ]; then
        continue
    fi

    if [ -z "$files" ]; then
        echo "  (no config files mapped — schema fetched but not applied)"
        continue
    fi

    for f in $files; do
        validate "$(realpath "$SCHEMA_FILE")" "$(realpath "$f")"
    done
done

echo ""
if [ "$FAIL" -eq 0 ]; then
    echo "All example configs valid."
else
    echo "Some configs failed validation."
    exit 1
fi
