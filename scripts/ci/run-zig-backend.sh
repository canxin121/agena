#!/usr/bin/env bash
set -euo pipefail

RUST_TARGET="${1:?Rust target triple is required}"
ZIG_TARGET="${2:?Zig target triple is required}"
shift 2
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

ZIG_VERSION="${AGENA_ZIG_VERSION:-0.15.2}"
ZIG_SYSROOT="${AGENA_ZIG_SYSROOT:-}"
TOOL_ROOT="${RUNNER_TEMP:-/tmp}/agena-zig-$ZIG_VERSION"
if [[ -n "${AGENA_ZIG:-}" ]]; then
  ZIG="$AGENA_ZIG"
else
  if [[ ! -x "$TOOL_ROOT/bin/python" ]]; then
    python3 -m venv "$TOOL_ROOT"
  fi
  if ! "$TOOL_ROOT/bin/python" -c 'import ziglang' >/dev/null 2>&1; then
    "$TOOL_ROOT/bin/pip" install --disable-pip-version-check --no-input "ziglang==$ZIG_VERSION"
  fi
  ZIG="$($TOOL_ROOT/bin/python - <<'PY'
import pathlib, ziglang
print(pathlib.Path(ziglang.__file__).with_name("zig"))
PY
)"
fi
[[ -x "$ZIG" ]] || { echo "ERROR: Zig executable not found at $ZIG" >&2; exit 1; }

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
