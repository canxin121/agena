#!/usr/bin/env bash
set -euo pipefail

target="${1:?target triple is required}"
build_std="${2:-false}"
target_rustflags="${3:-}"
combined_rustflags="${RUSTFLAGS:-}"
if [[ -n "$target_rustflags" ]]; then
  combined_rustflags="${combined_rustflags:+$combined_rustflags }$target_rustflags"
fi

case "$target" in
  armv7s-apple-ios|i386-apple-ios)
    # Current Xcode otherwise derives the SDK version as the deployment target,
    # but Apple capped 32-bit iOS at iOS 10.
    export IPHONEOS_DEPLOYMENT_TARGET=10.0
    ;;
  i686-apple-darwin)
    # New Apple SDKs unconditionally declare _Float16 math functions even
    # though Clang's retired 32-bit macOS target cannot represent _Float16.
    # Those declarations never existed for the i386 ABI; make them parse as
    # float for C/C++ dependencies without changing any exported Agena ABI.
    export CFLAGS_i686_apple_darwin="${CFLAGS_i686_apple_darwin:-} -D_Float16=float"
    export CXXFLAGS_i686_apple_darwin="${CXXFLAGS_i686_apple_darwin:-} -D_Float16=float"
    ;;
esac

args=(check --manifest-path Cargo.toml -p agena --target "$target" --locked)
case "$target" in
  mipsisa32r6-unknown-linux-gnu|mipsisa32r6el-unknown-linux-gnu)
    # rustc 1.97 SIGTRAPs while codegening core in the dev/check profile for
    # the MIPS32r6 built-ins. The same exact Rust 1.97 sources and target ABI
    # compile cleanly under the optimized release profile, which is also what
    # the release packaging path uses.
    args=(check --release --manifest-path Cargo.toml -p agena --target "$target" --locked)
    ;;
esac
if [[ "$build_std" == true ]]; then
  RUSTFLAGS="$combined_rustflags" \
    bash scripts/ci/run-build-std-cargo.sh "$target" "${args[@]}"
else
  RUSTFLAGS="$combined_rustflags" cargo "${args[@]}"
fi
