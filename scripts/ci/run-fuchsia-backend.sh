#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?Fuchsia target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: Fuchsia SDK builder requires Linux x86_64 host" >&2; exit 2 ;;
esac

# These are the versions currently pinned by Rust upstream's
# host-x86_64/dist-various-2 image. Reusing them keeps our target ABI/toolchain
# aligned with Rust's own Fuchsia distribution checks.
SDK_ID='version:26.20241211.7.1'
SDK_SHA256=2cb7a9a0419f7413a46e0ccef7dad89f7c9979940d7c1ee87fac70ff499757d6
SDK_URL="https://chrome-infra-packages.appspot.com/dl/fuchsia/sdk/core/linux-amd64/+/$SDK_ID"
CLANG_ID='git_revision:388d7f144880dcd85ff31f06793304405a9f44b6'
CLANG_SHA256=970d1f427b9c9a3049d8622c80c86830ff31b5334ad8da47a2f1e81143197e8b
CLANG_URL="https://chrome-infra-packages.appspot.com/dl/fuchsia/third_party/clang/linux-amd64/+/$CLANG_ID"
ROOT="${RUNNER_TEMP:-/tmp}/agena-fuchsia-toolchain"
SDK="$ROOT/sdk"
CLANG="$ROOT/clang"
mkdir -p "$ROOT"

fetch_zip() {
  local url="$1" expected="$2" archive="$3" dest="$4"
  python3 - "$url" "$expected" "$archive" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

url, expected, archive_path = sys.argv[1:]
archive = pathlib.Path(archive_path)

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
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
        raise SystemExit(f"Fuchsia archive SHA256 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY
  rm -rf "$dest"
  mkdir -p "$dest"
  unzip -q "$archive" -d "$dest"
}

if [[ ! -d "$SDK/arch" ]]; then
  fetch_zip "$SDK_URL" "$SDK_SHA256" "$ROOT/sdk.zip" "$SDK"
fi
if [[ ! -x "$CLANG/bin/clang" ]]; then
  fetch_zip "$CLANG_URL" "$CLANG_SHA256" "$ROOT/clang.zip" "$CLANG"
fi

case "$TARGET" in
  aarch64-unknown-fuchsia) sdk_arch=arm64; clang_target=aarch64-unknown-fuchsia ;;
  riscv64gc-unknown-fuchsia) sdk_arch=riscv64; clang_target=riscv64-unknown-fuchsia ;;
  x86_64-unknown-fuchsia) sdk_arch=x64; clang_target=x86_64-unknown-fuchsia ;;
  *) echo "ERROR: unsupported Fuchsia target: $TARGET" >&2; exit 2 ;;
esac

SYSROOT="$SDK/arch/$sdk_arch/sysroot"
LIBDIR="$SDK/arch/$sdk_arch/lib"
FDIO="$SDK/pkg/fdio/include"
[[ -d "$SYSROOT" && -d "$LIBDIR" && -d "$FDIO" ]] || {
  echo "ERROR: pinned Fuchsia SDK does not contain architecture '$sdk_arch' for $TARGET" >&2
  exit 1
}

WRAP="${RUNNER_TEMP:-/tmp}/agena-fuchsia-wrappers/$TARGET"
mkdir -p "$WRAP"
for mode in cc cxx; do
  compiler="$CLANG/bin/clang"
  [[ "$mode" == cxx ]] && compiler="$CLANG/bin/clang++"
  cat >"$WRAP/$mode" <<EOF
#!/bin/sh
exec "$compiler" --target="$clang_target" --sysroot="$SYSROOT" -I"$FDIO" "\$@"
EOF
  chmod +x "$WRAP/$mode"
done

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$WRAP/cc"
export "CXX_${key}=$WRAP/cxx"
export "AR_${key}=$CLANG/bin/llvm-ar"
export "CARGO_TARGET_${key_upper}_LINKER=$WRAP/cc"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAP/cc -C link-arg=--sysroot=$SYSROOT -Lnative=$SYSROOT/lib -Lnative=$LIBDIR"

exec "$@"
