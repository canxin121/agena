#!/usr/bin/env bash
set -euo pipefail

CLANG_ROOT="${1:?pinned Clang root is required}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: OpenHarmony LoongArch sysroot builder requires Linux x86_64 host" >&2; exit 2 ;;
esac

CLANG="$CLANG_ROOT/bin/clang"
AR="$CLANG_ROOT/bin/llvm-ar"
RANLIB="$CLANG_ROOT/bin/llvm-ranlib"
[[ -x "$CLANG" && -x "$AR" && -x "$RANLIB" ]] || {
  echo "ERROR: incomplete pinned Clang toolchain at $CLANG_ROOT" >&2
  exit 1
}

MUSL_COMMIT="1f559e1b84d6784e1ea4a91d67381565bef492cd"
MUSL_URL="https://gitee.com/openharmony/third_party_musl/repository/archive/${MUSL_COMMIT}.tar.gz"
MUSL_SHA256="fc483693f9081930d5986192ab90582154c43039d92ea6d18d2dedcb18faf67b"
ROOT="${RUNNER_TEMP:-/tmp}/agena-ohos-loongarch-musl/${MUSL_COMMIT}"
ARCHIVE="$ROOT/third_party_musl.tar.gz"
SOURCE="$ROOT/source"
SYSROOT="$ROOT/sysroot"

valid_sysroot() {
  [[ -f "$SYSROOT/usr/include/bits/alltypes.h" ]] \
    && [[ -f "$SYSROOT/usr/include/bits/syscall.h" ]] \
    && [[ -f "$SYSROOT/usr/lib/crt1.o" ]] \
    && [[ -f "$SYSROOT/usr/lib/libc.a" ]] \
    && [[ -f "$SYSROOT/usr/lib/libc.so" ]] \
    && [[ -f "$SYSROOT/lib/ld-musl-loongarch64.so.1" ]]
}

mkdir -p "$ROOT"
if ! valid_sysroot; then
  if [[ ! -f "$ARCHIVE" ]] || [[ "$(sha256sum "$ARCHIVE" | awk '{print $1}')" != "$MUSL_SHA256" ]]; then
    tmp_archive="$ARCHIVE.tmp"
    rm -f "$tmp_archive"
    curl --fail --location --retry 5 --retry-all-errors \
      --connect-timeout 30 --max-time 600 \
      "$MUSL_URL" -o "$tmp_archive"
    actual="$(sha256sum "$tmp_archive" | awk '{print $1}')"
    if [[ "$actual" != "$MUSL_SHA256" ]]; then
      rm -f "$tmp_archive"
      echo "OpenHarmony musl SHA256 mismatch: expected $MUSL_SHA256, got $actual" >&2
      exit 1
    fi
    mv "$tmp_archive" "$ARCHIVE"
  fi

  rm -rf "$SOURCE"
  mkdir -p "$SOURCE"
  tar -xzf "$ARCHIVE" --strip-components=1 -C "$SOURCE"

  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT"
  (
    cd "$SOURCE"
    CC="$CLANG --target=loongarch64-linux-gnu" \
    AR="$AR" \
    RANLIB="$RANLIB" \
      ./configure \
        --target=loongarch64-linux-musl \
        --prefix="$SYSROOT/usr" \
        --syslibdir=/lib \
        --disable-wrapper
    make -j"$(getconf _NPROCESSORS_ONLN)" \
      CC="$CLANG --target=loongarch64-linux-gnu" \
      AR="$AR" \
      RANLIB="$RANLIB" \
      LIBCC= \
      LDFLAGS="-fuse-ld=lld -rtlib=compiler-rt" >&2
    make install >&2
  )

  valid_sysroot || {
    echo "ERROR: incomplete OpenHarmony LoongArch musl sysroot" >&2
    exit 1
  }
fi

printf '%s\n' "$SYSROOT"
