#!/usr/bin/env bash
# Remove the native Agena user service; instance data is preserved.
set -euo pipefail

REMOVE_INSTALL_DIR=0
INSTALL_DIR="${AGENA_INSTALL_DIR:-$HOME/.local/share/agena}"
DATA_DIR="${AGENA_DATA_DIR:-$HOME/agena}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --remove-install-dir) REMOVE_INSTALL_DIR=1; shift ;;
    --install-dir) INSTALL_DIR="${2:-}"; shift 2 ;;
    --data-dir) DATA_DIR="${2:-}"; shift 2 ;;
    -h|--help)
      echo 'Usage: uninstall-service.sh [--install-dir DIR] [--data-dir DIR] [--remove-install-dir]'
      exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; exit 2 ;;
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

validate_storage_paths install-only
selected_install_dir="$INSTALL_DIR"
unset SERVICE_STATE_SCHEMA INSTALL_STATE_SCHEMA DATA_DIR

if [[ -f "$INSTALL_DIR/service-state.env" ]]; then
  # shellcheck disable=SC1090
  source "$INSTALL_DIR/service-state.env"
  [[ "${SERVICE_STATE_SCHEMA:-}" == "2" ]] || { echo 'ERROR: incompatible service installer state' >&2; exit 1; }
elif [[ -f "$INSTALL_DIR/install-state.env" ]]; then
  # shellcheck disable=SC1090
  source "$INSTALL_DIR/install-state.env"
  [[ "${INSTALL_STATE_SCHEMA:-}" == "2" ]] || { echo 'ERROR: incompatible installer state' >&2; exit 1; }
else
  echo "ERROR: no Agena installer state found at $INSTALL_DIR" >&2
  exit 1
fi
[[ -n "${DATA_DIR:-}" ]] || { echo 'ERROR: installer state has no data root' >&2; exit 1; }
stored_data_dir="$DATA_DIR"
validate_storage_paths
if [[ "$INSTALL_DIR" != "$selected_install_dir" || "$DATA_DIR" != "$stored_data_dir" ]]; then
  echo 'ERROR: installer state resolves to a different installation or data root' >&2
  exit 1
fi

if [[ ! -x "$INSTALL_DIR/bin/agena" ]]; then
  echo "ERROR: native service uninstall requires $INSTALL_DIR/bin/agena" >&2
  exit 1
fi
"$INSTALL_DIR/bin/agena" --data-dir "$DATA_DIR" server uninstall
if (( REMOVE_INSTALL_DIR == 1 )); then
  rm -rf "$INSTALL_DIR"
fi
echo "Agena background service removed. Instance data preserved at $DATA_DIR"
