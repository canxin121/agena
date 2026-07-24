#!/usr/bin/env bash
# Uninstall the user-level Agena service and optionally its files.
set -euo pipefail

REMOVE_INSTALL_DIR=0
INSTALL_DIR="${HOME}/agena"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --remove-install-dir) REMOVE_INSTALL_DIR=1; shift ;;
    --install-dir) INSTALL_DIR="${2:-}"; shift 2 ;;
    -h|--help)
      cat <<'EOF'
Usage: uninstall-service.sh [--install-dir DIR] [--remove-install-dir]
EOF
      exit 0
      ;;
    *)
      echo "ERROR: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

OS="$(uname -s)"
if [[ "$OS" == "Linux" ]]; then
  systemctl --user disable --now agena.service >/dev/null 2>&1 || true
  rm -f "$HOME/.config/systemd/user/agena.service"
  systemctl --user daemon-reload >/dev/null 2>&1 || true
elif [[ "$OS" == "Darwin" ]]; then
  PLIST_FILE="$HOME/Library/LaunchAgents/cn.cxits.agena.plist"
  launchctl unload "$PLIST_FILE" >/dev/null 2>&1 || true
  rm -f "$PLIST_FILE"
fi

if [[ "$REMOVE_INSTALL_DIR" -eq 1 ]]; then
  rm -rf "$INSTALL_DIR"
fi

echo "Agena background service removed."
