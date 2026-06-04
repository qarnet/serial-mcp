#!/bin/bash
# Run all cargo-fuzz targets against serial-mcp.
#
# Usage:
#   ./fuzz/run.sh               # 30 seconds per target (default)
#   ./fuzz/run.sh 300           # 5 minutes per target
#   ./fuzz/run.sh 1800          # 30 minutes per target
#
# Requires:
#   - rustup nightly toolchain:  rustup toolchain install nightly
#   - cargo-fuzz:                cargo install cargo-fuzz
#
# On NixOS/Ubuntu hybrid: if PATH gives you stable-from-Nix, set:
#   export PATH="$HOME/.rustup/toolchains/nightly-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
#   export LD_LIBRARY_PATH="$(find /nix/store -maxdepth 4 -name 'libstdc++.so.6' -not -path '*-static*' -exec dirname {} \; | head -1):$LD_LIBRARY_PATH"

set -euo pipefail
cd "$(dirname "$0")/.."

SECONDS_PER="${1:-30}"

TARGETS=(tool_call_json codec_roundtrip clamp_bounds)

for target in "${TARGETS[@]}"; do
    echo "=== fuzz/${target} (${SECONDS_PER}s) ==="
    cargo fuzz run "$target" -- -max_total_time="$SECONDS_PER"
    echo ""
done

echo "All fuzz targets completed."
