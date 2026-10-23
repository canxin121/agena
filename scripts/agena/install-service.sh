#!/usr/bin/env bash
set -euo pipefail
umask 077

usage() {
  cat <<'EOF'
Usage: install-service.sh [options]

Options:
  --archive PATH_OR_URL    Backend package archive (.tar.gz)
  --repo OWNER/REPO        GitHub repo for release downloads (default: canxin121/agena)
  --version VERSION        Beta release version, e.g. 0.1.0-beta.1
  --install-dir DIR        Binary/Web directory (default: ~/.local/share/agena)
  --data-dir DIR           Instance data root (default: AGENA_DATA_DIR or ~/agena)
  --host HOST              Backend host (default: 127.0.0.1)
  --port PORT              Backend port (default: 3210)
  --ui-password PASSWORD   Optional UI password
  --workspace-root PATH    Optional workspace root
  --set KEY=VALUE          Additional Agena overrides (repeatable)
  -h, --help               Show help
EOF
}

REPO="canxin121/agena"
VERSION=""
ARCHIVE=""
INSTALL_DIR="${AGENA_INSTALL_DIR:-$HOME/.local/share/agena}"
DATA_DIR="${AGENA_DATA_DIR:-$HOME/agena}"
HOST="127.0.0.1"
PORT="3210"
UI_PASSWORD=""
WORKSPACE_ROOT=""
SETS=()

while [[ $# -gt 0 ]]; do
  case "$1" in
    --archive) ARCHIVE="${2:-}"; shift 2 ;;
    --repo) REPO="${2:-}"; shift 2 ;;
    --version) VERSION="${2:-}"; shift 2 ;;
    --install-dir) INSTALL_DIR="${2:-}"; shift 2 ;;
    --data-dir) DATA_DIR="${2:-}"; shift 2 ;;
    --host) HOST="${2:-}"; shift 2 ;;
    --port) PORT="${2:-}"; shift 2 ;;
    --ui-password) UI_PASSWORD="${2:-}"; shift 2 ;;
    --workspace-root) WORKSPACE_ROOT="${2:-}"; shift 2 ;;
    --set) SETS+=("${2:-}"); shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

absolute_directory() {
  local path="$1" parent name
  [[ -n "$path" ]] || { echo "ERROR: directory must not be empty" >&2; return 2; }
  if [[ -d "$path" ]]; then
    (cd "$path" && pwd -P)
    return
  fi
  parent="$(absolute_directory "$(dirname "$path")")" || return
  name="$(basename "$path")"
  case "$name" in
    .) printf '%s' "$parent" ;;
    ..) dirname "$parent" ;;
    *) printf '%s/%s' "${parent%/}" "$name" ;;
  esac
}

validate_storage_paths() {
  INSTALL_DIR="$(absolute_directory "$INSTALL_DIR")"
  local account_home
  account_home="$(absolute_directory "$HOME")"
  case "$INSTALL_DIR" in
    /|"$account_home"|/bin|/usr|/usr/local|/etc|/var|/opt|/tmp)
      echo "ERROR: unsafe application install directory: $INSTALL_DIR" >&2; return 2 ;;
  esac
  # Lifecycle actions select the installed data root from their manifest.
  # Do not interpret an unrelated inherited AGENA_DATA_DIR before loading it.
  [[ "${1:-}" == "install-only" ]] && return 0
  DATA_DIR="$(absolute_directory "$DATA_DIR")"
  case "$DATA_DIR" in
    /|"$account_home"|/bin|/usr|/usr/local|/etc|/var|/opt|/tmp)
      echo "ERROR: unsafe instance data directory: $DATA_DIR" >&2; return 2 ;;
  esac
  if [[ "$INSTALL_DIR/" == "$DATA_DIR/"* || "$DATA_DIR/" == "$INSTALL_DIR/"* ]]; then
    echo "ERROR: application installation and instance data directories must be separate" >&2
    return 2
  fi
}

validate_storage_paths

OS="$(uname -s)"
ARCH="$(uname -m)"
case "${OS}:${ARCH}" in
  Linux:x86_64|Linux:amd64) TARGET_TRIPLE="x86_64-unknown-linux-gnu" ;;
  Linux:aarch64|Linux:arm64) TARGET_TRIPLE="aarch64-unknown-linux-gnu" ;;
  Darwin:x86_64) TARGET_TRIPLE="x86_64-apple-darwin" ;;
  Darwin:arm64|Darwin:aarch64) TARGET_TRIPLE="aarch64-apple-darwin" ;;
  *)
    echo "ERROR: unsupported platform ${OS}:${ARCH}" >&2
    exit 1
    ;;
esac

normalize_version() {
  local raw="$1"
  raw="${raw#agena-v}"
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
  if [[ ! "$VERSION" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-beta\.[1-9][0-9]*$ ]]; then
    echo "ERROR: --version must name a beta release, for example 0.1.0-beta.1" >&2
    exit 2
  fi
  PACKAGE_VERSION="${BASH_REMATCH[1]}"
  ARCHIVE="https://github.com/${REPO}/releases/download/agena-v${VERSION}/agena-backend-${TARGET_TRIPLE}-v${PACKAGE_VERSION}.tar.gz"
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
rm -rf "$INSTALL_DIR/bin" "$INSTALL_DIR/web-dist"
cp -R "$STAGE_DIR/bin" "$INSTALL_DIR/bin"
cp -R "$STAGE_DIR/web-dist" "$INSTALL_DIR/web-dist"
SERVICE_ARGS=(--data-dir "$DATA_DIR" server install --host "$HOST" --port "$PORT" --ui-dir "$INSTALL_DIR/web-dist")
[[ -n "$UI_PASSWORD" ]] && SERVICE_ARGS+=(--ui-password "$UI_PASSWORD")
[[ -n "$WORKSPACE_ROOT" ]] && SERVICE_ARGS+=(--workspace "$WORKSPACE_ROOT")
for item in "${SETS[@]}"; do
  SERVICE_ARGS+=(--set "$item")
done
"$INSTALL_DIR/bin/agena" "${SERVICE_ARGS[@]}"
{
  printf 'SERVICE_STATE_SCHEMA=2\n'
  printf 'DATA_DIR=%q\n' "$DATA_DIR"
} > "$INSTALL_DIR/service-state.env"
chmod 600 "$INSTALL_DIR/service-state.env"
echo "Installed Agena user service. Instance data: $DATA_DIR"
printf 'Status: %q --data-dir %q server status\n' "$INSTALL_DIR/bin/agena" "$DATA_DIR"
if [[ "$OS" == "Darwin" ]]; then
  printf 'macOS file access setup: %q --data-dir %q server permissions\n' "$INSTALL_DIR/bin/agena" "$DATA_DIR"
fi
