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
    && [[ -f "$SYSROOT/usr/include/assert.h" ]] \
    && [[ -f "$SYSROOT/usr/lib/crt1.o" ]] \
    && {
      # FreeBSD keeps the versioned libc runtime in /lib; some release
      # layouts also expose a versioned linker name under /usr/lib.  The
      # image extraction preserves both real directories, so accept either
      # official layout without inventing a linker or copying host files.
      compgen -G "$SYSROOT/lib/libc.so.*" >/dev/null \
        || compgen -G "$SYSROOT/usr/lib/libc.so.*" >/dev/null
    }
}

if ! valid_sysroot; then
  if ! command -v guestfish >/dev/null 2>&1; then
    if ! command -v sudo >/dev/null 2>&1; then
      echo "ERROR: libguestfs-tools is required to extract FreeBSD UFS images" >&2
      exit 1
    fi
    sudo apt-get update -y >&2
    sudo apt-get install -y --no-install-recommends libguestfs-tools >&2
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
    sudo apt-get update -y >&2
    sudo apt-get install -y --no-install-recommends linux-image-generic >&2
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
  # These ARM images use an MBR FreeBSD slice containing a BSD disklabel and
  # a UFS2 root partition. Linux/libguestfs exposes the outer MBR partition but
  # does not enumerate the nested BSD slices, so probing /dev/sda2 (or guessed
  # /dev/sda2a) cannot reach the real filesystem. Parse the official on-disk
  # labels, copy only the read-only root slice into a temporary sparse image,
  # and mount that exact UFS2 slice in libguestfs.
  readarray -t ROOT_GEOMETRY < <(python3 - "$IMAGE" <<'PY'
import os
import struct
import sys

image = sys.argv[1]
sector_size = 512
disk_magic = 0x82564557

with open(image, "rb") as source:
    mbr = source.read(sector_size)
    if len(mbr) != sector_size or mbr[510:512] != b"\x55\xaa":
        raise SystemExit("FreeBSD image does not contain a valid MBR")

    for index in range(4):
        entry = mbr[446 + index * 16 : 446 + (index + 1) * 16]
        partition_type = entry[4]
        partition_start, partition_size = struct.unpack_from("<II", entry, 8)
        if partition_type != 0xA5 or partition_size == 0:
            continue

        for label_relative_offset in (sector_size, 0):
            label_offset = (partition_start * sector_size) + label_relative_offset
            source.seek(label_offset)
            label = source.read(sector_size)
            if len(label) < 148:
                continue
            if struct.unpack_from("<I", label, 0)[0] != disk_magic:
                continue
            if struct.unpack_from("<I", label, 132)[0] != disk_magic:
                continue
            partition_count = struct.unpack_from("<H", label, 138)[0]
            if not 1 <= partition_count <= 22:
                continue

            for slice_index in (0, 2, 1, 3, 4, 5, 6, 7):
                if slice_index >= partition_count:
                    continue
                offset = 148 + slice_index * 16
                if offset + 16 > len(label):
                    continue
                slice_size, slice_start, _fragment_size = struct.unpack_from("<III", label, offset)
                filesystem_type = label[offset + 12]
                if filesystem_type not in (7, 8) or slice_size == 0:
                    continue
                if slice_start + slice_size > partition_size:
                    continue
                absolute_start = partition_start + slice_start
                absolute_end = absolute_start + slice_size
                if absolute_end * sector_size > os.path.getsize(image):
                    continue
                print(absolute_start)
                print(slice_size)
                raise SystemExit(0)

raise SystemExit("FreeBSD image has no bounded BSD disklabel UFS root slice")
PY
)
[[ "${#ROOT_GEOMETRY[@]}" -eq 2 ]] || {
  echo "ERROR: failed to parse the FreeBSD BSD disklabel in $FILE" >&2
  exit 1
}
ROOT_SKIP="${ROOT_GEOMETRY[0]}"
ROOT_COUNT="${ROOT_GEOMETRY[1]}"
SLICE_IMAGE="$ROOT/freebsd-ufs-root.img"
SLICE_BYTES=$((ROOT_COUNT * 512))
if [[ ! -f "$SLICE_IMAGE" ]] || [[ "$(stat -c '%s' "$SLICE_IMAGE")" != "$SLICE_BYTES" ]]; then
  rm -f "$SLICE_IMAGE"
  dd if="$IMAGE" of="$SLICE_IMAGE" bs=512 skip="$ROOT_SKIP" count="$ROOT_COUNT" \
    iflag=fullblock conv=sparse status=none
fi

if ! probe="$(guestfish --ro --format=raw -a "$SLICE_IMAGE" <<EOF
run
mount-options ro,ufstype=ufs2 /dev/sda /
exists /usr/include/stdio.h
exists /usr/include/assert.h
EOF
 )"; then
  echo "ERROR: guestfish could not mount the parsed FreeBSD UFS2 root slice from $FILE" >&2
  exit 1
fi
[[ "$probe" == *true*true* ]] || {
  echo "ERROR: parsed FreeBSD UFS root slice does not contain required C headers in $FILE" >&2
  exit 1
}

guestfish --ro --format=raw -a "$SLICE_IMAGE" <<EOF
run
mount-options ro,ufstype=ufs2 /dev/sda /
copy-out /usr/include $SYSROOT/usr
copy-out /usr/lib $SYSROOT/usr
copy-out /lib $SYSROOT
EOF

  valid_sysroot || { echo "ERROR: incomplete FreeBSD ARM sysroot extracted from $FILE" >&2; exit 1; }
fi

printf '%s\n' "$SYSROOT"
