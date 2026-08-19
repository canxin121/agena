#!/usr/bin/env bash
set -euo pipefail

target="${1:?target triple is required}"
export CARGO_TERM_COLOR=always
export CARGO_INCREMENTAL=0
export CROSS_NO_WARNINGS=0

printf 'Checking Agena for %s\n' "$target"
cross --version || true
cross check \
  --manifest-path Cargo.toml \
  -p agena \
  --target "$target" \
  --locked
