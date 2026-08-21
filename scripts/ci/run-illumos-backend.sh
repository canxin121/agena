#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?illumos target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

[[ "$TARGET" == aarch64-unknown-illumos ]] || {
  echo "ERROR: unsupported illumos target: $TARGET" >&2
  exit 2
}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: illumos AArch64 sysroot builder requires Linux x86_64" >&2; exit 2 ;;
esac

# The illumos project documents its sysroot artifacts as the supported source
# for cross-compiling C code.  The official illumos/sysroot repository does
# not publish an AArch64 artifact, so use the fixed AArch64 OmniOS IPS
# packages that are also used by the illumos AArch64 bootstrap project.  The
# package manifests and every referenced blob are verified before extraction;
# this is a target ABI sysroot, not a Linux compatibility shim.
IPS_BASE="https://pkg.omnios.org/bloody/braich"
ROOT="${RUNNER_TEMP:-/tmp}/agena-illumos/$TARGET"
SYSROOT="$ROOT/sysroot"
mkdir -p "$ROOT"

if [[ ! -f "$ROOT/.complete" ]]; then
  rm -rf "$SYSROOT" "$ROOT/manifests" "$ROOT/blobs"
  mkdir -p "$SYSROOT" "$ROOT/manifests" "$ROOT/blobs"

  python3 - "$IPS_BASE" "$ROOT" "$SYSROOT" <<'PY'
import concurrent.futures
import gzip
import hashlib
import os
import pathlib
import shlex
import sys
import time
import urllib.error
import urllib.request

base, root_name, sysroot_name = sys.argv[1:]
root = pathlib.Path(root_name)
sysroot = pathlib.Path(sysroot_name)
manifest_dir = root / "manifests"
blob_dir = root / "blobs"

packages = [
    (
        "system-header",
        "system%2Fheader@0.5.11%2C5.11-151059.0%3A20260820T125231Z",
        "fea44fd8a83ec98900904468355668185182b17ea6c4cf4989eb1379b8e45330",
    ),
    (
        "system-library",
        "system%2Flibrary@0.5.11%2C5.11-151059.0%3A20260820T125234Z",
        "b38773eb5773f301da1514238aa75b3069589c08db33155d88ac756d7bf5993a",
    ),
    (
        "system-library-math",
        "system%2Flibrary%2Fmath@0.5.11%2C5.11-151059.0%3A20260820T125233Z",
        "d176cdf7c45ff7c1405d2d051824374ced5696e736eea4107c54653f25bb8aa5",
    ),
    (
        "system-library-c-runtime",
        "system%2Flibrary%2Fc-runtime@0.5.11%2C5.11-151059.0%3A20260820T125232Z",
        "c6840fd75cf1cb90f20a8a178d684b4676d56dbc737e15a6372b7b50c2f8bd72",
    ),
    (
        "system-library-gcc-runtime",
        "system%2Flibrary%2Fgcc-runtime@15%2C5.11-151059.0%3A20260820T112139Z",
        "336c5fbc5e8319903f30c1ac837267fdc3fec1754c9669030b88559862a8a862",
    ),
    (
        "system-library-gxx-runtime",
        "system%2Flibrary%2Fg%2B%2B-runtime@15%2C5.11-151059.0%3A20260820T112152Z",
        "549400813644e1296733745503bf9805dd2653dd140a4691018d2e6fb6f6b5d4",
    ),
]


def sha512t256(data: bytes) -> str:
    return hashlib.new("sha512_256", data).hexdigest()


def download(url: str) -> bytes:
    last = None
    for attempt in range(6):
        try:
            request = urllib.request.Request(
                url,
                headers={"User-Agent": "agena-illumos-sysroot/1"},
            )
            with urllib.request.urlopen(request, timeout=180) as response:
                return response.read()
        except (OSError, urllib.error.URLError) as error:
            last = error
            if attempt == 5:
                break
            time.sleep(min(30, 2**attempt))
    raise SystemExit(f"failed to download {url}: {last}")


