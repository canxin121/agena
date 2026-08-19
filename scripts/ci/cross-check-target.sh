#!/usr/bin/env bash
set -euo pipefail

target="${1:?target triple is required}"
build_std="${2:-false}"
artifact_kind="${3:-backend}"
export CARGO_TERM_COLOR=always
export CARGO_INCREMENTAL=0
export CROSS_NO_WARNINGS=0

case "$target" in
  i586-unknown-linux-*)
    # ring requires SSE/SSE2 even on legacy x86 when used through rustls.
    export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=+sse,+sse2"
    ;;
  mips64-unknown-linux-gnuabi64|mips64el-unknown-linux-gnuabi64)
    export RUSTFLAGS="${RUSTFLAGS:-} -C relocation-model=static"
    ;;
  aarch64_be-unknown-linux-gnu)
    # Rustix's linux_raw backend does not support big-endian AArch64. Force
    # every transitive Rustix version onto its libc backend.
    export RUSTFLAGS="${RUSTFLAGS:-} --cfg rustix_use_libc"
    ;;
esac

printf 'Checking Agena for %s (build_std=%s, artifact_kind=%s)\n' "$target" "$build_std" "$artifact_kind"
cross --version || true

package="agena"
if [[ "$artifact_kind" == "web-runtime" ]]; then
  package="agena-web-runtime"
fi
args=(check --manifest-path Cargo.toml -p "$package" --target "$target" --locked)
if [[ "$build_std" == true ]]; then
  cross +nightly-2026-08-18 "${args[@]}" -Z build-std=std,panic_abort,proc_macro
else
  cross "${args[@]}"
fi
