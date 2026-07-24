#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: install-service.sh [options]

Options:
  --archive PATH_OR_URL    Backend package archive (.tar.gz)
  --repo OWNER/REPO        GitHub repo for release downloads (default: canxin121/agena)
  --version VERSION        Release version, e.g. 0.1.0 or v0.1.0
  --install-dir DIR        Install directory (default: ~/agena/studio)
  --host HOST              Backend host (default: 127.0.0.1)
  --port PORT              Backend port (default: 3210)
  --ui-password PASSWORD   Optional UI password
  --workspace-root PATH    Optional workspace root
  --database-path PATH     Optional SQLite database path
  --database-url URL       Optional database URL
  --set KEY=VALUE          Additional Agena overrides (repeatable)
  -h, --help               Show help
EOF
}

REPO="canxin121/agena"
VERSION=""
ARCHIVE=""
INSTALL_DIR="${HOME}/agena/studio"
HOST="127.0.0.1"
PORT="3210"
UI_PASSWORD=""
WORKSPACE_ROOT=""
DATABASE_PATH=""
DATABASE_URL=""
SETS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --archive) ARCHIVE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --install-dir) INSTALL_DIR="${2:-}"; shift 2 ;;
    --host) HOST="${2:-}"; shift 2 ;;
    --port) PORT="${2:-}"; shift 2 ;;
    --ui-password) UI_PASSWORD="${2:-}"; shift 2 ;;
    --workspace-root) WORKSPACE_ROOT="${2:-}"; shift 2 ;;
    --database-path) DATABASE_PATH="${2:-}"; shift 2 ;;
    --database-url) DATABASE_URL="${2:-}"; shift 2 ;;
    --set) SETS+=("${2:-}"); shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

OS="$(uname -s)"
ARCH="$(uname -m)"
case "${OS}:${ARCH}" in
  Linux:x86_64|Linux:amd64) TARGET_TRIPLE="x86_64-unknown-linux-gnu" ;;
  Darwin:x86_64) TARGET_TRIPLE="x86_64-apple-darwin" ;;
  Darwin:arm64|Darwin:aarch64)
    echo "ERROR: aarch64 macOS backend archives are not wired in this release flow yet." >&2
    exit 1
    ;;
  *)
    echo "ERROR: unsupported platform ${OS}:${ARCH}" >&2
    exit 1
    ;;
esac

normalize_version() {
  local raw="$1"
  raw="${raw#v}"
  printf '%s' "$raw"
}

VERSION="${VERSION:-}"
if [[ -z "$ARCHIVE" ]]; then
  if [[ -z "$VERSION" ]]; then
    echo "ERROR: provide --archive or --version" >&2
    exit 1
  fi
  VERSION="$(normalize_version "$VERSION")"
  ARCHIVE="https://github.com/${REPO}/releases/download/agena-studio-v${VERSION}/agena-studio-backend-${TARGET_TRIPLE}-v${VERSION}.tar.gz"
fi

TMP_DIR="$(mktemp -d)"
ARCHIVE_PATH="$TMP_DIR/backend.tar.gz"

cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

if [[ "$ARCHIVE" =~ ^https?:// ]]; then
  curl -fsSL "$ARCHIVE" -o "$ARCHIVE_PATH"
else
  cp "$ARCHIVE" "$ARCHIVE_PATH"
fi

STAGE_DIR="$TMP_DIR/extract"
mkdir -p "$STAGE_DIR"
tar -C "$STAGE_DIR" -xzf "$ARCHIVE_PATH"

mkdir -p "$INSTALL_DIR"
rm -rf "$INSTALL_DIR/bin" "$INSTALL_DIR/web-dist" "$INSTALL_DIR/logs"
cp -R "$STAGE_DIR/bin" "$INSTALL_DIR/bin"
cp -R "$STAGE_DIR/web-dist" "$INSTALL_DIR/web-dist"
mkdir -p "$INSTALL_DIR/logs"

LAUNCHER="$INSTALL_DIR/bin/run-agena-studio.sh"
{
  printf '#!/usr/bin/env bash\n'
  printf 'set -euo pipefail\n'
  printf 'exec %q' "$INSTALL_DIR/bin/agena-studio"
  printf ' --host %q --port %q --ui-dir %q' "$HOST" "$PORT" "$INSTALL_DIR/web-dist"
  if [[ -n "$UI_PASSWORD" ]]; then
    printf ' --ui-password %q' "$UI_PASSWORD"
  fi
  if [[ -n "$WORKSPACE_ROOT" ]]; then
    printf ' --workspace-root %q' "$WORKSPACE_ROOT"
  fi
  if [[ -n "$DATABASE_PATH" ]]; then
    printf ' --database-path %q' "$DATABASE_PATH"
  fi
  if [[ -n "$DATABASE_URL" ]]; then
    printf ' --database-url %q' "$DATABASE_URL"
  fi
  for item in "${SETS[@]}"; do
    printf ' --set %q' "$item"
  done
  printf '\n'
} > "$LAUNCHER"
chmod +x "$LAUNCHER"

if [[ "$OS" == "Linux" ]]; then
  SERVICE_DIR="$HOME/.config/systemd/user"
  SERVICE_FILE="$SERVICE_DIR/agena-studio.service"
  mkdir -p "$SERVICE_DIR"
  cat > "$SERVICE_FILE" <<EOF
[Unit]
Description=Agena Studio
After=network.target

[Service]
ExecStart=$LAUNCHER
Restart=on-failure
RestartSec=5
WorkingDirectory=$INSTALL_DIR
StandardOutput=append:$INSTALL_DIR/logs/stdout.log
StandardError=append:$INSTALL_DIR/logs/stderr.log

[Install]
WantedBy=default.target
EOF

  systemctl --user daemon-reload
  systemctl --user enable --now agena-studio.service
  echo "Installed Agena Studio user service."
  echo "Status: systemctl --user status agena-studio"
elif [[ "$OS" == "Darwin" ]]; then
  PLIST_DIR="$HOME/Library/LaunchAgents"
  PLIST_FILE="$PLIST_DIR/cn.cxits.agena-studio.plist"
  mkdir -p "$PLIST_DIR"
  cat > "$PLIST_FILE" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>cn.cxits.agena-studio</string>
  <key>ProgramArguments</key>
  <array>
    <string>$LAUNCHER</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>WorkingDirectory</key>
  <string>$INSTALL_DIR</string>
  <key>StandardOutPath</key>
  <string>$INSTALL_DIR/logs/stdout.log</string>
  <key>StandardErrorPath</key>
  <string>$INSTALL_DIR/logs/stderr.log</string>
</dict>
</plist>
EOF

  launchctl unload "$PLIST_FILE" >/dev/null 2>&1 || true
  launchctl load "$PLIST_FILE"
  echo "Installed Agena Studio launch agent."
  echo "Status: launchctl list | grep agena-studio"
fi
