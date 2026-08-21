#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?FreeBSD ARM target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$TARGET" in
  armv6-unknown-freebsd) clang_target=armv6-unknown-freebsd-gnueabihf ;;
  armv7-unknown-freebsd) clang_target=armv7-unknown-freebsd-gnueabihf ;;
  *) echo "ERROR: unsupported FreeBSD ARM target: $TARGET" >&2; exit 2 ;;
esac

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: FreeBSD ARM image builder requires Linux x86_64" >&2; exit 2 ;;
esac

SYSROOT="$(bash scripts/ci/fetch-freebsd-image-sysroot.sh "$TARGET")"
[[ -d "$SYSROOT" ]] || {
  echo "ERROR: FreeBSD image sysroot builder returned an invalid path: $SYSROOT" >&2
  exit 1
}
[[ -f "$SYSROOT/usr/include/assert.h" ]] || {
  echo "ERROR: FreeBSD image sysroot is missing the official assert.h: $SYSROOT/usr/include/assert.h" >&2
  exit 1
}
PINNED_CLANG="$(bash scripts/ci/fetch-pinned-clang.sh)"
CLANG="$PINNED_CLANG/bin/clang"
CLANGXX="$PINNED_CLANG/bin/clang++"
AR="$PINNED_CLANG/bin/llvm-ar"

STABLE_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
rust_sysroot="$(rustup run "$STABLE_TOOLCHAIN" rustc --print sysroot)"
host="$(rustup run "$STABLE_TOOLCHAIN" rustc -vV | sed -n 's/^host: //p')"
LLD="$rust_sysroot/lib/rustlib/$host/bin/gcc-ld/ld.lld"
[[ -x "$LLD" ]] || LLD="$rust_sysroot/lib/rustlib/$host/bin/rust-lld"
[[ -x "$LLD" ]] || { echo "ERROR: Rust LLD missing for host $host" >&2; exit 1; }

WRAP="${RUNNER_TEMP:-/tmp}/agena-freebsd-arm-wrappers/$TARGET"
mkdir -p "$WRAP"
write_wrapper() {
  local path="$1"
  cat >"$path" <<'EOF'
#!/usr/bin/env bash
set -eo pipefail
filtered=()
skip_next=false
for arg in "$@"; do
  if [[ "$skip_next" == true ]]; then skip_next=false; continue; fi
  case "$arg" in
    --target=*) continue ;;
    --target|-target) skip_next=true; continue ;;
    *) filtered+=("$arg") ;;
  esac
done
case "${0##*/}" in
  cc) compiler="${AGENA_FREEBSD_CLANG:-}" ;;
  cxx) compiler="${AGENA_FREEBSD_CLANGXX:-}" ;;
  *)
    echo "ERROR: unknown FreeBSD ARM compiler wrapper: ${0##*/}" >&2
    exit 2
    ;;
esac
[[ -n "$compiler" ]] || {
  echo "ERROR: FreeBSD ARM compiler wrapper has no compiler configured" >&2
  exit 2
}
exec "$compiler" --target="$AGENA_FREEBSD_CLANG_TARGET" \
  --sysroot="$AGENA_FREEBSD_SYSROOT" \
  -isystem "$AGENA_FREEBSD_SYSROOT/usr/include" \
  -fuse-ld="$AGENA_FREEBSD_LLD" \
  "${filtered[@]}"
EOF
  chmod +x "$path"
}
export AGENA_FREEBSD_CLANG="$CLANG"
export AGENA_FREEBSD_CLANGXX="$CLANGXX"
export AGENA_FREEBSD_CLANG_TARGET="$clang_target"
export AGENA_FREEBSD_SYSROOT="$SYSROOT"
export AGENA_FREEBSD_LLD="$LLD"
write_wrapper "$WRAP/cc"
write_wrapper "$WRAP/cxx"
bash -n "$WRAP/cc"
bash -n "$WRAP/cxx"

# Exercise the exact compiler/sysroot wrapper before Cargo invokes any C
# build script. This proves the target compiler can see the real FreeBSD
# headers and prevents a later ring error from hiding a wrapper/environment
# regression.
printf '#include <assert.h>\nint main(void) { return 0; }\n' |
  "$WRAP/cc" -x c -fsyntax-only - >/dev/null

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$WRAP/cc"
export "CXX_${key}=$WRAP/cxx"
export "AR_${key}=$AR"
export "CARGO_TARGET_${key_upper}_LINKER=$WRAP/cc"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAP/cc"

exec "$@"
