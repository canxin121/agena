#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?OpenBSD target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$TARGET" in
  aarch64-unknown-openbsd) clang_target=aarch64-unknown-openbsd ;;
  i686-unknown-openbsd) clang_target=i686-unknown-openbsd ;;
  powerpc-unknown-openbsd) clang_target=powerpc-unknown-openbsd ;;
  powerpc64-unknown-openbsd) clang_target=powerpc64-unknown-openbsd ;;
  riscv64gc-unknown-openbsd) clang_target=riscv64-unknown-openbsd ;;
  sparc64-unknown-openbsd) clang_target=sparc64-unknown-openbsd ;;
  x86_64-unknown-openbsd) clang_target=x86_64-unknown-openbsd ;;
  *) echo "ERROR: unsupported OpenBSD target: $TARGET" >&2; exit 2 ;;
esac

SYSROOT="$(bash scripts/ci/fetch-openbsd-sysroot.sh "$TARGET")"
STABLE_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
rust_sysroot="$(rustup run "$STABLE_TOOLCHAIN" rustc --print sysroot)"
host="$(rustup run "$STABLE_TOOLCHAIN" rustc -vV | sed -n 's/^host: //p')"
LLD="$rust_sysroot/lib/rustlib/$host/bin/gcc-ld/ld.lld"
if [[ ! -x "$LLD" ]]; then
  LLD="$rust_sysroot/lib/rustlib/$host/bin/rust-lld"
fi
[[ -x "$LLD" ]] || { echo "ERROR: Rust LLD not found for host $host" >&2; exit 1; }

if [[ -n "${AGENA_CLANG:-}" ]]; then
  CLANG="$AGENA_CLANG"
  CLANGXX="${AGENA_CLANGXX:-${AGENA_CLANG}++}"
  AR="${AGENA_LLVM_AR:-$(command -v llvm-ar || command -v ar || true)}"
elif [[ "$(uname -s)-$(uname -m)" == Linux-x86_64 && "$TARGET" == aarch64-unknown-openbsd ]]; then
  PINNED_CLANG="$(bash scripts/ci/fetch-pinned-clang.sh)"
  CLANG="$PINNED_CLANG/bin/clang"
  CLANGXX="$PINNED_CLANG/bin/clang++"
  AR="$PINNED_CLANG/bin/llvm-ar"
else
  CLANG="$(command -v clang || true)"
  CLANGXX="$(command -v clang++ || true)"
  AR="$(command -v llvm-ar || command -v ar || true)"
fi
[[ -x "$CLANG" && -x "$CLANGXX" && -x "$AR" ]] || {
  echo "ERROR: clang/clang++/ar are required for OpenBSD cross builds" >&2
  exit 1
}

WRAP="${RUNNER_TEMP:-/tmp}/agena-openbsd-wrappers/$TARGET"
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
exec "$compiler" --target="$clang_target" --sysroot="$SYSROOT" -fuse-ld="$LLD" "\${filtered[@]}"
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
if [[ "$TARGET" == aarch64-unknown-openbsd ]]; then
  # Both Ubuntu Clang 18 and Rust's pinned Clang 20 hit backend crashes while
  # compiling large C dependencies for AArch64 OpenBSD at cc-rs' dev-profile
  # -O0. Optimized codegen avoids those broken backend paths and matches the
  # release packaging profile.
  current_cflags="$(printenv "CFLAGS_${key}" || true)"
  current_cxxflags="$(printenv "CXXFLAGS_${key}" || true)"
  export "CFLAGS_${key}=${current_cflags:+$current_cflags }-O2"
  export "CXXFLAGS_${key}=${current_cxxflags:+$current_cxxflags }-O2"
fi
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAP/cc"

exec "$@"
