#!/usr/bin/env bash
#
# Offline deterministic tests for scripts/install-nrfutil-ci.sh.
#
# Exercise the installer through NRFUTIL_BASE_URL=file:// fixtures. Cover the
# full pipeline: download, SHA-256 verification, strip-components=2 extraction,
# executable check, atomic publish, and staging cleanup without a live Nordic
# network dependency. Generate tiny archives in a temp directory at run time;
# commit no binaries.
#
# Cases cover missing required environment, SHA-256 mismatch, missing and
# non-executable bin/nrfutil, and successful publication of both executables
# with staging cleanup.
#
# Version and checksum environment values here are fixtures, not production
# pins. Production values live in .github/workflows/ci.yml and are verified by
# the native-sim job itself.

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
INSTALLER="$ROOT/scripts/install-nrfutil-ci.sh"
TMP_BASE="${TMPDIR:-/tmp}"

WORK="$(mktemp -d "$TMP_BASE/install-nrfutil-ci-test.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

FIXTURES="$WORK/fixtures"
BASE_URL="file://$FIXTURES"
DEST="$WORK/dest"

PASS=0
FAIL=0

step() { printf '\n--- %s\n' "$*"; }
ok()   { printf 'ok:   %s\n' "$*"; PASS=$((PASS + 1)); }
bad()  { printf 'FAIL: %s\n' "$*" >&2; FAIL=$((FAIL + 1)); }

# Fixture helpers.

# make_archive <out.tar.gz> <pkg-root> <binary-name> <file-mode>
# Builds the Nordic package layout <pkg>/data/bin/<binary> (plus a share
# file) with all files/dirs forced to the given mode.
make_archive() {
  local out=$1 pkg=$2 binary=$3 file_mode=$4
  local src
  src="$(mktemp -d "$WORK/tree.XXXXXX")"
  mkdir -p "$src/$pkg/data/bin" "$src/$pkg/data/share"
  : > "$src/$pkg/data/share/dummy.txt"
  : > "$src/$pkg/data/bin/$binary"
  chmod 755 "$src/$pkg" "$src/$pkg/data" "$src/$pkg/data/bin" "$src/$pkg/data/share"
  chmod "$file_mode" "$src/$pkg/data/bin/$binary" "$src/$pkg/data/share/dummy.txt"
  tar -czf "$out" -C "$src" "$pkg"
  rm -rf "$src"
}

sha_of() { sha256sum "$1" | awk '{print $1}'; }

staging_leftovers() {
  find "$TMP_BASE" -maxdepth 1 -name 'nrfutil-ci.*' -print 2>/dev/null
}

# Reset fixtures for each case.
new_fixtures() {
  rm -rf "$FIXTURES" "$DEST"
  mkdir -p "$FIXTURES/nrfutil" "$FIXTURES/nrfutil-sdk-manager"
}

step "missing required env vars fail closed"
new_fixtures
if env -i PATH="$PATH" bash "$INSTALLER" "$DEST" >"$WORK/out.log" 2>&1; then
  bad "installer must fail when required env vars are missing"
elif [ -e "$DEST" ]; then
  bad "no destination may be created when env vars are missing"
else
  ok "missing env vars rejected, no destination created"
fi

step "SHA-256 mismatch fails before extraction"
new_fixtures
printf 'this is not the real tarball\n' > "$FIXTURES/nrfutil/nrfutil-x86_64-unknown-linux-gnu-8.2.0.tar.gz"
printf 'nor this one\n' > "$FIXTURES/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1.tar.gz"
leftover_before=$(staging_leftovers)
if NRFUTIL_BASE_URL="$BASE_URL" \
     NRFUTIL_VERSION=8.2.0 NRFUTIL_SHA256=0000000000000000000000000000000000000000000000000000000000000000 \
     NRFUTIL_SDK_MANAGER_VERSION=1.16.1 NRFUTIL_SDK_MANAGER_SHA256=0000000000000000000000000000000000000000000000000000000000000000 \
     bash "$INSTALLER" "$DEST" >"$WORK/out.log" 2>&1; then
  bad "installer must fail on SHA-256 mismatch"
elif [ -e "$DEST" ]; then
  bad "no destination may be published when verification fails"
elif [ -n "$leftover_before" ] || [ -n "$(staging_leftovers)" ]; then
  bad "staging dir must be cleaned after failure"
else
  ok "SHA-256 mismatch rejected, no destination, staging cleaned"
fi

