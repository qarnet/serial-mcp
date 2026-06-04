#!/usr/bin/env bash
set -euo pipefail

ROOT="$(dirname "$(realpath "$0")")"
DIR="$ROOT/example-configs"
CACHE_DIR="$ROOT/.schemas"
FAIL=0

JS="$HOME/.cargo/bin/jsonschema-cli"
if ! command -v "$JS" &>/dev/null; then
    echo "jsonschema-cli not found. Install: cargo install jsonschema-cli"
    exit 1
fi

fetch_schema() {
    local url="$1"
    local name="$2"
    mkdir -p "$CACHE_DIR"
    local dest="$CACHE_DIR/$name"
    if [ ! -f "$dest" ]; then
        echo "    fetching $url"
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
    local abs_schema
    local abs_file
    abs_schema="$(realpath "$schema")"
    abs_file="$(realpath "$file")"
    echo "  $file"
    if "$JS" validate "$abs_schema" -i "$abs_file" --errors-only 2>&1; then
        echo "    valid"
    else
        echo "    INVALID"
        FAIL=1
    fi
}

echo "=== Example config validation ==="

echo ""
echo "opencode:"
SCHEMA=$(fetch_schema "https://opencode.ai/config.json" "opencode.json" || true)
if [ -n "$SCHEMA" ]; then
    validate "$SCHEMA" "$DIR/opencode.json"
fi

echo ""
echo "Claude:"
SCHEMA=$(fetch_schema "https://json.schemastore.org/claude-code-settings.json" "claude-code.json" || true)
if [ -n "$SCHEMA" ]; then
    validate "$SCHEMA" "$DIR/claude_code.json"
    validate "$SCHEMA" "$DIR/claude_desktop.json"
fi

echo ""
echo "No published schema — JSON parse check only:"
for f in "$DIR"/cursor.json "$DIR"/vscode.json "$DIR"/zed.json; do
    echo "  $f"
    if python3 -m json.tool "$f" > /dev/null 2>&1; then
        echo "    valid JSON"
    else
        echo "    INVALID JSON"
        FAIL=1
    fi
done

echo ""
if [ "$FAIL" -eq 0 ]; then
    echo "All example configs valid."
else
    echo "Some configs failed validation."
    exit 1
fi
