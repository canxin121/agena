#!/usr/bin/env bash
# Agena one-click installer and lifecycle manager for macOS/Linux.
set -euo pipefail

DEFAULT_REPO="canxin121/agena"
ACTION="install"
REPO="${AGENA_INSTALL_REPO:-$DEFAULT_REPO}"
VERSION="${AGENA_INSTALL_VERSION:-}"
ARCHIVE=""
CHECKSUM=""
INSTALL_DIR="${AGENA_INSTALL_DIR:-$HOME/.local/share/agena}"
HOST="${AGENA_SERVER_HOST:-127.0.0.1}"
PORT="${AGENA_SERVER_PORT:-3210}"
WORKSPACE="${AGENA_WORKSPACE_ROOT:-$HOME}"
UI_PASSWORD="${AGENA_SERVER_UI_PASSWORD:-}"
SERVICE_MODE="${AGENA_INSTALL_SERVICE_MODE:-auto}"
UPDATE_PATH=1
PURGE_DATA=0

usage() {
  cat <<'EOF'
Usage: install.sh [install|upgrade|uninstall|start|stop|restart|status] [options]

One-click install (latest stable release):
  curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash

Lifecycle examples:
  curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- upgrade
  curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- stop
  curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- uninstall

Options:
  --repo OWNER/REPO        GitHub repository (default: canxin121/agena)
  --version VERSION        Release version/tag; default is latest stable
  --archive PATH_OR_URL    Install a specific release archive
  --checksum PATH_OR_URL   SHA256 file for --archive (defaults to ARCHIVE.sha256)
  --install-dir DIR        Install root (default: ~/.local/share/agena)
  --host HOST              Server host (default: 127.0.0.1)
  --port PORT              Server port (default: 3210)
  --workspace PATH         Server workspace (default: $HOME)
  --ui-password PASSWORD   Web/TUI operator password
  --service-mode MODE      auto, native, or detached (default: auto)
  --no-path-update         Do not add the install bin directory to shell PATH
  --purge-data             With uninstall, also remove ~/agena runtime data
  -h, --help               Show this help
EOF
}

if [[ $# -gt 0 && ! "${1:-}" =~ ^- ]]; then
  ACTION="$1"
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --archive) ARCHIVE="${2:-}"; shift 2 ;;
    --checksum) CHECKSUM="${2:-}"; shift 2 ;;
    --install-dir) INSTALL_DIR="${2:-}"; shift 2 ;;
    --host) HOST="${2:-}"; shift 2 ;;
    --port) PORT="${2:-}"; shift 2 ;;
    --workspace) WORKSPACE="${2:-}"; shift 2 ;;
    --ui-password) UI_PASSWORD="${2:-}"; shift 2 ;;
    --service-mode) SERVICE_MODE="${2:-}"; shift 2 ;;
    --no-path-update) UPDATE_PATH=0; shift ;;
    --purge-data) PURGE_DATA=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$ACTION" in
  install|upgrade|uninstall|start|stop|restart|status) ;;
  *) echo "ERROR: unsupported action: $ACTION" >&2; usage >&2; exit 2 ;;
esac
case "$SERVICE_MODE" in
  auto|native|detached) ;;
  *) echo "ERROR: --service-mode must be auto, native, or detached" >&2; exit 2 ;;
esac
if ! [[ "$PORT" =~ ^[0-9]+$ ]] || (( PORT < 1 || PORT > 65535 )); then
  echo "ERROR: invalid port: $PORT" >&2
  exit 2
fi

STATE_FILE="$INSTALL_DIR/install-state.env"
BIN_DIR="$INSTALL_DIR/bin"
AGENA_BIN="$BIN_DIR/agena"
WEB_DIR="$INSTALL_DIR/web-dist"
PATH_MARKER_BEGIN="# >>> agena installer >>>"
PATH_MARKER_END="# <<< agena installer <<<"

shell_quote() {
  printf '%q' "$1"
}

write_state() {
  mkdir -p "$INSTALL_DIR"
  umask 077
  {
    printf 'REPO=%s\n' "$(shell_quote "$REPO")"
    printf 'VERSION=%s\n' "$(shell_quote "$VERSION")"
    printf 'INSTALL_DIR=%s\n' "$(shell_quote "$INSTALL_DIR")"
    printf 'HOST=%s\n' "$(shell_quote "$HOST")"
    printf 'PORT=%s\n' "$(shell_quote "$PORT")"
    printf 'WORKSPACE=%s\n' "$(shell_quote "$WORKSPACE")"
    printf 'UI_PASSWORD=%s\n' "$(shell_quote "$UI_PASSWORD")"
    printf 'SERVICE_MODE=%s\n' "$(shell_quote "$SERVICE_MODE")"
    printf 'UPDATE_PATH=%s\n' "$(shell_quote "$UPDATE_PATH")"
  } > "$STATE_FILE"
  chmod 600 "$STATE_FILE"
}

