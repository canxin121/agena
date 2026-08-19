#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TARGET_TRIPLE="${1:?target triple is required}"
BUILD_STD="${2:-false}"
TARGET_RUSTFLAGS="${3:-}"
TARGET_DIR="$REPO_ROOT/target"
RELEASE_DIR="$REPO_ROOT/artifacts/agena"

VERSION="$(cargo metadata --manifest-path "$REPO_ROOT/Cargo.toml" --format-version 1 --no-deps --locked \
  | python3 -c 'import json, sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"] == "agena-portable-core"))')"

if [[ "$BUILD_STD" == true ]]; then
  STABLE_TOOLCHAIN="${AGENA_STABLE_TOOLCHAIN:-1.97.0}"
  NIGHTLY_TOOLCHAIN="${AGENA_NIGHTLY_TOOLCHAIN:-nightly-2026-08-18}"
  stable_rustc="$(rustup which --toolchain "$STABLE_TOOLCHAIN" rustc)"
  stable_rustdoc="$(rustup which --toolchain "$STABLE_TOOLCHAIN" rustdoc)"

  sysroot="$($stable_rustc --print sysroot)"
  declare -a rust_src_backups=()
  declare -a temp_files=()
  declare -a temp_dirs=()
  backup_rust_source() {
    local source_file="$1"
    local backup_file="${source_file}.agena-backup"
    [[ -f "$source_file" ]] || { echo "ERROR: Rust source not found at $source_file" >&2; exit 1; }
    cp "$source_file" "$backup_file"
    rust_src_backups+=("$backup_file")
  }
  cleanup_build_std() {
    local backup_file source_file temp_file temp_dir
    for backup_file in "${rust_src_backups[@]:-}"; do
      if [[ -n "$backup_file" && -f "$backup_file" ]]; then
        source_file="${backup_file%.agena-backup}"
        mv -f "$backup_file" "$source_file"
      fi
    done
    for temp_file in "${temp_files[@]:-}"; do
      [[ -z "$temp_file" ]] || rm -f "$temp_file"
    done
    for temp_dir in "${temp_dirs[@]:-}"; do
      [[ -z "$temp_dir" ]] || rm -rf "$temp_dir"
    done
  }
  trap cleanup_build_std EXIT

  cargo_target="$TARGET_TRIPLE"
  json_target_spec=false
  case "$TARGET_TRIPLE" in
    hexagon-*)
      scalar="$sysroot/lib/rustlib/src/rust/library/stdarch/crates/core_arch/src/hexagon/scalar.rs"
      backup_rust_source "$scalar"
      python3 - "$scalar" <<'PY_PATCH'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
needle = '#[inline(always)]\n#[cfg_attr(target_arch = "hexagon", target_feature'
count = text.count(needle)
if count == 0:
    raise SystemExit("Hexagon stdarch patch point not found")
path.write_text(text.replace(needle, '#[inline]\n#[cfg_attr(target_arch = "hexagon", target_feature'), encoding="utf-8")
print(f"Patched {count} Hexagon stdarch inline/target_feature sites")
PY_PATCH
      ;;
    xtensa-*)
      # Rust 1.97 exposes these target triples, but its target specs disagree
      # with the bundled LLVM Xtensa backend about data layout and ESP32-S2/S3
      # CPU names. The portable core is no_std and OS-agnostic, so derive a
      # temporary compatibility spec from the exact 1.97 built-in target and
      # preserve the original triple as the archive identity.
      if [[ -n "${RUNNER_TEMP:-}" ]]; then
        target_spec_dir="$RUNNER_TEMP/agena-target-specs"
        mkdir -p "$target_spec_dir"
      else
        target_spec_dir="$(mktemp -d -t agena-target-specs.XXXXXX)"
        temp_dirs+=("$target_spec_dir")
      fi
      target_spec="$target_spec_dir/${TARGET_TRIPLE}.json"
      temp_files+=("$target_spec")
      RUSTC_BOOTSTRAP=1 "$stable_rustc" -Z unstable-options \
        --print target-spec-json --target "$TARGET_TRIPLE" > "$target_spec"
      python3 - "$target_spec" "$TARGET_TRIPLE" <<'PY_SPEC'
import json
import sys
from pathlib import Path
path = Path(sys.argv[1])
target = sys.argv[2]
data = json.loads(path.read_text(encoding="utf-8"))
data["data-layout"] = "e-m:e-p:32:32-i8:8:32-i16:16:32-i64:64-n32"
if "esp32s2" in target or "esp32s3" in target:
    data["cpu"] = "generic"
