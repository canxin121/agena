#!/usr/bin/env bash
# Agena one-click installer and lifecycle manager for macOS/Linux.
set -euo pipefail

DEFAULT_REPO="canxin121/agena"
ACTION="install"
REPO="${AGENA_INSTALL_REPO:-$DEFAULT_REPO}"
VERSION="${AGENA_INSTALL_VERSION:-}"
RELEASE_VERSION=""
ARCHIVE=""
CHECKSUM=""
SOURCE_MODE=0
SOURCE=""
SOURCE_REF=""
SOURCE_ARCHIVE=""
SOURCE_CHECKSUM=""
SOURCE_ROOT=""
INSTALL_DIR="${AGENA_INSTALL_DIR:-$HOME/.local/share/agena}"
DATA_DIR="${AGENA_DATA_DIR:-$HOME/agena}"
HOST="${AGENA_SERVER_HOST:-127.0.0.1}"
PORT="${AGENA_SERVER_PORT:-3210}"
WORKSPACE="${AGENA_WORKSPACE_ROOT:-$HOME}"
UI_PASSWORD="${AGENA_SERVER_UI_PASSWORD:-}"
SERVICE_MODE="${AGENA_INSTALL_SERVICE_MODE:-auto}"
UPDATE_PATH=1
PURGE_DATA=0
REPO_SET=0
VERSION_SET=0
ARCHIVE_SET=0
CHECKSUM_SET=0
SOURCE_REF_SET=0
DATA_DIR_SET=0
HOST_SET=0
PORT_SET=0
WORKSPACE_SET=0
UI_PASSWORD_SET=0
SERVICE_MODE_SET=0

usage() {
  cat <<'EOF'
Usage: install.sh [install|reinstall|upgrade|uninstall|start|stop|restart|status|permissions] [options]

Actions:
  install     Install into an empty installation root
  reinstall   Replace the installed program; reuse its version by default
  upgrade     Replace the installed program with the newest release
  start, stop, restart, status, permissions, uninstall

One-click install (latest beta release):
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash

Source install (build from a checkout):
  bash scripts/agena/install.sh install --source

Rebuild the current checkout as the installed version:
  bash scripts/agena/install.sh reinstall --source

Change saved runtime settings while upgrading:
  bash scripts/agena/install.sh upgrade --source --host 0.0.0.0 --port 3210

Lifecycle examples:
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- upgrade
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- stop
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- uninstall

Options:
  --repo OWNER/REPO        GitHub repository (default: canxin121/agena)
  --version VERSION        Beta release version/tag; defaults to the recorded
                           release for reinstall and the newest beta otherwise
  --archive PATH_OR_URL    Install a specific release archive
  --checksum PATH_OR_URL   SHA256 file for --archive (defaults to ARCHIVE.sha256)
  --source [PATH_OR_URL]   Build from source instead of installing a release archive
  --source-ref REF         Branch or tag to clone with --source (remote sources only)
  --install-dir DIR        Install root (default: ~/.local/share/agena)
  --data-dir DIR           Instance data root (default: AGENA_DATA_DIR or ~/agena)
  --host HOST              Server host (default: 127.0.0.1)
  --port PORT              Server port (default: 3210)
  --workspace PATH         Server workspace (default: $HOME)
  --ui-password PASSWORD   Web/TUI operator password
  --service-mode MODE      auto, native, or detached (default: auto)
  Release and source options apply to install, reinstall, and upgrade.
  Runtime options replace the saved values during reinstall and upgrade;
  omitted runtime options keep the saved values.
  --no-path-update         Do not add the install bin directory to shell PATH
  --purge-data             With uninstall, also remove the installed instance data root
  -h, --help               Show this help
EOF
}

