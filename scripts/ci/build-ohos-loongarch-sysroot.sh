#!/usr/bin/env bash
set -euo pipefail

ZIG="${1:?Zig executable is required}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: OpenHarmony LoongArch sysroot builder requires Linux x86_64 host" >&2; exit 2 ;;
esac

[[ -x "$ZIG" ]] || { echo "ERROR: Zig executable is not executable: $ZIG" >&2; exit 1; }
"$ZIG" version >&2

MUSL_COMMIT="1f559e1b84d6784e1ea4a91d67381565bef492cd"
MUSL_URL="https://github.com/openharmony/third_party_musl/archive/${MUSL_COMMIT}.tar.gz"
MUSL_SHA256="fc483693f9081930d5986192ab90582154c43039d92ea6d18d2dedcb18faf67b"
BUILDER_REV="zig-2"
ROOT="${RUNNER_TEMP:-/tmp}/agena-ohos-loongarch-musl/${MUSL_COMMIT}-${BUILDER_REV}"
ARCHIVE="$ROOT/third_party_musl.tar.gz"
QUEUE_HEADER="$ROOT/freebsd-queue.h"
SOURCE="$ROOT/source"
SYSROOT="$ROOT/sysroot"

FREEBSD_QUEUE_URL="https://raw.githubusercontent.com/freebsd/freebsd-src/542e14a59bcaf97d7faed9f8d3fc5fed20625e3a/sys/sys/queue.h"
FREEBSD_QUEUE_SHA256="f4895e3567c8a78b06a5a81f9361572597ceab257e750ba95cbbb8cb4b3a1452"

valid_sysroot() {
  [[ -f "$SYSROOT/usr/include/bits/alltypes.h" ]] \
    && [[ -f "$SYSROOT/usr/include/bits/syscall.h" ]] \
    && [[ -f "$SYSROOT/usr/lib/crt1.o" ]] \
    && [[ -f "$SYSROOT/usr/lib/libc.a" ]] \
    && [[ -f "$SYSROOT/usr/lib/libc.so" ]] \
    && [[ -f "$SYSROOT/lib/ld-musl-loongarch64.so.1" ]]
}

download_verified() {
  local url="$1"
  local expected="$2"
  local output="$3"
  local actual

  if [[ -f "$output" ]]; then
    actual="$(sha256sum "$output" | awk '{print $1}')"
    [[ "$actual" == "$expected" ]] && return 0
  fi

  local temporary="${output}.tmp"
  rm -f "$temporary"
  curl --fail --location --retry 5 --retry-all-errors \
    --connect-timeout 30 --max-time 600 \
    "$url" -o "$temporary"
  actual="$(sha256sum "$temporary" | awk '{print $1}')"
  if [[ "$actual" != "$expected" ]]; then
    rm -f "$temporary"
    echo "SHA256 mismatch for $url: expected $expected, got $actual" >&2
    exit 1
  fi
  mv "$temporary" "$output"
}

