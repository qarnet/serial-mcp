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
