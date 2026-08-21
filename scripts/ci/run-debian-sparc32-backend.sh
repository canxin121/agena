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

# The Rust target is 32-bit SPARC glibc. Debian's sparc64 cross compiler has a
# real -m32 multilib when paired with libc6-dev-sparc-sparc64-cross. Keep the
# compiler, binutils, and target sysroot in a private tree: installing Debian
# packages with Ubuntu apt mixes binutils-common versions and can silently
# select the host linker.
DEBIAN_MIRROR="https://deb.debian.org/debian"
DEBIAN_SUITE="bookworm"
DEBIAN_KEY_URL="https://ftp-master.debian.org/keys/archive-key-12.asc"
DEBIAN_KEY_SHA256="c2a9a16fde95e037bafd0fa6b7e31f41b4ff1e85851de5558f19a2a2f0e955e2"
ROOT="${RUNNER_TEMP:-/tmp}/agena-debian-sparc32"
DEB_DIR="$ROOT/debs"
TOOLCHAIN_ROOT="$ROOT/toolchain"
SYSROOT="$TOOLCHAIN_ROOT/usr/sparc64-linux-gnu"
GCC_LIB="$TOOLCHAIN_ROOT/usr/lib/gcc-cross/sparc64-linux-gnu/12"
KEY_FILE="$ROOT/debian-release-12.asc"
KEYRING="$ROOT/debian-release-12.gpg"
INRELEASE="$ROOT/InRelease"
GPGV_STATUS="$ROOT/InRelease.gpgv-status"
GPGV_ERRORS="$ROOT/InRelease.gpgv-errors"
PACKAGES_XZ="$ROOT/Packages.xz"
PACKAGES="$ROOT/Packages"
mkdir -p "$ROOT" "$DEB_DIR" "$TOOLCHAIN_ROOT"

download() {
  local url="$1"
  local destination="$2"
  local temporary="$destination.tmp"
  curl --fail --location --retry 12 --retry-all-errors --retry-delay 5 \
    --connect-timeout 30 --max-time 180 --output "$temporary" "$url"
  mv "$temporary" "$destination"
}

download_verified() {
  local url="$1"
  local expected="$2"
  local destination="$3"
  local temporary="$destination.tmp"
  if [[ -f "$destination" ]] && [[ "$(sha256sum "$destination" | awk '{print $1}')" == "$expected" ]]; then
    return 0
  fi
  curl --fail --location --retry 12 --retry-all-errors --retry-delay 5 \
    --connect-timeout 30 --max-time 180 --output "$temporary" "$url"
  local actual
  actual="$(sha256sum "$temporary" | awk '{print $1}')"
  [[ "$actual" == "$expected" ]] || {
    echo "ERROR: SHA256 mismatch for $url: expected $expected, got $actual" >&2
    exit 1
  }
  mv "$temporary" "$destination"
}

command -v curl >/dev/null 2>&1 || { echo "ERROR: curl is required" >&2; exit 1; }
command -v sha256sum >/dev/null 2>&1 || { echo "ERROR: sha256sum is required" >&2; exit 1; }
command -v xz >/dev/null 2>&1 || { echo "ERROR: xz is required" >&2; exit 1; }
command -v dpkg-deb >/dev/null 2>&1 || { echo "ERROR: dpkg-deb is required" >&2; exit 1; }
command -v gpg >/dev/null 2>&1 || { echo "ERROR: gpg is required" >&2; exit 1; }
command -v gpgv >/dev/null 2>&1 || { echo "ERROR: gpgv is required" >&2; exit 1; }

# Verify the Debian archive metadata before trusting any package record. The
# package hashes below are fixed bookworm records; the signed Packages.xz is
# also checked so a mirror cannot substitute a different filename or version.
download_verified "$DEBIAN_KEY_URL" "$DEBIAN_KEY_SHA256" "$KEY_FILE"
gpg --batch --yes --dearmor --output "$KEYRING" "$KEY_FILE"
download "$DEBIAN_MIRROR/dists/$DEBIAN_SUITE/InRelease" "$INRELEASE"
# Debian can publish an InRelease with more than one current signing key
# during key rotation.  Require at least one valid signature from the pinned
# official archive keyring, while permitting an additional rotation signature
# that is not present in this fixed key file.  A metadata file with no valid
# official signature is still a hard failure; this is not a blanket gpgv
# error suppression.
if ! gpgv --status-fd 1 --keyring "$KEYRING" "$INRELEASE" \
    >"$GPGV_STATUS" 2>"$GPGV_ERRORS"; then
  if ! grep -q '^\[GNUPG:\] VALIDSIG ' "$GPGV_STATUS"; then
    cat "$GPGV_ERRORS" >&2
    echo "ERROR: Debian InRelease has no valid signature from the pinned archive keyring" >&2
    exit 1
  fi
