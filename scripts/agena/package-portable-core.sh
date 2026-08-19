#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TARGET_TRIPLE="${1:?target triple is required}"
TARGET_DIR="$REPO_ROOT/target"
RELEASE_DIR="$REPO_ROOT/artifacts/agena"

VERSION="$(cargo metadata --manifest-path "$REPO_ROOT/Cargo.toml" --format-version 1 --no-deps --locked \
  | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "agena-portable-core"))')"

cargo build \
  --manifest-path "$REPO_ROOT/Cargo.toml" \
  -p agena-portable-core \
  --release \
  --target "$TARGET_TRIPLE" \
  --locked \
  --target-dir "$TARGET_DIR"

LIB="$TARGET_DIR/$TARGET_TRIPLE/release/libagena_portable_core.rlib"
[[ -f "$LIB" ]] || { echo "ERROR: missing portable core artifact $LIB" >&2; exit 1; }

STAGE="$RELEASE_DIR/portable/$TARGET_TRIPLE"
rm -rf "$STAGE"
mkdir -p "$STAGE/lib"
cp "$LIB" "$STAGE/lib/"
python3 - "$STAGE/manifest.json" "$VERSION" "$TARGET_TRIPLE" <<'PY'
import json, sys
from pathlib import Path
path, version, target = sys.argv[1:]
Path(path).write_text(json.dumps({
    "name": "agena-portable-core",
    "version": version,
    "target": target,
    "artifact_kind": "portable-core",
    "abi_version": 1,
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
