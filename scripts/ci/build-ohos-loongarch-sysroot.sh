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
MUSL_URL="https://github.com/openharmony/third_party_musl/archive/${MUSL_COMMIT}.tar.gz"
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
  echo "OpenHarmony LoongArch sysroot: preparing pinned musl source $MUSL_COMMIT" >&2
  if [[ ! -f "$ARCHIVE" ]] || [[ "$(sha256sum "$ARCHIVE" | awk '{print $1}')" != "$MUSL_SHA256" ]]; then
    tmp_archive="$ARCHIVE.tmp"
    rm -f "$tmp_archive"
    echo "OpenHarmony LoongArch sysroot: downloading fixed musl archive" >&2
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
  echo "OpenHarmony LoongArch sysroot: verified archive $(sha256sum "$ARCHIVE" | awk '{print $1}')" >&2

  echo "OpenHarmony LoongArch sysroot: extracting musl source" >&2
  rm -rf "$SOURCE"
  mkdir -p "$SOURCE"
  tar -tzf "$ARCHIVE" >/dev/null
  tar -xzf "$ARCHIVE" --strip-components=1 -C "$SOURCE"
  [[ -x "$SOURCE/configure" ]] || {
    echo "ERROR: OpenHarmony musl archive did not contain configure at $SOURCE/configure" >&2
    exit 1
  }

  echo "OpenHarmony LoongArch sysroot: configuring musl with pinned Clang" >&2
  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT"
  TARGET_FLAGS=(--target=loongarch64-linux-gnu)
  COMPILER_RT="$("$CLANG" "${TARGET_FLAGS[@]}" -print-file-name=libclang_rt.builtins-loongarch64.a)"
  [[ -f "$COMPILER_RT" ]] || {
    echo "ERROR: pinned Clang has no LoongArch compiler runtime: $COMPILER_RT" >&2
    "$CLANG" --version >&2 || true
    exit 1
  }
  (
    cd "$SOURCE"
    CC="$CLANG ${TARGET_FLAGS[*]}" \
    AR="$AR" \
    RANLIB="$RANLIB" \
    LIBCC="$COMPILER_RT" \
      ./configure \
        --target=loongarch64-linux-musl \
        --prefix="$SYSROOT/usr" \
        --syslibdir=/lib \
        --disable-wrapper
    echo "OpenHarmony LoongArch sysroot: building musl" >&2
    make -j"$(getconf _NPROCESSORS_ONLN)" \
      CC="$CLANG ${TARGET_FLAGS[*]}" \
      AR="$AR" \
      RANLIB="$RANLIB" \
      LIBCC="$COMPILER_RT" \
      LDFLAGS="-fuse-ld=lld -rtlib=compiler-rt" >&2
    echo "OpenHarmony LoongArch sysroot: installing musl" >&2
    make install >&2
  )

  valid_sysroot || {
    echo "ERROR: incomplete OpenHarmony LoongArch musl sysroot" >&2
    exit 1
  }
fi

printf '%s\n' "$SYSROOT"
