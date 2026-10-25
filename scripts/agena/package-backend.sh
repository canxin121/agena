#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
SERVER_MANIFEST="$REPO_ROOT/Cargo.toml"
RELEASE_DIR="$REPO_ROOT/artifacts/agena"
WEB_PROJECT_DIR="$REPO_ROOT/packages/agena-web"
WEB_DIST_DIR="$WEB_PROJECT_DIR/dist"
export RUSTUP_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
# Cargo discovers configuration from its working directory, not --manifest-path.
cd "$REPO_ROOT"

detect_host_triple() {
  if rustc --print host-tuple >/dev/null 2>&1; then
    rustc --print host-tuple
    return
  fi
  rustc -Vv | awk '/^host:/{print $2; exit}'
}

for required_tool in cargo rustc python3 tar; do
  if ! command -v "$required_tool" >/dev/null 2>&1; then
    echo "ERROR: $required_tool is required to build Agena from source." >&2
    exit 1
  fi
done
if ! command -v sha256sum >/dev/null 2>&1 && ! command -v shasum >/dev/null 2>&1; then
  echo "ERROR: sha256sum or shasum is required to checksum the Agena package." >&2
  exit 1
fi

HOST_TRIPLE="$(detect_host_triple)"
TARGET_TRIPLE="${1:-$HOST_TRIPLE}"
BUILD_STD="${2:-false}"
TARGET_RUSTFLAGS="${3:-}"
COMBINED_RUSTFLAGS="${RUSTFLAGS:-}"
if [[ -n "$TARGET_RUSTFLAGS" ]]; then
  COMBINED_RUSTFLAGS="${COMBINED_RUSTFLAGS:+$COMBINED_RUSTFLAGS }$TARGET_RUSTFLAGS"
fi
CARGO_METADATA="$(cargo metadata --manifest-path "$SERVER_MANIFEST" --format-version 1 --no-deps --locked)"
VERSION="$(printf '%s' "$CARGO_METADATA" | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "agena"))')"
SERVER_TARGET_DIR="$(printf '%s' "$CARGO_METADATA" | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"


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
  bun "$SCRIPT_DIR/build-web.mjs" "$WEB_PROJECT_DIR"
fi
if [[ ! -f "$WEB_DIST_DIR/index.html" ]]; then
  echo "ERROR: Web frontend not found at $WEB_DIST_DIR" >&2
  exit 1
fi

build_args=(
  build
  --manifest-path "$SERVER_MANIFEST"
  -p agena
  --bin agena
  --release
  --locked
  --target-dir "$SERVER_TARGET_DIR"
  --message-format=json-render-diagnostics
)
# A host --target still creates a separate Cargo graph and artifact directory.
# Ordinary source installs must share the graph used by cargo build --release.
EXPLICIT_TARGET=0
if [[ "$TARGET_TRIPLE" != "$HOST_TRIPLE" || "$BUILD_STD" == true ||
      -n "$TARGET_RUSTFLAGS" || -n "${CARGO_BUILD_TARGET:-}" ||
      "${AGENA_FORCE_EXPLICIT_TARGET:-0}" == 1 ]]; then
  build_args+=(--target "$TARGET_TRIPLE")
  EXPLICIT_TARGET=1
fi
# Keep this array nonempty for macOS Bash 3.2 with set -u.
build_env=(env)
# Preserve unset RUSTFLAGS: exporting an empty value suppresses Cargo config.
if [[ -n "$TARGET_RUSTFLAGS" ]]; then
  if [[ -n "${CARGO_ENCODED_RUSTFLAGS+x}" ]]; then
    encoded_flags="$(python3 -c 'import shlex, sys; print(chr(31).join(([sys.argv[1]] if sys.argv[1] else []) + shlex.split(sys.argv[2])), end="")' "$CARGO_ENCODED_RUSTFLAGS" "$TARGET_RUSTFLAGS")"
    build_env+=("CARGO_ENCODED_RUSTFLAGS=$encoded_flags")
  else
    build_env+=("RUSTFLAGS=$COMBINED_RUSTFLAGS")
  fi
