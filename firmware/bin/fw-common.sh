#!/usr/bin/env bash

_fw_script_dir() {
  cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd
}

_fw_firmware_dir() {
  dirname -- "$(_fw_script_dir)"
}

_fw_repo_dir() {
  dirname -- "$(_fw_firmware_dir)"
}

_fw_build_dir() {
  if [ -n "${SERIAL_MCP_FW_BUILD_DIR:-}" ]; then
    printf '%s\n' "$SERIAL_MCP_FW_BUILD_DIR"
  else
    printf '%s\n' "$(_fw_repo_dir)/build"
  fi
}

# Guard: ensure the NCS/Zephyr dev shell environment is loaded.
# Exits with a clear message when west or ZEPHYR_BASE is missing.
_fw_require_env() {
  local missing=""

  if ! command -v west >/dev/null 2>&1; then
    missing="west not found on PATH"
  fi

  if [ -z "${ZEPHYR_BASE:-}" ]; then
    if [ -n "$missing" ]; then
      missing="$missing; ZEPHYR_BASE not set"
    else
      missing="ZEPHYR_BASE not set"
    fi
  elif [ ! -d "$ZEPHYR_BASE" ]; then
    if [ -n "$missing" ]; then
      missing="$missing; ZEPHYR_BASE ($ZEPHYR_BASE) does not exist"
    else
      missing="ZEPHYR_BASE ($ZEPHYR_BASE) does not exist"
    fi
  fi

  if [ -n "$missing" ]; then
    printf 'firmware tool error: %s\n' "$missing" >&2
    printf '\nEnter the dev shell first:\n' >&2
    printf '  cd "$(git rev-parse --show-toplevel)" && nix develop\n' >&2
    printf '  # or with direnv:\n' >&2
    printf '  cd "$(git rev-parse --show-toplevel)" && direnv allow\n' >&2
    exit 1
  fi
}