step "archive without bin/nrfutil fails at executable verification"
new_fixtures
make_archive "$FIXTURES/nrfutil/nrfutil-x86_64-unknown-linux-gnu-8.2.0.tar.gz" \
  "nrfutil-x86_64-unknown-linux-gnu-8.2.0" "not-nrfutil" 755
make_archive "$FIXTURES/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1.tar.gz" \
  "nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1" "nrfutil-sdk-manager" 755
core_sha=$(sha_of "$FIXTURES/nrfutil/nrfutil-x86_64-unknown-linux-gnu-8.2.0.tar.gz")
sdk_sha=$(sha_of "$FIXTURES/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1.tar.gz")
if NRFUTIL_BASE_URL="$BASE_URL" \
     NRFUTIL_VERSION=8.2.0 NRFUTIL_SHA256="$core_sha" \
     NRFUTIL_SDK_MANAGER_VERSION=1.16.1 NRFUTIL_SDK_MANAGER_SHA256="$sdk_sha" \
     bash "$INSTALLER" "$DEST" >"$WORK/out.log" 2>&1; then
  bad "installer must fail when bin/nrfutil is missing"
elif [ -e "$DEST" ]; then
  bad "no destination may be published when binaries are missing"
else
  ok "missing bin/nrfutil rejected, no destination"
fi

step "non-executable bin/nrfutil fails at executable verification"
new_fixtures
make_archive "$FIXTURES/nrfutil/nrfutil-x86_64-unknown-linux-gnu-8.2.0.tar.gz" \
  "nrfutil-x86_64-unknown-linux-gnu-8.2.0" "nrfutil" 644
make_archive "$FIXTURES/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1.tar.gz" \
  "nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1" "nrfutil-sdk-manager" 755
core_sha=$(sha_of "$FIXTURES/nrfutil/nrfutil-x86_64-unknown-linux-gnu-8.2.0.tar.gz")
sdk_sha=$(sha_of "$FIXTURES/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1.tar.gz")
if NRFUTIL_BASE_URL="$BASE_URL" \
     NRFUTIL_VERSION=8.2.0 NRFUTIL_SHA256="$core_sha" \
     NRFUTIL_SDK_MANAGER_VERSION=1.16.1 NRFUTIL_SDK_MANAGER_SHA256="$sdk_sha" \
     bash "$INSTALLER" "$DEST" >"$WORK/out.log" 2>&1; then
  bad "installer must fail when bin/nrfutil is not executable"
elif [ -e "$DEST" ]; then
  bad "no destination may be published when binaries are not executable"
else
  ok "non-executable bin/nrfutil rejected, no destination"
fi

step "happy path publishes both binaries and cleans staging"
new_fixtures
make_archive "$FIXTURES/nrfutil/nrfutil-x86_64-unknown-linux-gnu-8.2.0.tar.gz" \
  "nrfutil-x86_64-unknown-linux-gnu-8.2.0" "nrfutil" 755
make_archive "$FIXTURES/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1.tar.gz" \
  "nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1" "nrfutil-sdk-manager" 755
core_sha=$(sha_of "$FIXTURES/nrfutil/nrfutil-x86_64-unknown-linux-gnu-8.2.0.tar.gz")
sdk_sha=$(sha_of "$FIXTURES/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-1.16.1.tar.gz")
leftover_before=$(staging_leftovers)
if ! NRFUTIL_BASE_URL="$BASE_URL" \
     NRFUTIL_VERSION=8.2.0 NRFUTIL_SHA256="$core_sha" \
     NRFUTIL_SDK_MANAGER_VERSION=1.16.1 NRFUTIL_SDK_MANAGER_SHA256="$sdk_sha" \
     bash "$INSTALLER" "$DEST" >"$WORK/out.log" 2>&1; then
  cat "$WORK/out.log" >&2
  bad "happy path must succeed"
else
  if [ ! -x "$DEST/bin/nrfutil" ]; then
    bad "dest/bin/nrfutil missing or not executable"
  elif [ ! -x "$DEST/bin/nrfutil-sdk-manager" ]; then
    bad "dest/bin/nrfutil-sdk-manager missing or not executable"
  elif [ -n "$leftover_before" ] || [ -n "$(staging_leftovers)" ]; then
    bad "staging dir must be removed after successful publish"
  else
    ok "both binaries published under dest/bin and staging cleaned"
  fi
fi

printf '\n==== install-nrfutil-ci tests: %d passed, %d failed ====\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ]