load_state() {
  if [[ ! -f "$STATE_FILE" ]]; then
    echo "ERROR: Agena is not installed at $INSTALL_DIR" >&2
    exit 1
  fi
  # shellcheck disable=SC1090
  source "$STATE_FILE"
  BIN_DIR="$INSTALL_DIR/bin"
  AGENA_BIN="$BIN_DIR/agena"
  WEB_DIR="$INSTALL_DIR/web-dist"
}

normalize_version() {
  local raw="$1"
  raw="${raw#agena-v}"
  raw="${raw#v}"
  printf '%s' "$raw"
}

download_file() {
  local source="$1"
  local destination="$2"
  if [[ "$source" =~ ^https?:// ]]; then
    if command -v curl >/dev/null 2>&1; then
      curl -fL --retry 3 --retry-delay 1 "$source" -o "$destination"
    elif command -v wget >/dev/null 2>&1; then
      wget -qO "$destination" "$source"
    else
      echo "ERROR: curl or wget is required" >&2
      exit 1
    fi
  else
    cp "$source" "$destination"
  fi
}

latest_version() {
  local effective
  if command -v curl >/dev/null 2>&1; then
    effective="$(curl -fsSL -o /dev/null -w '%{url_effective}' "https://github.com/${REPO}/releases/latest")"
  elif command -v wget >/dev/null 2>&1; then
    effective="$(wget -qO- "https://api.github.com/repos/${REPO}/releases/latest" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)"
  else
    echo "ERROR: curl or wget is required" >&2
    exit 1
  fi
  local tag="${effective##*/}"
  if [[ "$tag" != agena-v* ]]; then
    tag="$effective"
  fi
  normalize_version "$tag"
}

target_triple() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "${os}:${arch}" in
    Linux:x86_64|Linux:amd64) printf 'x86_64-unknown-linux-gnu' ;;
    Linux:aarch64|Linux:arm64) printf 'aarch64-unknown-linux-gnu' ;;
    Darwin:x86_64|Darwin:amd64) printf 'x86_64-apple-darwin' ;;
    Darwin:arm64|Darwin:aarch64) printf 'aarch64-apple-darwin' ;;
    *) echo "ERROR: unsupported platform ${os}:${arch}" >&2; exit 1 ;;
  esac
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

verify_checksum() {
  local archive_path="$1"
  local checksum_path="$2"
  local expected actual
  expected="$(awk 'NF {print $1; exit}' "$checksum_path" | tr '[:upper:]' '[:lower:]')"
  actual="$(sha256_of "$archive_path" | tr '[:upper:]' '[:lower:]')"
  if [[ ! "$expected" =~ ^[0-9a-f]{64}$ ]]; then
    echo "ERROR: invalid SHA256 file: $checksum_path" >&2
    exit 1
  fi
  if [[ "$expected" != "$actual" ]]; then
    echo "ERROR: SHA256 mismatch for Agena archive" >&2
    echo "expected: $expected" >&2
    echo "actual:   $actual" >&2
    exit 1
  fi
}

profile_file() {
  if [[ "$(uname -s)" == "Darwin" ]]; then
    printf '%s/.zprofile' "$HOME"
  else
    printf '%s/.profile' "$HOME"
  fi
}

update_shell_path() {
  (( UPDATE_PATH == 1 )) || return 0
  case ":$PATH:" in
    *":$BIN_DIR:"*) return 0 ;;
  esac
  local profile quoted
  profile="$(profile_file)"
  quoted="$(shell_quote "$BIN_DIR")"
  touch "$profile"
  if grep -Fq "$PATH_MARKER_BEGIN" "$profile"; then
    return 0
  fi
  {
    printf '\n%s\n' "$PATH_MARKER_BEGIN"
    printf 'export PATH=%s:"$PATH"\n' "$quoted"
    printf '%s\n' "$PATH_MARKER_END"
  } >> "$profile"
  echo "Added $BIN_DIR to PATH in $profile"
}

remove_shell_path() {
  local profile temporary
  profile="$(profile_file)"
  [[ -f "$profile" ]] || return 0
  temporary="${profile}.agena.$$"
  awk -v begin="$PATH_MARKER_BEGIN" -v end="$PATH_MARKER_END" '
    $0 == begin { skip=1; next }
    $0 == end { skip=0; next }
    !skip { print }
  ' "$profile" > "$temporary"
  mv "$temporary" "$profile"
}

native_service_available() {
  case "$(uname -s)" in
    Darwin) command -v launchctl >/dev/null 2>&1 ;;
    Linux) command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1 ;;
    *) return 1 ;;
  esac
}

