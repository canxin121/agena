#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
TARGET="${1:-}"

"$REPO_ROOT/ops/agena-studio/scripts/build-frontend-dist.sh"

if [[ -n "$TARGET" ]]; then
  "$SCRIPT_DIR/prepare-sidecar.sh" "$TARGET"
else
  "$SCRIPT_DIR/prepare-sidecar.sh"
fi

export CARGO_TARGET_DIR="$REPO_ROOT/artifacts/t/desktop"
mkdir -p "$CARGO_TARGET_DIR"

cd "$REPO_ROOT/apps/agena-studio-desktop/src-tauri"

if [[ -n "$TARGET" ]]; then
  cargo tauri build --config tauri.conf.full.json --target "$TARGET"
else
  cargo tauri build --config tauri.conf.full.json
fi

if [[ -n "$TARGET" ]]; then
  BUNDLE_SOURCE_DIR="$CARGO_TARGET_DIR/$TARGET/release/bundle"
  BUNDLE_EXPORT_DIR="$REPO_ROOT/artifacts/agena-studio/desktop/$TARGET/standard"
else
  BUNDLE_SOURCE_DIR="$CARGO_TARGET_DIR/release/bundle"
  BUNDLE_EXPORT_DIR="$REPO_ROOT/artifacts/agena-studio/desktop/host/standard"
fi

rm -rf "$BUNDLE_EXPORT_DIR"
mkdir -p "$(dirname "$BUNDLE_EXPORT_DIR")"
cp -R "$BUNDLE_SOURCE_DIR" "$BUNDLE_EXPORT_DIR"
echo "Desktop bundle exported: $BUNDLE_EXPORT_DIR"