if [[ $# -gt 0 && ! "${1:-}" =~ ^- ]]; then
  ACTION="$1"
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="${2:-}"; REPO_SET=1; shift 2 ;;
    --version) VERSION="${2:-}"; VERSION_SET=1; shift 2 ;;
    --archive) ARCHIVE="${2:-}"; ARCHIVE_SET=1; shift 2 ;;
    --checksum) CHECKSUM="${2:-}"; CHECKSUM_SET=1; shift 2 ;;
    --source)
      SOURCE_MODE=1
      if [[ $# -ge 2 && "${2:-}" != -* ]]; then
        SOURCE="$2"
        shift 2
      else
        SOURCE=""
        shift
      fi ;;
    --source=*) SOURCE_MODE=1; SOURCE="${1#*=}"; shift ;;
    --source-ref) SOURCE_REF="${2:-}"; SOURCE_REF_SET=1; shift 2 ;;
    --install-dir) INSTALL_DIR="${2:-}"; shift 2 ;;
    --data-dir) DATA_DIR="${2:-}"; DATA_DIR_SET=1; shift 2 ;;
    --host) HOST="${2:-}"; HOST_SET=1; shift 2 ;;
    --port) PORT="${2:-}"; PORT_SET=1; shift 2 ;;
    --workspace) WORKSPACE="${2:-}"; WORKSPACE_SET=1; shift 2 ;;
    --ui-password) UI_PASSWORD="${2:-}"; UI_PASSWORD_SET=1; shift 2 ;;
    --service-mode) SERVICE_MODE="${2:-}"; SERVICE_MODE_SET=1; shift 2 ;;
    --no-path-update) UPDATE_PATH=0; shift ;;
    --purge-data) PURGE_DATA=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$ACTION" in
  install|reinstall|upgrade|uninstall|start|stop|restart|status|permissions) ;;
  *) echo "ERROR: unsupported action: $ACTION" >&2; usage >&2; exit 2 ;;
esac

# Reject options that the selected action would otherwise ignore silently.
case "$ACTION" in
  install|reinstall|upgrade) replacement_action=1 ;;
  *) replacement_action=0 ;;
esac
source_options=()
runtime_options=()
if (( ! replacement_action )); then
  (( REPO_SET )) && source_options+=(--repo)
  (( VERSION_SET )) && source_options+=(--version)
  (( ARCHIVE_SET )) && source_options+=(--archive)
  (( CHECKSUM_SET )) && source_options+=(--checksum)
  (( SOURCE_MODE )) && source_options+=(--source)
  (( SOURCE_REF_SET )) && source_options+=(--source-ref)
  (( DATA_DIR_SET )) && runtime_options+=(--data-dir)
  (( HOST_SET )) && runtime_options+=(--host)
  (( PORT_SET )) && runtime_options+=(--port)
  (( WORKSPACE_SET )) && runtime_options+=(--workspace)
  (( UI_PASSWORD_SET )) && runtime_options+=(--ui-password)
  (( SERVICE_MODE_SET )) && runtime_options+=(--service-mode)