def manifest_for(name: str, encoded: str, expected: str):
    path = manifest_dir / f"{name}.manifest"
    if path.is_file():
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() == expected:
            return data
    data = download(f"{base}/manifest/0/{encoded}")
    actual = hashlib.sha256(data).hexdigest()
    if actual != expected:
        raise SystemExit(
            f"manifest SHA256 mismatch for {name}: expected {expected}, got {actual}"
        )
    temporary = path.with_suffix(".tmp")
    temporary.write_bytes(data)
    os.replace(temporary, path)
    return data


def fields(line: str):
    tokens = shlex.split(line, comments=False, posix=True)
    result = {"action": tokens[0]}
    if tokens[0] in {"file", "license"}:
        result["hash"] = tokens[1]
        tokens = tokens[2:]
    else:
        tokens = tokens[1:]
    for token in tokens:
        if "=" in token:
            key, value = token.split("=", 1)
            previous = result.get(key)
            if previous is None:
                result[key] = value
            elif isinstance(previous, list):
                previous.append(value)
            else:
                result[key] = [previous, value]
    return result


def needed(package: str, path: str) -> bool:
    # Do not spend time downloading documentation and manpage links.  Keep all
    # headers, ABI/runtime libraries, CRT objects, and the compiler runtimes.
    if package == "system-header":
        return (
            path.startswith("usr/include/")
            or path.startswith("usr/platform/")
            or path.startswith("usr/xpg4/include/")
        )
    if package == "system-library":
        return (
            path.startswith("lib/")
            or path.startswith("usr/lib/")
            or path.startswith("usr/ccs/lib/")
            or path.startswith("usr/xpg4/lib/")
        )
    if package == "system-library-math":
        return path.startswith("lib/") or path.startswith("usr/lib/")
    return path.startswith("usr/")


actions = []
for package, encoded, expected in packages:
    data = manifest_for(package, encoded, expected).decode("utf-8")
    for line in data.splitlines():
        if not line or line.startswith("#"):
            continue
        action = fields(line)
        path = action.get("path", "")
        if not path or not needed(package, path):
            continue
        relative = pathlib.PurePosixPath(path)
        if relative.is_absolute() or ".." in relative.parts:
            raise SystemExit(f"unsafe IPS path: {path}")
        actions.append(action)


def action_path(action):
    return sysroot.joinpath(*pathlib.PurePosixPath(action["path"]).parts)


def mkdir_parent(path: pathlib.Path):
    path.parent.mkdir(parents=True, exist_ok=True)


def content_hash(action, kind):
    prefix = f"{kind}:sha512t_256:"
    values = action.get("pkg.content-hash", [])
    if isinstance(values, str):
        values = [values]
    for value in values:
        if value.startswith(prefix):
            return value[len(prefix) :]
    return None


file_actions = []
for action in actions:
    if action["action"] == "file":
        file_actions.append(action)


def fetch_blob(action):
    blob_id = action["hash"]
    destination = blob_dir / blob_id
    expected_gzip = content_hash(action, "gzip")
    if expected_gzip is None:
        raise SystemExit(f"IPS file has no gzip content hash: {action['path']}")
    if destination.is_file() and sha512t256(destination.read_bytes()) == expected_gzip:
        return action, destination
    data = download(f"{base}/file/1/{blob_id}")
    actual = sha512t256(data)
    if actual != expected_gzip:
        raise SystemExit(
            f"blob hash mismatch for {action['path']}: expected {expected_gzip}, got {actual}"
        )
    temporary = destination.with_name(f".{blob_id}.{os.getpid()}.tmp")
    temporary.write_bytes(data)
    os.replace(temporary, destination)
    return action, destination


print(f"Fetching {len(file_actions)} pinned illumos sysroot files", file=sys.stderr)
with concurrent.futures.ThreadPoolExecutor(max_workers=16) as pool:
    futures = [pool.submit(fetch_blob, action) for action in file_actions]
    fetched = [future.result() for future in futures]