data.pop("is-builtin", None)
path.write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
PY_SPEC
      cargo_target="$target_spec"
      json_target_spec=true

      # compiler-builtins exposes a full libm symbol set by default. The LLVM
      # Xtensa backend in Rust 1.97 cannot select a few of those floating-point
      # implementations (for example floorf), while Agena's no_std portable
      # core does not perform floating-point math. Keep libm's support module
      # for integer/float helper types, but do not export the unused math ABI.
      math_mod="$sysroot/lib/rustlib/src/rust/library/compiler-builtins/compiler-builtins/src/math/mod.rs"
      backup_rust_source "$math_mod"
      python3 - "$math_mod" <<'PY_MATH'
from pathlib import Path
import sys
path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")
full = "pub mod full_availability {"
partial = "#[cfg(not(any(\n"
if full not in text:
    raise SystemExit("compiler-builtins full_availability patch point not found")
if partial not in text:
    raise SystemExit("compiler-builtins partial_availability patch point not found")
text = text.replace(full, '#[cfg(not(target_arch = "xtensa"))]\n' + full, 1)
text = text.replace(partial, '#[cfg(not(target_arch = "xtensa"))]\n' + partial, 1)
path.write_text(text, encoding="utf-8")
print("Disabled unused compiler-builtins libm exports for Xtensa portable core")
PY_MATH
      ;;
  esac

  if [[ "$json_target_spec" == true ]]; then
    RUSTC_BOOTSTRAP=1 \
    RUSTC="$stable_rustc" \
    RUSTDOC="$stable_rustdoc" \
    RUSTFLAGS="$TARGET_RUSTFLAGS" \
      cargo "+$NIGHTLY_TOOLCHAIN" build \
        --manifest-path "$REPO_ROOT/Cargo.toml" \
        -p agena-portable-core \
        --release \
        --target "$cargo_target" \
        --locked \
        --target-dir "$TARGET_DIR" \
        -Z build-std=core \
        -Z json-target-spec
  else
    RUSTC_BOOTSTRAP=1 \
    RUSTC="$stable_rustc" \
    RUSTDOC="$stable_rustdoc" \
    RUSTFLAGS="$TARGET_RUSTFLAGS" \
      cargo "+$NIGHTLY_TOOLCHAIN" build \
        --manifest-path "$REPO_ROOT/Cargo.toml" \
        -p agena-portable-core \
        --release \
        --target "$cargo_target" \
        --locked \
        --target-dir "$TARGET_DIR" \
        -Z build-std=core
  fi
else
  RUSTFLAGS="$TARGET_RUSTFLAGS" cargo build \
    --manifest-path "$REPO_ROOT/Cargo.toml" \
    -p agena-portable-core \
    --release \
    --target "$TARGET_TRIPLE" \
    --locked \
    --target-dir "$TARGET_DIR"
fi

LIB="$TARGET_DIR/$TARGET_TRIPLE/release/libagena_portable_core.rlib"
[[ -f "$LIB" ]] || { echo "ERROR: missing portable core artifact $LIB" >&2; exit 1; }

STAGE="$RELEASE_DIR/portable/$TARGET_TRIPLE"
rm -rf "$STAGE"
mkdir -p "$STAGE/lib"
cp "$LIB" "$STAGE/lib/"
python3 - "$STAGE/manifest.json" "$VERSION" "$TARGET_TRIPLE" "$BUILD_STD" "$TARGET_RUSTFLAGS" <<'PY'
import json, sys
from pathlib import Path
path, version, target, build_std, rustflags = sys.argv[1:]
Path(path).write_text(json.dumps({
    "name": "agena-portable-core",
    "version": version,
    "target": target,
    "artifact_kind": "portable-core",
    "abi_version": 1,
    "build_std": build_std == "true",
    "rustflags": rustflags,
    "contents": ["lib/libagena_portable_core.rlib"],
}, indent=2) + "\n", encoding="utf-8")
PY
cat > "$STAGE/README.txt" <<EOF
Agena portable core
Version: $VERSION
Target: $TARGET_TRIPLE

This target does not host the full Agena daemon/TUI process model. The package
contains the target-native no_std Agena core rlib for embedding/linking.
EOF

ARCHIVE="agena-portable-core-${TARGET_TRIPLE}-v${VERSION}.tar.gz"
mkdir -p "$RELEASE_DIR"
tar -C "$STAGE" -czf "$RELEASE_DIR/$ARCHIVE" .
if command -v sha256sum >/dev/null 2>&1; then
  (cd "$RELEASE_DIR" && sha256sum "$ARCHIVE" > "$ARCHIVE.sha256")
else
  checksum="$(shasum -a 256 "$RELEASE_DIR/$ARCHIVE" | awk '{print $1}')"
  printf '%s  %s
' "$checksum" "$ARCHIVE" > "$RELEASE_DIR/$ARCHIVE.sha256"
fi
echo "Package ready: $RELEASE_DIR/$ARCHIVE"
