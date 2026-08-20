#!/usr/bin/env bash
set -euo pipefail

RUST_TARGET="${1:?Rust target triple is required}"
ZIG_TARGET="${2:?Zig target triple is required}"
shift 2
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

ZIG_SYSROOT="${AGENA_ZIG_SYSROOT:-}"
ZIG="$(bash scripts/ci/fetch-zig.sh)"

WRAPPER_ROOT="${RUNNER_TEMP:-/tmp}/agena-zig-wrappers/${RUST_TARGET}"
mkdir -p "$WRAPPER_ROOT"

write_compiler_wrapper() {
  local path="$1"
  local mode="$2"
  cat > "$path" <<EOF
#!/usr/bin/env bash
set -eo pipefail
filtered=()
skip_next=false
for arg in "\$@"; do
  if [[ "\$skip_next" == true ]]; then
    skip_next=false
    continue
  fi
  case "\$arg" in
    --target=*) continue ;;
    --target|-target) skip_next=true; continue ;;
    *) filtered+=("\$arg") ;;
  esac
done
extra=()
if [[ -n "$ZIG_SYSROOT" ]]; then
  extra+=(--sysroot "$ZIG_SYSROOT")
fi
exec "$ZIG" $mode -target "$ZIG_TARGET" "\${extra[@]}" "\${filtered[@]}"
EOF
  chmod +x "$path"
}

CC_WRAPPER="$WRAPPER_ROOT/cc"
CXX_WRAPPER="$WRAPPER_ROOT/cxx"
AR_WRAPPER="$WRAPPER_ROOT/ar"
write_compiler_wrapper "$CC_WRAPPER" cc
write_compiler_wrapper "$CXX_WRAPPER" c++
cat > "$AR_WRAPPER" <<EOF
#!/bin/sh
exec "$ZIG" ar "\$@"
EOF
chmod +x "$AR_WRAPPER"

target_key="${RUST_TARGET//-/_}"
target_key_upper="$(printf '%s' "$target_key" | tr '[:lower:]' '[:upper:]')"
export "CC_${target_key}=$CC_WRAPPER"
export "CXX_${target_key}=$CXX_WRAPPER"
export "AR_${target_key}=$AR_WRAPPER"
export "CARGO_TARGET_${target_key_upper}_LINKER=$CC_WRAPPER"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$CC_WRAPPER"

exec "$@"
