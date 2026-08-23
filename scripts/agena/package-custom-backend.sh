#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?target triple is required}"
TARGET_OS="${2:?target OS is required}"
BUILD_STD="${3:-false}"
BUILDER="${4:-$TARGET_OS}"

netbsd_zig_target() {
  case "$TARGET" in
    aarch64-unknown-netbsd) echo aarch64-netbsd ;;
    aarch64_be-unknown-netbsd) echo aarch64_be-netbsd ;;
    armv6-unknown-netbsd-eabihf|armv7-unknown-netbsd-eabihf) echo arm-netbsd-eabihf ;;
    i586-unknown-netbsd|i686-unknown-netbsd) echo x86-netbsd ;;
    mipsel-unknown-netbsd) echo mipsel-netbsd ;;
    powerpc-unknown-netbsd) echo powerpc-netbsd ;;
    sparc64-unknown-netbsd) echo sparc64-netbsd ;;
    *) return 1 ;;
  esac
}

openbsd_zig_target() {
  case "$TARGET" in
    aarch64-unknown-openbsd) echo aarch64-openbsd ;;
    i686-unknown-openbsd) echo x86-openbsd ;;
    powerpc-unknown-openbsd) echo powerpc-openbsd ;;
    powerpc64-unknown-openbsd) echo powerpc64-openbsd ;;
    riscv64gc-unknown-openbsd) echo riscv64-openbsd ;;
    sparc64-unknown-openbsd) echo sparc64-openbsd ;;
    x86_64-unknown-openbsd) echo x86_64-openbsd ;;
    *) return 1 ;;
  esac
}

freebsd_zig_target() {
  case "$TARGET" in
    powerpc-unknown-freebsd) echo powerpc-freebsd ;;
    powerpc64-unknown-freebsd) echo powerpc64-freebsd ;;
    powerpc64le-unknown-freebsd) echo powerpc64le-freebsd ;;
    riscv64gc-unknown-freebsd) echo riscv64-freebsd ;;
    *) return 1 ;;
  esac
}

case "$BUILDER" in
  cygwin)
    echo "ERROR: Cygwin requires the official Windows Cygwin toolchain and PowerShell package path" >&2
    exit 2
    ;;
  zig-netbsd)
    zig_target="$(netbsd_zig_target)"
    exec bash scripts/ci/run-zig-backend.sh "$TARGET" "$zig_target" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  freebsd-image-sysroot)
    exec bash scripts/ci/run-freebsd-image-backend.sh "$TARGET" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  hurd)
    exec bash scripts/ci/run-hurd-backend.sh "$TARGET" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  android-ndk)
    exec bash scripts/ci/run-android-ndk-backend.sh "$TARGET" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  fuchsia)
    exec bash scripts/ci/run-fuchsia-backend.sh "$TARGET" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  openbsd)
    exec bash scripts/ci/run-openbsd-backend.sh "$TARGET" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  redox)
    exec bash scripts/ci/run-redox-backend.sh "$TARGET" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  illumos)
    exec bash scripts/ci/run-illumos-backend.sh "$TARGET" -- \
      bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  freebsd-sysroot)
    zig_target="$(freebsd_zig_target)"
    AGENA_ZIG_SYSROOT="$(bash scripts/ci/fetch-freebsd-sysroot.sh "$TARGET")" \
      exec bash scripts/ci/run-zig-backend.sh "$TARGET" "$zig_target" -- \
        bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
  *)
    exec bash scripts/agena/package-backend.sh "$TARGET" "$BUILD_STD"
    ;;
esac
