#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?NetBSD target triple is required}"
shift
[[ "$TARGET" == riscv64gc-unknown-netbsd ]] || {
  echo "ERROR: unsupported NetBSD source builder target: $TARGET" >&2
  exit 2
}
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

ROOT="$(bash scripts/ci/build-netbsd-riscv64-sysroot.sh)"
SYSROOT="$ROOT/destdir.risc"
TOOLDIR="$ROOT/tools"
WRAPPER_ROOT="${RUNNER_TEMP:-/tmp}/agena-netbsd-riscv64-wrappers/$TARGET"
mkdir -p "$WRAPPER_ROOT"

NETBSD_GXX="$TOOLDIR/bin/riscv64--netbsd-g++"
if [[ ! -x "$NETBSD_GXX" ]]; then
  NETBSD_GXX="$TOOLDIR/bin/riscv64--netbsd-c++"
fi
[[ -x "$TOOLDIR/bin/riscv64--netbsd-gcc" && -x "$NETBSD_GXX" ]] || {
  echo "ERROR: NetBSD riscv64 compiler tools are incomplete under $TOOLDIR/bin" >&2
  exit 1
}

cat > "$WRAPPER_ROOT/cc" <<EOF
#!/usr/bin/env bash
exec "$TOOLDIR/bin/riscv64--netbsd-gcc" --sysroot="$SYSROOT" "\$@"
EOF
cat > "$WRAPPER_ROOT/cxx" <<EOF
#!/usr/bin/env bash
exec "$NETBSD_GXX" --sysroot="$SYSROOT" "\$@"
EOF
cat > "$WRAPPER_ROOT/ar" <<EOF
#!/usr/bin/env bash
exec "$TOOLDIR/bin/riscv64--netbsd-ar" "\$@"
EOF
cat > "$WRAPPER_ROOT/ranlib" <<EOF
#!/usr/bin/env bash
exec "$TOOLDIR/bin/riscv64--netbsd-ranlib" "\$@"
EOF
chmod +x "$WRAPPER_ROOT/cc" "$WRAPPER_ROOT/cxx" "$WRAPPER_ROOT/ar" "$WRAPPER_ROOT/ranlib"

target_key="${TARGET//-/_}"
target_key_upper="$(printf '%s' "$target_key" | tr '[:lower:]' '[:upper:]')"
export "CC_${target_key}=$WRAPPER_ROOT/cc"
export "CXX_${target_key}=$WRAPPER_ROOT/cxx"
export "AR_${target_key}=$WRAPPER_ROOT/ar"
export "RANLIB_${target_key}=$WRAPPER_ROOT/ranlib"
export "CARGO_TARGET_${target_key_upper}_LINKER=$WRAPPER_ROOT/cc"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAPPER_ROOT/cc"

exec "$@"