for action, blob in fetched:
    raw = gzip.decompress(blob.read_bytes())
    expected_file = content_hash(action, "file")
    if expected_file is not None and sha512t256(raw) != expected_file:
        raise SystemExit(f"file hash mismatch for {action['path']}")
    destination = action_path(action)
    mkdir_parent(destination)
    temporary = destination.with_name(f".{destination.name}.{os.getpid()}.tmp")
    temporary.write_bytes(raw)
    mode = action.get("mode")
    if mode:
        os.chmod(temporary, int(mode, 8))
    os.replace(temporary, destination)

for action in actions:
    if action["action"] == "dir":
        destination = action_path(action)
        destination.mkdir(parents=True, exist_ok=True)
        if action.get("mode"):
            os.chmod(destination, int(action["mode"], 8))

for action in actions:
    if action["action"] != "link":
        continue
    destination = action_path(action)
    mkdir_parent(destination)
    if destination.exists() or destination.is_symlink():
        destination.unlink()
    os.symlink(action["target"], destination)

for action in actions:
    if action["action"] != "hardlink":
        continue
    destination = action_path(action)
    target = sysroot.joinpath(*pathlib.PurePosixPath(action["target"]).parts)
    mkdir_parent(destination)
    if destination.exists() or destination.is_symlink():
        destination.unlink()
    os.link(target, destination)

required = [
    sysroot / "usr/include/stdio.h",
    sysroot / "usr/platform/armv8/include/sys/clock.h",
    sysroot / "usr/lib/crt1.o",
    sysroot / "usr/lib/crti.o",
    sysroot / "usr/lib/crtn.o",
    sysroot / "lib/ld.so.1",
    sysroot / "lib/libc.so.1",
    sysroot / "lib/libm.so.0",
    sysroot / "usr/lib/libgcc_s.so.1",
    sysroot / "usr/lib/libssp.so",
]
missing = [str(path.relative_to(sysroot)) for path in required if not path.exists()]
if missing:
    raise SystemExit("incomplete illumos AArch64 sysroot; missing: " + ", ".join(missing))
PY

  touch "$ROOT/.complete"
fi

STABLE_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
RUST_SYSROOT="$(rustup run "$STABLE_TOOLCHAIN" rustc --print sysroot)"
HOST="$(rustup run "$STABLE_TOOLCHAIN" rustc -vV | sed -n 's/^host: //p')"
LLD="$RUST_SYSROOT/lib/rustlib/$HOST/bin/gcc-ld/ld.lld"
[[ -x "$LLD" ]] || LLD="$RUST_SYSROOT/lib/rustlib/$HOST/bin/rust-lld"
[[ -x "$LLD" ]] || { echo "ERROR: Rust LLD missing for host $HOST" >&2; exit 1; }

CLANG="${AGENA_CLANG:-$(command -v clang || true)}"
CLANGXX="${AGENA_CLANGXX:-$(command -v clang++ || true)}"
AR="${AGENA_LLVM_AR:-$(command -v llvm-ar || true)}"
[[ -x "$CLANG" && -x "$CLANGXX" && -x "$AR" ]] || {
  echo "ERROR: clang, clang++, and llvm-ar are required for illumos cross builds" >&2
  exit 1
}

