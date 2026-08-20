#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?Haiku target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: Haiku cross-tools builder requires Linux x86_64" >&2; exit 2 ;;
esac

case "$TARGET" in
  i686-unknown-haiku)
    HAIKU_ARCH=x86
    GNU_TARGET=i586-pc-haiku
    CFLAGS_EXTRA=-m32
    ;;
  x86_64-unknown-haiku)
    HAIKU_ARCH=x86_64
    GNU_TARGET=x86_64-unknown-haiku
    CFLAGS_EXTRA=-m64
    ;;
  *) echo "ERROR: unsupported Haiku target: $TARGET" >&2; exit 2 ;;
esac

# Pin both official Haiku GitHub mirrors so cross-tools/sysroot generation is
# reproducible rather than following moving master branches.
HAIKU_COMMIT=dfaff659fa944da59db4014f50cde2daea9415bd
BUILDTOOLS_COMMIT=8375c2dbeaf109c520798cb234d57f0895463201
ROOT="${RUNNER_TEMP:-/tmp}/agena-haiku/$HAIKU_ARCH"
HAIKU="$ROOT/haiku"
BUILDTOOLS="$ROOT/buildtools"
OUTPUT="$ROOT/generated"
PACKAGE_ROOT="$ROOT/system"
SYSROOT="$OUTPUT/cross-tools-$HAIKU_ARCH/sysroot"
TOOLBIN="$OUTPUT/cross-tools-$HAIKU_ARCH/bin"
mkdir -p "$ROOT"

valid_toolchain() {
  [[ -x "$TOOLBIN/$GNU_TARGET-gcc" ]] \
    && [[ -f "$PACKAGE_ROOT/develop/headers/posix/stdio.h" || -f "$PACKAGE_ROOT/develop/headers/stdio.h" ]] \
    && find "$PACKAGE_ROOT" -type f -name 'libroot.so' -print -quit | grep -q .
}

if ! valid_toolchain; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "ERROR: sudo is required to install Haiku cross-tool build prerequisites" >&2
    exit 1
  fi
  sudo apt-get update -y
  sudo apt-get install -y --no-install-recommends \
    autoconf automake bison bzip2 ca-certificates cmake curl file flex g++ gawk git \
    libtool-bin make nasm pkg-config python3 texinfo wget xz-utils zlib1g-dev

  rm -rf "$HAIKU" "$BUILDTOOLS" "$OUTPUT" "$PACKAGE_ROOT" "$ROOT/bin"
  mkdir -p "$ROOT/bin" "$PACKAGE_ROOT"

  git init -q "$HAIKU"
  git -C "$HAIKU" remote add origin https://github.com/haiku/haiku.git
  git -C "$HAIKU" fetch -q --depth 1 origin "$HAIKU_COMMIT"
  git -C "$HAIKU" checkout -q FETCH_HEAD

  git init -q "$BUILDTOOLS"
  git -C "$BUILDTOOLS" remote add origin https://github.com/haiku/buildtools.git
  git -C "$BUILDTOOLS" fetch -q --depth 1 origin "$BUILDTOOLS_COMMIT"
  git -C "$BUILDTOOLS" checkout -q FETCH_HEAD

  (
    cd "$BUILDTOOLS/jam"
    make -j2
    ./jam0 -sBINDIR="$ROOT/bin" install
  )
  export PATH="$ROOT/bin:$PATH"

  mkdir -p "$OUTPUT" "$SYSROOT/boot"
  ln -sfn "$PACKAGE_ROOT" "$SYSROOT/boot/system"
  (
    cd "$OUTPUT"
    "$HAIKU/configure" \
      --cross-tools-source "$BUILDTOOLS" \
      --build-cross-tools "$HAIKU_ARCH"
    jam -q -j2 haiku.hpkg haiku_devel.hpkg '<build>package'
  )

  PACKAGE_TOOL="$(find "$OUTPUT/objects/linux" -type f -path '*/release/tools/package/package' -perm -111 -print -quit)"
  [[ -x "$PACKAGE_TOOL" ]] || { echo "ERROR: Haiku package extraction tool missing" >&2; exit 1; }
  host_libs="$OUTPUT/objects/linux/lib"
  extract_hpkg() {
    local hpkg="$1"
    [[ -f "$hpkg" ]] || return 0
    LD_LIBRARY_PATH="$host_libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
      "$PACKAGE_TOOL" extract -C "$PACKAGE_ROOT" "$hpkg"
  }

  extract_hpkg "$OUTPUT/objects/haiku/$HAIKU_ARCH/packaging/packages/haiku.hpkg"
  extract_hpkg "$OUTPUT/objects/haiku/$HAIKU_ARCH/packaging/packages/haiku_devel.hpkg"
  while IFS= read -r hpkg; do extract_hpkg "$hpkg"; done < <(find "$OUTPUT/download" -type f -name '*.hpkg' -print 2>/dev/null || true)

  if [[ -f "$PACKAGE_ROOT/lib/libgcc_s.so" && -d "$PACKAGE_ROOT/develop/lib" ]]; then
    ln -sfn ../../lib/libgcc_s.so "$PACKAGE_ROOT/develop/lib/libgcc_s.so"
  fi

  valid_toolchain || { echo "ERROR: incomplete Haiku cross-tools/sysroot for $TARGET" >&2; exit 1; }
fi

CC="$TOOLBIN/$GNU_TARGET-gcc"
CXX="$TOOLBIN/$GNU_TARGET-g++"
AR="$TOOLBIN/$GNU_TARGET-ar"
[[ -x "$CC" && -x "$CXX" && -x "$AR" ]] || { echo "ERROR: Haiku GCC tools missing" >&2; exit 1; }

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$CC"
export "CXX_${key}=$CXX"
export "AR_${key}=$AR"
export "CFLAGS_${key}=$CFLAGS_EXTRA"
export "CXXFLAGS_${key}=$CFLAGS_EXTRA"
export "CARGO_TARGET_${key_upper}_LINKER=$CC"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$CC"

exec "$@"
