#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?Android target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

[[ "$TARGET" == riscv64-linux-android ]] || {
  echo "ERROR: unsupported Android target: $TARGET" >&2
  exit 2
}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) host_tag=linux-x86_64 ;;
  *) echo "ERROR: Android NDK builder requires Linux x86_64 host" >&2; exit 2 ;;
esac

NDK_REV=r29
NDK_DIR="${RUNNER_TEMP:-/tmp}/agena-android-ndk/$NDK_REV/android-ndk-$NDK_REV"
ARCHIVE="${RUNNER_TEMP:-/tmp}/agena-android-ndk/$NDK_REV/android-ndk-$NDK_REV-linux.zip"
URL="https://dl.google.com/android/repository/android-ndk-$NDK_REV-linux.zip"
SHA1=87e2bb7e9be5d6a1c6cdf5ec40dd4e0c6d07c30b
mkdir -p "$(dirname "$ARCHIVE")"

if [[ ! -x "$NDK_DIR/toolchains/llvm/prebuilt/$host_tag/bin/clang" ]]; then
  python3 - "$URL" "$ARCHIVE" "$SHA1" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

url, archive_path, expected = sys.argv[1:]
archive = pathlib.Path(archive_path)

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha1()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

if not archive.exists() or digest(archive) != expected:
    tmp = archive.with_suffix(".tmp")
    with urllib.request.urlopen(url, timeout=300) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f"Android NDK SHA1 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY
  rm -rf "$NDK_DIR"
  unzip -q "$ARCHIVE" -d "$(dirname "$NDK_DIR")"
fi

BIN="$NDK_DIR/toolchains/llvm/prebuilt/$host_tag/bin"
# RISC-V Android is available starting at API 35. Use the current stable API
# floor that provides a complete libc/sysroot for the target.
API=35
CC="$BIN/riscv64-linux-android${API}-clang"
CXX="$BIN/riscv64-linux-android${API}-clang++"
AR="$BIN/llvm-ar"
[[ -x "$CC" && -x "$CXX" && -x "$AR" ]] || {
  echo "ERROR: Android NDK r29 RISC-V tools missing" >&2
  exit 1
}

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$CC"
export "CXX_${key}=$CXX"
export "AR_${key}=$AR"
export "CARGO_TARGET_${key_upper}_LINKER=$CC"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$CC"

exec "$@"