WRAPPER_ROOT="${RUNNER_TEMP:-/tmp}/agena-illumos-wrappers/$TARGET"
mkdir -p "$WRAPPER_ROOT"
LINKER_WRAPPER="$WRAPPER_ROOT/ld"
cat >"$LINKER_WRAPPER" <<EOF
#!/usr/bin/env bash
set -eo pipefail
args=("\$@")
filtered=()
for ((index = 0; index < \${#args[@]}; index++)); do
  arg="\${args[index]}"
  case "\$arg" in
    # Clang's Solaris driver emits this option for the native Solaris ld;
    # LLVM LLD does not implement it.
    -C) continue ;;
    # The pinned IPS runtime packages provide the real illumos crt1/crti/crtn
    # objects, but not GCC's crtbegin/crtend bookkeeping objects.  Clang's
    # compiler-rt/GCC startup pair is not part of the illumos sysroot ABI, so
    # do not ask LLD to resolve those absent Linux/GCC objects.
    crtbegin.o|crtend.o|*/crtbegin.o|*/crtend.o|-lgcc) continue ;;
    # These Solaris driver compatibility options are not needed by LLD.  Keep
    # other -z options intact if a future dependency passes one.
    -z)
      next="\${args[index + 1]:-}"
      if [[ "\$next" == ignore || "\$next" == record ]]; then
        index=\$((index + 1))
        continue
      fi
      ;;
  esac
  filtered+=("\$arg")
done
exec "$LLD" "\${filtered[@]}"
EOF
chmod +x "$LINKER_WRAPPER"
write_compiler_wrapper() {
  local path="$1" compiler="$2"
  cat >"$path" <<EOF
#!/usr/bin/env bash
set -eo pipefail
filtered=()
skip_next=false
for arg in "\$@"; do
  if [[ "\$skip_next" == true ]]; then skip_next=false; continue; fi
  case "\$arg" in
    --target=*) continue ;;
    --target|-target) skip_next=true; continue ;;
    *) filtered+=("\$arg") ;;
  esac
done
exec "$compiler" \
  --target=aarch64-unknown-solaris2.11 \
  --sysroot="$SYSROOT" \
  -isystem "$SYSROOT/usr/include" \
  -isystem "$SYSROOT/usr/xpg4/include" \
  -L"$SYSROOT/lib" \
  -L"$SYSROOT/usr/lib" \
  -L"$SYSROOT/usr/gcc/14/lib" \
  -fuse-ld="$LINKER_WRAPPER" \
  "\${filtered[@]}"
EOF
  chmod +x "$path"
}
write_compiler_wrapper "$WRAPPER_ROOT/cc" "$CLANG"
write_compiler_wrapper "$WRAPPER_ROOT/cxx" "$CLANGXX"
cat >"$WRAPPER_ROOT/ar" <<EOF
#!/usr/bin/env bash
exec "$AR" "\$@"
EOF
chmod +x "$WRAPPER_ROOT/ar"

# Link a real target-ABI C program before handing the toolchain to cc-rs and
# Cargo.  This catches a missing CRT, wrong linker ABI, or accidental host
# Linux link before a long build can report a misleading success.
PROBE="$ROOT/agena-illumos-probe.c"
PROBE_BIN="$ROOT/agena-illumos-probe"
cat >"$PROBE" <<'EOF'
#include <sys/socket.h>
#include <sys/wait.h>
#include <unistd.h>

int main(void) {
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd >= 0) close(fd);
    (void)waitpid(-1, 0, WNOHANG);
    return fork() == 0 ? 0 : 0;
}
EOF
"$WRAPPER_ROOT/cc" "$PROBE" -o "$PROBE_BIN"
if command -v readelf >/dev/null 2>&1; then
  readelf -h "$PROBE_BIN" | grep -Eq 'Class:[[:space:]]+ELF64' || {
    echo "ERROR: illumos probe is not ELF64" >&2
    exit 1
  }
  readelf -h "$PROBE_BIN" | grep -Eq 'Machine:[[:space:]]+AArch64' || {
    echo "ERROR: illumos probe is not AArch64" >&2
    exit 1
  }
fi

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$WRAPPER_ROOT/cc"
export "CXX_${key}=$WRAPPER_ROOT/cxx"
export "AR_${key}=$WRAPPER_ROOT/ar"
export "CARGO_TARGET_${key_upper}_LINKER=$WRAPPER_ROOT/cc"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAPPER_ROOT/cc"

exec "$@"
