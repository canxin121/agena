#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

require_binary_flag() {
  local name="$1"
  local value="$2"
  case "$value" in
    0|1) ;;
    *)
      printf '%s must be 0 or 1 (got %s)\n' "$name" "$value" >&2
      exit 2
      ;;
  esac
}

enforce="${ENFORCE_BUILD_TIMING:-0}"
measure_leaf_changes="${MEASURE_LEAF_CHANGES:-0}"
measure_cold_start="${MEASURE_COLD_START:-0}"
require_binary_flag "ENFORCE_BUILD_TIMING" "$enforce"
require_binary_flag "MEASURE_LEAF_CHANGES" "$measure_leaf_changes"
require_binary_flag "MEASURE_COLD_START" "$measure_cold_start"

measure() {
  local label="$1"
  local budget_seconds="$2"
  shift 2
  local timing_file
  timing_file="$(mktemp)"
  trap 'rm -f "$timing_file"' RETURN
  /usr/bin/time -p "$@" 2>"$timing_file"
  local elapsed
  elapsed="$(awk '$1 == "real" {print $2}' "$timing_file")"
  printf '%-42s %ss (budget %ss)\n' "$label" "$elapsed" "$budget_seconds"
  if [[ "$enforce" == "1" ]] && awk "BEGIN {exit !($elapsed > $budget_seconds)}"; then
    printf 'timing budget exceeded: %s\n' "$label" >&2
    return 1
  fi
}

# A cold baseline must not pollute the developer or bounded-gate target tree.
# Each sample gets a fresh, automatically removed target directory; its only
# purpose is to record the full dependency-graph cost for comparison with the
# edit-loop measurements below. This is an opt-in controlled broad build, so
# it intentionally disables incremental artifacts for the temporary target.
measure_cold() (
  local label="$1"
  shift
  local cold_target
  cold_target="$(mktemp -d "${TMPDIR:-/tmp}/agena-cold-target.XXXXXX")"
  cleanup_cold_target() {
    rm -rf -- "$cold_target"
  }
  trap cleanup_cold_target EXIT HUP INT TERM
  set +e
  measure "$label" 0 \
    env CARGO_TARGET_DIR="$cold_target" CARGO_INCREMENTAL=0 "$@"
  local status=$?
  set -e
  return "$status"
)

# The edit-loop samples deliberately keep normal incremental compilation. They
# still use the repository target, so fail before or after a sample if that
# retained cache has crossed the same operational safety ceiling as broad
# workspace gates. Cold samples are isolated above and are removed on exit.
target_dir="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
max_target_gib="${AGENA_MAX_TARGET_GIB:-40}"
if ! [[ "$max_target_gib" =~ ^[1-9][0-9]*$ ]]; then
  printf 'AGENA_MAX_TARGET_GIB must be a positive integer (got %s)\n' "$max_target_gib" >&2
  exit 2
fi

check_retained_target_size() {
  local target_kib=0
  if [[ -d "$target_dir" ]]; then
    target_kib="$(du -sk "$target_dir" 2>/dev/null | awk '{print $1}')"
    target_kib="${target_kib:-0}"
  fi
  local max_kib=$((max_target_gib * 1024 * 1024))
  if ((target_kib > max_kib)); then
    printf 'retained Cargo target exceeds %s GiB: %s\n' \
      "$max_target_gib" "$target_dir" >&2
    return 86
  fi
}

# Cargo's normal output only says which package changed. Verbose output exposes
# the rustc unit that actually ran, allowing the timing probe to guard the
# target graph as well as elapsed time. The accepted TUI-leaf rebuild set is
# intentionally minimal: agena-tui and the final app may rebuild, but a TUI
# source edit must not compile Runtime, provider leaves, SQLite, API server, or
# the remote client.
assert_tui_leaf_rebuild_attribution() {
  local log_file="$1"
  local forbidden_units='--crate-name (agena_runtime|agena_provider|agena_provider_google_auth|agena_provider_bedrock_auth|agena_provider_bedrock_signing|agena_provider_bedrock_streaming|agena_storage_sqlite|agena_api_server|agena_client)( |$)'
  if grep -E -- "$forbidden_units" "$log_file" >/dev/null; then
    printf '%s\n' 'TUI leaf rebuild compiled a forbidden concrete dependency:' >&2
    grep -E -- "$forbidden_units" "$log_file" >&2
    return 1
  fi
}

measure_with_tui_leaf_attribution() {
  local label="$1"
  local budget_seconds="$2"
  shift 2
  local timing_file
  local build_log
  timing_file="$(mktemp)"
  build_log="$(mktemp)"
  trap 'rm -f "$timing_file" "$build_log"' RETURN
  /usr/bin/time -p "$@" >"$build_log" 2>"$timing_file"
  cat "$timing_file" >>"$build_log"
  local elapsed
  elapsed="$(awk '$1 == "real" {print $2}' "$timing_file")"
  printf '%-42s %ss (budget %ss)\n' "$label" "$elapsed" "$budget_seconds"
  if [[ "$enforce" == "1" ]] && awk "BEGIN {exit !($elapsed > $budget_seconds)}"; then
    printf 'timing budget exceeded: %s\n' "$label" >&2
    return 1
  fi
  assert_tui_leaf_rebuild_attribution "$build_log"
}

check_retained_target_size

measure "no-change cargo check -p agena-tui" 1 \
  cargo check -p agena-tui --locked
check_retained_target_size
measure "no-change root cargo build" 2 \
  cargo build --locked
check_retained_target_size

if [[ "$measure_leaf_changes" == "1" ]]; then
  touch crates/agena-tui/src/input.rs
  measure_with_tui_leaf_attribution "TUI leaf change cargo check -p agena-tui" 15 \
    cargo check -p agena-tui --locked -vv
  check_retained_target_size

  touch crates/agena-cli/src/cli/cli_validation.rs
  measure "CLI leaf change cargo check -p agena-cli" 10 \
    cargo check -p agena-cli --locked
  check_retained_target_size

  measure_with_tui_leaf_attribution "TUI/CLI leaf change final agena build" 30 \
    cargo build -p agena --locked -vv
  check_retained_target_size
fi

if [[ "$measure_cold_start" == "1" ]]; then
  measure_cold "cold cargo check -p agena-tui" \
    cargo check -p agena-tui --locked
  measure_cold "cold cargo check -p agena-cli" \
    cargo check -p agena-cli --locked
  measure_cold "cold cargo build -p agena" \
    cargo build -p agena --locked
fi
