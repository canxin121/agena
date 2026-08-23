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

valid_sysroot() {
  [[ -f "$SYSROOT/usr/include/stdio.h" ]] \
    && [[ -f "$SYSROOT/usr/lib/crt0.o" ]] \
    && [[ -f "$SYSROOT/usr/lib/libcompiler_rt.a" ]] \
    && compgen -G "$SYSROOT/usr/lib/libc.so.*" >/dev/null
}

if ! valid_sysroot; then
  python3 - "$BASE_URL" "$ROOT" "$SUFFIX" <<'PY'
import hashlib
import pathlib
import re
import subprocess
import sys

base_url, root_path, suffix = sys.argv[1:]
root = pathlib.Path(root_path)


def download(url: str, destination: pathlib.Path, timeout: int) -> None:
    """Download an official OpenBSD file atomically with retryable transport errors."""
    temporary = destination.with_name(destination.name + ".tmp")
    temporary.unlink(missing_ok=True)
    try:
        subprocess.run(
            [
                "curl",
                "--fail",
                "--location",
                "--retry",
                "12",
                "--retry-all-errors",
                "--retry-delay",
                "5",
                "--retry-max-time",
                "900",
                "--connect-timeout",
                "30",
                "--max-time",
                str(timeout),
                "--user-agent",
                "agena-openbsd-sysroot/1",
                "--output",
                str(temporary),
                url,
            ],
            check=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        temporary.unlink(missing_ok=True)
        raise SystemExit(f"OpenBSD download failed for {url}: {error}") from error
    temporary.replace(destination)


checksum_path = root / "SHA256"
download(base_url + "/SHA256", checksum_path, timeout=120)
checksum_text = checksum_path.read_text(encoding="utf-8")
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
    download(f"{base_url}/{name}", archive, timeout=1800)
    actual = digest(archive)
    if actual != expected[name]:
        archive.unlink(missing_ok=True)
        raise SystemExit(f"OpenBSD {name} SHA256 mismatch: expected {expected[name]}, got {actual}")
PY
  if [[ -e "$SYSROOT" ]]; then
    chmod -R u+rwX "$SYSROOT" 2>/dev/null || true
  fi
  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT"
  tar -xzf "$ROOT/comp${SUFFIX}.tgz" -C "$SYSROOT"
  python3 - "$ROOT/base${SUFFIX}.tgz" "$SYSROOT" <<'PY'
import pathlib
import sys
import tarfile

archive_path, sysroot_path = sys.argv[1:]
sysroot = pathlib.Path(sysroot_path)
wanted_exact = {
    "./usr/lib/crt0.o",
    "./usr/lib/rcrt0.o",
    "./usr/lib/crtbegin.o",
    "./usr/lib/crtbeginS.o",
    "./usr/lib/crtend.o",
    "./usr/lib/crtendS.o",
    "./usr/lib/libcompiler_rt.a",
    "./usr/libexec/ld.so",
}
with tarfile.open(archive_path, "r:gz") as tf:
    members = []
    for member in tf.getmembers():
        name = member.name
        if name in wanted_exact or name.startswith("./usr/lib/libc.so."):
            members.append(member)
    # Members are selected from a fixed allow-list above; avoid Python 3.12-only
    # tarfile filters so GitHub/macOS Python 3.9 can build the sysroot too.
    tf.extractall(sysroot, members=members)
PY
  libc_so="$(find "$SYSROOT/usr/lib" -maxdepth 1 -type f -name 'libc.so.*' | sort -V | tail -1)"
  [[ -n "$libc_so" ]] || { echo "ERROR: OpenBSD libc.so missing from base set" >&2; exit 1; }
  ln -sf "$(basename "$libc_so")" "$SYSROOT/usr/lib/libc.so"
  valid_sysroot || { echo "ERROR: incomplete OpenBSD sysroot for $TARGET" >&2; exit 1; }
fi

printf '%s\n' "$SYSROOT"
