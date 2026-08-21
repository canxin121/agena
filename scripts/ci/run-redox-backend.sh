#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?Redox target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$TARGET" in
  aarch64-unknown-redox|i586-unknown-redox|riscv64gc-unknown-redox|x86_64-unknown-redox) ;;
  *) echo "ERROR: unsupported Redox target: $TARGET" >&2; exit 2 ;;
esac

if ! command -v redoxer >/dev/null 2>&1; then
  cargo install redoxer --locked --version 0.2.63
fi

export TARGET
export AGENA_TARGET_TRIPLE="$TARGET"
redoxer toolchain
NIGHTLY_TOOLCHAIN="${AGENA_NIGHTLY_TOOLCHAIN:-nightly-2026-08-18}"
export AGENA_CARGO_DRIVER="$(rustup which --toolchain "$NIGHTLY_TOOLCHAIN" cargo)"

# redoxer env exports both target-specific variables (which native C build
# scripts need) and global CC/CXX/AR variables.  The latter are inherited by
# Cargo build scripts that compile for the Linux host, so a host build-script
# executable can accidentally be linked from Redox objects.  Keep the
# target-specific CC_<triple>/AR_<triple>/CARGO_TARGET_* variables while
# removing only the global target-tool selections and flags.
exec redoxer env env \
  -u CC -u CXX -u AR -u AS -u LD -u NM -u OBJCOPY -u OBJDUMP \
  -u RANLIB -u READELF -u STRIP -u PKG_CONFIG \
  -u CPPFLAGS -u CFLAGS -u CXXFLAGS -u LDFLAGS -u RUSTFLAGS \
  -- "$@"
