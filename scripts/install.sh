#!/usr/bin/env bash
set -euo pipefail

INSTALL_DIR="${UNSPACE_AGENT_INSTALL_DIR:-/opt/unspace/drone-dock-agent}"
PACKAGE_URL="${UNSPACE_AGENT_PACKAGE_URL:-https://github.com/unspacellc/aldai-drone-script/archive/refs/heads/main.zip}"
PYTHON_BIN="${PYTHON_BIN:-python3}"
SUDO=""

if [ "$(id -u)" -ne 0 ]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "This installer needs root permissions. Re-run with sudo or install sudo." >&2
    exit 1
  fi
  SUDO="sudo"
fi

if ! command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  echo "python3 is required before installing the Unspace agent." >&2
  exit 1
fi

$SUDO mkdir -p "$INSTALL_DIR"
$SUDO "$PYTHON_BIN" -m venv "$INSTALL_DIR/.venv"
$SUDO "$INSTALL_DIR/.venv/bin/python" -m pip install --upgrade pip
$SUDO "$INSTALL_DIR/.venv/bin/python" -m pip install --upgrade "$PACKAGE_URL"
$SUDO ln -sf "$INSTALL_DIR/.venv/bin/unspace" /usr/local/bin/unspace
$SUDO ln -sf "$INSTALL_DIR/.venv/bin/drone-dock-agent" /usr/local/bin/drone-dock-agent

cat <<EOF
Unspace CLI installed.

Next command:
  sudo unspace install --api-key ysk_KEY --yard-id YARD_1 --dock-id DOCK_42

Optional status check:
  sudo unspace status
EOF
