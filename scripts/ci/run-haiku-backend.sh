#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?Haiku target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: Haiku cross-tools builder requires Linux x86_64" >&2; exit 2 ;;
esac

case "$TARGET" in
  i686-unknown-haiku)
    HAIKU_ARCH=x86
    GNU_TARGET=i586-pc-haiku
    CFLAGS_EXTRA=-m32
    ;;
  x86_64-unknown-haiku)
    HAIKU_ARCH=x86_64
    GNU_TARGET=x86_64-unknown-haiku
    CFLAGS_EXTRA=-m64
    ;;
  *) echo "ERROR: unsupported Haiku target: $TARGET" >&2; exit 2 ;;
esac

# Pin both official Haiku GitHub mirrors so cross-tools/sysroot generation is
# reproducible rather than following moving master branches.
HAIKU_COMMIT=dfaff659fa944da59db4014f50cde2daea9415bd
BUILDTOOLS_COMMIT=8375c2dbeaf109c520798cb234d57f0895463201
# Haiku's bootstrap build is a three-repository build.  Pin the repositories
# independently so a moving HaikuPorts master cannot silently change the
# generated compiler/sysroot for a release target.  The cross repository pin
# contains the GCC 13.3 bootstrap recipe named by the pinned Haiku source.
HAIKUPORTER_COMMIT=690d2215daffb4ff260b45be16192af94a98e034
# This revision matches Haiku's pinned bootstrap repository definitions:
# bash_bootstrap 5.3 is enabled for x86/x86_64 and Python remains 3.10.19.
HAIKUPORTS_CROSS_COMMIT=195374f9922eb6253783fd57ca4b8ea8ea03f13b
HAIKUPORTS_COMMIT=ad4f7e86f917445bdc12ee9cb0003e9e6780700b
PATCH_FILE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/third_party/haiku-bootstrap-smbios.patch"
ROOT="${RUNNER_TEMP:-/tmp}/agena-haiku/$HAIKU_ARCH"
HAIKU="$ROOT/haiku"
BUILDTOOLS="$ROOT/buildtools"
HAIKUPORTER="$ROOT/haikuporter"
HAIKUPORTS_CROSS="$ROOT/haikuports.cross"
HAIKUPORTS="$ROOT/haikuports"
OUTPUT="$ROOT/generated"
PACKAGE_ROOT="$ROOT/system"
SYSROOT="$OUTPUT/cross-tools-$HAIKU_ARCH/sysroot"
TOOLBIN="$OUTPUT/cross-tools-$HAIKU_ARCH/bin"
BOOTSTRAP_STAMP="$OUTPUT/agena-bootstrap-versions"
mkdir -p "$ROOT"

valid_toolchain() {
  [[ -f "$BOOTSTRAP_STAMP" ]] \
    && grep -Fxq "haiku=$HAIKU_COMMIT" "$BOOTSTRAP_STAMP" \
    && grep -Fxq "buildtools=$BUILDTOOLS_COMMIT" "$BOOTSTRAP_STAMP" \
    && grep -Fxq "haikuporter=$HAIKUPORTER_COMMIT" "$BOOTSTRAP_STAMP" \
    && grep -Fxq "haikuports.cross=$HAIKUPORTS_CROSS_COMMIT" "$BOOTSTRAP_STAMP" \
    && grep -Fxq "haikuports=$HAIKUPORTS_COMMIT" "$BOOTSTRAP_STAMP" \
    && [[ -x "$TOOLBIN/$GNU_TARGET-gcc" ]] \
    && [[ -f "$PACKAGE_ROOT/develop/headers/posix/stdio.h" || -f "$PACKAGE_ROOT/develop/headers/stdio.h" ]] \
    && [[ -d "$PACKAGE_ROOT/develop/headers/c++" ]] \
    && [[ -d "$PACKAGE_ROOT/develop/headers/gcc" ]] \
    && [[ -f "$PACKAGE_ROOT/develop/lib/libsupc++-kernel.a" ]] \
    && [[ -f "$PACKAGE_ROOT/develop/lib/libgcc-kernel.a" ]] \
    && find "$PACKAGE_ROOT" -type f -name 'libroot.so' -print -quit | grep -q .
}

