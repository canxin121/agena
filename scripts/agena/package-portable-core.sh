#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TARGET_TRIPLE="${1:?target triple is required}"
BUILD_STD="${2:-false}"
TARGET_RUSTFLAGS="${3:-}"
TARGET_DIR="$REPO_ROOT/target"
RELEASE_DIR="$REPO_ROOT/artifacts/agena"

VERSION="$(cargo metadata --manifest-path "$REPO_ROOT/Cargo.toml" --format-version 1 --no-deps --locked \
  | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "agena-portable-core"))')"

if [[ "$BUILD_STD" == true ]]; then
  STABLE_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
  NIGHTLY_TOOLCHAIN="${AGENA_NIGHTLY_TOOLCHAIN:-nightly-2026-08-18}"
  stable_rustc="$(rustup which --toolchain "$STABLE_TOOLCHAIN" rustc)"
  stable_rustdoc="$(rustup which --toolchain "$STABLE_TOOLCHAIN" rustdoc)"
  RUSTC_BOOTSTRAP=1 \
  RUSTC="$stable_rustc" \
  RUSTDOC="$stable_rustdoc" \
  RUSTFLAGS="$TARGET_RUSTFLAGS" \
    cargo "+$NIGHTLY_TOOLCHAIN" build \
      --manifest-path "$REPO_ROOT/Cargo.toml" \
      -p agena-portable-core \
      --release \
      --target "$TARGET_TRIPLE" \
      --locked \
      --target-dir "$TARGET_DIR" \
      -Z build-std=core
else
  RUSTFLAGS="$TARGET_RUSTFLAGS" cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    -p agena-portable-core \
    --release \
    --target "$TARGET_TRIPLE" \
    --locked \
    --target-dir "$TARGET_DIR"
fi

LIB="$TARGET_DIR/$TARGET_TRIPLE/release/libagena_portable_core.rlib"
[[ -f "$LIB" ]] || { echo "ERROR: missing portable core artifact $LIB" >&2; exit 1; }

STAGE="$RELEASE_DIR/portable/$TARGET_TRIPLE"
rm -rf "$STAGE"
mkdir -p "$STAGE/lib"
cp "$LIB" "$STAGE/lib/"
python3 - "$STAGE/manifest.json" "$VERSION" "$TARGET_TRIPLE" "$BUILD_STD" "$TARGET_RUSTFLAGS" <<'PY'
import json, sys
from pathlib import Path
path, version, target, build_std, rustflags = sys.argv[1:]
Path(path).write_text(json.dumps({
    "name": "agena-portable-core",
    "version": version,
    "target": target,
    "artifact_kind": "portable-core",
    "abi_version": 1,
    "build_std": build_std == "true",
    "rustflags": rustflags,
    "contents": ["lib/libagena_portable_core.rlib"],
}, indent=2) + "\n", encoding="utf-8")
PY
cat > "$STAGE/README.txt" <<EOF
Agena portable core
Version: $VERSION
Target: $TARGET_TRIPLE

This target does not host the full Agena daemon/TUI process model. The package
contains the target-native no_std Agena core rlib for embedding/linking.
EOF

ARCHIVE="agena-portable-core-${TARGET_TRIPLE}-v${VERSION}.tar.gz"
mkdir -p "$RELEASE_DIR"
tar -C "$STAGE" -czf "$RELEASE_DIR/$ARCHIVE" .
(cd "$RELEASE_DIR" && sha256sum "$ARCHIVE" > "$ARCHIVE.sha256")
echo "Package ready: $RELEASE_DIR/$ARCHIVE"
