#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?target triple is required}"
BUILD_STD="${2:-false}"
BUILDER="${3:?builder is required}"
ZIG_TARGET="${4:-}"
TARGET_RUSTFLAGS="${5:-}"

if [[ "$BUILD_STD" == false ]]; then
  rustup target add "$TARGET" --toolchain 1.97.0
fi

case "$BUILDER" in
  zig-linux)
    [[ -n "$ZIG_TARGET" ]] || { echo "ERROR: Zig target is required for $TARGET" >&2; exit 2; }
    exec bash scripts/ci/run-zig-backend.sh "$TARGET" "$ZIG_TARGET" -- \
      bash scripts/ci/check-native-backend.sh "$TARGET" "$BUILD_STD" "$TARGET_RUSTFLAGS"
    ;;
  csky-gcc)
    exec bash scripts/ci/run-csky-backend.sh "$TARGET" -- \
      bash scripts/ci/check-native-backend.sh "$TARGET" "$BUILD_STD" "$TARGET_RUSTFLAGS"
    ;;
  ohos-ndk)
    exec bash scripts/ci/run-ohos-backend.sh "$TARGET" -- \
      bash scripts/ci/check-native-backend.sh "$TARGET" "$BUILD_STD" "$TARGET_RUSTFLAGS"
    ;;
  uclibc-sysroot)
    case "$TARGET" in
      armv5te-unknown-linux-uclibceabi)
        category=armv5-eabi; toolchain=armv5-eabi--uclibc--stable-2025.08-1; cflags='' ;;
      armv7-unknown-linux-uclibceabi)
        category=armv5-eabi; toolchain=armv5-eabi--uclibc--stable-2025.08-1; cflags='-march=armv7-a' ;;
      armv7-unknown-linux-uclibceabihf)
        category=armv7-eabihf; toolchain=armv7-eabihf--uclibc--stable-2025.08-1; cflags='' ;;
      mips-unknown-linux-uclibc)
        category=mips32; toolchain=mips32--uclibc--stable-2025.08-1; cflags='' ;;
      mipsel-unknown-linux-uclibc)
        category=mips32el; toolchain=mips32el--uclibc--stable-2025.08-1; cflags='' ;;
      *) echo "ERROR: no Bootlin uClibc mapping for $TARGET" >&2; exit 2 ;;
    esac
    exec bash scripts/ci/run-bootlin-backend.sh "$TARGET" "$category" "$toolchain" "$cflags" -- \
      bash scripts/ci/check-native-backend.sh "$TARGET" "$BUILD_STD" "$TARGET_RUSTFLAGS"
    ;;
  m68k-gcc)
    exec bash scripts/ci/run-bootlin-backend.sh \
      "$TARGET" m68k-68xxx m68k-68xxx--glibc--stable-2025.08-1 '' -- \
      bash scripts/ci/check-native-backend.sh "$TARGET" "$BUILD_STD" "$TARGET_RUSTFLAGS"
    ;;
  *)
    echo "Using Linux backend builder '$BUILDER' for $TARGET" >&2
    exec bash scripts/ci/check-native-backend.sh "$TARGET" "$BUILD_STD" "$TARGET_RUSTFLAGS"
    ;;
esac
