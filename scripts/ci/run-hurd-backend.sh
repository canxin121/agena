#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?GNU/Hurd target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: Hurd sysroot builder requires a Linux x86_64 host" >&2; exit 2 ;;
esac

case "$TARGET" in
  i686-unknown-hurd-gnu)
    deb_arch=hurd-i386
    clang_target=i686-unknown-gnu
    multiarch=i386-gnu
    ;;
  x86_64-unknown-hurd-gnu)
    deb_arch=hurd-amd64
    clang_target=x86_64-unknown-gnu
    multiarch=x86_64-gnu
    ;;
  *) echo "ERROR: unsupported Hurd target: $TARGET" >&2; exit 2 ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-hurd-sysroots/$deb_arch"
INDEX="$ROOT/Packages.xz"
SYSROOT="$ROOT/root"
BASE_URL="https://deb.debian.org/debian-ports"
INDEX_URL="$BASE_URL/dists/sid/main/binary-$deb_arch/Packages.xz"
mkdir -p "$ROOT/packages"

python3 - "$INDEX_URL" "$INDEX" <<'PY'
import pathlib, sys, urllib.request
url, path = sys.argv[1:]
path = pathlib.Path(path)
if not path.exists():
    tmp = path.with_suffix('.tmp')
    with urllib.request.urlopen(url, timeout=180) as src, tmp.open('wb') as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    tmp.replace(path)
PY

python3 - "$INDEX" "$ROOT/selected.tsv" <<'PY'
import lzma, pathlib, sys
index, out = sys.argv[1:]
wanted = {
    'libc0.3', 'libc0.3-dev', 'hurd-dev', 'gnumach-dev', 'hurd-libs0.3',
    'gcc-16-base', 'libgcc-s1', 'libgcc-16-dev', 'libstdc++6', 'libstdc++-16-dev',
    'libatomic1', 'libgomp1', 'libquadmath0', 'libcrypt1', 'zlib1g', 'libbz2-1.0',
}
text = lzma.open(index, 'rt', encoding='utf-8', errors='replace').read()
rows = []
for para in text.split('\n\n'):
    fields = {}; last = None
    for line in para.splitlines():
        if line.startswith(' ') and last:
            fields[last] += ' ' + line.strip(); continue
        if ': ' in line:
            k, v = line.split(': ', 1); fields[k] = v; last = k
    if fields.get('Package') in wanted:
        rows.append((fields['Package'], fields['Filename'], fields['SHA256']))
found = {row[0] for row in rows}
missing = sorted(wanted - found)
if missing:
    raise SystemExit('missing Hurd sysroot packages: ' + ', '.join(missing))
pathlib.Path(out).write_text('\n'.join('\t'.join(row) for row in sorted(rows)) + '\n')
PY

while IFS=$'\t' read -r package filename sha256; do
  deb="$ROOT/packages/${package}.deb"
  python3 - "$BASE_URL/$filename" "$deb" "$sha256" <<'PY'
import hashlib, pathlib, sys, urllib.request
url, path, expected = sys.argv[1:]
path = pathlib.Path(path)
def digest(p):
    h = hashlib.sha256()
    with p.open('rb') as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b''):
            h.update(chunk)
    return h.hexdigest()
if not path.exists() or digest(path) != expected:
    tmp = path.with_suffix('.tmp')
    with urllib.request.urlopen(url, timeout=300) as src, tmp.open('wb') as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f'{path.name} SHA256 mismatch: expected {expected}, got {actual}')
    tmp.replace(path)
PY
done < "$ROOT/selected.tsv"

valid_sysroot() {
  [[ -f "$SYSROOT/usr/include/stdio.h" || -f "$SYSROOT/usr/include/i386-gnu/stdio.h" || -f "$SYSROOT/usr/include/x86_64-gnu/stdio.h" ]] \
    && find "$SYSROOT" -type f -name 'crt1.o' -print -quit | grep -q . \
    && find "$SYSROOT" -type f -name 'libc.so*' -print -quit | grep -q . \
    && find "$SYSROOT" -type f -name 'libgcc.a' -print -quit | grep -q .
}

if ! valid_sysroot; then
  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT"
  while IFS=$'\t' read -r package _filename _sha256; do
    dpkg-deb -x "$ROOT/packages/${package}.deb" "$SYSROOT"
  done < "$ROOT/selected.tsv"
  valid_sysroot || { echo "ERROR: incomplete Debian Hurd sysroot for $TARGET" >&2; exit 1; }
fi

STABLE_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
rust_sysroot="$(rustup run "$STABLE_TOOLCHAIN" rustc --print sysroot)"
host="$(rustup run "$STABLE_TOOLCHAIN" rustc -vV | sed -n 's/^host: //p')"
LLD="$rust_sysroot/lib/rustlib/$host/bin/gcc-ld/ld.lld"
[[ -x "$LLD" ]] || LLD="$rust_sysroot/lib/rustlib/$host/bin/rust-lld"
[[ -x "$LLD" ]] || { echo "ERROR: Rust LLD missing" >&2; exit 1; }

CLANG="${AGENA_CLANG:-$(command -v clang || true)}"
CLANGXX="${AGENA_CLANGXX:-$(command -v clang++ || true)}"
AR="${AGENA_LLVM_AR:-$(command -v llvm-ar || command -v ar || true)}"
[[ -x "$CLANG" && -x "$CLANGXX" && -x "$AR" ]] || { echo "ERROR: clang/clang++/ar required" >&2; exit 1; }

WRAP="${RUNNER_TEMP:-/tmp}/agena-hurd-wrappers/$TARGET"
mkdir -p "$WRAP"
write_wrapper() {
  local path="$1" compiler="$2"
  cat >"$path" <<EOF
#!/usr/bin/env bash
set -eo pipefail
filtered=()
skip_next=false
for arg in "\$@"; do
  if [[ "\$skip_next" == true ]]; then skip_next=false; continue; fi
  case "\$arg" in
    --target=*) continue ;;
    --target|-target) skip_next=true; continue ;;
    *) filtered+=("\$arg") ;;
  esac
done
exec "$compiler" --target="$clang_target" --sysroot="$SYSROOT" --gcc-toolchain="$SYSROOT/usr" \
  -isystem "$SYSROOT/usr/include/$multiarch" \
  -L"$SYSROOT/usr/lib/$multiarch" -L"$SYSROOT/lib/$multiarch" \
  -fuse-ld="$LLD" -rtlib=libgcc "\${filtered[@]}"
EOF
  chmod +x "$path"
}
write_wrapper "$WRAP/cc" "$CLANG"
write_wrapper "$WRAP/cxx" "$CLANGXX"

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$WRAP/cc"
export "CXX_${key}=$WRAP/cxx"
export "AR_${key}=$AR"
export "CARGO_TARGET_${key_upper}_LINKER=$WRAP/cc"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAP/cc"

exec "$@"
