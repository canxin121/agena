#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?PowerPC SPE target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *)
    echo "ERROR: PowerPC SPE builders require a Linux x86_64 host; got $(uname -s)-$(uname -m)" >&2
    exit 2
    ;;
esac

case "$TARGET" in
  powerpc-unknown-linux-gnuspe)
    MODE=gnu
    ;;
  powerpc-unknown-linux-muslspe)
    MODE=musl
    ;;
  *)
    echo "ERROR: unsupported PowerPC SPE target: $TARGET" >&2
    exit 2
    ;;
esac

ROOT="${RUNNER_TEMP:-/tmp}/agena-powerpc-spe/$TARGET"
mkdir -p "$ROOT"

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"

download_verified() {
  local relative_url="$1"
  local expected_sha256="$2"
  local destination="$3"
  local actual
  mkdir -p "$(dirname "$destination")"
  if [[ -f "$destination" ]]; then
    actual="$(sha256sum "$destination" | awk '{print $1}')"
    if [[ "$actual" == "$expected_sha256" ]]; then
      return
    fi
    rm -f "$destination"
  fi
  local temporary="${destination}.tmp.$$"
  curl --fail --location \
    --retry 12 --retry-all-errors --retry-delay 5 \
    --connect-timeout 30 --max-time 1800 \
    --user-agent 'agena-powerpc-spe-builder' \
    "${relative_url}" -o "$temporary"
  actual="$(sha256sum "$temporary" | awk '{print $1}')"
  if [[ "$actual" != "$expected_sha256" ]]; then
    rm -f "$temporary"
    echo "ERROR: SHA256 mismatch for $relative_url: expected $expected_sha256, got $actual" >&2
    exit 1
  fi
  mv "$temporary" "$destination"
}

configure_environment() {
  local compiler="$1"
  local cxx_compiler="$2"
  local ar="$3"
  local ranlib="$4"
  local sysroot="$5"
  local compiler_prefix="$6"
  local host_library_root="${7:-}"
  local wrapper_root="$ROOT/wrappers"
  local cc_wrapper="$wrapper_root/cc"
  local cxx_wrapper="$wrapper_root/cxx"

  mkdir -p "$wrapper_root"
  cat > "$cc_wrapper" <<EOF
#!/usr/bin/env bash
set -euo pipefail
export GCC_EXEC_PREFIX=$(printf '%q' "$compiler_prefix")
export COMPILER_PATH=$(printf '%q' "$compiler_prefix")
exec $(printf '%q' "$compiler") --sysroot=$(printf '%q' "$sysroot") "\$@"
EOF
  cat > "$cxx_wrapper" <<EOF
#!/usr/bin/env bash
set -euo pipefail
export GCC_EXEC_PREFIX=$(printf '%q' "$compiler_prefix")
export COMPILER_PATH=$(printf '%q' "$compiler_prefix")
exec $(printf '%q' "$cxx_compiler") --sysroot=$(printf '%q' "$sysroot") "\$@"
EOF
  chmod +x "$cc_wrapper" "$cxx_wrapper"

  if [[ -n "$host_library_root" ]]; then
    local host_library_path
    host_library_path="$host_library_root/usr/lib/x86_64-linux-gnu:$host_library_root/lib/x86_64-linux-gnu:$host_library_root/usr/lib/gcc/x86_64-linux-gnu/8"
    export LD_LIBRARY_PATH="${host_library_path}${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  fi

  export "CC_${key}=$cc_wrapper"
  export "CXX_${key}=$cxx_wrapper"
  export "AR_${key}=$ar"
  export "RANLIB_${key}=$ranlib"
  export "CARGO_TARGET_${key_upper}_LINKER=$cc_wrapper"
  export "CFLAGS_${key}=--sysroot=$sysroot"
  export "CXXFLAGS_${key}=--sysroot=$sysroot"
  export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$cc_wrapper"
}

