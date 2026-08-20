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
mkdir -p "$ROOT"

if [[ ! -x "$EXTRACTED/$TOOLCHAIN/bin/$PREFIX-gcc" ]]; then
  python3 - "$URL" "$ARCHIVE" "$SHA256" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

url, archive_path, expected = sys.argv[1:]
archive = pathlib.Path(archive_path)

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

if not archive.exists() or digest(archive) != expected:
    tmp = archive.with_suffix(archive.suffix + ".tmp")
    req = urllib.request.Request(url, headers={"User-Agent": "agena-musl-cross"})
    with urllib.request.urlopen(req, timeout=900) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f"musl cross toolchain SHA256 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY
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