fi
if (( ${#source_options[@]} > 0 )); then
  echo "ERROR: ${source_options[*]} only applies to install, reinstall, and upgrade" >&2
  exit 2
fi
if (( ${#runtime_options[@]} > 0 )); then
  echo "ERROR: ${runtime_options[*]} only applies to install, reinstall, and upgrade" >&2
  exit 2
fi
if (( PURGE_DATA == 1 )) && [[ "$ACTION" != "uninstall" ]]; then
  echo "ERROR: --purge-data only applies to uninstall" >&2
  exit 2
fi
case "$SERVICE_MODE" in
  auto|native|detached) ;;
  *) echo "ERROR: --service-mode must be auto, native, or detached" >&2; exit 2 ;;
esac
if (( SOURCE_MODE )); then
  case "$ACTION" in
    install|reinstall|upgrade) ;;
    *) echo "ERROR: --source is only supported for install, reinstall, and upgrade" >&2; exit 2 ;;
  esac
  if [[ -n "$ARCHIVE" ]]; then
    echo "ERROR: --source cannot be combined with --archive" >&2
    exit 2
  fi
  if [[ -n "$CHECKSUM" ]]; then
    echo "ERROR: --source cannot be combined with --checksum" >&2
    exit 2
  fi
  if [[ -n "$VERSION" ]]; then
    echo "ERROR: --version applies to release installs; use --source-ref to select a branch or tag" >&2
    exit 2
  fi
elif [[ -n "$SOURCE_REF" ]]; then
  echo "ERROR: --source-ref requires --source" >&2
  exit 2
fi
if ! [[ "$PORT" =~ ^[0-9]+$ ]] || (( PORT < 1 || PORT > 65535 )); then
  echo "ERROR: invalid port: $PORT" >&2
  exit 2
fi

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

if [[ "$ACTION" == "install" ]]; then
  validate_storage_paths
else
  validate_storage_paths install-only
fi

STATE_FILE="$INSTALL_DIR/install-state.env"
BIN_DIR="$INSTALL_DIR/bin"
AGENA_BIN="$BIN_DIR/agena"
WEB_DIR="$INSTALL_DIR/web-dist"
CLIENT_ENV_FILE="$INSTALL_DIR/client-env.sh"
PATH_MARKER_BEGIN="# >>> agena installer >>>"
PATH_MARKER_END="# <<< agena installer <<<"

shell_quote() {
  printf '%q' "$1"
}

write_state() {
  mkdir -p "$INSTALL_DIR"
  umask 077
  {
    printf 'INSTALL_STATE_SCHEMA=2\n'
    printf 'REPO=%s\n' "$(shell_quote "$REPO")"
    printf 'VERSION=%s\n' "$(shell_quote "$VERSION")"
    printf 'RELEASE_VERSION=%s\n' "$(shell_quote "$RELEASE_VERSION")"
    printf 'INSTALL_DIR=%s\n' "$(shell_quote "$INSTALL_DIR")"
    printf 'DATA_DIR=%s\n' "$(shell_quote "$DATA_DIR")"
    printf 'HOST=%s\n' "$(shell_quote "$HOST")"
    printf 'PORT=%s\n' "$(shell_quote "$PORT")"
    printf 'WORKSPACE=%s\n' "$(shell_quote "$WORKSPACE")"
    printf 'UI_PASSWORD=%s\n' "$(shell_quote "$UI_PASSWORD")"
    printf 'SERVICE_MODE=%s\n' "$(shell_quote "$SERVICE_MODE")"
    printf 'UPDATE_PATH=%s\n' "$(shell_quote "$UPDATE_PATH")"
  } > "$STATE_FILE"
  chmod 600 "$STATE_FILE"
}

write_client_env() {
  mkdir -p "$INSTALL_DIR"
  umask 077
  {
    printf '# Generated by the Agena installer. Source this file from your shell profile.\n'
    printf 'export AGENA_DATA_DIR=%s\n' "$(shell_quote "$DATA_DIR")"
    if [[ -n "$UI_PASSWORD" ]]; then
      printf 'export AGENA_SERVER_PASSWORD=%s\n' "$(shell_quote "$UI_PASSWORD")"
    fi
  } > "$CLIENT_ENV_FILE"
  chmod 600 "$CLIENT_ENV_FILE"
}

load_state() {
  if [[ ! -f "$STATE_FILE" ]]; then
    echo "ERROR: Agena is not installed at $INSTALL_DIR" >&2
    exit 1
  fi
  local selected_install_dir="$INSTALL_DIR" stored_data_dir
  unset INSTALL_STATE_SCHEMA DATA_DIR RELEASE_VERSION
  # shellcheck disable=SC1090
  source "$STATE_FILE"
  if [[ "${INSTALL_STATE_SCHEMA:-}" != "2" || -z "${DATA_DIR:-}" ]]; then
    echo "ERROR: installer state is incompatible; reinstall with an explicit --data-dir" >&2
    exit 1
  fi
  stored_data_dir="$DATA_DIR"
  validate_storage_paths
  if [[ "$INSTALL_DIR" != "$selected_install_dir" || "$DATA_DIR" != "$stored_data_dir" ]]; then
    echo "ERROR: installer state resolves to a different installation or data root" >&2
    exit 1
  fi
  BIN_DIR="$INSTALL_DIR/bin"
  AGENA_BIN="$BIN_DIR/agena"
  WEB_DIR="$INSTALL_DIR/web-dist"
  CLIENT_ENV_FILE="$INSTALL_DIR/client-env.sh"
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
  local releases tag
  if command -v curl >/dev/null 2>&1; then
    releases="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases?per_page=100")"
  elif command -v wget >/dev/null 2>&1; then
    releases="$(wget -qO- "https://api.github.com/repos/${REPO}/releases?per_page=100")"
  else
    echo "ERROR: curl or wget is required" >&2
    exit 1
  fi
  tag="$(printf '%s\n' "$releases" \
    | sed -En 's/.*"tag_name"[[:space:]]*:[[:space:]]*"(agena-v[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*)".*/\1/p' \
    | head -1)"
  if [[ -z "$tag" ]]; then
    echo "ERROR: no Agena beta release was found for ${REPO}" >&2
    exit 1
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

is_source_root() {
  local path="$1"
  [[ -f "$path/Cargo.toml" &&
     -f "$path/packages/agena-web/package.json" &&
     -f "$path/scripts/agena/package-backend.sh" ]]
}

find_source_root() {
  local current
  current="$(cd "$1" 2>/dev/null && pwd -P)" || return 1
  while :; do
    if is_source_root "$current"; then
      printf '%s' "$current"
      return 0
    fi
    [[ "$current" == "/" ]] && break
    current="$(dirname "$current")"
  done
  return 1
}

installer_source_root() {
  local script="${BASH_SOURCE[0]:-}" directory
  [[ -n "$script" && -f "$script" ]] || return 1
  directory="$(cd "$(dirname "$script")" && pwd -P)" || return 1
  find_source_root "$(cd "$directory/../.." && pwd -P)"
}

is_git_source() {
  case "$1" in
    http://*|https://*|ssh://*|git://*|file://*|git@*:*) return 0 ;;
    *) return 1 ;;
  esac
}

clone_source() {
  local repository="$1" destination="$2"
  if ! command -v git >/dev/null 2>&1; then
    echo "ERROR: git is required to install from a remote source repository" >&2
    return 1
  fi
  local -a clone_args=(clone --depth 1)
  if [[ -n "$SOURCE_REF" ]]; then
    clone_args+=(--branch "$SOURCE_REF")
  fi
  echo "Cloning Agena source from $repository" >&2
  git "${clone_args[@]}" "$repository" "$destination"
}

resolve_source_root() {
  SOURCE_ROOT=""
  local temp="$1" requested="$SOURCE" root
  if [[ -n "$requested" ]]; then
    if [[ -d "$requested" ]]; then
      root="$(find_source_root "$requested")" || {
        echo "ERROR: --source directory is not an Agena checkout: $requested" >&2
        return 1
      }
      SOURCE_ROOT="$root"
      return 0
    fi
    if is_git_source "$requested"; then
      clone_source "$requested" "$temp/source" || return
      root="$(find_source_root "$temp/source")" || {
        echo "ERROR: cloned source is not an Agena checkout: $requested" >&2
        return 1
      }
      SOURCE_ROOT="$root"
      return 0
    fi
    echo "ERROR: --source must name an Agena checkout or a Git URL" >&2
    return 1
  fi
  if root="$(find_source_root "$PWD")"; then
    SOURCE_ROOT="$root"
    return 0
  fi
  if root="$(installer_source_root)"; then
    SOURCE_ROOT="$root"
    return 0
  fi
  clone_source "https://github.com/${REPO}.git" "$temp/source" || return
  root="$(find_source_root "$temp/source")" || {
    echo "ERROR: cloned source is not an Agena checkout" >&2
    return 1
  }
  SOURCE_ROOT="$root"
}

prepare_source_archive() {
  local temp="$1" target="$2" source_root package_script build_log
  resolve_source_root "$temp" || exit 1
  source_root="$SOURCE_ROOT"
  if [[ -n "$SOURCE_REF" && "$source_root" != "$temp/source" ]]; then
    echo "WARNING: --source-ref only applies when cloning; using the existing checkout as-is" >&2
  fi
  package_script="$source_root/scripts/agena/package-backend.sh"
  echo "Building Agena from source at $source_root"
  build_log="$temp/source-build.log"
  if ! (
    cd "$source_root"
    bash "$package_script" "$target"
  ) 2>&1 | tee "$build_log"; then
    echo "ERROR: Agena source build failed" >&2
    exit 1
  fi
  SOURCE_ARCHIVE="$(sed -n 's/^Package ready: //p' "$build_log" | tail -1)"
  SOURCE_CHECKSUM="$(sed -n 's/^Checksum ready: //p' "$build_log" | tail -1)"
  [[ -n "$SOURCE_CHECKSUM" ]] || SOURCE_CHECKSUM="${SOURCE_ARCHIVE}.sha256"
  if [[ -z "$SOURCE_ARCHIVE" || ! -f "$SOURCE_ARCHIVE" || ! -f "$SOURCE_CHECKSUM" ]]; then
    echo "ERROR: source build did not produce an Agena archive and checksum" >&2
    exit 1
  fi
  echo "Source package ready: $SOURCE_ARCHIVE"
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

managed_profile_files() {
  local zdotdir="${ZDOTDIR:-$HOME}"
  case "${SHELL:-}" in
    */zsh) printf '%s\n' "$zdotdir/.zshenv" "$zdotdir/.zshrc" ;;
    *) printf '%s\n' "$HOME/.profile" ;;
  esac
}

profile_file() {
  managed_profile_files | tail -n 1
}

profile_files() {
  local zdotdir="${ZDOTDIR:-$HOME}"
  printf '%s\n' "$HOME/.profile" "$HOME/.zprofile" "$zdotdir/.zshenv" "$zdotdir/.zshrc" |
    awk '!seen[$0]++'
}

is_managed_profile_file() {
  local candidate="$1" managed
  while IFS= read -r managed; do
    [[ "$candidate" == "$managed" ]] && return 0
  done < <(managed_profile_files)
  return 1
}

remove_managed_block() {
  local profile="$1" temporary
  [[ -f "$profile" ]] || return 0
  grep -Fq "$PATH_MARKER_BEGIN" "$profile" || return 0
  temporary="${profile}.agena.$$"
  awk -v begin="$PATH_MARKER_BEGIN" -v end="$PATH_MARKER_END" '
    $0 == begin { skip=1; next }
    $0 == end { skip=0; next }
    !skip { print }
  ' "$profile" > "$temporary"
  # Write through the original path so dotfile symlinks are preserved.
  cat "$temporary" > "$profile"
  rm -f "$temporary"
}

update_shell_path() {
  (( UPDATE_PATH == 1 )) || return 0
  local profile quoted client_env_quoted
  quoted="$(shell_quote "$BIN_DIR")"
  client_env_quoted="$(shell_quote "$CLIENT_ENV_FILE")"
  while IFS= read -r profile; do
    is_managed_profile_file "$profile" || remove_managed_block "$profile"
  done < <(profile_files)
  while IFS= read -r profile; do
    touch "$profile"
    remove_managed_block "$profile"
    {
      printf '\n%s\n' "$PATH_MARKER_BEGIN"
      printf 'export PATH=%s:"$PATH"\n' "$quoted"
      printf 'if [ -r %s ]; then . %s; fi\n' "$client_env_quoted" "$client_env_quoted"
      printf '%s\n' "$PATH_MARKER_END"
    } >> "$profile"
    echo "Added Agena PATH/client environment setup to $profile"
  done < <(managed_profile_files)
}

remove_shell_path() {
  local profile
  while IFS= read -r profile; do
    remove_managed_block "$profile"
  done < <(profile_files)
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

# Every lifecycle call names the data root explicitly so an inherited
# AGENA_DATA_DIR cannot redirect it to a different instance.
agena_lifecycle_at() {
  local data_dir="$1"
  shift
  env \
    -u AGENA_DATA_DIR \
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
    "$AGENA_BIN" --data-dir "$data_dir" "$@"
}

agena_lifecycle() {
  agena_lifecycle_at "$DATA_DIR" "$@"
}

server_is_running_at() {
  [[ -x "$AGENA_BIN" ]] || return 1
  local output
  output="$(agena_lifecycle_at "$1" server status 2>&1 || true)"
  [[ "$output" == *" is running at "* ]]
}

server_is_running() {
  server_is_running_at "$DATA_DIR"
}

# Stop or deregister the instance described by the given settings. During a
# replacement the global settings already describe the candidate, so the
# current installation passes its own saved values here.
stop_installation() {
  local data_dir="$1" service_mode="$2"
  [[ -x "$AGENA_BIN" ]] || return 0
  if [[ "$service_mode" == "native" ]]; then
    agena_lifecycle_at "$data_dir" server uninstall >/dev/null
  elif server_is_running_at "$data_dir"; then
    agena_lifecycle_at "$data_dir" server stop >/dev/null
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

release_package_version() {
  if [[ ! "$1" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-beta\.[1-9][0-9]*$ ]]; then
    echo "ERROR: --version must name a beta release, for example 0.1.0-beta.1" >&2
    exit 2
  fi
  printf '%s' "${BASH_REMATCH[1]}"
}

# Replacement pipeline shared by install, reinstall, and upgrade.
#
#   1. resolve_replacement   read saved state, overlay explicit options
#   2. prepare_candidate     download or build, verify, unpack, probe version
#   3. activate_candidate    stop the current instance, swap program files
#   4. finalize_replacement  start, restore prior run state, save state
#
# A failure in 3 or 4 runs rollback_replacement, which restores the previous
# program files, installer state, settings, and running state. The stages only
# differ in their preconditions: install requires no existing installation,
# while reinstall and upgrade require one.

TEMP_DIR=""
BACKUP_DIR=""
CANDIDATE_DIR=""
CANDIDATE_VERSION=""
CANDIDATE_INSTALLED=0
ACTIVATION_STARTED=0
HAD_STATE=0
OLD_DATA_DIR=""
OLD_SERVICE_MODE=""
OLD_WAS_RUNNING=0

resolve_replacement() {
  local requested_repo="$REPO"
  local requested_version="$VERSION"
  local requested_data_dir="$DATA_DIR"
  local requested_host="$HOST"
  local requested_port="$PORT"
  local requested_workspace="$WORKSPACE"
  local requested_ui_password="$UI_PASSWORD"
  local requested_service_mode="$SERVICE_MODE"

  case "$ACTION" in
    install)
      if [[ -f "$STATE_FILE" ]]; then
        echo "ERROR: Agena is already installed at $INSTALL_DIR" >&2
        echo "  reinstall  replace it with the installed version" >&2
        echo "  upgrade    install the newest or requested version" >&2
        exit 1
      fi
      if (( SOURCE_MODE )); then
        RELEASE_VERSION=""
      elif (( ARCHIVE_SET )); then
        RELEASE_VERSION="$requested_version"
      else
        RELEASE_VERSION="$requested_version"
      fi
      ;;
    reinstall|upgrade)
      load_state
      OLD_DATA_DIR="$DATA_DIR"
      OLD_SERVICE_MODE="$SERVICE_MODE"
      server_is_running_at "$OLD_DATA_DIR" && OLD_WAS_RUNNING=1 || true
      (( REPO_SET )) && REPO="$requested_repo"
      # An upgrade selects a new release; a reinstall repeats the recorded
      # release unless another source or version was requested explicitly.
      RELEASE_VERSION="${RELEASE_VERSION:-}"
      if (( SOURCE_MODE )); then
        RELEASE_VERSION=""
      elif (( ARCHIVE_SET )); then
        RELEASE_VERSION="$requested_version"
        (( VERSION_SET )) || RELEASE_VERSION=""
      elif (( VERSION_SET )); then
        RELEASE_VERSION="$requested_version"
      elif [[ "$ACTION" == "upgrade" ]]; then
        RELEASE_VERSION=""
      fi
      (( DATA_DIR_SET )) && DATA_DIR="$requested_data_dir"
      (( HOST_SET )) && HOST="$requested_host"
      (( PORT_SET )) && PORT="$requested_port"
      (( WORKSPACE_SET )) && WORKSPACE="$requested_workspace"
      (( UI_PASSWORD_SET )) && UI_PASSWORD="$requested_ui_password"
      (( SERVICE_MODE_SET )) && SERVICE_MODE="$requested_service_mode"
      validate_storage_paths
      if [[ "$ACTION" == "reinstall" ]] && (( ! SOURCE_MODE )) &&
         [[ -z "$ARCHIVE" && -z "$RELEASE_VERSION" ]]; then
        echo "ERROR: this installation did not record a release version" >&2
        echo "  reinstall with --archive PATH, --source, or --version VERSION" >&2
        exit 1
      fi
      ;;
  esac

  resolve_service_mode
  mkdir -p "$WORKSPACE"
}

prepare_candidate() {
  local temp="$1"
  local target package_version="" archive_source checksum_source archive_path checksum_path
  target="$(target_triple)"
  if (( SOURCE_MODE )); then
    RELEASE_VERSION=""
    prepare_source_archive "$temp" "$target"
    archive_source="$SOURCE_ARCHIVE"
    checksum_source="$SOURCE_CHECKSUM"
  elif [[ -z "$ARCHIVE" ]]; then
    if [[ -z "$RELEASE_VERSION" ]]; then
      RELEASE_VERSION="$(latest_version)"
    else
      RELEASE_VERSION="$(normalize_version "$RELEASE_VERSION")"
    fi
    package_version="$(release_package_version "$RELEASE_VERSION")"
    archive_source="https://github.com/${REPO}/releases/download/agena-v${RELEASE_VERSION}/agena-backend-${target}-v${package_version}.tar.gz"
    checksum_source="${archive_source}.sha256"
  else
    archive_source="$ARCHIVE"
    checksum_source="${CHECKSUM:-${ARCHIVE}.sha256}"
    if [[ -n "$RELEASE_VERSION" ]]; then
      RELEASE_VERSION="$(normalize_version "$RELEASE_VERSION")"
      package_version="$(release_package_version "$RELEASE_VERSION")"
    fi
  fi

  archive_path="$temp/agena.tar.gz"
  checksum_path="$temp/agena.sha256"
  download_file "$archive_source" "$archive_path"
  download_file "$checksum_source" "$checksum_path"
  verify_checksum "$archive_path" "$checksum_path"

  CANDIDATE_DIR="$temp/stage"
  mkdir -p "$CANDIDATE_DIR"
  tar -C "$CANDIDATE_DIR" -xzf "$archive_path"
  if [[ ! -f "$CANDIDATE_DIR/bin/agena" || ! -f "$CANDIDATE_DIR/web-dist/index.html" ]]; then
    echo "ERROR: release archive is missing bin/agena or web-dist/index.html" >&2
    exit 1
  fi
  chmod +x "$CANDIDATE_DIR/bin/agena"
  CANDIDATE_VERSION="$("$CANDIDATE_DIR/bin/agena" --version | awk '{print $NF}' | head -1)"
  if [[ -z "$CANDIDATE_VERSION" ]]; then
    echo "ERROR: installed Agena binary did not report a version" >&2
    exit 1
  fi
  if [[ -n "$package_version" && "$CANDIDATE_VERSION" != "$package_version" ]]; then
    echo "ERROR: archive contains Agena $CANDIDATE_VERSION but release $RELEASE_VERSION contains $package_version" >&2
    exit 1
  fi
  VERSION="$CANDIDATE_VERSION"
}

activate_candidate() {
  local temp="$1"
  BACKUP_DIR="$temp/backup"
  mkdir -p "$BACKUP_DIR"
  if [[ -f "$STATE_FILE" ]]; then
    HAD_STATE=1
    cp "$STATE_FILE" "$BACKUP_DIR/install-state.env"
  fi

  if [[ -x "$AGENA_BIN" ]]; then
    echo "Stopping the installed Agena instance"
    stop_installation "$OLD_DATA_DIR" "$OLD_SERVICE_MODE"
  fi

  ACTIVATION_STARTED=1
  mkdir -p "$INSTALL_DIR"
  [[ -d "$BIN_DIR" ]] && mv "$BIN_DIR" "$BACKUP_DIR/bin"
  [[ -d "$WEB_DIR" ]] && mv "$WEB_DIR" "$BACKUP_DIR/web-dist"
  mv "$CANDIDATE_DIR/bin" "$BIN_DIR"
  mv "$CANDIDATE_DIR/web-dist" "$WEB_DIR"
  CANDIDATE_INSTALLED=1
}

finalize_replacement() {
  if ! start_installed; then
    echo "ERROR: the replacement Agena version failed to start" >&2
    return 1
  fi
  # A replacement preserves whether the instance was running before.
  if [[ "$ACTION" != "install" ]] && (( OLD_WAS_RUNNING == 0 )); then
    if ! agena_lifecycle server stop >/dev/null; then
      echo "ERROR: the replacement could not return the instance to its stopped state" >&2
      return 1
    fi
  fi
  if ! write_state || ! write_client_env; then
    echo "ERROR: the replacement started but its installer state could not be saved" >&2
    return 1
  fi
  update_shell_path || echo "WARNING: Agena is installed, but shell environment update failed" >&2
}

restore_program_dir() {
  local current="$INSTALL_DIR/$1" saved="$BACKUP_DIR/$1"
  [[ -d "$saved" ]] || return 0
  rm -rf "$current"
  mv "$saved" "$current"
}

rollback_replacement() {
  echo "ERROR: restoring the previous Agena installation" >&2
  if [[ -x "$AGENA_BIN" ]]; then
    stop_installation "$DATA_DIR" "$SERVICE_MODE" || true
  fi
  restore_program_dir bin
  restore_program_dir web-dist

  if (( HAD_STATE == 1 )); then
    cp "$BACKUP_DIR/install-state.env" "$STATE_FILE"
    load_state
    write_client_env || echo "ERROR: rollback restored state but could not restore the client environment" >&2
    if [[ "$SERVICE_MODE" == "native" ]]; then
      if start_installed; then
        if (( OLD_WAS_RUNNING == 0 )); then
          agena_lifecycle server stop >/dev/null 2>&1 || true
        fi
      else
        echo "ERROR: rollback restored the old files but could not restore the native service" >&2
      fi
    elif (( OLD_WAS_RUNNING == 1 )); then
      start_installed || echo "ERROR: rollback restored the old files but could not restart the old server" >&2
    fi
  else
    if (( CANDIDATE_INSTALLED == 1 )); then
      rm -rf "$BIN_DIR" "$WEB_DIR"
    fi
    rm -f "$STATE_FILE" "$CLIENT_ENV_FILE"
  fi
}

cleanup_replacement() {
  if (( ACTIVATION_STARTED == 1 )); then
    rollback_replacement || true
    ACTIVATION_STARTED=0
  fi
  if [[ -n "$TEMP_DIR" ]]; then
    rm -rf "$TEMP_DIR"
  fi
}

report_replacement() {
  local verb="installed"
  case "$ACTION" in
    reinstall) verb="reinstalled" ;;
    upgrade) verb="upgraded" ;;
  esac
  echo "Agena $VERSION $verb at $INSTALL_DIR"
  echo "Instance data: $DATA_DIR"
  echo "Web UI: http://${HOST}:${PORT}"
  if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo "Open a new shell to load the Agena PATH and client environment."
  elif [[ -n "$UI_PASSWORD" && "${AGENA_SERVER_PASSWORD:-}" != "$UI_PASSWORD" ]]; then
    echo "Open a new shell (or source $(profile_file)) to load AGENA_SERVER_PASSWORD for the TUI."
  fi
}