resolve_service_mode() {
  if [[ "$SERVICE_MODE" == "auto" ]]; then
    if native_service_available; then
      SERVICE_MODE="native"
    else
      SERVICE_MODE="detached"
      echo "Native user service manager is unavailable; using detached Agena server lifecycle."
    fi
  fi
  if [[ "$SERVICE_MODE" == "native" ]] && ! native_service_available; then
    echo "ERROR: native user service manager is unavailable on this host" >&2
    exit 1
  fi
}

server_config_args() {
  SERVER_ARGS=(
    --host "$HOST"
    --port "$PORT"
    --workspace "$WORKSPACE"
    --ui-dir "$WEB_DIR"
  )
  if [[ -n "$UI_PASSWORD" ]]; then
    SERVER_ARGS+=(--ui-password "$UI_PASSWORD")
  fi
}

agena_lifecycle() {
  env \
    -u AGENA_DATABASE_URL \
    -u AGENA_DATABASE_PATH \
    -u AGENA_SERVER_HOST \
    -u AGENA_SERVER_PORT \
    -u AGENA_SERVER_UI_PASSWORD \
    -u AGENA_MCP_ENABLED \
    -u AGENA_MCP_PUBLIC_URL \
    -u AGENA_MCP_OAUTH_ISSUER_URL \
    -u AGENA_MCP_AUTH_MODE \
    -u AGENA_MCP_ANONYMOUS_ACCESS \
    -u AGENA_MCP_CLIENT_REGISTRATION \
    -u AGENA_WORKSPACE_ROOT \
    -u AGENA_SERVER_UI_DIR \
    "$AGENA_BIN" "$@"
}

server_is_running() {
  [[ -x "$AGENA_BIN" ]] || return 1
  local output
  output="$(agena_lifecycle server status 2>&1 || true)"
  [[ "$output" == *" is running at "* ]]
}

stop_for_upgrade() {
  [[ -x "$AGENA_BIN" ]] || return 0
  if [[ "$SERVICE_MODE" == "native" ]]; then
    agena_lifecycle server uninstall >/dev/null
  elif server_is_running; then
    agena_lifecycle server stop >/dev/null
  fi
}

start_installed() {
  server_config_args
  if [[ "$SERVICE_MODE" == "native" ]]; then
    agena_lifecycle server install "${SERVER_ARGS[@]}"
  else
    agena_lifecycle server start "${SERVER_ARGS[@]}"
  fi
}

