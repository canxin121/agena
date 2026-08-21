#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?MIPS musl target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: pinned musl.cc cross toolchains require Linux x86_64" >&2; exit 2 ;;
esac

case "$TARGET" in
  mips-unknown-linux-musl)
    TOOLCHAIN=mips-linux-muslsf-cross
    PREFIX=mips-linux-muslsf
    SHA256=572476c458730f41c86e4db8ccd341ed1e66585897b214f282f2f05a445f47d3
    ;;
  mipsel-unknown-linux-musl)
    TOOLCHAIN=mipsel-linux-muslsf-cross
    PREFIX=mipsel-linux-muslsf
    SHA256=a61c3bbf9fbb0be80fe2abdb4ea8b6f5afdf664b5b4104a3784a326270905216
    ;;
  mips64-unknown-linux-muslabi64)
    TOOLCHAIN=mips64-linux-musl-cross
    PREFIX=mips64-linux-musl
    SHA256=a0e62bf38f33664e825987ab8c191c75032f5189c6103a25a8adc0361e63a1cf
    ;;
  mips64el-unknown-linux-muslabi64)
    TOOLCHAIN=mips64el-linux-musl-cross
    PREFIX=mips64el-linux-musl
    SHA256=fdb3c2ae76f80d7145132a1ec3303362f310b8c6349cce151f3035d0515c35b0
    ;;
  *) echo "ERROR: unsupported MIPS musl target: $TARGET" >&2; exit 2 ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-musl-cross/$TOOLCHAIN"
ARCHIVE="$ROOT/$TOOLCHAIN.tgz"
EXTRACTED="$ROOT/root"
URL="https://musl.cc/$TOOLCHAIN.tgz"
FALLBACK_URL="http://musl.cc/$TOOLCHAIN.tgz"
MIRROR_URL="https://github.com/tsl0922/musl-toolchains/releases/download/2021-11-23/$TOOLCHAIN.tgz"
mkdir -p "$ROOT"

if [[ ! -x "$EXTRACTED/$TOOLCHAIN/bin/$PREFIX-gcc" ]]; then
  actual=""
  if [[ -f "$ARCHIVE" ]]; then
    actual="$(sha256sum "$ARCHIVE" | awk '{print $1}')"
  fi
  if [[ "$actual" != "$SHA256" ]]; then
    tmp="$ARCHIVE.tmp"
    rm -f "$tmp"
    download_args=(
      --fail
      --location
      --ipv4
      --retry 12
      --retry-all-errors
      --retry-delay 5
      --connect-timeout 30
      --max-time 1800
      --user-agent agena-musl-cross
      --output "$tmp"
    )
    downloaded=0
    if curl "${download_args[@]}" "$URL"; then
      downloaded=1
    else
      # musl.cc is the pinned upstream distribution for these exact ABI
      # archives.  GitHub-hosted runners have intermittently been unable to
      # establish TLS to its 443 endpoint even while the same immutable file
      # is available over port 80.  The SHA256 check below remains mandatory
      # before the archive can be used.
      echo "musl.cc HTTPS download failed; retrying the same checksum-pinned archive over HTTP" >&2
      rm -f "$tmp"
      if curl "${download_args[@]}" "$FALLBACK_URL"; then
        downloaded=1
      else
        echo "musl.cc HTTP download failed; retrying the exact archive from the pinned GitHub mirror" >&2
        rm -f "$tmp"
        if curl "${download_args[@]}" "$MIRROR_URL"; then
          downloaded=1
        fi
      fi
    fi
    if [[ "$downloaded" != 1 ]]; then
      rm -f "$tmp"
      echo "ERROR: unable to download checksum-pinned MIPS musl toolchain $TOOLCHAIN" >&2
      exit 1
    fi
    actual="$(sha256sum "$tmp" | awk '{print $1}')"
    if [[ "$actual" != "$SHA256" ]]; then
      rm -f "$tmp"
      echo "ERROR: musl cross toolchain SHA256 mismatch: expected $SHA256, got $actual" >&2
      exit 1
    fi
    mv "$tmp" "$ARCHIVE"
  fi
  rm -rf "$EXTRACTED"
  mkdir -p "$EXTRACTED"
  tar -xzf "$ARCHIVE" -C "$EXTRACTED"
fi

BIN="$EXTRACTED/$TOOLCHAIN/bin"
CC="$BIN/$PREFIX-gcc"
CXX="$BIN/$PREFIX-g++"
AR="$BIN/$PREFIX-ar"
[[ -x "$CC" && -x "$CXX" && -x "$AR" ]] || {
  echo "ERROR: incomplete musl cross toolchain for $TARGET" >&2
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
