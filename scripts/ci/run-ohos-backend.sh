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

if [[ "$TARGET" == loongarch64-unknown-linux-ohos ]]; then
  # Zig does not claim an OpenHarmony libc target.  The target-specific C
  # provider is intentionally the Linux musl ABI built from the pinned
  # OpenHarmony third_party_musl source below; the Rust target remains the
  # OpenHarmony target triple so rustc uses the correct std PAL.
  ZIG="$(bash scripts/ci/fetch-zig.sh)"
  # The source-built musl/zlib bootstrap is intentionally verbose.  Capture
  # only its final machine-readable sysroot path while teeing the complete
  # build log to stderr.  Capturing raw stdout would turn the compiler log
  # into the value of AGENA_ZIG_SYSROOT and inject it into every C command.
  SYSROOT="$(
    bash scripts/ci/build-ohos-loongarch-sysroot.sh "$ZIG" 2>&1 |
      tee /dev/stderr |
      tail -n 1
  )"
  export AGENA_ZIG="$ZIG"
  export AGENA_ZIG_SYSROOT="$SYSROOT"
  # libz-sys intentionally links the platform zlib without emitting include
  # metadata for *-ohos targets.  libgit2-sys still needs the matching public
  # header while compiling its real Git implementation.
  export DEP_Z_INCLUDE="$SYSROOT/usr/include"
  exec bash scripts/ci/run-zig-backend.sh "$TARGET" loongarch64-linux-musl -- "$@"
else
  SDK_RELEASE=5.0.0-Release
  SDK_CACHE_KEY=5.0.0

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
  native_zip=linux/native-linux-x64-5.0.0.71-Release.zip
  tar -xzf "$ARCHIVE" -C "$ROOT" "$native_zip"
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
  x86_64-unknown-linux-ohos)
    clang_target=x86_64-linux-ohos; extra=() ;;
  *) echo "ERROR: unsupported OHOS target: $TARGET" >&2; exit 2 ;;
esac
fi

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
