#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TARGET_TRIPLE="${1:?target triple is required}"
BUILD_STD="${2:-false}"
ARTIFACT_KIND="${3:-backend}"
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

case "$TARGET_TRIPLE" in
  i586-unknown-linux-*)
    export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+sse,+sse2"
    ;;
  mips64-unknown-linux-gnuabi64|mips64el-unknown-linux-gnuabi64)
    export RUSTFLAGS="${RUSTFLAGS:-} -C relocation-model=static"
    ;;
  aarch64_be-unknown-linux-gnu)
    # Rustix's linux_raw backend does not support big-endian AArch64. Force
    # every transitive Rustix version onto its libc backend.
    export RUSTFLAGS="${RUSTFLAGS:-} --cfg rustix_use_libc"
    ;;
esac

PACKAGE="agena"
BINARY_BASENAME="agena"
ARCHIVE_PREFIX="agena-backend"
if [[ "$ARTIFACT_KIND" == "web-runtime" ]]; then
  PACKAGE="agena-web-runtime"
  BINARY_BASENAME="agena-web-runtime"
  ARCHIVE_PREFIX="agena-web-runtime"
fi

build_args=(
  build
  --manifest-path "$SERVER_MANIFEST"
  -p "$PACKAGE"
  --release
  --target "$TARGET_TRIPLE"
  --locked
  --target-dir "$SERVER_TARGET_DIR"
)
if [[ "$BUILD_STD" == true ]]; then
  NIGHTLY="$(python3 -c 'import json; print(json.load(open("scripts/agena/universal-targets.json"))["nightly_toolchain"])')"
  cross "+$NIGHTLY" "${build_args[@]}" -Z build-std=std,panic_abort,proc_macro
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
    [[ -f "$OUTPUT_DIR/${BINARY_BASENAME}.exe" ]] || { echo "ERROR: missing $OUTPUT_DIR/${BINARY_BASENAME}.exe" >&2; exit 1; }
    cp "$OUTPUT_DIR/${BINARY_BASENAME}.exe" "$STAGE_DIR/bin/${BINARY_BASENAME}.exe"
    archive_ext="zip"
    ;;
  wasm32-unknown-emscripten)
    found=0
    for suffix in js wasm data worker.js; do
      candidate="$OUTPUT_DIR/${BINARY_BASENAME}.$suffix"
      if [[ -f "$candidate" ]]; then
        cp "$candidate" "$STAGE_DIR/bin/"
        found=1
      fi
    done
    [[ "$found" == 1 ]] || { echo "ERROR: no Emscripten Agena outputs found in $OUTPUT_DIR" >&2; exit 1; }
    archive_ext="tar.gz"
    ;;
  *)
    [[ -f "$OUTPUT_DIR/$BINARY_BASENAME" ]] || { echo "ERROR: missing $OUTPUT_DIR/$BINARY_BASENAME" >&2; exit 1; }
    cp "$OUTPUT_DIR/$BINARY_BASENAME" "$STAGE_DIR/bin/$BINARY_BASENAME"
    archive_ext="tar.gz"
    ;;
esac

python3 - "$STAGE_DIR/manifest.json" "$VERSION" "$TARGET_TRIPLE" "$BUILD_STD" "$ARTIFACT_KIND" <<'PY'
import json, sys
from pathlib import Path
path, version, target, build_std, artifact_kind = sys.argv[1:]
Path(path).write_text(json.dumps({
    "name": "agena",
    "version": version,
    "target": target,
    "build_std": build_std == "true",
    "artifact_kind": artifact_kind,
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
ARCHIVE_NAME="${ARCHIVE_PREFIX}-${TARGET_TRIPLE}-v${VERSION}.${archive_ext}"
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

ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
sha256sum "$ARCHIVE_PATH" | sed "s#  $ARCHIVE_PATH#  $ARCHIVE_NAME#" > "$ARCHIVE_PATH.sha256"
echo "Package ready: $ARCHIVE_PATH"
echo "Checksum ready: $ARCHIVE_PATH.sha256"
