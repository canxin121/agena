#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?C-SKY target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: Rust's documented C-SKY toolchain is a Linux x86_64 host tool" >&2; exit 2 ;;
esac

case "$TARGET" in
  csky-unknown-linux-gnuabiv2)
    # C-SKY gas/GCC do not reliably relax very large short branches. Generated
    # tree-sitter parsers overflow the branch range under cc-rs' dev-profile
    # -O0; optimized code is substantially smaller and is also what Release
    # packaging uses.
    extra_cflags="-Os"
    extra_rustflags=""
    ;;
  csky-unknown-linux-gnuabiv2hf)
    # Match the Rust built-in target: ck860fv + hard-float calling convention.
    extra_cflags="-Os -mcpu=ck860fv -mhard-float"
    extra_rustflags="-C link-arg=-mhard-float"
    ;;
  *) echo "ERROR: unsupported C-SKY target: $TARGET" >&2; exit 2 ;;
esac

URL="https://occ-oss-prod.oss-cn-hangzhou.aliyuncs.com/resource/1356021/1619528643136/csky-linux-gnuabiv2-tools-x86_64-glibc-linux-4.9.56-20210423.tar.gz"
SHA256="1a20b552977e2f7793b4ef242d41fbe6dc24fd2d369bdec5c96beb0d209fb676"
ROOT="${RUNNER_TEMP:-/tmp}/agena-csky-toolchain/20210423"
ARCHIVE="$ROOT/toolchain.tar.gz"
TOOLCHAIN="$ROOT/root"
mkdir -p "$ROOT"

if [[ ! -x "$TOOLCHAIN/bin/csky-linux-gnuabiv2-gcc" ]]; then
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
    tmp = archive.with_suffix(".tmp")
    with urllib.request.urlopen(url, timeout=600) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f"C-SKY toolchain SHA256 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY
  rm -rf "$TOOLCHAIN"
  mkdir -p "$TOOLCHAIN"
  tar -xzf "$ARCHIVE" -C "$TOOLCHAIN"
fi

CC="$TOOLCHAIN/bin/csky-linux-gnuabiv2-gcc"
CXX="$TOOLCHAIN/bin/csky-linux-gnuabiv2-g++"
AR="$TOOLCHAIN/bin/csky-linux-gnuabiv2-ar"
[[ -x "$CC" && -x "$CXX" && -x "$AR" ]] || {
  echo "ERROR: incomplete C-SKY toolchain" >&2
  exit 1
}

# The bundled 2021 C-SKY assembler has a 64 KiB PC-relative branch range and
# no usable long-branch relaxation.  Tree-sitter's generated lexer is a single
# state-machine function large enough to exceed that range.  The wrapper only
# transforms generated files containing ts_lex and forwards every other C/C++
# invocation unchanged; it preserves the lexer state machine and does not
# remove or replace any grammar.
WRAP="${RUNNER_TEMP:-/tmp}/agena-csky-wrappers/$TARGET"
mkdir -p "$WRAP"
python3 - "$WRAP/cc" "$WRAP/cxx" "$PWD/scripts/ci/csky-cc-wrapper.py" "$CC" "$CXX" <<'PY'
import pathlib
import shlex
import sys

cc_path, cxx_path, wrapper, cc, cxx = sys.argv[1:]
for path, compiler in ((cc_path, cc), (cxx_path, cxx)):
    pathlib.Path(path).write_text(
        "#!/bin/sh\nexec python3 "
        + shlex.quote(wrapper)
        + " "
        + shlex.quote(compiler)
        + " \"$@\"\n",
        encoding="utf-8",
    )
PY
chmod +x "$WRAP/cc" "$WRAP/cxx"

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$WRAP/cc"
export "CXX_${key}=$WRAP/cxx"
export "AR_${key}=$AR"
export "CARGO_TARGET_${key_upper}_LINKER=$WRAP/cc"
if [[ -n "$extra_cflags" ]]; then
  export "CFLAGS_${key}=$extra_cflags"
  export "CXXFLAGS_${key}=$extra_cflags"
fi
export RUSTFLAGS="${RUSTFLAGS:-}${RUSTFLAGS:+ }$extra_rustflags"

exec "$@"
