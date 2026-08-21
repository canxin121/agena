#!/usr/bin/env bash
set -euo pipefail

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) ;;
  *) echo "ERROR: pinned LLVM 22 installer requires Linux x86_64" >&2; exit 2 ;;
esac

# Use the complete upstream LLVM packaging rather than the Fuchsia toolchain,
# whose distribution intentionally omits backend support and has crashed on
# non-Fuchsia C targets.  The repository is authenticated by apt and the
# snapshot key is pinned independently before it is installed.
LLVM_ROOT='/usr/lib/llvm-22'
KEY_SHA256='8b2a587ffd672c4687e7581dad4b2f6c1bb2ad6b480cd9771ba2ff48e0b8c75d'
KEY_URL='https://apt.llvm.org/llvm-snapshot.gpg.key'

if [[ -x "$LLVM_ROOT/bin/clang" && -x "$LLVM_ROOT/bin/clang++" && -x "$LLVM_ROOT/bin/llvm-ar" ]]; then
  if "$LLVM_ROOT/bin/clang" --version | grep -Fq '22.1.8'; then
    printf '%s\n' "$LLVM_ROOT"
    exit 0
  fi
fi

command -v sudo >/dev/null 2>&1 || {
  echo "ERROR: sudo is required to install the official LLVM 22 apt packages" >&2
  exit 1
}
command -v curl >/dev/null 2>&1 || {
  echo "ERROR: curl is required to fetch the official LLVM apt key" >&2
  exit 1
}
command -v sha256sum >/dev/null 2>&1 || {
  echo "ERROR: sha256sum is required to verify the official LLVM apt key" >&2
  exit 1
}

source /etc/os-release
case "${VERSION_CODENAME:-}" in
  noble) LLVM_VERSION='1:22.1.8~++20260714014902+ca7933e47d3a-1~exp1~20260714135019.80' ;;
  jammy) LLVM_VERSION='1:22.1.8~++20260613092327+e80beda6e255-1~exp1~20260613092437.81' ;;
  focal) LLVM_VERSION='1:22.1.8~++20260714015029+ca7933e47d3a-1~exp1~20260714135040.88' ;;
  *)
    echo "ERROR: unsupported Ubuntu/Debian codename for apt.llvm.org: ${VERSION_CODENAME:-unknown}" >&2
    exit 1
    ;;
esac

APT_ROOT="${RUNNER_TEMP:-/tmp}/agena-llvm-22"
mkdir -p "$APT_ROOT"
KEY_FILE="$APT_ROOT/llvm-snapshot.gpg.key"
if [[ ! -f "$KEY_FILE" ]] || [[ "$(sha256sum "$KEY_FILE" | awk '{print $1}')" != "$KEY_SHA256" ]]; then
  curl -fsSL --retry 5 --retry-all-errors --connect-timeout 20 --max-time 120 "$KEY_URL" -o "$KEY_FILE"
fi
actual_key_sha256="$(sha256sum "$KEY_FILE" | awk '{print $1}')"
[[ "$actual_key_sha256" == "$KEY_SHA256" ]] || {
  echo "ERROR: LLVM apt key SHA256 mismatch: expected $KEY_SHA256, got $actual_key_sha256" >&2
  exit 1
}

sudo install -d -m 0755 /etc/apt/keyrings
sudo install -m 0644 "$KEY_FILE" /etc/apt/keyrings/agena-llvm-snapshot.asc
printf 'deb [arch=amd64 signed-by=/etc/apt/keyrings/agena-llvm-snapshot.asc] https://apt.llvm.org/%s/ llvm-toolchain-%s-22 main\n' \
  "$VERSION_CODENAME" "$VERSION_CODENAME" |
  sudo tee /etc/apt/sources.list.d/agena-llvm-22.list >/dev/null

sudo apt-get update -o Acquire::Retries=5 >&2
sudo apt-get install -y --no-install-recommends \
  "clang-22=$LLVM_VERSION" \
  "llvm-22=$LLVM_VERSION" \
  "llvm-22-runtime=$LLVM_VERSION" \
  "llvm-22-linker-tools=$LLVM_VERSION" \
  "llvm-22-tools=$LLVM_VERSION" \
  "lld-22=$LLVM_VERSION" >&2

for tool in clang clang++ llvm-ar; do
  [[ -x "$LLVM_ROOT/bin/$tool" ]] || {
    echo "ERROR: official LLVM 22 package did not provide $LLVM_ROOT/bin/$tool" >&2
    exit 1
  }
done
"$LLVM_ROOT/bin/clang" --version | grep -Fq '22.1.8' || {
  echo "ERROR: installed LLVM compiler is not the pinned 22.1.8 build" >&2
  exit 1
}

printf '%s\n' "$LLVM_ROOT"
