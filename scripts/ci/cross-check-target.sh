#!/usr/bin/env bash
set -euo pipefail

target="${1:?target triple is required}"
build_std="${2:-false}"
artifact_kind="${3:-backend}"
export CARGO_TERM_COLOR=always
export CARGO_INCREMENTAL=0
export CROSS_NO_WARNINGS=0


printf 'Checking Agena for %s (build_std=%s, artifact_kind=%s)\n' "$target" "$build_std" "$artifact_kind"
cross --version || true
[[ "$artifact_kind" == "backend" ]] || {
  echo "ERROR: only full Agena backend artifacts are supported: $artifact_kind" >&2
  exit 2
}

package="agena"
args=(check --manifest-path Cargo.toml -p "$package" --target "$target" --locked)
if [[ "$build_std" == true ]]; then
  cross +nightly-2026-08-18 "${args[@]}" -Z build-std=std,panic_abort,proc_macro
else
  cross "${args[@]}"
fi
