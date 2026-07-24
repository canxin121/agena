#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SERVER_MANIFEST="$REPO_ROOT/Cargo.toml"
SERVER_TARGET_DIR="$REPO_ROOT/target"
RELEASE_DIR="$REPO_ROOT/artifacts/agena"
WEB_DIST_DIR="$REPO_ROOT/packages/agena-web-ui/dist"

detect_host_triple() {
  if rustc --print host-tuple >/dev/null 2>&1; then
    rustc --print host-tuple
    return
  fi
  rustc -Vv | awk '/^host:/{print $2; exit}'
}

read_version() {
  cargo metadata --manifest-path "$SERVER_MANIFEST" --format-version 1 --no-deps --locked \
    | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "agena"))'
}

TARGET_TRIPLE="${1:-$(detect_host_triple)}"
VERSION="$(read_version)"

if [[ -z "$VERSION" ]]; then
  echo "ERROR: failed to read agena version from $SERVER_MANIFEST" >&2
  exit 1
fi

EXT=""
ARCHIVE_EXT="tar.gz"
case "$TARGET_TRIPLE" in
  *-pc-windows-*) EXT=".exe"; ARCHIVE_EXT="zip" ;;
esac

bash "$SCRIPT_DIR/build-frontend-dist.sh"

echo "Building agena backend for ${TARGET_TRIPLE}..."
cargo build \
  --manifest-path "$SERVER_MANIFEST" \
  --release \
  --target "$TARGET_TRIPLE" \
  --locked \
  --target-dir "$SERVER_TARGET_DIR"

BIN_PATH="$SERVER_TARGET_DIR/$TARGET_TRIPLE/release/agena$EXT"
if [[ ! -f "$BIN_PATH" ]]; then
  echo "ERROR: built backend binary not found at $BIN_PATH" >&2
  exit 1
fi

STAGE_DIR="$RELEASE_DIR/backend/$TARGET_TRIPLE"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/web-dist"

cp "$BIN_PATH" "$STAGE_DIR/bin/agena$EXT"
cp -R "$WEB_DIST_DIR/." "$STAGE_DIR/web-dist/"

cat > "$STAGE_DIR/README.txt" <<EOF
Agena server package
Version: $VERSION
Target: $TARGET_TRIPLE

Contents:
- bin/agena$EXT
- web-dist/

Example:
  agena server --ui-dir ./web-dist
EOF

ARCHIVE_NAME="agena-backend-${TARGET_TRIPLE}-v${VERSION}.${ARCHIVE_EXT}"
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
