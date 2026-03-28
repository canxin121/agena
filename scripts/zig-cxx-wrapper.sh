#!/usr/bin/env bash
set -euo pipefail

rewritten_args=()
has_target_flag=0
for arg in "$@"; do
  case "$arg" in
    --target=x86_64-unknown-linux-gnu)
      rewritten_args+=("--target=x86_64-linux-gnu")
      has_target_flag=1
      ;;
    --target=aarch64-unknown-linux-gnu)
      rewritten_args+=("--target=aarch64-linux-gnu")
      has_target_flag=1
      ;;
    --target=aarch64-apple-darwin)
      rewritten_args+=("--target=aarch64-macos")
      has_target_flag=1
      ;;
    --target=x86_64-apple-darwin)
      rewritten_args+=("--target=x86_64-macos")
      has_target_flag=1
      ;;
    -target)
      rewritten_args+=("$arg")
      has_target_flag=1
      ;;
    *)
      rewritten_args+=("$arg")
      ;;
  esac
done

if [[ "$has_target_flag" -eq 0 && -n "${AGENA_ZIG_TARGET:-}" ]]; then
  rewritten_args=("-target" "${AGENA_ZIG_TARGET}" "${rewritten_args[@]}")
fi

exec zig c++ "${rewritten_args[@]}"
