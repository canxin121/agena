#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?target triple is required}"
shift
[[ $# -gt 0 ]] || { echo "ERROR: cargo command is required" >&2; exit 2; }

STABLE_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
NIGHTLY_TOOLCHAIN="${AGENA_NIGHTLY_TOOLCHAIN:-nightly-2026-08-18}"
stable_rustc="$(rustup which --toolchain "$STABLE_TOOLCHAIN" rustc)"
stable_rustdoc="$(rustup which --toolchain "$STABLE_TOOLCHAIN" rustdoc)"
sysroot="$("$stable_rustc" --print sysroot)"

backups=()
backup_source() {
  local path="$1"
  local backup
  backup="$(mktemp -t agena-rust-src.XXXXXX)"
  cp "$path" "$backup"
  backups+=("$path::$backup")
}

cleanup() {
  local item path backup
  for item in "${backups[@]:-}"; do
    [[ -n "$item" ]] || continue
    path="${item%%::*}"
    backup="${item#*::}"
    cp "$backup" "$path"
    rm -f "$backup"
  done
}
trap cleanup EXIT

case "$TARGET" in
  hexagon-*)
    scalar="$sysroot/lib/rustlib/src/rust/library/stdarch/crates/core_arch/src/hexagon/scalar.rs"
    backup_source "$scalar"
    python3 - "$scalar" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = '#[inline(always)]\n#[cfg_attr(target_arch = "hexagon", target_feature'
count = text.count(needle)
if count == 0:
    raise SystemExit("Hexagon stdarch patch point not found")
path.write_text(
    text.replace(
        needle,
        '#[inline]\n#[cfg_attr(target_arch = "hexagon", target_feature',
    ),
    encoding="utf-8",
)
print(f"Patched {count} Hexagon stdarch inline/target_feature sites")
PY
    ;;
esac

RUSTC_BOOTSTRAP=1 \
RUSTC="$stable_rustc" \
RUSTDOC="$stable_rustdoc" \
  cargo "+$NIGHTLY_TOOLCHAIN" "$@" -Z build-std=std,panic_abort,proc_macro
