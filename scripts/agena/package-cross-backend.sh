#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TARGET_TRIPLE="${1:?target triple is required}"
BUILD_STD="${2:-false}"
SERVER_MANIFEST="$REPO_ROOT/Cargo.toml"
SERVER_TARGET_DIR="$REPO_ROOT/target"
RELEASE_DIR="$REPO_ROOT/artifacts/agena"
WEB_DIST_DIR="$REPO_ROOT/packages/agena-web/dist"

read_version() {
  cargo metadata --manifest-path "$SERVER_MANIFEST" --format-version 1 --no-deps --locked \
    | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "agena"))'
}

VERSION="$(read_version)"
[[ -n "$VERSION" ]] || { echo "ERROR: failed to read Agena version" >&2; exit 1; }
[[ -f "$WEB_DIST_DIR/index.html" ]] || {
  echo "ERROR: prebuilt Web frontend not found at $WEB_DIST_DIR" >&2
  exit 1
}
command -v cross >/dev/null 2>&1 || { echo "ERROR: cross is required" >&2; exit 1; }

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
  NIGHTLY="$(python3 -c 'import json; print(json.load(open("scripts/agena/universal-targets.json"))["nightly_toolchain"])')"
  cross "+$NIGHTLY" "${build_args[@]}" -Z build-std=std,panic_abort
else
  cross "${build_args[@]}"
fi

OUTPUT_DIR="$SERVER_TARGET_DIR/$TARGET_TRIPLE/release"
STAGE_DIR="$RELEASE_DIR/backend/$TARGET_TRIPLE"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/web-dist"
cp -R "$WEB_DIST_DIR/." "$STAGE_DIR/web-dist/"

case "$TARGET_TRIPLE" in
  *-windows-*)
    [[ -f "$OUTPUT_DIR/agena.exe" ]] || { echo "ERROR: missing $OUTPUT_DIR/agena.exe" >&2; exit 1; }
    cp "$OUTPUT_DIR/agena.exe" "$STAGE_DIR/bin/agena.exe"
    archive_ext="zip"
    ;;
  wasm32-unknown-emscripten)
    found=0
    for suffix in js wasm data worker.js; do
      candidate="$OUTPUT_DIR/agena.$suffix"
      if [[ -f "$candidate" ]]; then
        cp "$candidate" "$STAGE_DIR/bin/"
        found=1
      fi
    done
    [[ "$found" == 1 ]] || { echo "ERROR: no Emscripten Agena outputs found in $OUTPUT_DIR" >&2; exit 1; }
    archive_ext="tar.gz"
    ;;
  *)
    [[ -f "$OUTPUT_DIR/agena" ]] || { echo "ERROR: missing $OUTPUT_DIR/agena" >&2; exit 1; }
    cp "$OUTPUT_DIR/agena" "$STAGE_DIR/bin/agena"
    archive_ext="tar.gz"
    ;;
esac

python3 - "$STAGE_DIR/manifest.json" "$VERSION" "$TARGET_TRIPLE" "$BUILD_STD" <<'PY'
import json, sys
from pathlib import Path
path, version, target, build_std = sys.argv[1:]
Path(path).write_text(json.dumps({
    "name": "agena",
    "version": version,
    "target": target,
    "build_std": build_std == "true",
    "contents": ["bin/", "web-dist/"],
}, indent=2) + "\n", encoding="utf-8")
PY

cat > "$STAGE_DIR/README.txt" <<EOF
Agena universal backend package
Version: $VERSION
Target: $TARGET_TRIPLE

Contents:
- bin/ (target-native Agena executable/runtime files)
- web-dist/ (Agena Web frontend)
- manifest.json
EOF

mkdir -p "$RELEASE_DIR"
ARCHIVE_NAME="agena-backend-${TARGET_TRIPLE}-v${VERSION}.${archive_ext}"
rm -f "$RELEASE_DIR/$ARCHIVE_NAME"
if [[ "$archive_ext" == zip ]]; then
  python3 - "$STAGE_DIR" "$RELEASE_DIR/$ARCHIVE_NAME" <<'PY'
from pathlib import Path
import sys, zipfile
root=Path(sys.argv[1]); dest=Path(sys.argv[2])
with zipfile.ZipFile(dest, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as zf:
    for path in sorted(root.rglob("*")):
        if path.is_file():
            zf.write(path, path.relative_to(root))
PY
else
  tar -C "$STAGE_DIR" -czf "$RELEASE_DIR/$ARCHIVE_NAME" .
fi

echo "Package ready: $RELEASE_DIR/$ARCHIVE_NAME"
