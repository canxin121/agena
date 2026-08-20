#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?OHOS target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: OpenHarmony SDK builder requires Linux x86_64 host" >&2; exit 2 ;;
esac

case "$TARGET" in
  loongarch64-unknown-linux-ohos)
    SDK_RELEASE=6.0.0.2-Release
    SDK_CACHE_KEY=6.0.0.2
    ;;
  *)
    SDK_RELEASE=5.0.0-Release
    SDK_CACHE_KEY=5.0.0
    ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-ohos-sdk/$SDK_CACHE_KEY"
ARCHIVE="$ROOT/ohos-sdk-windows_linux-public.tar.gz"
SDK="$ROOT/sdk"
URL="https://repo.huaweicloud.com/openharmony/os/$SDK_RELEASE/ohos-sdk-windows_linux-public.tar.gz"
SHA_URL="$URL.sha256"
mkdir -p "$ROOT"

if [[ ! -x "$SDK/native/llvm/bin/clang" ]]; then
  python3 - "$URL" "$SHA_URL" "$ARCHIVE" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

url, sha_url, archive_path = sys.argv[1:]
archive = pathlib.Path(archive_path)
expected = urllib.request.urlopen(sha_url, timeout=60).read().decode().strip().split()[0].lower()

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
        raise SystemExit(f"OpenHarmony SDK SHA256 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY
  rm -rf "$SDK" "$ROOT/linux"
  mkdir -p "$SDK"
  if [[ "$SDK_CACHE_KEY" == 5.0.0 ]]; then
    native_zip=linux/native-linux-x64-5.0.0.71-Release.zip
    tar -xzf "$ARCHIVE" -C "$ROOT" "$native_zip"
  else
    # OpenHarmony patch releases include the native component's internal build
    # number in the zip filename. Match the single official Linux native SDK
    # component instead of coupling this builder to that incidental build id.
    tar -xzf "$ARCHIVE" -C "$ROOT" --wildcards 'linux/native-linux-x64-*.zip'
    native_zip="$(find "$ROOT/linux" -maxdepth 1 -type f -name 'native-linux-x64-*.zip' -print -quit)"
    [[ -n "$native_zip" ]] || { echo "ERROR: native Linux component missing from OpenHarmony SDK" >&2; exit 1; }
    native_zip="${native_zip#$ROOT/}"
  fi
  unzip -q "$ROOT/$native_zip" -d "$SDK"
  rm -rf "$ROOT/linux"
fi

CLANG="$SDK/native/llvm/bin/clang"
CLANGXX="$SDK/native/llvm/bin/clang++"
AR="$SDK/native/llvm/bin/llvm-ar"
SYSROOT="$SDK/native/sysroot"
[[ -x "$CLANG" && -x "$CLANGXX" && -x "$AR" && -d "$SYSROOT" ]] || {
  echo "ERROR: incomplete OpenHarmony native SDK" >&2
  exit 1
}

case "$TARGET" in
  aarch64-unknown-linux-ohos)
    clang_target=aarch64-linux-ohos; extra=() ;;
  armv7-unknown-linux-ohos)
    clang_target=arm-linux-ohos
    extra=(-march=armv7-a -mfloat-abi=softfp -mtune=generic-armv7-a -mthumb) ;;
  loongarch64-unknown-linux-ohos)
    clang_target=loongarch64-linux-ohos; extra=() ;;
  x86_64-unknown-linux-ohos)
    clang_target=x86_64-linux-ohos; extra=() ;;
  *) echo "ERROR: unsupported OHOS target: $TARGET" >&2; exit 2 ;;
esac

WRAP="${RUNNER_TEMP:-/tmp}/agena-ohos-wrappers/$TARGET"
mkdir -p "$WRAP"
python3 - "$WRAP/cc" "$CLANG" "$clang_target" "$SYSROOT" "${extra[*]}" <<'PY'
import pathlib, shlex, sys
path, compiler, target, sysroot, extra = sys.argv[1:]
args = " ".join(shlex.quote(x) for x in shlex.split(extra))
pathlib.Path(path).write_text(
    f'#!/bin/sh\nexec {shlex.quote(compiler)} -target {shlex.quote(target)} '
    f'--sysroot={shlex.quote(sysroot)} -D__MUSL__ {args} "$@"\n'
)
PY
python3 - "$WRAP/cxx" "$CLANGXX" "$clang_target" "$SYSROOT" "${extra[*]}" <<'PY'
import pathlib, shlex, sys
path, compiler, target, sysroot, extra = sys.argv[1:]
args = " ".join(shlex.quote(x) for x in shlex.split(extra))
pathlib.Path(path).write_text(
    f'#!/bin/sh\nexec {shlex.quote(compiler)} -target {shlex.quote(target)} '
    f'--sysroot={shlex.quote(sysroot)} -D__MUSL__ {args} "$@"\n'
)
PY
chmod +x "$WRAP/cc" "$WRAP/cxx"

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$WRAP/cc"
export "CXX_${key}=$WRAP/cxx"
export "AR_${key}=$AR"
export "CARGO_TARGET_${key_upper}_LINKER=$WRAP/cc"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAP/cc"

exec "$@"
