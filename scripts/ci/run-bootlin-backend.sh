#!/usr/bin/env bash
set -euo pipefail

RUST_TARGET="${1:?Rust target triple is required}"
CATEGORY="${2:?Bootlin category is required}"
TOOLCHAIN="${3:?Bootlin toolchain name is required}"
EXTRA_CFLAGS="${4:-}"
shift 4
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *)
    echo "ERROR: Bootlin prebuilts are Linux x86_64 host tools; got $(uname -s)-$(uname -m)" >&2
    exit 2
    ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-bootlin/${TOOLCHAIN}"
ARCHIVE="$ROOT/${TOOLCHAIN}.tar.xz"
EXTRACTED="$ROOT/root"
BASE="https://toolchains.bootlin.com/downloads/releases/toolchains/${CATEGORY}/tarballs"
mkdir -p "$ROOT"

if [[ ! -f "$ROOT/.verified" ]]; then
  python3 - "$BASE" "$TOOLCHAIN" "$ARCHIVE" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

base, name, archive_path = sys.argv[1:]
archive = pathlib.Path(archive_path)
checksum_text = urllib.request.urlopen(f"{base}/{name}.sha256", timeout=60).read().decode().strip()
expected = checksum_text.split()[0]

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

if not archive.exists() or digest(archive) != expected:
    tmp = archive.with_suffix(".tmp")
    with urllib.request.urlopen(f"{base}/{name}.tar.xz", timeout=180) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f"Bootlin SHA256 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY
  rm -rf "$EXTRACTED"
  mkdir -p "$EXTRACTED"
  tar -xJf "$ARCHIVE" -C "$EXTRACTED"
  touch "$ROOT/.verified"
fi

gcc="$(find "$EXTRACTED" -type f -path '*/bin/*-gcc' -perm -111 | head -1)"
[[ -n "$gcc" ]] || { echo "ERROR: Bootlin gcc not found in $EXTRACTED" >&2; exit 1; }
prefix="${gcc%-gcc}"
gxx="${prefix}-g++"
ar="${prefix}-ar"
[[ -x "$gxx" ]] || { echo "ERROR: Bootlin g++ not found at $gxx" >&2; exit 1; }
[[ -x "$ar" ]] || { echo "ERROR: Bootlin ar not found at $ar" >&2; exit 1; }

target_key="${RUST_TARGET//-/_}"
target_key_upper="$(printf '%s' "$target_key" | tr '[:lower:]' '[:upper:]')"
export "CC_${target_key}=$gcc"
export "CXX_${target_key}=$gxx"
export "AR_${target_key}=$ar"
export "CARGO_TARGET_${target_key_upper}_LINKER=$gcc"
if [[ -n "$EXTRA_CFLAGS" ]]; then
  export "CFLAGS_${target_key}=$EXTRA_CFLAGS"
  export "CXXFLAGS_${target_key}=$EXTRA_CFLAGS"
fi
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$gcc"

exec "$@"