if [[ "$MODE" == gnu ]]; then
  command -v dpkg-deb >/dev/null 2>&1 || {
    echo "ERROR: dpkg-deb is required to extract the pinned Debian SPE sysroot" >&2
    exit 1
  }

  DEB_ROOT="$ROOT/debs"
  SYSROOT="$ROOT/sysroot"
  HOST_ROOT="$ROOT/host-runtime"
  BASE_URL='https://archive.debian.org/debian'

  download_deb() {
    local relative_path="$1"
    local expected_sha256="$2"
    download_verified "$BASE_URL/$relative_path" "$expected_sha256" "$DEB_ROOT/$(basename "$relative_path")"
  }

  # Debian buster's ports repository is the last public archive containing the
  # actual SPE ABI compiler and glibc sysroot. Keep every package pinned: a
  # normal PowerPC compiler or an e500 ABI sysroot is not interchangeable.
  download_deb 'pool/main/b/binutils/binutils-common_2.31.1-16_amd64.deb' \
    '95c39f813e7c99f93d7de3bab571f4c2f52c917a64d4dc33be8ef7c4dad14adb'
  download_deb 'pool/main/b/binutils/binutils-powerpc-linux-gnuspe_2.31.1-16_amd64.deb' \
    '51294e5ff6de71046de75dacd82e64bcb566e413ffb37d7a1cf6d32eea3d610e'
  download_deb 'pool/main/g/gcc-8-cross-ports/gcc-8-cross-base-ports_8.3.0-2cross2_all.deb' \
    'cb8626c014a03cfc4e969c420df187e0e30697ba2cc71ee39974f366c95612e5'
  download_deb 'pool/main/g/gcc-8-cross-ports/gcc-8-powerpc-linux-gnuspe-base_8.3.0-2cross2_amd64.deb' \
    '2b78090e18f4ec75a18fc48034321ea5b418915369dd66840e8ad6b3dda420c4'
  download_deb 'pool/main/g/gcc-8-cross-ports/cpp-8-powerpc-linux-gnuspe_8.3.0-2cross2_amd64.deb' \
    '28df9b0b0215fe93045830862bfc1141118783195d69f81ed65a8ce27885f1d9'
  download_deb 'pool/main/g/gcc-8-cross-ports/gcc-8-powerpc-linux-gnuspe_8.3.0-2cross2_amd64.deb' \
    'd6e46a57be7ede2f7cc0468221eb8ddf22d882f9b9c2cb2798894849e405b1ff'
  download_deb 'pool/main/g/gcc-8-cross-ports/g++-8-powerpc-linux-gnuspe_8.3.0-2cross2_amd64.deb' \
    '4ff71d4bc77a0b5a1c8ba8532bcaa60186d5825dea75de968ad205658456169b'
  download_deb 'pool/main/g/gcc-8-cross-ports/libgcc-8-dev-powerpcspe-cross_8.3.0-2cross2_all.deb' \
    '9779721b21b21171182d5c12a42564b8147b81e29d6f5dca1f15d81e6b9dba66'
  download_deb 'pool/main/g/gcc-8-cross-ports/libgcc1-powerpcspe-cross_8.3.0-2cross2_all.deb' \
    'dcc269eae3a5859f692dc50683a4ee1c736ba699f1e6f82f26bf47224b00e37c'
  download_deb 'pool/main/g/gcc-8-cross-ports/libgomp1-powerpcspe-cross_8.3.0-2cross2_all.deb' \
    'bb4a937db97b8df1400fc2730ed65d50eb28decdcaa05b35025345e04d3ccec1'
  download_deb 'pool/main/g/gcc-8-cross-ports/libatomic1-powerpcspe-cross_8.3.0-2cross2_all.deb' \
    'e6c4d8011ad5e263a872fbda0267445027282a45e018779f6aebb44fbf2bd637'
  download_deb 'pool/main/g/gcc-8-cross-ports/libstdc++6-powerpcspe-cross_8.3.0-2cross2_all.deb' \
    '32e3c899fcb915babbdc67c6f2553a6cb6016ff88280ed64c75647cf052b0ebf'
  download_deb 'pool/main/g/gcc-8-cross-ports/libstdc++-8-dev-powerpcspe-cross_8.3.0-2cross2_all.deb' \
    'fd8d85e64197ab87d6945032f851341df3787b12bbda5a180af0e9e42818f1dc'
  download_deb 'pool/main/c/cross-toolchain-base-ports/libc6-dev-powerpcspe-cross_2.28-7cross1_all.deb' \
    '153caaf1f58bfc580e20c75912fdf776c8d99f97bce0f9e6e32e489d028b5912'
  download_deb 'pool/main/c/cross-toolchain-base-ports/libc6-powerpcspe-cross_2.28-7cross1_all.deb' \
    'dafdaa79f9b5265f57dcd9827b6d973bb8ab72a49c035a4bab6d6f975cdbb17c'
  download_deb 'pool/main/c/cross-toolchain-base-ports/linux-libc-dev-powerpcspe-cross_4.19.20-1cross1_all.deb' \
    '1bc753bd083120e172414f0b501c5063f0f5f60bd73e893591d8452ba399808a'

  # These host packages keep GCC 8's libisl/libgmp/libmpfr/libmpc and C++
  # runtime isolated from whatever newer Ubuntu image happens to be current.
  download_deb 'pool/main/g/gcc-8/gcc-8-base_8.3.0-6_amd64.deb' \
    '1b00f7cef567645a7e695caf6c1ad395577e7d2e903820097ebd3496ddcfcc84'
  download_deb 'pool/main/g/gcc-8/libcc1-0_8.3.0-6_amd64.deb' \
    '579c11dd6004f06ac2639b338c320fde794ed3c36a1d2be559ec282ea3042dd7'
  download_deb 'pool/main/g/gcc-8/libgcc1_8.3.0-6_amd64.deb' \
    'b1bb7611f3372732889d502cb1d09fe572b5fbb5288a4a8b1ed0363fecc3555a'
  download_deb 'pool/main/g/gcc-8/libstdc++6_8.3.0-6_amd64.deb' \
    '5cc70625329655ff9382580971d4616db8aa39af958b7c995ee84598f142a4ee'
  download_deb 'pool/main/g/gmp/libgmp10_6.1.2+dfsg-4+deb10u1_amd64.deb' \
    '91f8037c4ffaf7937a957a33de939a04ad42d088c07b383c19051dfd8476036b'
  download_deb 'pool/main/i/isl/libisl19_0.20-2_amd64.deb' \
    'd51e27d3fcba9bd0fe5f3303b61d08ebbd1a3bc57c40d467338b34f5d4ee762f'
  download_deb 'pool/main/m/mpclib3/libmpc3_1.1.0-1_amd64.deb' \
    'a73b05c10399636a7c7bff266205de05631dc4af502bfb441cbbc6af0a7deb2a'
  download_deb 'pool/main/m/mpfr4/libmpfr6_4.0.2-1_amd64.deb' \
    'd005438229811b09ea9783491c98b145c9bcf6489284ad7870c19d2d09a8f571'
  download_deb 'pool/main/z/zlib/zlib1g_1.2.11.dfsg-1+deb10u1_amd64.deb' \
    'a14bcffc39528f422625715cb8c05a921b283bc4d37e27c6db8d77106cd7d8a9'

  if [[ ! -x "$SYSROOT/usr/bin/powerpc-linux-gnuspe-gcc-8" ]]; then
    rm -rf "$SYSROOT" "$HOST_ROOT"
    mkdir -p "$SYSROOT" "$HOST_ROOT"
    for deb_file in "$DEB_ROOT"/*.deb; do
      case "$(basename "$deb_file")" in
        # binutils-common carries some host-side libbfd/libopcodes shared
        # libraries. Keep those with the host runtime. The target binutils
        # package also carries its versioned SPE libbfd/libopcodes libraries;
        # extract it into both roots so the exact SPE tools remain in the
        # target sysroot while their host loader dependencies are available
        # through LD_LIBRARY_PATH. Do not substitute a non-SPE binutils.
        binutils-common_*)
          dpkg-deb -x "$deb_file" "$HOST_ROOT"
          ;;
        binutils-powerpc-linux-gnuspe_*)
          dpkg-deb -x "$deb_file" "$SYSROOT"
          dpkg-deb -x "$deb_file" "$HOST_ROOT"
          ;;
        *powerpcspe*|*powerpc-linux-gnuspe*|gcc-8-cross-base-ports_*)
          dpkg-deb -x "$deb_file" "$SYSROOT"
          ;;
        *)
          dpkg-deb -x "$deb_file" "$HOST_ROOT"
          ;;
      esac
    done
  fi

  GCC="$SYSROOT/usr/bin/powerpc-linux-gnuspe-gcc-8"
  GXX="$SYSROOT/usr/bin/powerpc-linux-gnuspe-g++-8"
  AR="$SYSROOT/usr/bin/powerpc-linux-gnuspe-ar"
  RANLIB="$SYSROOT/usr/bin/powerpc-linux-gnuspe-ranlib"
  COMPILER_PREFIX="$SYSROOT/usr/lib/gcc-cross/"
  [[ -x "$GCC" && -x "$GXX" && -x "$AR" && -x "$RANLIB" ]] || {
    echo "ERROR: extracted Debian PowerPC SPE compiler is incomplete" >&2
    exit 1
  }
  configure_environment "$GCC" "$GXX" "$AR" "$RANLIB" "$SYSROOT" "$COMPILER_PREFIX" "$HOST_ROOT"
else
  MCM_COMMIT='227df8b99103f9c59f6570babf892978e293082f'
  MCM_SHA256='bb3fc7851088e1e5e1274ee56a0ab6ae176043d160fdf0b71027934b091f208a'
  MCM_ARCHIVE="$ROOT/musl-cross-make.tar.gz"
  MCM_DIR="$ROOT/musl-cross-make-$MCM_COMMIT"
  OUTPUT="$ROOT/output"
  PREFIX='powerpc-linux-muslspe'

  download_verified \
    "https://codeload.github.com/richfelker/musl-cross-make/tar.gz/$MCM_COMMIT" \
    "$MCM_SHA256" "$MCM_ARCHIVE"

  if [[ ! -x "$OUTPUT/bin/$PREFIX-gcc" ]]; then
    if [[ ! -d "$MCM_DIR" ]]; then
      tar -xzf "$MCM_ARCHIVE" -C "$ROOT"
    fi
    [[ -d "$MCM_DIR" ]] || {
      echo "ERROR: musl-cross-make source directory was not extracted" >&2
      exit 1
    }
    cat > "$MCM_DIR/config.mak" <<EOF
TARGET = $PREFIX
OUTPUT = $OUTPUT
BINUTILS_VER = 2.32
GCC_VER = 8.5.0
MUSL_VER = 1.2.5
GMP_VER = 6.1.2
MPC_VER = 1.1.0
MPFR_VER = 4.0.2
LINUX_VER = headers-4.19.88-2
DL_CMD = curl --fail --location --retry 12 --retry-all-errors --retry-delay 5 --connect-timeout 30 --max-time 1800 -o
SHA1_CMD = sha1sum -c
COMMON_CONFIG += --disable-nls
# GCC 8 still contains the exact powerpcspe backend, but marks this target
# obsolete unless the build explicitly opts in.  Keep the SPE ABI rather than
# substituting a normal PowerPC/e500 compiler.
GCC_CONFIG += --disable-libquadmath --disable-decimal-float --disable-libitm --enable-obsolete
EOF
    make -C "$MCM_DIR" -j"$(nproc)" install
  fi

  GCC="$OUTPUT/bin/$PREFIX-gcc"
  GXX="$OUTPUT/bin/$PREFIX-g++"
  AR="$OUTPUT/bin/$PREFIX-ar"
  RANLIB="$OUTPUT/bin/$PREFIX-ranlib"
  [[ -x "$GCC" && -x "$GXX" && -x "$AR" && -x "$RANLIB" ]] || {
    echo "ERROR: musl-cross-make did not produce a complete PowerPC SPE toolchain" >&2
    exit 1
  }
  configure_environment "$GCC" "$GXX" "$AR" "$RANLIB" "$OUTPUT/$PREFIX" "$OUTPUT/lib/gcc/"
fi

exec "$@"