fi
grep -q '^\[GNUPG:\] VALIDSIG ' "$GPGV_STATUS" || {
  cat "$GPGV_ERRORS" >&2
  echo "ERROR: Debian InRelease has no valid signature from the pinned archive keyring" >&2
  exit 1
}

PACKAGES_SHA256="$(
  awk '
    /^SHA256:/ { in_sha256 = 1; next }
    /^SHA512:/ { in_sha256 = 0 }
    in_sha256 && $3 == "main/binary-amd64/Packages.xz" {
      print $1
      exit
    }
  ' "$INRELEASE"
)"
[[ "$PACKAGES_SHA256" =~ ^[[:xdigit:]]{64}$ ]] || {
  echo "ERROR: signed Debian InRelease does not contain main amd64 Packages.xz" >&2
  exit 1
}
download_verified \
  "$DEBIAN_MIRROR/dists/$DEBIAN_SUITE/main/binary-amd64/Packages.xz" \
  "$PACKAGES_SHA256" "$PACKAGES_XZ"
xz -dc "$PACKAGES_XZ" > "$PACKAGES"

# name|version|sha256|relative filename
package_specs=(
  'binutils-common|2.40-2|ab314134f43a0891a48f69a9bc33d825da748fa5e0ba2bebb7a5c491b026f1a0|pool/main/b/binutils/binutils-common_2.40-2_amd64.deb'
  'libbinutils|2.40-2|fcf55b99e5f8a78f3c8ce9b6957f1024f394cf20c196b100d308a57e43547710|pool/main/b/binutils/libbinutils_2.40-2_amd64.deb'
  'binutils-sparc64-linux-gnu|2.40-2|a63320069515822c4580e3646722a13cb5a3fb8da5ca91f4a6a8eedaf956d944|pool/main/b/binutils/binutils-sparc64-linux-gnu_2.40-2_amd64.deb'
  'gcc-12-base|12.2.0-14+deb12u1|1896a2aacf4ad681ff5eacc24a5b0ca4d5d9c9b9c9e4b6de5197bc1e116ea619|pool/main/g/gcc-12/gcc-12-base_12.2.0-14+deb12u1_amd64.deb'
  'libcc1-0|12.2.0-14+deb12u1|4ee6009633003f47de98333ea7f7d8835ec563ae2ac4f30251dbfee04d75d46b|pool/main/g/gcc-12/libcc1-0_12.2.0-14+deb12u1_amd64.deb'
  'gcc-12-cross-base-ports|12.2.0-13cross1|c264867591f21c2552a882f67926313461d3ec43e48252e1aa4800bf2eba50f6|pool/main/g/gcc-12-cross-ports/gcc-12-cross-base-ports_12.2.0-13cross1_all.deb'
  'gcc-12-sparc64-linux-gnu-base|12.2.0-13cross1|3957aa6cdea19dac345ebb22696a0409602eb43ad53e76987de90176475a75de|pool/main/g/gcc-12-cross-ports/gcc-12-sparc64-linux-gnu-base_12.2.0-13cross1_amd64.deb'
  'cpp-12-sparc64-linux-gnu|12.2.0-13cross1|8f04a5fee90d8c4387b88a9769ea6d91a63385988c6c56d9857244cb83fbc406|pool/main/g/gcc-12-cross-ports/cpp-12-sparc64-linux-gnu_12.2.0-13cross1_amd64.deb'
  'gcc-12-sparc64-linux-gnu|12.2.0-13cross1|98cf1c2734a5ec1f3a7f93f536da8f4de65ec294c966eabaf2606ec33997033b|pool/main/g/gcc-12-cross-ports/gcc-12-sparc64-linux-gnu_12.2.0-13cross1_amd64.deb'
  'g++-12-sparc64-linux-gnu|12.2.0-13cross1|e37823a90442c2d63219bd2c5a4639c91dc9d0bdcae6586cc18ecbc4ca9d5c0c|pool/main/g/gcc-12-cross-ports/g++-12-sparc64-linux-gnu_12.2.0-13cross1_amd64.deb'
  'libc6-sparc64-cross|2.36-8cross1|ea35b418b6d4e26d7181b209bc19f935b93a2ada2c7d9125e7100063e434c8cc|pool/main/c/cross-toolchain-base-ports/libc6-sparc64-cross_2.36-8cross1_all.deb'
  'libc6-dev-sparc64-cross|2.36-8cross1|d9f5bb157a3081e8cb47d5cfb603ea1910d9b30e4ffa05d1f932d07b2a7a3961|pool/main/c/cross-toolchain-base-ports/libc6-dev-sparc64-cross_2.36-8cross1_all.deb'
  'libc6-sparc-sparc64-cross|2.36-8cross1|b4dd5ebba6c196f1a79201ed3f3210b61b1edd41a5958b9b40c9ab67eea646b0|pool/main/c/cross-toolchain-base-ports/libc6-sparc-sparc64-cross_2.36-8cross1_all.deb'
  'libc6-dev-sparc-sparc64-cross|2.36-8cross1|35ac86fccdb4c5bbc1b4506e889ab5a8b3b097f5f19b7cd8ca37306b4fcb7f0f|pool/main/c/cross-toolchain-base-ports/libc6-dev-sparc-sparc64-cross_2.36-8cross1_all.deb'
  'linux-libc-dev-sparc64-cross|6.1.4-1cross1|326470872731690c8c842e97f922e7c80c4571c4619238ec15cd8fd1ffebfe2d|pool/main/c/cross-toolchain-base-ports/linux-libc-dev-sparc64-cross_6.1.4-1cross1_all.deb'
  'libgcc-s1-sparc64-cross|12.2.0-13cross1|1de4f5794feec8334d97b6cbac9621907432fbd7d656ecfd33af45485dea5061|pool/main/g/gcc-12-cross-ports/libgcc-s1-sparc64-cross_12.2.0-13cross1_all.deb'
  'libgomp1-sparc64-cross|12.2.0-13cross1|460e9c787b545dbfa003ab2a662ccc6cb8a3b00be63b55af6c5e0a542c69d8c4|pool/main/g/gcc-12-cross-ports/libgomp1-sparc64-cross_12.2.0-13cross1_all.deb'
  'libatomic1-sparc64-cross|12.2.0-13cross1|150997407f53b05882c066f7c9e4d38292fd70017dfae2d5e5640d8daf6c1b1c|pool/main/g/gcc-12-cross-ports/libatomic1-sparc64-cross_12.2.0-13cross1_all.deb'
  'libasan8-sparc64-cross|12.2.0-13cross1|6b49dcbf57810049d09e78467aa8382a34681d77ba382d70bc9d5e0ec5f8d7c5|pool/main/g/gcc-12-cross-ports/libasan8-sparc64-cross_12.2.0-13cross1_all.deb'
  'libubsan1-sparc64-cross|12.2.0-13cross1|a6c88550a21109b4eeb1ec450a7e79714a6abde88b983f94febfe888077ae5d6|pool/main/g/gcc-12-cross-ports/libubsan1-sparc64-cross_12.2.0-13cross1_all.deb'
  'libitm1-sparc64-cross|12.2.0-13cross1|1b51721189d807a9863c09a5f5f72f94bfcd4af5547516ab3bfa58199511677b|pool/main/g/gcc-12-cross-ports/libitm1-sparc64-cross_12.2.0-13cross1_all.deb'
  'libgcc-12-dev-sparc64-cross|12.2.0-13cross1|96e4830708f93c85c5a3590ec3d5aefe33169b5084959990ba9c1d8a3fd56f0e|pool/main/g/gcc-12-cross-ports/libgcc-12-dev-sparc64-cross_12.2.0-13cross1_all.deb'
  'libstdc++6-sparc64-cross|12.2.0-13cross1|68c6df9637215f0586f64d898b2b46e1fec3e3d7446f63de5729b72ba4596d4f|pool/main/g/gcc-12-cross-ports/libstdc++6-sparc64-cross_12.2.0-13cross1_all.deb'
  'libstdc++-12-dev-sparc64-cross|12.2.0-13cross1|be7524dfe743e4fd40244705017eee6c1892186c7984240a7677763ffcd8fd49|pool/main/g/gcc-12-cross-ports/libstdc++-12-dev-sparc64-cross_12.2.0-13cross1_all.deb'
)

