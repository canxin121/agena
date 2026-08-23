#!/usr/bin/env bash
set -euo pipefail

TARGET="${1:?Redox target triple is required}"
shift
[[ "${1:-}" == -- ]] && shift
[[ $# -gt 0 ]] || { echo "ERROR: command is required" >&2; exit 2; }

case "$TARGET" in
  aarch64-unknown-redox|i586-unknown-redox|riscv64gc-unknown-redox|x86_64-unknown-redox) ;;
  *) echo "ERROR: unsupported Redox target: $TARGET" >&2; exit 2 ;;
esac

if ! command -v redoxer >/dev/null 2>&1; then
  cargo install redoxer --locked --version 0.2.63
fi

export TARGET
export AGENA_TARGET_TRIPLE="$TARGET"

install_i586_package_toolchain() {
  local host_target="x86_64-unknown-linux-gnu"
  local toolchain_source="https://static.redox-os.org/toolchain/${host_target}/${TARGET}"
  local package_source="https://static.redox-os.org/pkg/${TARGET}"
  local package_key_source="https://static.redox-os.org/pkg/id_ed25519.pub.toml"
  local package_public_key='pkey = "578b09da56eb4ae1d1ab356b84ab860ca763ca859bed7557a91954c3bf59677e"'
  local work_dir
  local toolchain_partial
  local package_root
  local cxx_root
  local sum_file
  local archive
  local expected
  local pkgar_bin

  work_dir="$(mktemp -d "${RUNNER_TEMP:-/tmp}/agena-redox-i586.XXXXXX")"
  toolchain_partial="$work_dir/toolchain"
  package_root="$work_dir/relibc-package"
  cxx_root="$work_dir/cxx"
  sum_file="$work_dir/SHA256SUM"
  mkdir -p "$toolchain_partial" "$package_root" "$cxx_root"

  # The Redox i586 toolchain directory currently has a publication race:
  # relibc-install.tar.gz is replaced before its SHA256SUM entry is updated.
  # Keep the compiler archives on the normal, independently checked path and
  # obtain relibc from the signed package repository instead of accepting a
  # hash that is not present in trusted upstream metadata.
  curl --proto '=https' --tlsv1.2 --fail --location \
    --retry 5 --retry-all-errors --connect-timeout 20 --max-time 180 \
    --output "$sum_file" "$toolchain_source/SHA256SUM"

  for archive in gcc-install.tar.gz rust-install.tar.gz clang-install.tar.gz; do
    expected="$(awk -v name="$archive" '$2 == "*" name || $2 == name { print $1; exit }' "$sum_file")"
    [[ "$expected" =~ ^[[:xdigit:]]{64}$ ]] || {
      echo "ERROR: Redox SHA256SUM has no valid entry for $archive" >&2
      return 1
    }
    curl --proto '=https' --tlsv1.2 --fail --location \
      --retry 5 --retry-all-errors --connect-timeout 20 --max-time 900 \
      --output "$work_dir/$archive.partial" "$toolchain_source/$archive"
    printf '%s  %s\n' "$expected" "$work_dir/$archive.partial" | sha256sum --check --status -
    mv "$work_dir/$archive.partial" "$work_dir/$archive"
    tar --extract --file "$work_dir/$archive" --directory "$toolchain_partial" --no-same-owner --strip-components=1
  done

  curl --proto '=https' --tlsv1.2 --fail --location \
    --retry 5 --retry-all-errors --connect-timeout 20 --max-time 120 \
    --output "$work_dir/id_ed25519.pub.toml" "$package_key_source"
  grep -Fqx "$package_public_key" "$work_dir/id_ed25519.pub.toml" || {
    echo "ERROR: Redox package signing key changed unexpectedly" >&2
    return 1
  }

  curl --proto '=https' --tlsv1.2 --fail --location \
    --retry 5 --retry-all-errors --connect-timeout 20 --max-time 300 \
    --output "$work_dir/relibc.pkgar.partial" "$package_source/relibc.pkgar"
  mv "$work_dir/relibc.pkgar.partial" "$work_dir/relibc.pkgar"

  # pkgar verifies the signed package header and every extracted entry's
  # BLAKE3 before committing the extraction.  This is the real Redox relibc,
  # not a substitute library or a compile-only placeholder.
  pkgar_bin="$work_dir/pkgar/bin/pkgar"
  cargo install pkgar --locked --version 0.2.3 --features cli --root "$work_dir/pkgar"
  "$pkgar_bin" extract \
    --pkey "$work_dir/id_ed25519.pub.toml" \
    --archive "$work_dir/relibc.pkgar" \
    "$package_root"
  [[ -f "$package_root/usr/include/unistd.h" ]] || {
    echo "ERROR: signed Redox relibc package did not contain headers" >&2
    return 1
  }
  [[ -f "$package_root/usr/lib/libc.a" && -f "$package_root/usr/lib/crt0.o" ]] || {
    echo "ERROR: signed Redox relibc package did not contain libc/crt objects" >&2
    return 1
  }

  # Mirror Redox's own prefix assembly: keep the compiler's libstdc++ headers,
  # install relibc under the GNU target sysroot, and expose /usr as the same
  # relative symlinks used by the official relibc-install archive.
  if [[ -d "$toolchain_partial/$TARGET/include/c++" ]]; then
    cp -a "$toolchain_partial/$TARGET/include/c++" "$cxx_root/"
  fi
  rm -rf "$toolchain_partial/$TARGET/include" "$toolchain_partial/$TARGET/usr"
  mkdir -p "$toolchain_partial/$TARGET"
  cp -a "$package_root/usr/." "$toolchain_partial/$TARGET/"
  if [[ -d "$cxx_root/c++" ]]; then
    mkdir -p "$toolchain_partial/$TARGET/include"
    cp -a "$cxx_root/c++" "$toolchain_partial/$TARGET/include/"
  fi
  mkdir -p "$toolchain_partial/$TARGET/usr"
  ln -s ../include "$toolchain_partial/$TARGET/usr/include"
  ln -s ../lib "$toolchain_partial/$TARGET/usr/lib"

  export REDOXER_TOOLCHAIN="$toolchain_partial"
}

if [[ "$TARGET" == i586-unknown-redox ]]; then
  install_i586_package_toolchain
else
  redoxer toolchain
fi
NIGHTLY_TOOLCHAIN="${AGENA_NIGHTLY_TOOLCHAIN:-nightly-2026-08-18}"
export AGENA_CARGO_DRIVER="$(rustup which --toolchain "$NIGHTLY_TOOLCHAIN" cargo)"

# redoxer env exports both target-specific variables (which native C build
# scripts need) and global CC/CXX/AR variables.  The latter are inherited by
# Cargo build scripts that compile for the Linux host, so a host build-script
# executable can accidentally be linked from Redox objects.  Keep the
# target-specific CC_<triple>/AR_<triple>/CARGO_TARGET_* variables while
# removing only the global target-tool selections and flags.
exec redoxer env env \
  -u CC -u CXX -u AR -u AS -u LD -u NM -u OBJCOPY -u OBJDUMP \
  -u RANLIB -u READELF -u STRIP -u PKG_CONFIG \
  -u CPPFLAGS -u CFLAGS -u CXXFLAGS -u LDFLAGS -u RUSTFLAGS \
  -- "$@"
