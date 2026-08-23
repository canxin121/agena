#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?target triple is required}"
VERSION="${AGENA_FREEBSD_VERSION:-14.3}"

case "$TARGET" in
  powerpc-unknown-freebsd) release_arch="powerpc/powerpc" ;;
  riscv64gc-unknown-freebsd) release_arch="riscv/riscv64" ;;
  *)
    echo "ERROR: no base.txz FreeBSD sysroot mapping for $TARGET" >&2
    exit 2
    ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-freebsd-sysroots/${VERSION}/${TARGET}"
ARCHIVE="$ROOT/base.txz"
SYSROOT="$ROOT/root"
BASE_URL="https://download.freebsd.org/releases/${release_arch}/${VERSION}-RELEASE"
mkdir -p "$ROOT"

if [[ ! -f "$ROOT/.verified" ]]; then
  python3 - "$BASE_URL" "$ARCHIVE" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

base_url, archive_path = sys.argv[1:]
archive = pathlib.Path(archive_path)
manifest = urllib.request.urlopen(base_url + "/MANIFEST", timeout=60).read().decode()
expected = None
for line in manifest.splitlines():
    fields = line.split("\t")
    if fields and fields[0] == "base.txz":
        expected = fields[1]
        break
if expected is None:
    raise SystemExit("base.txz checksum missing from FreeBSD MANIFEST")

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

if not archive.exists() or digest(archive) != expected:
    tmp = archive.with_suffix(".tmp")
    with urllib.request.urlopen(base_url + "/base.txz", timeout=120) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    if digest(tmp) != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit("FreeBSD base.txz SHA256 mismatch")
    tmp.replace(archive)
PY
  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT"
  tar -xJf "$ARCHIVE" -C "$SYSROOT"
  touch "$ROOT/.verified"
fi

printf '%s\n' "$SYSROOT"
