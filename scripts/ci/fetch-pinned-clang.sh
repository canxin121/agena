#!/usr/bin/env bash
set -euo pipefail

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: pinned cross Clang is published for Linux x86_64 hosts" >&2; exit 2 ;;
esac

# Match the Clang build pinned by Rust upstream's Fuchsia distribution image.
CLANG_ID='git_revision:388d7f144880dcd85ff31f06793304405a9f44b6'
CLANG_SHA256='970d1f427b9c9a3049d8622c80c86830ff31b5334ad8da47a2f1e81143197e8b'
CLANG_URL="https://chrome-infra-packages.appspot.com/dl/fuchsia/third_party/clang/linux-amd64/+/$CLANG_ID"
ROOT="${RUNNER_TEMP:-/tmp}/agena-pinned-clang"
ARCHIVE="$ROOT/clang.zip"
CLANG="$ROOT/clang"
mkdir -p "$ROOT"

if [[ ! -x "$CLANG/bin/clang" ]]; then
  python3 - "$CLANG_URL" "$CLANG_SHA256" "$ARCHIVE" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

url, expected, archive_path = sys.argv[1:]
archive = pathlib.Path(archive_path)

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

if not archive.exists() or digest(archive) != expected:
    tmp = archive.with_suffix(".tmp")
    with urllib.request.urlopen(url, timeout=300) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f"pinned Clang SHA256 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY
  rm -rf "$CLANG"
  mkdir -p "$CLANG"
  unzip -q "$ARCHIVE" -d "$CLANG"
fi

[[ -x "$CLANG/bin/clang" && -x "$CLANG/bin/clang++" && -x "$CLANG/bin/llvm-ar" ]] || {
  echo "ERROR: incomplete pinned Clang toolchain at $CLANG" >&2
  exit 1
}

printf '%s\n' "$CLANG"
