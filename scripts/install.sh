#!/usr/bin/env bash
# unspace-dock installer.
#
# Fresh install (one-liner on the Pi):
#   curl -fsSL https://YOUR-URL/install.sh | sudo bash -s -- \
#     --api-key ysk_KEY --dock-id DOCK_42
#
# Update binary in place (keeps existing config):
#   curl -fsSL https://YOUR-URL/install.sh | sudo bash
#
# The default download URL points at $DEFAULT_RELEASE_URL below; replace it
# with your hosting URL before publishing this script, or override at runtime
# with --release-url URL or UNSPACE_RELEASE_URL=URL.

set -euo pipefail

DEFAULT_RELEASE_URL="https://releases.unspace.com/unspace-dock-linux-arm64"

RELEASE_URL="${UNSPACE_RELEASE_URL:-$DEFAULT_RELEASE_URL}"
SHA_URL="${UNSPACE_RELEASE_SHA_URL:-${RELEASE_URL}.sha256}"
BIN_PATH="/usr/local/bin/unspace-dock"
CONFIG_PATH="${UNSPACE_CONFIG_PATH:-/etc/unspace/config.json}"

API_KEY="${UNSPACE_API_KEY:-}"
DOCK_ID="${UNSPACE_DOCK_ID:-}"
SKIP_SHA="${UNSPACE_SKIP_SHA:-}"

usage() {
  cat <<USAGE
unspace-dock installer

Fresh install:
  curl -fsSL https://YOUR-URL/install.sh | sudo bash -s -- \\
    --api-key ysk_KEY --dock-id DOCK_42

Update binary in place (keeps existing config):
  curl -fsSL https://YOUR-URL/install.sh | sudo bash

Options:
  --api-key  ysk_KEY      API key   (required on first install; ignored on update)
  --dock-id  DOCK_ID      Dock id   (required on first install; ignored on update)
  --release-url URL       Override the binary download URL
  --no-verify             Skip SHA256 checksum verification
  -h | --help             Show this help

Environment overrides:
  UNSPACE_API_KEY, UNSPACE_DOCK_ID, UNSPACE_RELEASE_URL,
  UNSPACE_RELEASE_SHA_URL, UNSPACE_SKIP_SHA, UNSPACE_CONFIG_PATH
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --api-key)     API_KEY="${2:?--api-key needs a value}"; shift 2 ;;
    --dock-id)     DOCK_ID="${2:?--dock-id needs a value}"; shift 2 ;;
    --release-url) RELEASE_URL="${2:?--release-url needs a value}"; SHA_URL="${RELEASE_URL}.sha256"; shift 2 ;;
    --no-verify)   SKIP_SHA=1; shift ;;
    -h|--help)     usage; exit 0 ;;
    *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

log() { printf '[unspace-dock-install] %s\n' "$*"; }
fail() { printf '[unspace-dock-install] ERROR: %s\n' "$*" >&2; exit 1; }

# Pre-flight
[ "$(id -u)" -eq 0 ]               || fail "run as root (sudo)."
[ "$(uname -s)" = "Linux" ]        || fail "Linux only (got $(uname -s))."
[ "$(uname -m)" = "aarch64" ]      || fail "aarch64 / ARM64 only (got $(uname -m))."
command -v systemctl >/dev/null    || fail "systemd is required."
command -v curl      >/dev/null    || fail "curl is required."
command -v sha256sum >/dev/null    || fail "sha256sum is required."

# Decide whether this is a fresh install or an in-place update.
if [ -f "$CONFIG_PATH" ] && [ -z "$API_KEY" ] && [ -z "$DOCK_ID" ]; then
  MODE=update
  log "existing config at $CONFIG_PATH found; running in UPDATE mode."
else
  MODE=install
  [ -n "$API_KEY" ] || fail "--api-key is required for first install (or set UNSPACE_API_KEY)."
  [ -n "$DOCK_ID" ] || fail "--dock-id is required for first install (or set UNSPACE_DOCK_ID)."
fi

# Download + verify
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
log "downloading $RELEASE_URL"
curl -fsSL --retry 3 "$RELEASE_URL" -o "$TMP/unspace-dock"

if [ -z "$SKIP_SHA" ]; then
  if curl -fsSL --retry 3 "$SHA_URL" -o "$TMP/expected.sha256" 2>/dev/null; then
    expected="$(awk 'NR==1{print $1}' "$TMP/expected.sha256")"
    actual="$(sha256sum "$TMP/unspace-dock" | awk '{print $1}')"
    if [ "$expected" != "$actual" ]; then
      fail "SHA256 mismatch (expected $expected, got $actual)."
    fi
    log "sha256 verified"
  else
    log "WARN: no sha256 file at $SHA_URL; skipping verification."
  fi
fi

# Install binary
install -m 0755 "$TMP/unspace-dock" "$BIN_PATH"
log "installed: $("$BIN_PATH" --version)"

# Configure (fresh install) or just restart (update)
if [ "$MODE" = install ]; then
  "$BIN_PATH" install --api-key "$API_KEY" --dock-id "$DOCK_ID"
else
  systemctl restart unspace-dock
  log "service restarted with new binary"
fi

# Verify
sleep 2
if "$BIN_PATH" healthcheck; then
  log "unspace-dock is running."
  log "Status:  systemctl status unspace-dock"
  log "Logs:    journalctl -u unspace-dock -f"
else
  log "WARN: healthcheck failed; inspect 'journalctl -u unspace-dock -f' or run 'unspace-dock healthcheck' for details."
  exit 1
fi
