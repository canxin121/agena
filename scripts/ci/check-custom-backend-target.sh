#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?target triple is required}"
TARGET_OS="${2:?target OS is required}"
BUILD_STD="${3:-false}"
BUILDER="${4:-$TARGET_OS}"

if [[ "$BUILD_STD" == false ]]; then
  rustup target add "$TARGET" --toolchain 1.97.0
fi

run_cargo_check() {
  if [[ "$BUILD_STD" == true ]]; then
    stable_rustc="$(rustup which --toolchain 1.97.0 rustc)"
    stable_rustdoc="$(rustup which --toolchain 1.97.0 rustdoc)"
    RUSTC_BOOTSTRAP=1 \
    RUSTC="$stable_rustc" \
    RUSTDOC="$stable_rustdoc" \
      cargo +nightly-2026-08-18 check \
        --manifest-path Cargo.toml \
        -p agena \
        --target "$TARGET" \
        --locked \
        -Z build-std=std,panic_abort,proc_macro
  else
    cargo check --manifest-path Cargo.toml -p agena --target "$TARGET" --locked
  fi
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

case "$BUILDER" in
  zig-netbsd)
    zig_target="$(netbsd_zig_target)"
    exec bash scripts/ci/run-zig-backend.sh "$TARGET" "$zig_target" -- bash "$0" "$TARGET" "$TARGET_OS" "$BUILD_STD" direct
    ;;
  android-ndk)
    exec bash scripts/ci/run-android-ndk-backend.sh "$TARGET" -- \
      bash "$0" "$TARGET" "$TARGET_OS" "$BUILD_STD" direct
    ;;
  fuchsia)
    exec bash scripts/ci/run-fuchsia-backend.sh "$TARGET" -- \
      bash "$0" "$TARGET" "$TARGET_OS" "$BUILD_STD" direct
    ;;
  openbsd)
    exec bash scripts/ci/run-openbsd-backend.sh "$TARGET" -- \
      bash "$0" "$TARGET" "$TARGET_OS" "$BUILD_STD" direct
    ;;
  redox)
    exec bash scripts/ci/run-redox-backend.sh "$TARGET" -- \
      bash "$0" "$TARGET" "$TARGET_OS" "$BUILD_STD" direct
    ;;
  freebsd-sysroot)
    zig_target="$(freebsd_zig_target)"
    AGENA_ZIG_SYSROOT="$(bash scripts/ci/fetch-freebsd-sysroot.sh "$TARGET")" \
      exec bash scripts/ci/run-zig-backend.sh "$TARGET" "$zig_target" -- \
        bash "$0" "$TARGET" "$TARGET_OS" "$BUILD_STD" direct
    ;;
  direct)
    run_cargo_check
    ;;
  *)
    echo "Using custom backend builder '$BUILDER' for $TARGET" >&2
    run_cargo_check
    ;;
esac