run_replacement() {
  resolve_replacement
  TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/agena-install.XXXXXX")"
  trap cleanup_replacement EXIT
  prepare_candidate "$TEMP_DIR"
  activate_candidate "$TEMP_DIR"

  local status=0
  finalize_replacement || status=$?
  if (( status != 0 )); then
    ACTIVATION_STARTED=0
    rollback_replacement
    trap - EXIT
    rm -rf "$TEMP_DIR"
    return "$status"
  fi

  report_replacement
  ACTIVATION_STARTED=0
  trap - EXIT
  rm -rf "$TEMP_DIR"
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

do_permissions() {
  load_state
  agena_lifecycle server permissions
}

do_uninstall() {
  load_state
  if (( PURGE_DATA == 1 )) && [[ -e "$DATA_DIR" && ! -f "$DATA_DIR/agena.db" ]]; then
    echo "ERROR: refusing to purge a directory without an Agena instance database: $DATA_DIR" >&2
    return 2
  fi
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
    rm -rf "$DATA_DIR"
    echo "Removed Agena runtime data at $DATA_DIR"
  fi
  echo "Agena uninstalled. Configuration/session data was preserved unless --purge-data was used."
}

case "$ACTION" in
  install|reinstall|upgrade) run_replacement ;;
  uninstall) do_uninstall ;;
  start) do_start ;;
  stop) do_stop ;;
  restart) do_stop || true; do_start ;;
  status) do_status ;;
  permissions) do_permissions ;;
esac
