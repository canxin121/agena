#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SERVER_MANIFEST="$REPO_ROOT/Cargo.toml"
SERVER_TARGET_DIR="$REPO_ROOT/target"
RELEASE_DIR="$REPO_ROOT/artifacts/agena"
WEB_PROJECT_DIR="$REPO_ROOT/packages/agena-web"
WEB_DIST_DIR="$WEB_PROJECT_DIR/dist"
export RUSTUP_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"

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
BUILD_STD="${2:-false}"
TARGET_RUSTFLAGS="${3:-}"
COMBINED_RUSTFLAGS="${RUSTFLAGS:-}"
if [[ -n "$TARGET_RUSTFLAGS" ]]; then
  COMBINED_RUSTFLAGS="${COMBINED_RUSTFLAGS:+$COMBINED_RUSTFLAGS }$TARGET_RUSTFLAGS"
fi
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

echo "Building agena for ${TARGET_TRIPLE}..."
if [[ "${AGENA_WEB_DIST_PREBUILT:-0}" != "1" ]]; then
  if ! command -v bun >/dev/null 2>&1; then
    echo "ERROR: bun is required to build the Agena Web frontend." >&2
    exit 1
  fi
  if [[ ! -d "$WEB_PROJECT_DIR/node_modules" ]]; then
    (
      cd "$WEB_PROJECT_DIR"
      bun install --frozen-lockfile
    )
  fi
  (
    cd "$WEB_PROJECT_DIR"
    bun run build
  )
fi
if [[ ! -f "$WEB_DIST_DIR/index.html" ]]; then
  echo "ERROR: Web frontend not found at $WEB_DIST_DIR" >&2
  exit 1
fi

build_args=(
  build
  --manifest-path "$SERVER_MANIFEST"
  -p agena
  --release
  --target "$TARGET_TRIPLE"
  --locked
  --target-dir "$SERVER_TARGET_DIR"
)
if [[ "$BUILD_STD" == true ]]; then
  RUSTFLAGS="$COMBINED_RUSTFLAGS" \
    bash "$REPO_ROOT/scripts/ci/run-build-std-cargo.sh" "$TARGET_TRIPLE" "${build_args[@]}"
else
  RUSTFLAGS="$COMBINED_RUSTFLAGS" cargo "${build_args[@]}"
fi

BIN_PATH="$SERVER_TARGET_DIR/$TARGET_TRIPLE/release/agena$EXT"
if [[ ! -f "$BIN_PATH" ]]; then
  echo "ERROR: built binary not found at $BIN_PATH" >&2
  exit 1
fi

STAGE_DIR="$RELEASE_DIR/backend/$TARGET_TRIPLE"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/web-dist"

cp "$BIN_PATH" "$STAGE_DIR/bin/agena$EXT"
cp -R "$WEB_DIST_DIR/." "$STAGE_DIR/web-dist/"

cat > "$STAGE_DIR/README.txt" <<EOF
Agena package
Version: $VERSION
Target: $TARGET_TRIPLE

Contents:
- bin/agena$EXT
- web-dist/ (served by the Agena server on the same host and port)
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

ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$RELEASE_DIR" && sha256sum "$ARCHIVE_NAME" > "$ARCHIVE_NAME.sha256")
else
  checksum="$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')"
  printf '%s  %s
' "$checksum" "$ARCHIVE_NAME" > "$ARCHIVE_PATH.sha256"
fi

echo "Package ready: $ARCHIVE_PATH"
echo "Checksum ready: $ARCHIVE_PATH.sha256"
