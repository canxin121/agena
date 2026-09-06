#!/usr/bin/env bash
# Dependency inspection entrypoint for local and CI reporting.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
MODE="${1:-check}"

cd "$REPO_ROOT"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "ERROR: required command '$1' is not installed" >&2
    exit 1
  fi
}

run_report() {
  local title="$1"
  shift
  echo
  echo "## $title"
  if ! "$@"; then
    echo "WARNING: '$title' reported findings; see the output above" >&2
  fi
}

check_bun_version() {
  local expected
  local actual
  expected="$(tr -d '[:space:]' < .bun-version)"
  actual="$(bun --version)"
  echo "Expected: $expected"
  echo "Running:  $actual"
  [[ "$actual" == "$expected" ]]
}

check_dependencies() {
  require_command cargo-deny
  require_command cargo-audit
  require_command cargo-machete
  require_command bun
  require_command git

  check_bun_version
  cargo metadata --locked --format-version 1 > /dev/null
  cargo deny check bans licenses sources advisories
  cargo audit --ignore RUSTSEC-2023-0071
  cargo machete

  (
    cd packages/agena-web
    bun install --frozen-lockfile
    bun audit
  )
}

report_dependencies() {
  require_command cargo-deny
  require_command cargo-audit
  require_command cargo-machete
  require_command bun
  require_command git

  run_report "Bun runtime version" check_bun_version
  run_report "Cargo manifest upgrades" \
    cargo upgrade --dry-run --incompatible allow --pinned allow \
      --exclude ratatui-image \
      --exclude ratex-layout \
      --exclude ratex-parser \
      --exclude ratex-render \
      --exclude ratex-types
  run_report "Cargo lockfile freshness" cargo update --workspace --dry-run
  run_report "RustSec and cargo-deny" cargo deny check advisories
  # Keep the same documented exceptions as deny.toml.
  run_report "Complete Cargo.lock audit" cargo audit --ignore RUSTSEC-2023-0071
  run_report "Unused direct Cargo dependencies" cargo machete
  run_report "Web package upgrades" bash -c \
    'cd packages/agena-web && bun outdated'
  run_report "Web package audit" bash -c \
    'cd packages/agena-web && bun audit'
}

case "$MODE" in
  check)
    check_dependencies
    ;;
  report)
    require_command cargo-upgrade
    report_dependencies
    ;;
  *)
    echo "Usage: $0 [check|report]" >&2
    exit 2
    ;;
esac
