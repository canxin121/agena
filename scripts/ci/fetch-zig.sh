#!/usr/bin/env bash
set -euo pipefail

ZIG_VERSION="${AGENA_ZIG_VERSION:-0.15.2}"

if [[ -n "${AGENA_ZIG:-}" ]]; then
  [[ -x "$AGENA_ZIG" ]] || {
    echo "ERROR: AGENA_ZIG is not executable: $AGENA_ZIG" >&2
    exit 1
  }
  printf '%s\n' "$AGENA_ZIG"
  exit 0
fi

TOOL_ROOT="${RUNNER_TEMP:-/tmp}/agena-zig-$ZIG_VERSION"
if [[ ! -x "$TOOL_ROOT/bin/python" ]]; then
  python3 -m venv "$TOOL_ROOT"
fi

if ! "$TOOL_ROOT/bin/python" -c 'import ziglang' >/dev/null 2>&1; then
  "$TOOL_ROOT/bin/pip" install \
    --disable-pip-version-check \
    --no-input \
    "ziglang==$ZIG_VERSION"
fi

ZIG="$($TOOL_ROOT/bin/python -c 'import pathlib, ziglang; print(pathlib.Path(ziglang.__file__).with_name("zig"))')"
[[ -x "$ZIG" ]] || {
  echo "ERROR: Zig executable not found at $ZIG" >&2
  exit 1
}

printf '%s\n' "$ZIG"
