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
redoxer toolchain
NIGHTLY_TOOLCHAIN="${AGENA_NIGHTLY_TOOLCHAIN:-nightly-2026-08-18}"
export AGENA_CARGO_DRIVER="$(rustup which --toolchain "$NIGHTLY_TOOLCHAIN" cargo)"

# redoxer env exports the target GCC/binutils/relibc sysroot variables that
# native C build scripts need. We still invoke the repository's normal Cargo
# command so Rust 1.97 and build-std policy remain under our control.
exec redoxer env "$@"
