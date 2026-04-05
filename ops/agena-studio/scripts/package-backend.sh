#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
SERVER_MANIFEST="$REPO_ROOT/apps/agena-studio-server/Cargo.toml"
SERVER_TARGET_DIR="$REPO_ROOT/target"
RELEASE_DIR="$REPO_ROOT/artifacts/agena-studio"
WEB_DIST_DIR="$REPO_ROOT/packages/agena-studio-web/dist"

detect_host_triple() {
  if rustc --print host-tuple >/dev/null 2>&1; then
    rustc --print host-tuple
    return
  fi
  rustc -Vv | awk '/^host:/{print $2; exit}'
}

read_version() {
  sed -nE 's/^version = "([^"]+)"/\1/p' "$SERVER_MANIFEST" | head -n1
}

TARGET_TRIPLE="${1:-$(detect_host_triple)}"
VERSION="$(read_version)"

if [[ -z "$VERSION" ]]; then
  echo "ERROR: failed to read agena-studio version from $SERVER_MANIFEST" >&2
  exit 1
fi

EXT=""
ARCHIVE_EXT="tar.gz"
case "$TARGET_TRIPLE" in
  *-pc-windows-*) EXT=".exe"; ARCHIVE_EXT="zip" ;;
esac

"$SCRIPT_DIR/build-frontend-dist.sh"

echo "Building agena-studio backend for ${TARGET_TRIPLE}..."
cargo build \
  --manifest-path "$SERVER_MANIFEST" \
  --release \
  --target "$TARGET_TRIPLE" \
  --locked \
  --target-dir "$SERVER_TARGET_DIR"

BIN_PATH="$SERVER_TARGET_DIR/$TARGET_TRIPLE/release/agena-studio$EXT"
if [[ ! -f "$BIN_PATH" ]]; then
  echo "ERROR: built backend binary not found at $BIN_PATH" >&2
  exit 1
fi

STAGE_DIR="$RELEASE_DIR/backend/$TARGET_TRIPLE"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/web-dist"

cp "$BIN_PATH" "$STAGE_DIR/bin/agena-studio$EXT"
cp -R "$WEB_DIST_DIR/." "$STAGE_DIR/web-dist/"

cat > "$STAGE_DIR/README.txt" <<EOF
Agena Studio backend package
Version: $VERSION
Target: $TARGET_TRIPLE

Contents:
- bin/agena-studio$EXT
- web-dist/

Example:
  agena-studio --ui-dir ./web-dist
EOF

ARCHIVE_NAME="agena-studio-backend-${TARGET_TRIPLE}-v${VERSION}.${ARCHIVE_EXT}"
mkdir -p "$RELEASE_DIR"

if [[ "$ARCHIVE_EXT" == "zip" ]]; then
  rm -f "$RELEASE_DIR/$ARCHIVE_NAME"
  DEST_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
  if command -v cygpath >/dev/null 2>&1; then
    DEST_PATH="$(cygpath -w "$DEST_PATH")"
  fi
  POWERSHELL_BIN="powershell"
  if command -v pwsh >/dev/null 2>&1; then
    POWERSHELL_BIN="pwsh"
  elif ! command -v powershell >/dev/null 2>&1; then
    echo "ERROR: PowerShell is required to create Windows zip archives." >&2
    exit 1
  fi
  (
    cd "$STAGE_DIR"
    "$POWERSHELL_BIN" -NoProfile -NonInteractive -Command \
      "Compress-Archive -Path * -DestinationPath '$DEST_PATH' -Force"
  )
else
  tar -C "$STAGE_DIR" -czf "$RELEASE_DIR/$ARCHIVE_NAME" .
fi

echo "Backend package ready: $RELEASE_DIR/$ARCHIVE_NAME"