for spec in "${package_specs[@]}"; do
  IFS='|' read -r package_name package_version package_sha package_filename <<<"$spec"
  metadata="$(awk -v package_name="$package_name" 'BEGIN { RS=""; FS="\n" } $1 == "Package: " package_name { print; exit }' "$PACKAGES")"
  [[ -n "$metadata" ]] || {
    echo "ERROR: signed Debian Packages index has no $package_name" >&2
    exit 1
  }
  actual_version="$(printf '%s\n' "$metadata" | awk '$1 == "Version:" { print $2; exit }')"
  actual_filename="$(printf '%s\n' "$metadata" | awk '$1 == "Filename:" { print $2; exit }')"
  actual_sha="$(printf '%s\n' "$metadata" | awk '$1 == "SHA256:" { print $2; exit }')"
  [[ "$actual_version" == "$package_version" && "$actual_filename" == "$package_filename" && "$actual_sha" == "$package_sha" ]] || {
    echo "ERROR: signed metadata changed for $package_name" >&2
    echo "expected: $package_version $package_filename $package_sha" >&2
    echo "actual:   $actual_version $actual_filename $actual_sha" >&2
    exit 1
  }
  deb_path="$DEB_DIR/${package_filename##*/}"
  download_verified "$DEBIAN_MIRROR/$package_filename" "$package_sha" "$deb_path"
  dpkg-deb --extract "$deb_path" "$TOOLCHAIN_ROOT"
