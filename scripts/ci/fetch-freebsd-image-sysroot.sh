#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?FreeBSD ARM target triple is required}"

case "$TARGET" in
  armv6-unknown-freebsd)
    VERSION=13.5
    FILE="FreeBSD-13.5-RELEASE-arm-armv6-RPI-B.img.xz"
    URL="https://archive.freebsd.org/old-releases/ISO-IMAGES/13.5/$FILE"
    SHA256="913f6ebb2a6c5acab5fb97c95e5ad0e2ecc747d42a3cbe38666e8fac566e7c61"
    ;;
  armv7-unknown-freebsd)
    VERSION=14.3
    FILE="FreeBSD-14.3-RELEASE-arm-armv7-GENERICSD.img.xz"
    URL="https://download.freebsd.org/releases/arm/armv7/ISO-IMAGES/14.3/$FILE"
    SHA256="b3627a92f8f8cc4b4eb0caaafe809fe0a2139a41cbaa8bee2a3c451ad81c4048"
    ;;
  *) echo "ERROR: no FreeBSD image mapping for $TARGET" >&2; exit 2 ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-freebsd-image-sysroots/$VERSION/$TARGET"
ARCHIVE="$ROOT/$FILE"
IMAGE="$ROOT/${FILE%.xz}"
SYSROOT="$ROOT/root"
mkdir -p "$ROOT"

python3 - "$URL" "$ARCHIVE" "$SHA256" <<'PY'
import hashlib
import pathlib
import sys
import urllib.request

url, archive_path, expected = sys.argv[1:]
archive = pathlib.Path(archive_path)

