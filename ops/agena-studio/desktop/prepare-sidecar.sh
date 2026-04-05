#!/usr/bin/env bash
set -euo pipefail

# Build the Rust backend and place it where Tauri expects bundled binaries:
#   apps/agena-studio-desktop/src-tauri/binaries/agena-studio-$TARGET_TRIPLE[.exe]
#   apps/agena-studio-desktop/src-tauri-cef/binaries/agena-studio-$TARGET_TRIPLE[.exe]

usage() {
  cat <<'EOF'
Usage: prepare-sidecar.sh [--cef] [TARGET_TRIPLE]

Options:
  --cef            Install backend binary into apps/agena-studio-desktop/src-tauri-cef/binaries

Arguments:
  TARGET_TRIPLE    Rust target triple (defaults to host)
EOF
}

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SERVER_MANIFEST="$REPO_ROOT/apps/agena-studio-server/Cargo.toml"
SERVER_TARGET_DIR="$REPO_ROOT/target"

TAURI_VARIANT="src-tauri"
TARGET_TRIPLE=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --cef)
      TAURI_VARIANT="src-tauri-cef"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      if [[ -z "$TARGET_TRIPLE" ]]; then
        TARGET_TRIPLE="$1"
        shift
      else
        echo "ERROR: unexpected argument: $1" >&2
        usage >&2
        exit 2
      fi
      ;;
  esac
done

TAURI_BIN_DIR="$REPO_ROOT/apps/agena-studio-desktop/$TAURI_VARIANT/binaries"

if [[ -z "$TARGET_TRIPLE" ]]; then
  if rustc --print host-tuple >/dev/null 2>&1; then
    TARGET_TRIPLE="$(rustc --print host-tuple)"
  else
    TARGET_TRIPLE="$(rustc -Vv | awk '/^host:/{print $2; exit}')"
  fi
fi

EXT=""
case "${TARGET_TRIPLE}" in
  *-pc-windows-*) EXT=".exe";;
esac

echo "Building backend service binary for ${TARGET_TRIPLE}..."
cargo build --manifest-path "$SERVER_MANIFEST" --release --target "$TARGET_TRIPLE" --locked --target-dir "$SERVER_TARGET_DIR"

SRC_BIN="$SERVER_TARGET_DIR/$TARGET_TRIPLE/release/agena-studio$EXT"
if [[ ! -f "$SRC_BIN" ]]; then
  echo "ERROR: built binary not found at: $SRC_BIN" >&2
  exit 1
fi

mkdir -p "$TAURI_BIN_DIR"
DEST_BIN="$TAURI_BIN_DIR/agena-studio-$TARGET_TRIPLE$EXT"
cp "$SRC_BIN" "$DEST_BIN"

echo "Backend binary ready: $DEST_BIN"