install_or_upgrade() {
  local requested_action="$1"
  local requested_version="$VERSION"
  local old_service_mode="$SERVICE_MODE"
  local old_was_running=0

  if [[ "$requested_action" == "upgrade" ]]; then
    load_state
    server_is_running && old_was_running=1 || true
    VERSION="$requested_version"
    old_service_mode="$SERVICE_MODE"
  elif [[ -f "$STATE_FILE" ]]; then
    echo "ERROR: Agena is already installed at $INSTALL_DIR; use the upgrade action" >&2
    exit 1
  fi

  resolve_service_mode
  mkdir -p "$WORKSPACE"

  local target archive_source checksum_source temp archive_path checksum_path stage binary_version
  target="$(target_triple)"
  if [[ -z "$ARCHIVE" ]]; then
    if [[ -z "$VERSION" ]]; then
      VERSION="$(latest_version)"
    else
      VERSION="$(normalize_version "$VERSION")"
    fi
    archive_source="https://github.com/${REPO}/releases/download/agena-v${VERSION}/agena-backend-${target}-v${VERSION}.tar.gz"
    checksum_source="${archive_source}.sha256"
  else
    archive_source="$ARCHIVE"
    checksum_source="${CHECKSUM:-${ARCHIVE}.sha256}"
  fi

  temp="$(mktemp -d "${TMPDIR:-/tmp}/agena-install.XXXXXX")"
  trap 'rm -rf "$temp"' EXIT
  archive_path="$temp/agena.tar.gz"
  checksum_path="$temp/agena.sha256"
  download_file "$archive_source" "$archive_path"
  download_file "$checksum_source" "$checksum_path"
  verify_checksum "$archive_path" "$checksum_path"

  stage="$temp/stage"
  mkdir -p "$stage"
  tar -C "$stage" -xzf "$archive_path"
  if [[ ! -f "$stage/bin/agena" || ! -f "$stage/web-dist/index.html" ]]; then
    echo "ERROR: release archive is missing bin/agena or web-dist/index.html" >&2
    exit 1
  fi
  chmod +x "$stage/bin/agena"
  binary_version="$("$stage/bin/agena" --version | awk '{print $NF}' | head -1)"
  if [[ -z "$binary_version" ]]; then
    echo "ERROR: installed Agena binary did not report a version" >&2
    exit 1
  fi
  if [[ -n "$VERSION" && "$binary_version" != "$VERSION" ]]; then
    echo "ERROR: archive contains Agena $binary_version but $VERSION was requested" >&2
    exit 1
  fi
  VERSION="$binary_version"

  local backup="$temp/backup"
  mkdir -p "$backup"
  [[ -f "$STATE_FILE" ]] && cp "$STATE_FILE" "$backup/install-state.env"

  if [[ -x "$AGENA_BIN" ]]; then
    SERVICE_MODE="$old_service_mode"
    stop_for_upgrade
    if [[ "$requested_action" != "upgrade" ]]; then
      SERVICE_MODE="$old_service_mode"
    fi
  fi

  mkdir -p "$INSTALL_DIR"
  [[ -d "$BIN_DIR" ]] && mv "$BIN_DIR" "$backup/bin"
  [[ -d "$WEB_DIR" ]] && mv "$WEB_DIR" "$backup/web-dist"
  mv "$stage/bin" "$BIN_DIR"
  mv "$stage/web-dist" "$WEB_DIR"

  local install_error=0
  if start_installed; then
    if [[ "$requested_action" == "upgrade" ]] && (( old_was_running == 0 )); then
      if agena_lifecycle server stop >/dev/null; then
        :
      else
        install_error=$?
      fi
    fi
    if (( install_error == 0 )); then
      if write_state; then
        update_shell_path || echo "WARNING: Agena installed, but PATH update failed" >&2
      else
        install_error=$?
      fi
    fi
  else
    install_error=$?
  fi

  if (( install_error != 0 )); then
    echo "ERROR: the new Agena version failed to start; restoring the previous installation" >&2
    if [[ -x "$AGENA_BIN" ]]; then
      if [[ "$SERVICE_MODE" == "native" ]]; then
        agena_lifecycle server uninstall >/dev/null 2>&1 || true
      elif server_is_running; then
        agena_lifecycle server stop >/dev/null 2>&1 || true
      fi
    fi
    rm -rf "$BIN_DIR" "$WEB_DIR"
    [[ -d "$backup/bin" ]] && mv "$backup/bin" "$BIN_DIR"
    [[ -d "$backup/web-dist" ]] && mv "$backup/web-dist" "$WEB_DIR"

    if [[ -f "$backup/install-state.env" ]]; then
      cp "$backup/install-state.env" "$STATE_FILE"
      load_state
      if [[ "$SERVICE_MODE" == "native" ]]; then
        if start_installed; then
          if (( old_was_running == 0 )); then
            agena_lifecycle server stop >/dev/null 2>&1 || true
          fi
        else
          echo "ERROR: rollback restored the old files but could not restore the native service" >&2
        fi
      elif (( old_was_running == 1 )); then
        start_installed || echo "ERROR: rollback restored the old files but could not restart the old server" >&2
      fi
    else
      rm -f "$STATE_FILE"
    fi
    trap - EXIT
    rm -rf "$temp"
    return "$install_error"
  fi

  echo "Agena $VERSION installed at $INSTALL_DIR"
  echo "Web UI: http://${HOST}:${PORT}"
  if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo "Open a new shell, or run: export PATH=\"$BIN_DIR:\$PATH\""
  fi
  trap - EXIT
  rm -rf "$temp"
}

do_start() {
  load_state
  server_config_args
  if [[ "$SERVICE_MODE" == "native" ]]; then
    agena_lifecycle server start
  else
    agena_lifecycle server start "${SERVER_ARGS[@]}"
  fi
}

do_stop() {
  load_state
  agena_lifecycle server stop
}

do_status() {
  load_state
  agena_lifecycle --version
  agena_lifecycle server status
}

do_uninstall() {
  load_state
  if [[ -x "$AGENA_BIN" ]]; then
    if [[ "$SERVICE_MODE" == "native" ]]; then
      agena_lifecycle server uninstall >/dev/null
    elif server_is_running; then
      agena_lifecycle server stop >/dev/null
    fi
  fi
  remove_shell_path
  rm -rf "$INSTALL_DIR"
  if (( PURGE_DATA == 1 )); then
    rm -rf "$HOME/agena"
    echo "Removed Agena runtime data at $HOME/agena"
  fi
  echo "Agena uninstalled. Configuration/session data was preserved unless --purge-data was used."
}

case "$ACTION" in
  install) install_or_upgrade install ;;
  upgrade) install_or_upgrade upgrade ;;
  uninstall) do_uninstall ;;
  start) do_start ;;
  stop) do_stop ;;
  restart) do_stop || true; do_start ;;
  status) do_status ;;
esac