if ! valid_toolchain; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "ERROR: sudo is required to install Haiku cross-tool build prerequisites" >&2
    exit 1
  fi
  sudo apt-get update -y
  sudo apt-get install -y --no-install-recommends \
    autoconf automake autopoint bison bzip2 ca-certificates cmake curl file flex g++ gawk git \
    libncurses-dev libtool-bin make nasm pkg-config python3 texinfo wget xz-utils zlib1g-dev

  rm -rf "$HAIKU" "$BUILDTOOLS" "$HAIKUPORTER" "$HAIKUPORTS_CROSS" \
    "$HAIKUPORTS" "$OUTPUT" "$PACKAGE_ROOT" "$ROOT/bin"
  mkdir -p "$ROOT/bin" "$PACKAGE_ROOT"

  checkout_repo() {
    local destination="$1"
    local repository="$2"
    local commit="$3"
    git init -q "$destination"
    git -C "$destination" remote add origin "$repository"
    git -C "$destination" fetch -q --depth 1 origin "$commit"
    git -C "$destination" checkout -q --detach FETCH_HEAD
  }

  checkout_repo "$HAIKU" https://github.com/haiku/haiku.git "$HAIKU_COMMIT"
  git -C "$HAIKU" apply --check "$PATCH_FILE"
  git -C "$HAIKU" apply "$PATCH_FILE"

  checkout_repo "$BUILDTOOLS" https://github.com/haiku/buildtools.git "$BUILDTOOLS_COMMIT"
  checkout_repo "$HAIKUPORTER" https://github.com/haikuports/haikuporter.git "$HAIKUPORTER_COMMIT"
  checkout_repo "$HAIKUPORTS_CROSS" https://github.com/haikuports/haikuports.cross.git "$HAIKUPORTS_CROSS_COMMIT"
  checkout_repo "$HAIKUPORTS" https://github.com/haikuports/haikuports.git "$HAIKUPORTS_COMMIT"
  [[ -x "$HAIKUPORTER/haikuporter" ]] || {
    echo "ERROR: pinned HaikuPorter checkout does not contain an executable haikuporter" >&2
    exit 1
  }

  (
    cd "$BUILDTOOLS/jam"
    make -j2
    ./jam0 -sBINDIR="$ROOT/bin" install
  )
  export PATH="$ROOT/bin:$PATH"

  mkdir -p "$OUTPUT" "$SYSROOT/boot"
  ln -sfn "$PACKAGE_ROOT" "$SYSROOT/boot/system"
  (
    cd "$OUTPUT"
    "$HAIKU/configure" \
      --cross-tools-source "$BUILDTOOLS" \
      --build-cross-tools "$HAIKU_ARCH" \
      --bootstrap "$HAIKUPORTER/haikuporter" "$HAIKUPORTS_CROSS" "$HAIKUPORTS" \
      --no-downloads
    # The source checkout is the complete, fixed Haiku source revision.  Build
    # the two packages needed for the sysroot entirely from that checkout so
    # the build does not depend on the moving HaikuPorts repository metadata.
    # This still produces Haiku's real runtime/development packages; it does
    # not replace missing files with a synthetic sysroot.
    # The fixed source commit is intentionally fetched shallowly and has no
    # hrev tags. Seed the deterministic metadata that Haiku's revision helper
    # consumes so it does not fall back to git-describe. The last-built value
    # matches the checked-out commit, so this exact source revision is kept.
    mkdir -p "$OUTPUT/build"
    printf '%s\n' "$HAIKU_COMMIT" > "$OUTPUT/build/haiku-revision"
    printf '%s\n' "$HAIKU_COMMIT" > "$OUTPUT/build/last-built-revision"
    # Haiku's bootstrap profile deliberately adds -nostdinc while the
    # cross-compiler is being used to build the real headers. The GCC
    # intrinsic headers are still required by those headers (for example
    # limits.h includes float.h), so pass the directories generated by this
    # exact compiler through Jam. These are genuine compiler headers, not
    # synthetic replacements for the Haiku sysroot.
    GCC_INCLUDE_DIR="$("$TOOLBIN/$GNU_TARGET-gcc" -print-file-name=include)"
    GCC_FIXED_INCLUDE_DIR="$("$TOOLBIN/$GNU_TARGET-gcc" -print-file-name=include-fixed)"
    [[ -f "$GCC_INCLUDE_DIR/float.h" ]] || {
      echo "ERROR: Haiku cross GCC intrinsic headers are missing: $GCC_INCLUDE_DIR" >&2
      exit 1
    }
    [[ -d "$GCC_FIXED_INCLUDE_DIR" ]] || {
      echo "ERROR: Haiku cross GCC fixed headers are missing: $GCC_FIXED_INCLUDE_DIR" >&2
      exit 1
    }
    GCC_HEADER_FLAGS="-isystem$GCC_INCLUDE_DIR -isystem$GCC_FIXED_INCLUDE_DIR"
    # The bootstrap profile also adds -nostdinc to C++ compilations.  The
    # compiler's intrinsic headers above are not enough for libstdc++: headers
    # such as <new> and <bits/c++config.h> live in the target compiler's C++
    # include directories and are deliberately not part of the Haiku sysroot.
    # Ask this exact cross g++ for its search list instead of guessing a GCC
    # version or an ABI directory.  Every directory passed below therefore
    # belongs to the pinned Haiku cross toolchain, never to the Ubuntu host.
    CXX_SEARCH_OUTPUT="$(printf '%s\n' '' | "$TOOLBIN/$GNU_TARGET-g++" -E -v -x c++ - 2>&1)"
    CXX_HEADER_FLAGS="$GCC_HEADER_FLAGS"
    CXX_TOOL_ROOT="$OUTPUT/cross-tools-$HAIKU_ARCH"
    CXX_HEADER_COUNT=0
    CXX_BASE_HEADER_DIR=""
    CXX_CONFIG_FOUND=false
    while IFS= read -r CXX_INCLUDE_DIR; do
      [[ -d "$CXX_INCLUDE_DIR" ]] || continue
      case "$CXX_INCLUDE_DIR" in
        "$CXX_TOOL_ROOT"/*/include/c++/*)
          CXX_HEADER_FLAGS+=" -isystem$CXX_INCLUDE_DIR"
          CXX_HEADER_COUNT=$((CXX_HEADER_COUNT + 1))
          if [[ -f "$CXX_INCLUDE_DIR/new" ]]; then
            CXX_BASE_HEADER_DIR="$CXX_INCLUDE_DIR"
          fi
          if [[ -f "$CXX_INCLUDE_DIR/bits/c++config.h" ]]; then
            CXX_CONFIG_FOUND=true
          fi
          ;;
      esac
    done < <(
      printf '%s\n' "$CXX_SEARCH_OUTPUT" |
        awk '
          /#include <\.\.\.> search starts here:/ { in_search = 1; next }
          /^End of search list\./ { in_search = 0 }
          in_search {
            sub(/^[[:space:]]+/, "")
            print
          }
        '
    )
    [[ "$CXX_HEADER_COUNT" -gt 0 && -n "$CXX_BASE_HEADER_DIR" ]] || {
      echo "ERROR: Haiku cross g++ did not expose its libstdc++ headers" >&2
      echo "$CXX_SEARCH_OUTPUT" >&2
      exit 1
    }
    [[ "$CXX_CONFIG_FOUND" == true ]] || {
      echo "ERROR: Haiku cross g++ target-specific bits/c++config.h is missing" >&2
      echo "$CXX_SEARCH_OUTPUT" >&2
      exit 1
    }
    # The profile must be passed through Jam's @profile command-line syntax;
    # setting HAIKU_BUILD_PROFILE with -s is overwritten while Jam parses its
    # targets and silently leaves the build in the regular profile.  Do not
    # pass Jam -j here: Haiku's bootstrap documentation warns that parallel
    # top-level Jam instances race while haikuporter generates package metadata.
    # HAIKU_PORTER_CONCURRENT_JOBS still parallelizes the individual real
    # third-party package builds.
    jam -q \
      "-sHAIKU_PORTER_CONCURRENT_JOBS=${HAIKU_PORTER_CONCURRENT_JOBS:-2}" \
      "-sHAIKU_CCFLAGS_${HAIKU_ARCH}=${GCC_HEADER_FLAGS}" \
      "-sHAIKU_C++FLAGS_${HAIKU_ARCH}=${CXX_HEADER_FLAGS}" \
      @bootstrap-raw
  )

  PACKAGE_TOOL="$(find "$OUTPUT/objects/linux" -type f -path '*/release/tools/package/package' -perm -111 -print -quit)"
  [[ -x "$PACKAGE_TOOL" ]] || { echo "ERROR: Haiku package extraction tool missing" >&2; exit 1; }
  host_libs="$OUTPUT/objects/linux/lib"
  extract_hpkg() {
    local hpkg="$1"
    [[ -f "$hpkg" ]] || return 0
    LD_LIBRARY_PATH="$host_libs${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
      "$PACKAGE_TOOL" extract -C "$PACKAGE_ROOT" "$hpkg"
  }

  while IFS= read -r hpkg; do extract_hpkg "$hpkg"; done < <(
    find "$OUTPUT/objects/haiku/$HAIKU_ARCH/packaging/packages" \
      -type f -name '*.hpkg' -print 2>/dev/null || true
  )
  while IFS= read -r hpkg; do extract_hpkg "$hpkg"; done < <(find "$OUTPUT/download" -type f -name '*.hpkg' -print 2>/dev/null || true)

  if [[ -f "$PACKAGE_ROOT/lib/libgcc_s.so" && -d "$PACKAGE_ROOT/develop/lib" ]]; then
    ln -sfn ../../lib/libgcc_s.so "$PACKAGE_ROOT/develop/lib/libgcc_s.so"
  fi

  printf '%s\n' \
    "haiku=$HAIKU_COMMIT" \
    "buildtools=$BUILDTOOLS_COMMIT" \
    "haikuporter=$HAIKUPORTER_COMMIT" \
    "haikuports.cross=$HAIKUPORTS_CROSS_COMMIT" \
    "haikuports=$HAIKUPORTS_COMMIT" > "$BOOTSTRAP_STAMP"
  valid_toolchain || { echo "ERROR: incomplete Haiku cross-tools/sysroot for $TARGET" >&2; exit 1; }
fi

CC="$TOOLBIN/$GNU_TARGET-gcc"
CXX="$TOOLBIN/$GNU_TARGET-g++"
AR="$TOOLBIN/$GNU_TARGET-ar"
[[ -x "$CC" && -x "$CXX" && -x "$AR" ]] || { echo "ERROR: Haiku GCC tools missing" >&2; exit 1; }

key="${TARGET//-/_}"
key_upper="$(printf '%s' "$key" | tr '[:lower:]' '[:upper:]')"
export "CC_${key}=$CC"
export "CXX_${key}=$CXX"
export "AR_${key}=$AR"
export "CFLAGS_${key}=$CFLAGS_EXTRA"
export "CXXFLAGS_${key}=$CFLAGS_EXTRA"
export "CARGO_TARGET_${key_upper}_LINKER=$CC"
export RUSTFLAGS="${RUSTFLAGS:-} -C linker=$CC"

exec "$@"
