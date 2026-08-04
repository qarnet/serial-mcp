#!/usr/bin/env bash
# Provision pinned nrfutil core + sdk-manager into an isolated directory.
#
# This replaces the old native-sim CI flow, which curl'd an UNVERSIONED
# launcher into /usr/local/bin, chmod'd and sudo-mv'd whatever bytes came
# back (a 160-byte HTTP 504 HTML page when Nordic's CDN timed out), and then
# floated the core module and sdk-manager through `nrfutil self-upgrade` /
# `nrfutil install sdk-manager` on every run. When the launcher download
# silently succeeded with HTML, the next `nrfutil self-upgrade` tried to
# shell-parse that HTML and the whole job died.
#
# This installer instead:
#   * downloads exact versioned Nordic package tarballs (never the
#     unversioned launcher), with curl failing closed (--fail-with-body)
#     and bounded retries for transient network/server errors, a connect
#     timeout, and an overall per-transfer timeout;
#   * verifies the SHA-256 of every archive before anything is extracted;
#   * stages both archives in a temp dir, verifies bin/nrfutil and
#     bin/nrfutil-sdk-manager exist and are executable, and only then
#     publishes the destination with a single atomic rename;
#   * cleans the staging dir on any failure via a trap;
#   * never uses sudo, never executes downloaded content, never pipes a
#     download into a shell.
#
# Usage:
#   scripts/install-nrfutil-ci.sh <destination-dir>
#
# Required environment:
#   NRFUTIL_VERSION                exact core version, e.g. 8.2.0
#   NRFUTIL_SHA256                 sha256 of the core tarball
#   NRFUTIL_SDK_MANAGER_VERSION    exact sdk-manager version, e.g. 1.16.1
#   NRFUTIL_SDK_MANAGER_SHA256     sha256 of the sdk-manager tarball
#
# Optional environment:
#   NRFUTIL_BASE_URL               package base URL (default: official
#                                  Nordic Artifactory "packages" tree)
#   NRFUTIL_CURL_RETRIES           bounded retries for transient errors
#   NRFUTIL_CURL_RETRY_DELAY       seconds between retries
#   NRFUTIL_CURL_CONNECT_TIMEOUT   seconds to establish a connection
#   NRFUTIL_CURL_MAX_TIME          max seconds for one transfer attempt
#
# Both archives use the known layout <pkg>/data/bin/<binary>, so
# `tar --strip-components=2` lands both binaries in destination/bin/.

set -euo pipefail

dest=${1:?usage: install-nrfutil-ci.sh <destination-dir>}

: "${NRFUTIL_VERSION:?NRFUTIL_VERSION must be set}"
: "${NRFUTIL_SHA256:?NRFUTIL_SHA256 must be set}"
: "${NRFUTIL_SDK_MANAGER_VERSION:?NRFUTIL_SDK_MANAGER_VERSION must be set}"
: "${NRFUTIL_SDK_MANAGER_SHA256:?NRFUTIL_SDK_MANAGER_SHA256 must be set}"

base_url=${NRFUTIL_BASE_URL:-https://files.nordicsemi.com/artifactory/swtools/external/nrfutil/packages}
curl_retries=${NRFUTIL_CURL_RETRIES:-5}
curl_retry_delay=${NRFUTIL_CURL_RETRY_DELAY:-3}
curl_connect_timeout=${NRFUTIL_CURL_CONNECT_TIMEOUT:-15}
curl_max_time=${NRFUTIL_CURL_MAX_TIME:-300}

nrfutil_url="${base_url}/nrfutil/nrfutil-x86_64-unknown-linux-gnu-${NRFUTIL_VERSION}.tar.gz"
sdk_manager_url="${base_url}/nrfutil-sdk-manager/nrfutil-sdk-manager-x86_64-unknown-linux-gnu-${NRFUTIL_SDK_MANAGER_VERSION}.tar.gz"

stage=$(mktemp -d "${TMPDIR:-/tmp}/nrfutil-ci.XXXXXX")
trap 'if [ -n "${stage:-}" ]; then rm -rf "$stage"; fi' EXIT

log() { printf '[install-nrfutil-ci] %s\n' "$*" >&2; }

fetch() {
  local url=$1 out=$2
  curl --fail-with-body --show-error --location \
    --connect-timeout "$curl_connect_timeout" \
    --max-time "$curl_max_time" \
    --retry "$curl_retries" \
    --retry-delay "$curl_retry_delay" \
    --retry-connrefused \
    --output "$out" "$url"
}

verify_sha256() {
  local expected=$1 file=$2
  local actual
  actual=$(sha256sum "$file" | awk '{print $1}')
  if [ "$actual" != "$expected" ]; then
    log "sha256 mismatch for $(basename "$file"): expected ${expected}, got ${actual}"
    return 1
  fi
  log "sha256 ok for $(basename "$file")"
}

log "provisioning nrfutil ${NRFUTIL_VERSION} + sdk-manager ${NRFUTIL_SDK_MANAGER_VERSION} into ${dest}"

log "downloading ${nrfutil_url}"
fetch "$nrfutil_url" "$stage/nrfutil.tar.gz"
verify_sha256 "$NRFUTIL_SHA256" "$stage/nrfutil.tar.gz"

log "downloading ${sdk_manager_url}"
fetch "$sdk_manager_url" "$stage/sdk-manager.tar.gz"
verify_sha256 "$NRFUTIL_SDK_MANAGER_SHA256" "$stage/sdk-manager.tar.gz"

tar -xzf "$stage/nrfutil.tar.gz" -C "$stage" --strip-components=2
tar -xzf "$stage/sdk-manager.tar.gz" -C "$stage" --strip-components=2

for exe in bin/nrfutil bin/nrfutil-sdk-manager; do
  if [ ! -x "$stage/$exe" ]; then
    log "error: ${exe} missing or not executable after extraction" >&2
    exit 1
  fi
done
log "extracted binaries verified"

# Publish with a single atomic rename; a failure before this point leaves
# no partial destination (the trap removes the staging dir).
mkdir -p "$(dirname "$dest")"
rm -rf "$dest"
mv "$stage" "$dest"
stage=

log "installed into ${dest}"