fi
BUILD_MESSAGES="$(mktemp -t agena-cargo-build.XXXXXX)"
trap 'rm -f "$BUILD_MESSAGES"' EXIT
rust_started=$SECONDS
echo "Building Rust backend (Cargo cache: $SERVER_TARGET_DIR)..."
if [[ "$BUILD_STD" == true ]]; then
  "${build_env[@]}" bash "$REPO_ROOT/scripts/ci/run-build-std-cargo.sh" "$TARGET_TRIPLE" "${build_args[@]}" > "$BUILD_MESSAGES"
else
  "${build_env[@]}" cargo "${build_args[@]}" > "$BUILD_MESSAGES"
fi
echo "Rust backend ready in $((SECONDS - rust_started))s."

BIN_PATH="$(python3 - "$BUILD_MESSAGES" "$SERVER_TARGET_DIR" "$TARGET_TRIPLE" "$EXT" "$EXPLICIT_TARGET" <<'PY'
import json, sys
from pathlib import Path
messages, target_dir, triple, ext, explicit = sys.argv[1:]
artifacts = set()
for line in Path(messages).read_text().splitlines():
    try:
        item = json.loads(line)
    except ValueError:
        continue
    target = item.get('target', {})
    if (item.get('reason') == 'compiler-artifact' and target.get('name') == 'agena'
            and 'bin' in target.get('kind', []) and item.get('executable')):
        artifacts.add(Path(item['executable']).resolve())
allowed = {(Path(target_dir) / triple / 'release' / ('agena' + ext)).resolve()}
if explicit == '0':
    allowed.add((Path(target_dir) / 'release' / ('agena' + ext)).resolve())
if len(artifacts) != 1 or not artifacts <= allowed:
    raise SystemExit('ERROR: Cargo did not report the requested Agena executable; '
                     'check build.target or set AGENA_FORCE_EXPLICIT_TARGET=1')
print(artifacts.pop())
PY
)"
if [[ ! -f "$BIN_PATH" ]]; then
  echo "ERROR: built binary not found at $BIN_PATH" >&2
  exit 1
fi

ARCHIVE_NAME="agena-backend-${TARGET_TRIPLE}-v${VERSION}.${ARCHIVE_EXT}"
ARCHIVE_PATH="$RELEASE_DIR/$ARCHIVE_NAME"
package_key() {
  python3 "$SCRIPT_DIR/package-cache.py" fingerprint "$BIN_PATH" "$WEB_DIST_DIR" "$VERSION" "$TARGET_TRIPLE" "$SCRIPT_DIR/package-backend.sh"
}
PACKAGE_KEY="$(package_key)"
if python3 "$SCRIPT_DIR/package-cache.py" check "$ARCHIVE_PATH" "$PACKAGE_KEY"; then
  echo "Package unchanged; reusing verified archive."
  echo "Package ready: $ARCHIVE_PATH"
  echo "Checksum ready: $ARCHIVE_PATH.sha256"
  exit 0
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

mkdir -p "$RELEASE_DIR"
echo "Creating package archive..."

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

if command -v sha256sum >/dev/null 2>&1; then
  (cd "$RELEASE_DIR" && sha256sum "$ARCHIVE_NAME" > "$ARCHIVE_NAME.sha256")
else
  checksum="$(shasum -a 256 "$ARCHIVE_PATH" | awk '{print $1}')"
  printf '%s  %s
' "$checksum" "$ARCHIVE_NAME" > "$ARCHIVE_PATH.sha256"
fi

if [[ "$(package_key)" != "$PACKAGE_KEY" ]]; then
  echo "ERROR: build inputs changed while packaging; rerun the source build." >&2
  exit 1
fi
python3 "$SCRIPT_DIR/package-cache.py" record "$ARCHIVE_PATH" "$PACKAGE_KEY"

echo "Package ready: $ARCHIVE_PATH"
echo "Checksum ready: $ARCHIVE_PATH.sha256"
