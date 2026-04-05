#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
TARGET="${1:-}"

if ! command -v cargo-tauri >/dev/null 2>&1; then
  echo "ERROR: cargo-tauri is not installed." >&2
  echo "Install (CEF): cargo install tauri-cli --locked --git https://github.com/tauri-apps/tauri --branch feat/cef" >&2
  exit 1
fi

if [[ "$(uname -s)" == "Linux" ]]; then
  if ! command -v pkg-config >/dev/null 2>&1; then
    echo "ERROR: pkg-config is required to build on Linux." >&2
    echo "On Debian/Ubuntu: sudo apt install -y pkg-config" >&2
    exit 1
  fi

  if ! pkg-config --exists gtk+-3.0 2>/dev/null; then
    echo "ERROR: GTK3 development packages are missing (pkg-config: gtk+-3.0)." >&2
    echo "On Debian/Ubuntu: sudo apt install -y libgtk-3-dev" >&2
    exit 1
  fi

  if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null && ! pkg-config --exists webkit2gtk-4.0 2>/dev/null; then
    echo "ERROR: WebKitGTK development packages are missing (pkg-config: webkit2gtk-4.1 or webkit2gtk-4.0)." >&2
    echo "On Debian/Ubuntu: sudo apt install -y libwebkit2gtk-4.1-dev" >&2
    exit 1
  fi
fi

"$REPO_ROOT/ops/agena-studio/scripts/build-frontend-dist.sh"

if [[ -n "$TARGET" ]]; then
  "$SCRIPT_DIR/prepare-sidecar.sh" --cef "$TARGET"
else
  "$SCRIPT_DIR/prepare-sidecar.sh" --cef
fi

export CARGO_TARGET_DIR="$REPO_ROOT/artifacts/t/cef"
mkdir -p "$CARGO_TARGET_DIR"

cd "$REPO_ROOT/apps/agena-studio-desktop/src-tauri-cef"

BUNDLE_ARGS=()
if [[ -n "${TAURI_BUNDLES:-}" ]]; then
  if [[ "${TAURI_BUNDLES}" == "none" ]]; then
    BUNDLE_ARGS=(--no-bundle)
  else
    if [[ "$(uname -s)" == "Linux" ]] && [[ "${TAURI_BUNDLES}" == *appimage* ]] && ! command -v zsyncmake >/dev/null 2>&1; then
      echo "ERROR: AppImage bundling requires \`zsyncmake\` (package: zsync)." >&2
      echo "On Debian/Ubuntu: sudo apt install -y zsync" >&2
      exit 1
    fi
    BUNDLE_ARGS=(--bundles "${TAURI_BUNDLES}")
  fi
elif [[ "$(uname -s)" == "Linux" ]] && ! command -v zsyncmake >/dev/null 2>&1; then
  echo "WARN: zsyncmake not found; skipping AppImage bundle (install \`zsync\` to enable)." >&2
  BUNDLE_ARGS=(--bundles deb,rpm)
fi

if [[ -n "$TARGET" ]]; then
  cargo tauri build --config tauri.conf.full.json --target "$TARGET" --features cef "${BUNDLE_ARGS[@]}"
else
  cargo tauri build --config tauri.conf.full.json --features cef "${BUNDLE_ARGS[@]}"
fi

if [[ -n "$TARGET" ]]; then
  BUNDLE_SOURCE_DIR="$CARGO_TARGET_DIR/$TARGET/release/bundle"
  BUNDLE_EXPORT_DIR="$REPO_ROOT/artifacts/agena-studio/desktop/$TARGET/cef"
else
  BUNDLE_SOURCE_DIR="$CARGO_TARGET_DIR/release/bundle"
  BUNDLE_EXPORT_DIR="$REPO_ROOT/artifacts/agena-studio/desktop/host/cef"
fi

rm -rf "$BUNDLE_EXPORT_DIR"
mkdir -p "$(dirname "$BUNDLE_EXPORT_DIR")"
cp -R "$BUNDLE_SOURCE_DIR" "$BUNDLE_EXPORT_DIR"
echo "Desktop bundle exported: $BUNDLE_EXPORT_DIR"
