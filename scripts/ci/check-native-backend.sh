#!/usr/bin/env bash
set -euo pipefail

target="${1:?target triple is required}"
build_std="${2:-false}"
target_rustflags="${3:-}"
export RUSTUP_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
combined_rustflags="${RUSTFLAGS:-}"
if [[ -n "$target_rustflags" ]]; then
  combined_rustflags="${combined_rustflags:+$combined_rustflags }$target_rustflags"
fi


args=(check --manifest-path Cargo.toml -p agena --target "$target" --locked)
if [[ "$build_std" == true ]]; then
  RUSTFLAGS="$combined_rustflags" \
    bash scripts/ci/run-build-std-cargo.sh "$target" "${args[@]}"
else
  RUSTFLAGS="$combined_rustflags" cargo "${args[@]}"
fi
