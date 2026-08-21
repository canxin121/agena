#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?SPARC target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }
[[ "$TARGET" == sparc-unknown-linux-gnu ]] || {
  echo "ERROR: unsupported Debian SPARC target: $TARGET" >&2
  exit 2
}

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: Debian SPARC cross packages require a Linux x86_64 host" >&2; exit 2 ;;
esac

# Debian's official sparc64 cross compiler has a real 32-bit multilib.  The
# multilib package depends on libc6-dev-sparc-sparc64-cross, which supplies the
# target's glibc 32-bit headers, libraries, and gnu/stubs-32.h.  This is the
# target ABI named by sparc-unknown-linux-gnu; it is not a sparc64 compiler
# pointed at an incomplete sysroot and it is not a uClibc substitution.
DEBIAN_KEY_SHA256=521e9f6a9f9b92ee8d5ce74345e8cfd04028dae9db6f571259d584b293549824
DEBIAN_KEY_URL=https://ftp-master.debian.org/keys/release-12.asc
ROOT="${RUNNER_TEMP:-/tmp}/agena-debian-sparc32"
KEY_FILE="$ROOT/debian-release-12.asc"
SOURCE_FILE="$ROOT/bookworm.list"
mkdir -p "$ROOT"

download_verified() {
  local url="$1"
  local expected="$2"
  local destination="$3"
  local temporary="$destination.tmp"
  if [[ -f "$destination" ]] && [[ "$(sha256sum "$destination" | awk '{print $1}')" == "$expected" ]]; then
    return 0
  fi
  rm -f "$temporary"
  curl --fail --location --retry 12 --retry-all-errors --retry-delay 5 \
    --connect-timeout 30 --max-time 120 --output "$temporary" "$url"
  local actual
  actual="$(sha256sum "$temporary" | awk '{print $1}')"
  [[ "$actual" == "$expected" ]] || {
    rm -f "$temporary"
    echo "ERROR: Debian archive key SHA256 mismatch: expected $expected, got $actual" >&2
    exit 1
  }
  mv "$temporary" "$destination"
}

if ! command -v sparc64-linux-gnu-gcc-12 >/dev/null 2>&1 \
  || [[ ! -f /usr/sparc64-linux-gnu/include/gnu/stubs-32.h ]]; then
  command -v sudo >/dev/null 2>&1 || {
    echo "ERROR: sudo is required to install the official Debian SPARC multilib packages" >&2
    exit 1
  }
  download_verified "$DEBIAN_KEY_URL" "$DEBIAN_KEY_SHA256" "$KEY_FILE"
  sudo install -d -m 0755 /usr/share/keyrings
  sudo install -m 0644 "$KEY_FILE" /usr/share/keyrings/agena-debian-release-12.asc
  printf '%s\n' \
    'deb [arch=amd64 signed-by=/usr/share/keyrings/agena-debian-release-12.asc] https://deb.debian.org/debian bookworm main' \
    > "$SOURCE_FILE"
  trap 'sudo rm -f /usr/share/keyrings/agena-debian-release-12.asc; rm -f "$SOURCE_FILE"' EXIT

  apt_args=(
    -o "Dir::Etc::sourcelist=$SOURCE_FILE"
    -o Dir::Etc::sourceparts=-
    -o APT::Get::List-Cleanup=0
    -o Acquire::Retries=8
  )
  sudo apt-get "${apt_args[@]}" update
  DEBIAN_FRONTEND=noninteractive sudo apt-get "${apt_args[@]}" install -y --no-install-recommends \
    gcc-12-multilib-sparc64-linux-gnu \
    g++-12-multilib-sparc64-linux-gnu
fi

CC="$(command -v sparc64-linux-gnu-gcc-12 || true)"
CXX="$(command -v sparc64-linux-gnu-g++-12 || true)"
AR="$(command -v sparc64-linux-gnu-ar || true)"
RANLIB="$(command -v sparc64-linux-gnu-ranlib || true)"
[[ -x "$CC" && -x "$CXX" && -x "$AR" && -x "$RANLIB" ]] || {
  echo "ERROR: official Debian SPARC cross compiler tools are incomplete" >&2
  exit 1
}
[[ -f /usr/sparc64-linux-gnu/include/gnu/stubs-32.h ]] || {
  echo "ERROR: Debian SPARC multilib did not provide glibc gnu/stubs-32.h" >&2
  exit 1
}

WRAPPER_ROOT="${RUNNER_TEMP:-/tmp}/agena-debian-sparc32-wrappers/$TARGET"
mkdir -p "$WRAPPER_ROOT"
cat > "$WRAPPER_ROOT/cc" <<EOF
#!/usr/bin/env bash
exec "$CC" -m32 "\$@"
EOF
cat > "$WRAPPER_ROOT/cxx" <<EOF
#!/usr/bin/env bash
exec "$CXX" -m32 "\$@"
EOF
cat > "$WRAPPER_ROOT/ar" <<EOF
#!/usr/bin/env bash
exec "$AR" "\$@"
EOF
cat > "$WRAPPER_ROOT/ranlib" <<EOF
#!/usr/bin/env bash
exec "$RANLIB" "\$@"
EOF
chmod +x "$WRAPPER_ROOT/cc" "$WRAPPER_ROOT/cxx" "$WRAPPER_ROOT/ar" "$WRAPPER_ROOT/ranlib"

# Link a real 32-bit SPARC glibc probe before Cargo starts.  This prevents a
# later C dependency failure from being mistaken for a successful Rust-only
# target check and proves that the selected toolchain is not using sparc64
# objects or a host compiler.
PROBE="$ROOT/agena-sparc32-probe.c"
PROBE_BIN="$ROOT/agena-sparc32-probe"
cat > "$PROBE" <<'EOF'
#include <stdio.h>
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
  readelf -h "$PROBE_BIN" | grep -Eq 'Class:[[:space:]]+ELF32' || {
    echo "ERROR: Debian SPARC probe is not ELF32" >&2
    exit 1
  }
  readelf -h "$PROBE_BIN" | grep -Eq 'Machine:[[:space:]]+SPARC' || {
    echo "ERROR: Debian SPARC probe is not SPARC" >&2
    exit 1
  }
fi

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$WRAPPER_ROOT/cc"
export "CXX_${key}=$WRAPPER_ROOT/cxx"
export "AR_${key}=$WRAPPER_ROOT/ar"
export "RANLIB_${key}=$WRAPPER_ROOT/ranlib"
export "CFLAGS_${key}=-m32"
export "CXXFLAGS_${key}=-m32"
export "CARGO_TARGET_${key_upper}_LINKER=$WRAPPER_ROOT/cc"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$WRAPPER_ROOT/cc"

exec "$@"
