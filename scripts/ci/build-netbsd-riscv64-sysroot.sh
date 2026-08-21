#!/usr/bin/env bash
set -euo pipefail

# NetBSD 10.1 did not publish a riscv64 release set.  Build the target
# toolchain and userland from the immutable 10.1 source set instead of letting
# cc-rs guess a host compiler name.  The source archive and its official
# SHA512 are published by NetBSD's CDN.
NETBSD_SOURCE_URL="https://cdn.netbsd.org/pub/NetBSD/NetBSD-10.1/source/sets/src.tgz"
NETBSD_SOURCE_SHA512="6ae2053b4b75821238c0757d4f7258daece425de72524c616e07d3adee7c48d87422dd47d852a137918cec3dd3c0d339e372f4504dfe9f1bc5520011775bdb86"
NETBSD_SOURCE_ID="NetBSD-10.1"

ROOT="${RUNNER_TEMP:-/tmp}/agena-netbsd-riscv64/${NETBSD_SOURCE_ID}"
ARCHIVE="$ROOT/src.tgz"
SOURCE="$ROOT/src"
OBJ="$ROOT/obj"
TOOLDIR="$ROOT/tools"
DESTDIR="$ROOT/destdir.riscv"
JOBS="${AGENA_NETBSD_JOBS:-$(getconf _NPROCESSORS_ONLN)}"

valid_sysroot() {
  [[ -x "$TOOLDIR/bin/riscv64--netbsd-gcc" ]] \
    && { [[ -x "$TOOLDIR/bin/riscv64--netbsd-g++" ]] || [[ -x "$TOOLDIR/bin/riscv64--netbsd-c++" ]]; } \
    && [[ -x "$TOOLDIR/bin/riscv64--netbsd-ar" ]] \
    && [[ -x "$TOOLDIR/bin/riscv64--netbsd-ld" ]] \
    && [[ -f "$DESTDIR/usr/include/stdio.h" ]] \
    && [[ -f "$DESTDIR/usr/include/unistd.h" ]] \
    && [[ -f "$DESTDIR/usr/lib/crt0.o" ]] \
    && [[ -f "$DESTDIR/usr/lib/crtbegin.o" ]] \
    && [[ -f "$DESTDIR/usr/lib/libc.a" ]] \
    && [[ -f "$DESTDIR/usr/lib/libgcc.a" ]] \
    && { [[ -f "$DESTDIR/usr/lib/libc.so" ]] || compgen -G "$DESTDIR/usr/lib/libc.so.*" >/dev/null; }
}

download_verified() {
  local url="$1"
  local expected="$2"
  local destination="$3"
  local temporary="${destination}.tmp"
  if [[ -f "$destination" ]] && printf '%s  %s\n' "$expected" "$destination" | shasum -a 512 -c - >/dev/null 2>&1; then
    return 0
  fi
  curl --fail --location --retry 8 --retry-all-errors --retry-delay 5 \
    --connect-timeout 30 --max-time 900 --output "$temporary" "$url"
  printf '%s  %s\n' "$expected" "$temporary" | shasum -a 512 -c -
  mv "$temporary" "$destination"
}

if ! valid_sysroot; then
  mkdir -p "$ROOT"
  download_verified "$NETBSD_SOURCE_URL" "$NETBSD_SOURCE_SHA512" "$ARCHIVE"

  if [[ ! -x "$SOURCE/build.sh" ]]; then
    rm -rf "$SOURCE"
    mkdir -p "$SOURCE"
    # src.tgz has the fixed archive prefix usr/src (not just src).  Strip
    # both packaging components so build.sh lands at $SOURCE/build.sh and the
    # source builder cannot accidentally invoke a missing path.
    tar -xzf "$ARCHIVE" --strip-components=2 -C "$SOURCE"
  fi

  common_env=(
    MKX11=no
    MKDOC=no
    MKMAN=no
    MKINFO=no
    MKNLS=no
    MKPAM=no
    MKHESIOD=no
    MKYP=no
    MKATF=no
    MKTESTS=no
    MKDEBUG=no
    MKGDB=no
    MKLLVM=no
    MKLINT=no
    MKGCC=yes
    MKGCCCMDS=no
  )

  echo "Building NetBSD ${NETBSD_SOURCE_ID} riscv64 tools" >&2
  env "${common_env[@]}" \
    bash "$SOURCE/build.sh" -U -O "$OBJ" -T "$TOOLDIR" -j"$JOBS" \
      -m riscv -a riscv64 tools

  echo "Building NetBSD ${NETBSD_SOURCE_ID} riscv64 distribution" >&2
  env "${common_env[@]}" \
    bash "$SOURCE/build.sh" -U -u -O "$OBJ" -T "$TOOLDIR" -D "$DESTDIR" \
      -j"$JOBS" -m riscv -a riscv64 distribution

  # NetBSD's build.sh names the C++ driver c++ when MKGCCCMDS=no.  cc-rs and
  # the Rust target linker convention use g++, so provide an alias to the
  # exact tool produced by build.sh rather than making cc-rs guess another
  # host compiler.  This is only a name-level compatibility link; the ABI
  # compiler and sysroot remain the official NetBSD build output.
  if [[ ! -e "$TOOLDIR/bin/riscv64--netbsd-g++" && -x "$TOOLDIR/bin/riscv64--netbsd-c++" ]]; then
    ln -s "riscv64--netbsd-c++" "$TOOLDIR/bin/riscv64--netbsd-g++"
  fi

  valid_sysroot || {
    echo "ERROR: incomplete NetBSD riscv64 sysroot" >&2
    exit 1
  }
fi

printf '%s\n' "$ROOT"
