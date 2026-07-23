#!/usr/bin/env bash
# Run once at a coherent runtime/session/tool slice checkpoint, not after each edit.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

cargo fmt --all --check
scripts/cargo-bounded.sh test -p agena-runtime --locked --quiet
scripts/cargo-bounded.sh test -p architecture-check --locked --quiet
scripts/cargo-bounded.sh run -p architecture-check --locked --quiet
git diff --check