done

GCC="$TOOLCHAIN_ROOT/usr/bin/sparc64-linux-gnu-gcc-12"
GXX="$TOOLCHAIN_ROOT/usr/bin/sparc64-linux-gnu-g++-12"
AR="$TOOLCHAIN_ROOT/usr/bin/sparc64-linux-gnu-ar"
RANLIB="$TOOLCHAIN_ROOT/usr/bin/sparc64-linux-gnu-ranlib"
READELF="$TOOLCHAIN_ROOT/usr/bin/sparc64-linux-gnu-readelf"
[[ -x "$GCC" && -x "$GXX" && -x "$AR" && -x "$RANLIB" ]] || {
  echo "ERROR: extracted Debian SPARC compiler tools are incomplete" >&2
  exit 1
}
[[ -x "$GCC_LIB/cc1" && -x "$GCC_LIB/cc1plus" ]] || {
  echo "ERROR: extracted Debian GCC is missing cc1/cc1plus" >&2
  exit 1
}
[[ -f "$SYSROOT/include/gnu/stubs-32.h" ]] || {
  echo "ERROR: extracted Debian SPARC multilib is missing gnu/stubs-32.h" >&2
  exit 1
}

# The compiler binaries run on the Ubuntu host, but all target headers,
# startup objects, libgcc, and binutils are resolved from the extracted tree.
# Host shared libraries remain host-runtime dependencies and cannot affect the
# SPARC ABI; the exact Debian libbinutils is preferred when present.
export PATH="$TOOLCHAIN_ROOT/usr/bin:$PATH"
if [[ -d "$TOOLCHAIN_ROOT/usr/lib/x86_64-linux-gnu" ]]; then
  export LD_LIBRARY_PATH="$TOOLCHAIN_ROOT/usr/lib/x86_64-linux-gnu${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
fi

WRAPPER_ROOT="${RUNNER_TEMP:-/tmp}/agena-debian-sparc32-wrappers/$TARGET"
mkdir -p "$WRAPPER_ROOT"
cat > "$WRAPPER_ROOT/cc" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$GCC" -m32 -no-canonical-prefixes --sysroot="$SYSROOT" \
  -B"$TOOLCHAIN_ROOT/usr/bin/" \
  -B"$GCC_LIB/" \
  -B"$SYSROOT/bin/" "\$@"
EOF
cat > "$WRAPPER_ROOT/cxx" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$GXX" -m32 -no-canonical-prefixes --sysroot="$SYSROOT" \
  -B"$TOOLCHAIN_ROOT/usr/bin/" \
  -B"$GCC_LIB/" \
  -B"$SYSROOT/bin/" "\$@"
EOF
cat > "$WRAPPER_ROOT/ar" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$AR" "\$@"
EOF
cat > "$WRAPPER_ROOT/ranlib" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$RANLIB" "\$@"
EOF
chmod +x "$WRAPPER_ROOT/cc" "$WRAPPER_ROOT/cxx" "$WRAPPER_ROOT/ar" "$WRAPPER_ROOT/ranlib"

CRT1="$($WRAPPER_ROOT/cc -print-file-name=crt1.o)"
[[ -f "$CRT1" && "$CRT1" == "$TOOLCHAIN_ROOT"/* ]] || {
  echo "ERROR: SPARC compiler did not resolve the extracted 32-bit crt1.o: $CRT1" >&2
  exit 1
}

# Link a real 32-bit SPARC glibc probe before Cargo starts. This proves the
# selected compiler/sysroot has the subprocess and socket ABI Agena requires,
# and prevents a Rust-only check from hiding a broken C/linker toolchain.
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
if [[ -x "$READELF" ]]; then
  "$READELF" -h "$PROBE_BIN" | grep -Eq 'Class:[[:space:]]+ELF32' || {
    echo "ERROR: Debian SPARC probe is not ELF32" >&2
    exit 1
  }
  "$READELF" -h "$PROBE_BIN" | grep -Eq 'Machine:[[:space:]]+SPARC' || {
    echo "ERROR: Debian SPARC probe is not SPARC" >&2
    exit 1
  }
else
  echo "ERROR: extracted Debian SPARC readelf is missing" >&2
  exit 1
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
