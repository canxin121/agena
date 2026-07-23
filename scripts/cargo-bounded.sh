#!/usr/bin/env bash
# Run Cargo while enforcing a repository-local target directory size ceiling.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
MAX_TARGET_GIB="${AGENA_MAX_TARGET_GIB:-40}"
CHECK_INTERVAL_SECONDS="${AGENA_TARGET_CHECK_INTERVAL_SECONDS:-5}"

if [[ $# -eq 0 ]]; then
  echo "usage: scripts/cargo-bounded.sh <cargo arguments...>" >&2
  exit 2
fi
if ! [[ "$MAX_TARGET_GIB" =~ ^[1-9][0-9]*$ ]]; then
  echo "AGENA_MAX_TARGET_GIB must be a positive integer" >&2
  exit 2
fi

mkdir -p "$TARGET_DIR"
export CARGO_TARGET_DIR="$TARGET_DIR"
export CARGO_INCREMENTAL=0

max_kib=$((MAX_TARGET_GIB * 1024 * 1024))
cargo "$@" &
cargo_pid=$!

monitor_target() {
  while kill -0 "$cargo_pid" 2>/dev/null; do
    target_kib="$(du -sk "$TARGET_DIR" 2>/dev/null | awk '{print $1}')"
    target_kib="${target_kib:-0}"
    if ((target_kib > max_kib)); then
      echo "error: $TARGET_DIR exceeded ${MAX_TARGET_GIB} GiB; stopping Cargo (currently $((target_kib / 1024 / 1024)) GiB)" >&2
      kill -TERM "$cargo_pid" 2>/dev/null || true
      return 86
    fi
    sleep "$CHECK_INTERVAL_SECONDS"
  done
}

monitor_target &
monitor_pid=$!
set +e
wait "$cargo_pid"
cargo_status=$?
kill "$monitor_pid" 2>/dev/null || true
wait "$monitor_pid" 2>/dev/null
monitor_status=$?
set -e

if ((monitor_status == 86)); then
  exit 86
fi
exit "$cargo_status"
