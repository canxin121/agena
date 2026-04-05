#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
WEB_DIR="$REPO_ROOT/packages/agena-studio-web"

if ! command -v bun >/dev/null 2>&1; then
  echo "ERROR: bun is required to build packages/agena-studio-web" >&2
  exit 1
fi

cd "$WEB_DIR"
bun install
bun run build
