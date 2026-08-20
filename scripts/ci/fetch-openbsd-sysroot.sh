#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?target triple is required}"
VERSION="${AGENA_OPENBSD_VERSION:-7.9}"
SUFFIX="${VERSION/./}"

case "$TARGET" in
  aarch64-unknown-openbsd) release_arch=arm64 ;;
  i686-unknown-openbsd) release_arch=i386 ;;
  powerpc-unknown-openbsd) release_arch=macppc ;;
  powerpc64-unknown-openbsd) release_arch=powerpc64 ;;
  riscv64gc-unknown-openbsd) release_arch=riscv64 ;;
  sparc64-unknown-openbsd) release_arch=sparc64 ;;
  x86_64-unknown-openbsd) release_arch=amd64 ;;
  *) echo "ERROR: no OpenBSD release mapping for $TARGET" >&2; exit 2 ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-openbsd-sysroots/${VERSION}/${TARGET}"
SYSROOT="$ROOT/root"
BASE_URL="https://cdn.openbsd.org/pub/OpenBSD/${VERSION}/${release_arch}"
mkdir -p "$ROOT"

if [[ ! -f "$ROOT/.verified" ]]; then
  python3 - "$BASE_URL" "$ROOT" "$SUFFIX" <<'PY'
import hashlib
import pathlib
import re
import sys
import urllib.request

base_url, root_path, suffix = sys.argv[1:]
root = pathlib.Path(root_path)
checksum_text = urllib.request.urlopen(base_url + "/SHA256", timeout=60).read().decode()
expected = {}
for line in checksum_text.splitlines():
    match = re.match(r"SHA256 \(([^)]+)\) = ([0-9a-fA-F]{64})$", line.strip())
    if match:
        expected[match.group(1)] = match.group(2).lower()

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

for name in (f"base{suffix}.tgz", f"comp{suffix}.tgz"):
    if name not in expected:
        raise SystemExit(f"{name} checksum missing from OpenBSD SHA256")
    archive = root / name
    if archive.exists() and digest(archive) == expected[name]:
        continue
    tmp = archive.with_suffix(archive.suffix + ".tmp")
    with urllib.request.urlopen(f"{base_url}/{name}", timeout=300) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected[name]:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f"OpenBSD {name} SHA256 mismatch: expected {expected[name]}, got {actual}")
    tmp.replace(archive)
PY
  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT"
  tar -xzf "$ROOT/base${SUFFIX}.tgz" -C "$SYSROOT"
  tar -xzf "$ROOT/comp${SUFFIX}.tgz" -C "$SYSROOT"
  touch "$ROOT/.verified"
fi

printf '%s\n' "$SYSROOT"