prepare_source() {
  echo "OpenHarmony LoongArch sysroot: extracting pinned source $MUSL_COMMIT" >&2
  rm -rf "$SOURCE"
  mkdir -p "$SOURCE"
  archive_size="$(wc -c < "$ARCHIVE")"
  echo "OpenHarmony LoongArch sysroot: source archive size=$archive_size" >&2
  if tar -tzf "$ARCHIVE" >/dev/null; then
    :
  else
    status=$?
    echo "ERROR: failed to list OpenHarmony musl source archive $ARCHIVE (status $status)" >&2
    exit "$status"
  fi
  if tar -xzf "$ARCHIVE" --strip-components=1 -C "$SOURCE"; then
    :
  else
    status=$?
    echo "ERROR: failed to extract OpenHarmony musl source archive into $SOURCE (status $status)" >&2
    exit "$status"
  fi
  [[ -x "$SOURCE/configure" ]] || {
    echo "ERROR: OpenHarmony musl archive did not contain configure at $SOURCE/configure" >&2
    exit 1
  }

  # OpenHarmony's scripts/porting.sh performs these overlays with unchecked
  # cp calls.  Keep the same source selection but make every input explicit
  # and fail closed if the pinned archive changes shape.
  local mapping from to
  local -a mappings=(
    "src/internal/linux:src/internal"
    "src/hook/linux:src/hook"
    "crt/linux:crt"
    "src/linux/arm/linux:src/linux/arm"
    "src/linux/aarch64/linux:src/linux/aarch64"
    "src/linux/x86_64/linux:src/linux/x86_64"
    "src/exit/linux:src/exit"
    "src/fdsan/linux:src/fdsan"
    "src/fortify/linux:src/fortify"
    "src/gwp_asan/linux:src/gwp_asan"
    "src/hilog/linux:src/hilog"
    "src/linux/linux:src/linux"
    "src/network/linux:src/network"
    "src/syscall_hooks/linux:src/syscall_hooks"
    "src/signal/linux:src/signal"
    "src/thread/linux:src/thread"
    "src/trace/linux:src/trace"
    "include/trace/linux:include/trace"
    "src/info/linux:src/info"
    "ldso/linux:ldso"
    "include/sys/linux:include/sys"
    "include/info/linux:include/info"
    "include/fortify/linux:include/fortify"
    "include/linux:include"
    "src/ldso/arm/linux:src/ldso/arm"
    "src/ldso/aarch64/linux:src/ldso/aarch64"
    "src/ldso/x86_64/linux:src/ldso/x86_64"
    "src/misc/aarch64/linux:src/misc/aarch64"
    "src/malloc/linux:src/malloc"
    "src/sigchain/linux:src/sigchain"
  )
  for mapping in "${mappings[@]}"; do
    from="${mapping%%:*}"
    to="${mapping#*:}"
    [[ -d "$SOURCE/$from" ]] || {
      echo "ERROR: missing OpenHarmony porting input: $SOURCE/$from" >&2
      exit 1
    }
    mkdir -p "$SOURCE/$to"
    if cp -a "$SOURCE/$from"/. "$SOURCE/$to"/; then
      :
    else
      status=$?
      echo "ERROR: failed to overlay OpenHarmony porting input $SOURCE/$from -> $SOURCE/$to (status $status)" >&2
      exit "$status"
    fi
  done

  [[ -d "$SOURCE/scripts/linux" ]] || {
    echo "ERROR: missing OpenHarmony Linux Makefile overlay: $SOURCE/scripts/linux" >&2
    exit 1
  }
  if cp -a "$SOURCE/scripts/linux"/. "$SOURCE"/; then
    :
  else
    status=$?
    echo "ERROR: failed to install OpenHarmony Linux Makefile overlay from $SOURCE/scripts/linux (status $status)" >&2
    exit "$status"
  fi

  download_verified "$FREEBSD_QUEUE_URL" "$FREEBSD_QUEUE_SHA256" "$QUEUE_HEADER"
  mkdir -p "$SOURCE/include/sys"
  install -m 0644 "$QUEUE_HEADER" "$SOURCE/include/sys/queue.h" || {
    status=$?
    echo "ERROR: failed to install FreeBSD queue.h into $SOURCE/include/sys/queue.h (status $status)" >&2
    exit "$status"
  }
  [[ -f "$SOURCE/include/sys/queue.h" ]] || {
    echo "ERROR: FreeBSD queue.h was not installed at $SOURCE/include/sys/queue.h" >&2
    exit 1
  }
  queue_size="$(wc -c < "$SOURCE/include/sys/queue.h")"
  queue_sha256="$(sha256sum "$SOURCE/include/sys/queue.h" | awk '{print $1}')"
  echo "OpenHarmony LoongArch sysroot: installed queue.h size=$queue_size sha256=$queue_sha256" >&2

  patch_file="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/openharmony-loongarch-musl.patch"
  [[ -f "$patch_file" ]] || {
    echo "ERROR: OpenHarmony LoongArch patch not found: $patch_file" >&2
    exit 1
  }
  command -v patch >&2 || {
    echo "ERROR: GNU patch is required to apply the OpenHarmony LoongArch overlay" >&2
    exit 1
  }
  patch --version | head -n 1 >&2
  patch --fuzz=0 --forward --batch -p1 < "$patch_file" || {
    status=$?
    echo "ERROR: failed to apply OpenHarmony LoongArch patch $patch_file (status $status)" >&2
    exit "$status"
  }
}

mkdir -p "$ROOT"
if ! valid_sysroot; then
  echo "OpenHarmony LoongArch sysroot: preparing pinned musl source $MUSL_COMMIT" >&2
  download_verified "$MUSL_URL" "$MUSL_SHA256" "$ARCHIVE"
  echo "OpenHarmony LoongArch sysroot: verified archive $(sha256sum "$ARCHIVE" | awk '{print $1}')" >&2

  prepare_source

  echo "OpenHarmony LoongArch sysroot: configuring musl with Zig Linux musl provider" >&2
  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT"
  ZIG_CC="$ZIG cc -target loongarch64-linux-musl"
  ZIG_AR="$ZIG ar"
  ZIG_RANLIB="$ZIG ranlib"
  (
    cd "$SOURCE"
    CC="$ZIG_CC" \
    AR="$ZIG_AR" \
    RANLIB="$ZIG_RANLIB" \
    CFLAGS="-DCXA_THREAD_USE_TSD" \
    LIBCC="" \
    LDFLAGS="-fuse-ld=lld -rtlib=compiler-rt" \
      ./configure \
        --target=loongarch64-linux-musl \
        --prefix="$SYSROOT/usr" \
        --syslibdir=/lib \
        --disable-wrapper
    echo "OpenHarmony LoongArch sysroot: building musl" >&2
    make -j"$(getconf _NPROCESSORS_ONLN)" \
      CC="$ZIG_CC" \
      AR="$ZIG_AR" \
      RANLIB="$ZIG_RANLIB" \
      CFLAGS="-DCXA_THREAD_USE_TSD" \
      LIBCC="" \
      LDFLAGS="-fuse-ld=lld -rtlib=compiler-rt" >&2
    echo "OpenHarmony LoongArch sysroot: installing musl" >&2
    make install >&2
  )

  # The upstream rule deliberately swallows failure for the dynamic linker;
  # install the real linked libc.so explicitly and fail if it is unavailable.
  mkdir -p "$SYSROOT/lib"
  [[ -s "$SOURCE/lib/libc.so" ]] || {
    echo "ERROR: OpenHarmony musl did not produce a shared libc" >&2
    exit 1
  }
  install -m 0755 "$SOURCE/lib/libc.so" "$SYSROOT/lib/ld-musl-loongarch64.so.1"

  valid_sysroot || {
    echo "ERROR: incomplete OpenHarmony LoongArch musl sysroot" >&2
    exit 1
  }
fi

printf '%s\n' "$SYSROOT"