def digest(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()

if not archive.exists() or digest(archive) != expected:
    tmp = archive.with_suffix(archive.suffix + ".tmp")
    req = urllib.request.Request(url, headers={"User-Agent": "agena-freebsd-sysroot"})
    with urllib.request.urlopen(req, timeout=900) as src, tmp.open("wb") as dst:
        while True:
            chunk = src.read(1024 * 1024)
            if not chunk:
                break
            dst.write(chunk)
    actual = digest(tmp)
    if actual != expected:
        tmp.unlink(missing_ok=True)
        raise SystemExit(f"FreeBSD image SHA256 mismatch: expected {expected}, got {actual}")
    tmp.replace(archive)
PY

if [[ ! -f "$IMAGE" ]]; then
  xz -dkf "$ARCHIVE"
fi

valid_sysroot() {
  [[ -f "$SYSROOT/usr/include/stdio.h" ]] \
    && [[ -f "$SYSROOT/usr/lib/crt1.o" ]] \
    && compgen -G "$SYSROOT/usr/lib/libc.so.*" >/dev/null
}

if ! valid_sysroot; then
  if ! command -v guestfish >/dev/null 2>&1; then
    if ! command -v sudo >/dev/null 2>&1; then
      echo "ERROR: libguestfs-tools is required to extract FreeBSD UFS images" >&2
      exit 1
    fi
    sudo apt-get update -y
    sudo apt-get install -y --no-install-recommends libguestfs-tools
  fi

  # Ubuntu hosted runners can have libguestfs-tools installed while exposing
  # only an unreadable Azure kernel to the unprivileged runner user. Supermin
  # will choose that kernel before it ever opens the supplied FreeBSD image.
  # Prefer a readable non-Azure kernel with matching modules, and install the
  # real generic kernel package when the hosted image does not provide one.
  supermin_kernel=""
  supermin_modules=""
  find_supermin_kernel() {
    local skip_azure="${1:-true}"
    local candidate_kernel candidate_version candidate_modules
    while IFS= read -r candidate_kernel; do
      case "$candidate_kernel" in
        /boot/vmlinuz-*)
          candidate_version="${candidate_kernel##*/}"
          candidate_version="${candidate_version#vmlinuz-}"
          candidate_modules="/lib/modules/$candidate_version"
          ;;
        */modules/*/vmlinuz)
          candidate_modules="${candidate_kernel%/vmlinuz}"
          candidate_version="${candidate_modules##*/}"
          if [[ ! -d "$candidate_modules" && -d "/lib/modules/$candidate_version" ]]; then
            candidate_modules="/lib/modules/$candidate_version"
          fi
          ;;
        *)
          continue
          ;;
      esac
      sudo test -r "$candidate_kernel" || continue
      sudo test -d "$candidate_modules" || continue
      if [[ "$skip_azure" == true && "$candidate_version" == *-azure ]]; then
        continue
      fi
      supermin_kernel="$candidate_kernel"
      supermin_modules="$candidate_modules"
      return 0
    done < <(
      sudo find /boot /lib/modules /usr/lib/modules -maxdepth 3 -type f \
        \( -name 'vmlinuz-*' -o -name vmlinuz \) -print 2>/dev/null | sort -V
    )
    return 1
  }

  find_supermin_kernel true || true

  if [[ -z "$supermin_kernel" ]]; then
    if ! command -v sudo >/dev/null 2>&1; then
      echo "ERROR: a bootable Linux kernel is required by libguestfs supermin" >&2
      exit 1
    fi
    sudo apt-get update -y
    sudo apt-get install -y --no-install-recommends linux-image-generic
  fi

  if [[ -z "$supermin_kernel" ]]; then
    find_supermin_kernel true || find_supermin_kernel false || true
  fi
  [[ -n "$supermin_kernel" && -n "$supermin_modules" ]] || {
    echo "ERROR: libguestfs supermin has no readable kernel with matching modules" >&2
    exit 1
  }

  # A hosted runner may expose a matching kernel only through root-readable
  # paths.  Stage that real kernel and its matching real modules in the job's
  # temporary directory so the unprivileged libguestfs process can read them.
  if [[ ! -r "$supermin_kernel" || ! -r "$supermin_modules" ]]; then
    supermin_stage="$ROOT/supermin-kernel"
    mkdir -p "$supermin_stage"
    sudo cp "$supermin_kernel" "$supermin_stage/vmlinuz"
    sudo cp -a "$supermin_modules" "$supermin_stage/modules"
    sudo chown -R "$(id -u):$(id -g)" "$supermin_stage"
    supermin_kernel="$supermin_stage/vmlinuz"
    supermin_modules="$supermin_stage/modules"
  fi

  # Keep the appliance launch diagnostics in the job log.  This makes a
  # future hosted-image regression actionable without changing the read-only
  # image extraction semantics below.
  export LIBGUESTFS_BACKEND="direct"
  export LIBGUESTFS_DEBUG="1"
  export LIBGUESTFS_TRACE="1"
  export SUPERMIN_KERNEL="$supermin_kernel"
  export SUPERMIN_MODULES="$supermin_modules"

  rm -rf "$SYSROOT"
  mkdir -p "$SYSROOT/usr"
  fs_list="$ROOT/filesystems.txt"
  guestfish --ro -a "$IMAGE" <<'EOF' >"$fs_list"
run
list-filesystems
EOF
  rootdev="$(awk -F: '$2 ~ /ufs/ {gsub(/[[:space:]]/, "", $1); print $1; exit}' "$fs_list")"
  [[ -n "$rootdev" ]] || {
    echo "ERROR: no UFS root filesystem found in $FILE" >&2
    cat "$fs_list" >&2
    exit 1
  }

  guestfish --ro -a "$IMAGE" -m "$rootdev" <<EOF
copy-out /usr/include $SYSROOT/usr
copy-out /usr/lib $SYSROOT/usr
copy-out /lib $SYSROOT
EOF

  valid_sysroot || { echo "ERROR: incomplete FreeBSD ARM sysroot extracted from $FILE" >&2; exit 1; }
fi

printf '%s\n' "$SYSROOT"
