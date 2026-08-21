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
[[ "$ARTIFACT_KIND" == "backend" ]] || {
  echo "ERROR: only full Agena backend artifacts are supported: $ARTIFACT_KIND" >&2
  exit 2
}

case "$TARGET_TRIPLE" in
  i586-unknown-linux-*)
    export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+sse,+sse2"
    ;;
  mips-unknown-linux-gnu|mipsel-unknown-linux-gnu)
    # Large Agena codegen can otherwise reach LLVM's integrated assembler with
    # an out-of-range PC16 branch. Force the MIPS long-branch expansion pass.
    export RUSTFLAGS="${RUSTFLAGS:-} -C llvm-args=--force-mips-long-branch"
    ;;
  mips64-unknown-linux-gnuabi64|mips64el-unknown-linux-gnuabi64)
    # MIPS n64 PLT entries must remain in the signed 32-bit addressable range.
    # The GNU linker otherwise places non-PIE executables above 4 GiB by
    # default, which makes .got.plt unusable for the n64 PLT sequence.
    export RUSTFLAGS="${RUSTFLAGS:-} -C relocation-model=static -C code-model=large -C llvm-args=--force-mips-long-branch -C link-arg=-Wl,-Ttext-segment=0x10000000"
    ;;
  sparcv9-sun-solaris|x86_64-pc-solaris)
    # The pinned cross Solaris 10 images provide /compat.o to map the XPG7
    # symbols used by modern Rust std back to the Solaris 10 ABI.
    export RUSTFLAGS="${RUSTFLAGS:-} -C link-arg=/compat.o"
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
