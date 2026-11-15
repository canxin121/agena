#!/usr/bin/env bash
# Agena one-click installer and lifecycle manager for macOS/Linux.
set -euo pipefail

DEFAULT_REPO="canxin121/agena"
ACTION="install"
REPO="${AGENA_INSTALL_REPO:-$DEFAULT_REPO}"
VERSION="${AGENA_INSTALL_VERSION:-}"
RELEASE_VERSION=""
ARCHIVE=""
CHECKSUM=""
SOURCE_MODE=0
SOURCE=""
SOURCE_REF=""
SOURCE_ARCHIVE=""
SOURCE_CHECKSUM=""
SOURCE_ROOT=""
INSTALL_DIR="${AGENA_INSTALL_DIR:-$HOME/.local/share/agena}"
DATA_DIR="${AGENA_DATA_DIR:-$HOME/agena}"
HOST="${AGENA_SERVER_HOST:-127.0.0.1}"
PORT="${AGENA_SERVER_PORT:-3210}"
WORKSPACE="${AGENA_WORKSPACE_ROOT:-$HOME}"
SERVER_PASSWORD="${AGENA_SERVER_PASSWORD:-}"
SERVICE_MODE="${AGENA_INSTALL_SERVICE_MODE:-auto}"
UPDATE_PATH=1
NO_PATH_UPDATE_SET=0
PURGE_DATA=0
REPO_SET=0
VERSION_SET=0
ARCHIVE_SET=0
CHECKSUM_SET=0
SOURCE_REF_SET=0
DATA_DIR_SET=0
HOST_SET=0
PORT_SET=0
WORKSPACE_SET=0
SERVER_PASSWORD_SET=0
SERVICE_MODE_SET=0
SETUP_MODE="${AGENA_INSTALL_SETUP_MODE:-auto}"
SETUP_MODE_SET=0
SANDBOX_REPORT=""

usage() {
  cat <<'EOF'
Usage: install.sh [install|reinstall|upgrade|uninstall|start|stop|restart|status|permissions] [options]

Actions:
  install     Install into an empty installation root
  reinstall   Replace the installed program; reuse its version by default
  upgrade     Replace the installed program with the newest release
  start, stop, restart, status, uninstall
  permissions Repair or check runtime dependencies and host permissions

One-click install (latest beta release):
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash

Source install (build from a checkout):
  bash scripts/agena/install.sh install --source

Rebuild the current checkout as the installed version:
  bash scripts/agena/install.sh reinstall --source

Change saved runtime settings while upgrading:
  bash scripts/agena/install.sh upgrade --source --host 0.0.0.0 --port 3210

Lifecycle examples:
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- upgrade
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- stop
  curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- uninstall

Options:
  --repo OWNER/REPO        GitHub repository (default: canxin121/agena)
  --version VERSION        Beta release version/tag; defaults to the recorded
                           release for reinstall and the newest beta otherwise
  --archive PATH_OR_URL    Install a specific release archive
  --checksum PATH_OR_URL   SHA256 file for --archive (defaults to ARCHIVE.sha256)
  --source [PATH_OR_URL]   Build from source instead of installing a release archive
  --source-ref REF         Branch or tag to clone with --source (remote sources only)
  --install-dir DIR        Install root (default: ~/.local/share/agena)
  --data-dir DIR           Instance data root (default: AGENA_DATA_DIR or ~/agena)
  --host HOST              Server host (default: 127.0.0.1)
  --port PORT              Server port (default: 3210)
  --workspace PATH         Server workspace (default: $HOME)
  --password PASSWORD      Server password shared by Web/TUI and MCP OAuth
  --service-mode MODE      auto, native, or detached (default: auto)
  Release and source options apply to install, reinstall, and upgrade.
  Runtime options replace the saved values during reinstall and upgrade;
  omitted runtime options keep the saved values.
  --setup-mode MODE        auto (default): prepare missing dependencies/permissions;
                           check: verify existing host setup without changing it
                           Applies to install, reinstall, upgrade, and permissions
  --no-path-update         Do not add the install bin directory to shell PATH
  --purge-data             With uninstall, also remove the installed instance data root
  -h, --help               Show this help
EOF
}

if [[ $# -gt 0 && ! "${1:-}" =~ ^- ]]; then
  ACTION="$1"
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --repo) REPO="${2:-}"; REPO_SET=1; shift 2 ;;
    --version) VERSION="${2:-}"; VERSION_SET=1; shift 2 ;;
    --archive) ARCHIVE="${2:-}"; ARCHIVE_SET=1; shift 2 ;;
    --checksum) CHECKSUM="${2:-}"; CHECKSUM_SET=1; shift 2 ;;
    --source)
      SOURCE_MODE=1
      if [[ $# -ge 2 && "${2:-}" != -* ]]; then
        SOURCE="$2"
        shift 2
      else
        SOURCE=""
        shift
      fi ;;
    --source=*) SOURCE_MODE=1; SOURCE="${1#*=}"; shift ;;
    --source-ref) SOURCE_REF="${2:-}"; SOURCE_REF_SET=1; shift 2 ;;
    --install-dir) INSTALL_DIR="${2:-}"; shift 2 ;;
    --data-dir) DATA_DIR="${2:-}"; DATA_DIR_SET=1; shift 2 ;;
    --host) HOST="${2:-}"; HOST_SET=1; shift 2 ;;
    --port) PORT="${2:-}"; PORT_SET=1; shift 2 ;;
    --workspace) WORKSPACE="${2:-}"; WORKSPACE_SET=1; shift 2 ;;
    --password) SERVER_PASSWORD="${2:-}"; SERVER_PASSWORD_SET=1; shift 2 ;;
    --service-mode) SERVICE_MODE="${2:-}"; SERVICE_MODE_SET=1; shift 2 ;;
    --setup-mode) SETUP_MODE="${2:-}"; SETUP_MODE_SET=1; shift 2 ;;
    --no-path-update) UPDATE_PATH=0; NO_PATH_UPDATE_SET=1; shift ;;
    --purge-data) PURGE_DATA=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "ERROR: unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

case "$ACTION" in
  install|reinstall|upgrade|uninstall|start|stop|restart|status|permissions) ;;
  *) echo "ERROR: unsupported action: $ACTION" >&2; usage >&2; exit 2 ;;
esac

# Reject options that the selected action would otherwise ignore silently.
case "$ACTION" in
  install|reinstall|upgrade) replacement_action=1 ;;
  *) replacement_action=0 ;;
esac
source_options=()
runtime_options=()
if (( ! replacement_action )); then
  (( REPO_SET )) && source_options+=(--repo)
  (( VERSION_SET )) && source_options+=(--version)
  (( ARCHIVE_SET )) && source_options+=(--archive)
  (( CHECKSUM_SET )) && source_options+=(--checksum)
  (( SOURCE_MODE )) && source_options+=(--source)
  (( SOURCE_REF_SET )) && source_options+=(--source-ref)
  (( DATA_DIR_SET )) && runtime_options+=(--data-dir)
  (( HOST_SET )) && runtime_options+=(--host)
  (( PORT_SET )) && runtime_options+=(--port)
  (( WORKSPACE_SET )) && runtime_options+=(--workspace)
  (( SERVER_PASSWORD_SET )) && runtime_options+=(--password)
  (( SERVICE_MODE_SET )) && runtime_options+=(--service-mode)
fi
if (( ${#source_options[@]} > 0 )); then
  echo "ERROR: ${source_options[*]} only applies to install, reinstall, and upgrade" >&2
  exit 2
fi
if (( ${#runtime_options[@]} > 0 )); then
  echo "ERROR: ${runtime_options[*]} only applies to install, reinstall, and upgrade" >&2
  exit 2
fi
if (( PURGE_DATA == 1 )) && [[ "$ACTION" != "uninstall" ]]; then
  echo "ERROR: --purge-data only applies to uninstall" >&2
  exit 2
fi
if (( SETUP_MODE_SET )) && (( ! replacement_action )) && [[ "$ACTION" != permissions ]]; then
  echo "ERROR: --setup-mode only applies to install, reinstall, upgrade, and permissions" >&2
  exit 2
fi
case "$SETUP_MODE" in
  auto|check) ;;
  *) echo "ERROR: --setup-mode must be auto or check" >&2; exit 2 ;;
esac

case "$SERVICE_MODE" in
  auto|native|detached) ;;
  *) echo "ERROR: --service-mode must be auto, native, or detached" >&2; exit 2 ;;
esac
if (( SOURCE_MODE )); then
  case "$ACTION" in
    install|reinstall|upgrade) ;;
    *) echo "ERROR: --source is only supported for install, reinstall, and upgrade" >&2; exit 2 ;;
  esac
  if [[ -n "$ARCHIVE" ]]; then
    echo "ERROR: --source cannot be combined with --archive" >&2
    exit 2
  fi
  if [[ -n "$CHECKSUM" ]]; then
    echo "ERROR: --source cannot be combined with --checksum" >&2
    exit 2
  fi
  if [[ -n "$VERSION" ]]; then
    echo "ERROR: --version applies to release installs; use --source-ref to select a branch or tag" >&2
    exit 2
  fi
elif [[ -n "$SOURCE_REF" ]]; then
  echo "ERROR: --source-ref requires --source" >&2
  exit 2
fi
if ! [[ "$PORT" =~ ^[0-9]+$ ]] || (( PORT < 1 || PORT > 65535 )); then
  echo "ERROR: invalid port: $PORT" >&2
  exit 2
fi

# BEGIN AGENA HOST SETUP
# Embedded by install.sh; sourcing this section performs no host operations.
# Official installers are downloaded as artifacts and checked before execution.

agena_setup_error() { printf 'ERROR: %s\n' "$*" >&2; return 1; }
agena_setup_note() { printf 'Agena setup: %s\n' "$*" >&2; }
agena_setup_system_path() { printf '%s' "$1"; }
agena_setup_has() {
  local executable information
  executable="$(command -v "$1" 2>/dev/null)" || return 1
  if [[ "$1" == python3 ]]; then
    # macOS can expose a non-working developer-tools shim at this path.
    if [[ "$executable" == /usr/bin/python3 && "$(uname -s)" == Darwin ]] &&
       ! xcode-select -p >/dev/null 2>&1; then return 1; fi
    "$executable" -c 'import sys; raise SystemExit(0 if sys.version_info >= (3, 8) else 1)' >/dev/null 2>&1
  elif [[ "$1" == unshare ]]; then
    information="$("$executable" --help 2>&1)" || return 1
    [[ "$information" == *--map-current-user* && "$information" == *--keep-caps* ]]
  elif [[ "$1" == nsenter ]]; then
    information="$("$executable" --version 2>&1)" || return 1
    [[ "$information" == *util-linux* ]]
  elif [[ "$1" == ip ]]; then
    information="$("$executable" -Version 2>&1)" || return 1
    [[ "$information" == *iproute2* ]]
  else
    return 0
  fi
}
agena_setup_has_tty() { ( : </dev/tty ) >/dev/null 2>&1; }
agena_setup_from_tty() { "$@" </dev/tty; }

agena_setup_quote_command() {
  local argument part command=exec
  for argument in "$@"; do
    command="$command '"
    # Avoid Bash-version-dependent replacement-string backslash handling.
    # Close the single quotes, append a double-quoted apostrophe, and reopen.
    while [[ "$argument" == *"'"* ]]; do
      part="${argument%%\'*}"
      command="$command$part'\"'\"'"
      argument="${argument#*\'}"
    done
    command="$command$argument'"
  done
  printf '%s' "$command"
}

agena_setup_as_root() {
  # Password input belongs to the OS elevation tool and its controlling TTY.
  (( $# )) || { agena_setup_error "A privileged command is required."; return 1; }
  if [[ "$(id -u)" == 0 ]]; then
    "$@"
  elif agena_setup_has sudo; then
    if sudo -n true >/dev/null 2>&1; then
      sudo -n -- "$@"
    elif agena_setup_has_tty; then
      agena_setup_from_tty sudo -- "$@"
    else
      agena_setup_error "Administrator authentication is required for: $*. Rerun this installer in an interactive terminal, or run sudo -v in that terminal first. Passwords are read only by sudo."
    fi
  elif agena_setup_has doas; then
    if doas -n true >/dev/null 2>&1; then
      doas -n "$@"
    elif agena_setup_has_tty; then
      agena_setup_from_tty doas "$@"
    else
      agena_setup_error "doas authentication is required for: $*. Run this installer in an interactive terminal or configure the administrator-approved doas authorization."
    fi
  elif agena_setup_has su && agena_setup_has_tty; then
    local command
    command="$(agena_setup_quote_command "$@")" || return
    agena_setup_from_tty su -s /bin/sh -c "$command" root
  else
    agena_setup_error "Administrator access is required for: $*. Use a terminal with sudo, doas, or su (root password), or prepare the listed dependencies as an administrator."
  fi
}

agena_setup_user() {
  [[ "$(id -u)" != 0 ]] ||
    agena_setup_error "Run this installer as your normal user, without sudo. It elevates only the required system preparation commands."
}

agena_setup_mode() {
  case "$1" in auto|check) return 0 ;; esac
  agena_setup_error "Host setup mode must be auto or check."
}

agena_setup_sha256() {
  if agena_setup_has sha256sum; then sha256sum "$@" | awk '{print $1}'
  elif agena_setup_has shasum; then shasum -a 256 "$@" | awk '{print $1}'
  else agena_setup_error "sha256sum or shasum is required."; fi
}

agena_setup_ca_ready() {
  local candidate
  for candidate in "${SSL_CERT_FILE:-}" \
    /etc/ssl/certs/ca-certificates.crt /etc/pki/tls/certs/ca-bundle.crt \
    /etc/ssl/ca-bundle.pem /etc/ssl/cert.pem; do
    [[ -n "$candidate" ]] || continue
    [[ -s "$(agena_setup_system_path "$candidate")" ]] && return 0
  done
  return 1
}

agena_setup_distro_words() {
  local file key value words=""
  file="$(agena_setup_system_path /etc/os-release)"
  [[ -r "$file" ]] || file="$(agena_setup_system_path /usr/lib/os-release)"
  [[ -r "$file" ]] || return 0
  while IFS='=' read -r key value || [[ -n "$key" ]]; do
    case "$key" in ID|ID_LIKE|VARIANT_ID)
      value="${value#\"}"; value="${value%\"}"
      value="${value#\'}"; value="${value%\'}"
      [[ "$value" =~ ^[a-zA-Z0-9._\ -]*$ ]] ||
        { agena_setup_error "Invalid distribution identity in $file"; return 1; }
      words="$words $value"
      ;;
    esac
  done < "$file"
  printf '%s' "$words" | tr '[:upper:]' '[:lower:]'
}

agena_setup_package_manager() {
  local words candidate managers=""
  words=" $(agena_setup_distro_words) " || return
  if [[ -e "$(agena_setup_system_path /run/ostree-booted)" ]]; then
    agena_setup_error "This OSTree/immutable host must provision dependencies through its image or rpm-ostree configuration; the installer will not run a mutable-system package manager."
    return 1
  fi
  case "$words" in
    *' nixos '*) agena_setup_error "NixOS dependencies must be declared in the NixOS system configuration; this installer will not invoke apt or dnf."; return 1 ;;
    *' opensuse-microos '*|*' microos '*|*' aeon '*|*' kalpa '*)
      agena_setup_error "This transactional/immutable SUSE system must provision dependencies through its transactional system configuration."; return 1 ;;
    *' alpine '*) managers=apk ;;
    *' opensuse '*|*' opensuse-tumbleweed '*|*' opensuse-leap '*|*' suse '*|*' sles '*) managers=zypper ;;
    *' arch '*|*' manjaro '*) managers=pacman ;;
    *' debian '*|*' ubuntu '*|*' linuxmint '*) managers="apt-get apt" ;;
    *' fedora '*|*' rhel '*|*' centos '*|*' rocky '*|*' almalinux '*|*' amzn '*|*' ol '*) managers="dnf dnf5 yum" ;;
  esac
  if [[ -n "$managers" ]]; then
    for candidate in $managers; do
      if agena_setup_has "$candidate"; then printf '%s' "$candidate"; return 0; fi
    done
    agena_setup_error "Distribution $words requires one of these package managers: $managers"
    return 1
  fi
  if [[ -n "${words// /}" ]]; then
    agena_setup_error "Unsupported distribution identity:$words. Install the listed dependencies using its native package manager."
    return 1
  fi
  # Minimal containers may omit os-release. Infer only an unambiguous family.
  local selected="" family count=0
  for family in "apt-get apt" "dnf dnf5 yum" pacman zypper apk; do
    for candidate in $family; do
      if agena_setup_has "$candidate"; then
        selected="$candidate"; count=$((count + 1)); break
      fi
    done
  done
  if (( count == 1 )); then printf '%s' "$selected"; return 0; fi
  agena_setup_error "Cannot identify a unique supported Linux package manager; check /etc/os-release."
}

agena_setup_libc() {
  local words information candidate
  words=" $(agena_setup_distro_words) " || return
  if [[ "$words" == *' alpine '* || -r "$(agena_setup_system_path /etc/alpine-release)" ]]; then
    printf musl; return 0
  fi
  if agena_setup_has getconf; then
    information="$(getconf GNU_LIBC_VERSION 2>/dev/null)" || information=""
    if [[ "$information" == *glibc* ]]; then printf gnu; return 0; fi
  fi
  if agena_setup_has ldd; then
    information="$(ldd --version 2>&1)" || :
    case "$information" in
      *musl*) printf musl; return 0 ;;
      *GLIBC*|*'GNU libc'*|*'GNU C Library'*) printf gnu; return 0 ;;
    esac
  fi
  for candidate in "$(agena_setup_system_path /lib)"/ld-musl-*.so.1; do
    if [[ -f "$candidate" ]]; then printf musl; return 0; fi
  done
  case "$words" in
    *' debian '*|*' ubuntu '*|*' fedora '*|*' rhel '*|*' centos '*|*' rocky '*|*' almalinux '*|*' amzn '*|*' arch '*|*' suse '*|*' opensuse '*)
      printf gnu; return 0 ;;
  esac
  agena_setup_error "Cannot determine whether this Linux system uses glibc or musl."
}

agena_setup_target_triple() {
  local os arch libc
  os="$(uname -s)"; arch="$(uname -m)"
  if [[ "$os" == Linux ]]; then libc="$(agena_setup_libc)" || return; fi
  case "$os:$arch" in
    Linux:x86_64|Linux:amd64)
      case "$libc" in gnu) printf x86_64-unknown-linux-gnu ;; musl) printf x86_64-unknown-linux-musl ;; esac ;;
    Linux:aarch64|Linux:arm64)
      case "$libc" in gnu) printf aarch64-unknown-linux-gnu ;; musl) printf aarch64-unknown-linux-musl ;; esac ;;
    Darwin:x86_64|Darwin:amd64) printf x86_64-apple-darwin ;;
    Darwin:aarch64|Darwin:arm64) printf aarch64-apple-darwin ;;
    *) agena_setup_error "Unsupported build host: $os/$arch"; return 1 ;;
  esac
}

agena_setup_package_name() {
  local manager="$1" item="$2"
  case "$item" in
    bwrap) printf bubblewrap ;;
    slirp4netns) printf slirp4netns ;;
    ip) case "$manager" in dnf|dnf5|yum) printf iproute ;; *) printf iproute2 ;; esac ;;
    iptables|ip6tables|iptables-restore|ip6tables-restore)
      case "$manager" in dnf|dnf5|yum) printf iptables-nft ;; *) printf iptables ;; esac ;;
    unshare|nsenter) case "$manager" in apk) printf util-linux-misc ;; *) printf util-linux ;; esac ;;
    modprobe) printf kmod ;;
    apparmor_parser) case "$manager" in zypper) printf apparmor-parser ;; *) printf apparmor ;; esac ;;
    apparmor-policy) case "$manager" in apk) printf apparmor-profiles-tunables ;; zypper) printf apparmor-abstractions ;; *) printf apparmor ;; esac ;;
    openssl-dev)
      case "$manager" in
        apt-get|apt) printf libssl-dev ;;
        dnf|dnf5|yum) printf openssl-devel ;;
        zypper) printf libopenssl-devel ;;
        pacman) printf openssl ;;
        apk) printf openssl-dev ;;
        *) agena_setup_error "No OpenSSL development package mapping for $manager"; return 1 ;;
      esac ;;
    cc|c++)
      case "$manager:$item" in
        apt-get:*|apt:*) printf build-essential ;;
        dnf:c++|dnf5:c++|yum:c++) printf gcc-c++ ;;
        zypper:c++) printf gcc-c++ ;;
        apk:*) printf build-base ;;
        *:c++) printf gcc ;;
        *) printf gcc ;;
      esac ;;
    pkg-config) case "$manager" in dnf|dnf5|yum) printf pkgconf-pkg-config ;; pacman|apk|brew) printf pkgconf ;; *) printf pkg-config ;; esac ;;
    sha256) printf coreutils ;;
    python3) case "$manager" in brew) printf python ;; *) printf python3 ;; esac ;;
    *) printf '%s' "$item" ;;
  esac
}

agena_setup_packages() {
  local mode="$1" os="$2"; shift 2
  (( $# > 0 )) || return 0
  local manager="" candidate item package seen
  local packages=()
  if [[ "$mode" == check ]]; then
    agena_setup_error "Missing required tools/packages: $*. Rerun with --setup-mode auto to install them."
    return 1
  fi
  if [[ "$os" == Darwin ]]; then
    agena_setup_homebrew "$mode" || return
    manager=brew
  else
    manager="$(agena_setup_package_manager)" || return
  fi
  for item in "$@"; do
    package="$(agena_setup_package_name "$manager" "$item")" || return
    seen=0
    for candidate in "${packages[@]:-}"; do [[ "$candidate" == "$package" ]] && seen=1; done
    (( seen )) || packages+=("$package")
  done
  agena_setup_note "Installing missing dependencies: ${packages[*]}"
  case "$manager" in
    apt-get|apt)
      if [[ "${_AGENA_SETUP_APT_UPDATED:-0}" != 1 ]]; then
        agena_setup_as_root "$manager" update || return
        _AGENA_SETUP_APT_UPDATED=1
      fi
      agena_setup_as_root "$manager" install -y --no-install-recommends "${packages[@]}" ;;
    dnf|dnf5|yum) agena_setup_as_root "$manager" install -y "${packages[@]}" ;;
    pacman) agena_setup_as_root pacman -S --needed --noconfirm "${packages[@]}" ;;
    zypper) agena_setup_as_root zypper --non-interactive install "${packages[@]}" ;;
    apk) agena_setup_as_root apk add --no-cache "${packages[@]}" ;;
    brew) HOMEBREW_NO_AUTO_UPDATE=1 brew install "${packages[@]}" ;;
  esac
}

agena_setup_require_commands() {
  local mode="$1" os="$2"; shift 2
  local name missing=()
  for name in "$@"; do agena_setup_has "$name" || missing+=("$name"); done
  if (( ${#missing[@]} )); then
    agena_setup_packages "$mode" "$os" "${missing[@]}" || return
  fi
  for name in "$@"; do
    agena_setup_has "$name" ||
      { agena_setup_error "$name is still unavailable on PATH after preparation."; return 1; }
  done
}

agena_setup_find_homebrew() {
  agena_setup_has brew && return 0
  local candidate directory
  for candidate in /opt/homebrew/bin/brew /usr/local/bin/brew; do
    candidate="$(agena_setup_system_path "$candidate")"
    if [[ -x "$candidate" ]]; then
      directory="$(dirname "$candidate")"
      export PATH="$directory:$(dirname "$directory")/sbin:$PATH"
      return 0
    fi
  done
  return 1
}

agena_setup_homebrew() {
  local mode="$1" temp
  agena_setup_find_homebrew && return 0
  [[ "$mode" == auto ]] ||
    { agena_setup_error "Homebrew is missing; rerun with --setup-mode auto to install it."; return 1; }
  agena_setup_has curl ||
    { agena_setup_error "The macOS system curl is required to download Homebrew."; return 1; }
  if ! agena_setup_has sha256sum && ! agena_setup_has shasum; then
    agena_setup_error "The macOS system shasum is required to verify Homebrew."; return 1
  fi
  # Official Homebrew/install snapshot, reviewed 2026-11-15. The immutable
  # commit and digest are both pinned; a changed download is never executed.
  local commit=09c62fc577ec170172b0a184060f141a2c622dc1
  local checksum=5f333bbe53bc490e51e7ccb1df8779b3dd6ee73a1a7379efda216edb08ccb148
  temp="$(mktemp -d "${TMPDIR:-/tmp}/agena-homebrew.XXXXXX")" || return
  if ! agena_setup_download "https://raw.githubusercontent.com/Homebrew/install/$commit/install.sh" "$temp/install.sh" ||
     ! agena_setup_verify_download "$temp/install.sh" "$checksum"; then
    rm -rf "$temp"; return 1
  fi
  agena_setup_note "Installing Homebrew with its verified official installer; review its installation plan and administrator prompt."
  if ( : </dev/tty ) >/dev/null 2>&1; then
    if ! (unset CI NONINTERACTIVE; INTERACTIVE=1 /bin/bash "$temp/install.sh" </dev/tty); then
      rm -rf "$temp"; agena_setup_error "The Homebrew installer did not complete."; return 1
    fi
  else
    if ! (unset INTERACTIVE; NONINTERACTIVE=1 /bin/bash "$temp/install.sh"); then
      rm -rf "$temp"
      agena_setup_error "Homebrew installation failed without a terminal. Run the same command in a terminal for its confirmation and sudo authentication, or prepare passwordless administrator access."
      return 1
    fi
  fi
  rm -rf "$temp"
  agena_setup_find_homebrew ||
    { agena_setup_error "Homebrew is still unavailable after its installer completed."; return 1; }
}

agena_setup_openssl() {
  local mode="$1" os="$2"
  [[ "$os" == Linux ]] || return 0
  if pkg-config --exists openssl; then return 0; fi
  agena_setup_note "OpenSSL development headers/libraries are missing from pkg-config."
  agena_setup_packages "$mode" "$os" openssl-dev || return
  pkg-config --exists openssl ||
    { agena_setup_error "OpenSSL development files are still unavailable to pkg-config after installation."; return 1; }
}

agena_setup_macos_clt() {
  local mode="$1" timeout="${AGENA_SETUP_CLT_TIMEOUT_SECONDS:-1800}" deadline
  if xcode-select -p >/dev/null 2>&1; then return 0; fi
  [[ "$mode" == auto ]] ||
    { agena_setup_error "Apple Command Line Tools are missing; rerun with --setup-mode auto."; return 1; }
  [[ "$timeout" =~ ^[0-9]+$ && ${#timeout} -le 4 ]] ||
    { agena_setup_error "AGENA_SETUP_CLT_TIMEOUT_SECONDS must be an integer from 1 to 3600."; return 1; }
  timeout=$((10#$timeout))
  (( timeout >= 1 && timeout <= 3600 )) ||
    { agena_setup_error "AGENA_SETUP_CLT_TIMEOUT_SECONDS must be an integer from 1 to 3600."; return 1; }
  if ! xcode-select --install; then
    xcode-select -p >/dev/null 2>&1 ||
      { agena_setup_error "Apple could not open its Command Line Tools installer."; return 1; }
  fi
  agena_setup_note "Complete the Apple Command Line Tools dialog. Waiting up to $timeout seconds; this installation continues automatically when the tools are ready."
  deadline=$((SECONDS + timeout))
  while ! xcode-select -p >/dev/null 2>&1; do
    if (( SECONDS >= deadline )); then
      agena_setup_error "Apple Command Line Tools installation did not finish within $timeout seconds. Complete the system dialog, then rerun this command."
      return 1
    fi
    sleep 1 || return
  done
}

agena_setup_macos() {
  local phase="$1" mode="$2" version major
  version="$(sw_vers -productVersion)" || return
  major="${version%%.*}"
  [[ "$major" =~ ^[0-9]+$ ]] || { agena_setup_error "Cannot determine the macOS version: $version"; return 1; }
  agena_setup_note "macOS $version"
  if [[ "$phase" == source ]]; then
    if [[ "${AGENA_WEB_DIST_PREBUILT:-0}" != 1 ]] && (( major < 13 )); then
      agena_setup_error "The pinned Bun build requires macOS 13 or later."
      return 1
    fi
    agena_setup_macos_clt "$mode" || return
  else
    agena_setup_note "Desktop permissions are granted in System Settings > Privacy & Security > Accessibility, Screen & System Audio Recording (Screen Recording on older macOS), and Files and Folders / Full Disk Access, for the app that launches Agena."
  fi
}

agena_setup_bootstrap() {
  local mode="${1:-auto}" source_mode="${2:-0}" os
  agena_setup_mode "$mode" || return
  [[ "$source_mode" == 0 || "$source_mode" == 1 ]] ||
    { agena_setup_error "source_mode must be 0 or 1."; return 1; }
  [[ "$mode" == check ]] || agena_setup_user || return
  os="$(uname -s)"
  case "$os" in Linux|Darwin) ;; *) agena_setup_error "Unsupported Unix host: $os"; return 1 ;; esac
  if [[ "$os" == Darwin && "$source_mode" == 1 ]]; then
    agena_setup_macos source "$mode" || return
  fi
  local missing=() name
  for name in curl python3 tar; do agena_setup_has "$name" || missing+=("$name"); done
  if ! agena_setup_has sha256sum && ! agena_setup_has shasum; then missing+=(sha256); fi
  if [[ "$source_mode" == 1 ]] && ! agena_setup_has git; then missing+=(git); fi
  if [[ "$os" == Linux ]] && ! agena_setup_ca_ready; then missing+=(ca-certificates); fi
  if (( ${#missing[@]} )); then agena_setup_packages "$mode" "$os" "${missing[@]}" || return; fi
  agena_setup_require_commands check "$os" curl python3 tar || return
  if ! agena_setup_has sha256sum && ! agena_setup_has shasum; then
    agena_setup_error "A SHA256 tool is still unavailable after preparation."; return 1
  fi
  if [[ "$source_mode" == 1 ]]; then agena_setup_require_commands check "$os" git || return; fi
  if [[ "$os" == Linux ]] && ! agena_setup_ca_ready; then
    agena_setup_error "No readable system CA certificate bundle was found after preparation."; return 1
  fi
}

agena_setup_download() {
  curl --proto '=https' --proto-redir '=https' --tlsv1.2 -fL --retry 3 \
    --connect-timeout 15 --max-time 600 --output "$2" "$1"
}

agena_setup_verify_download() {
  local file="$1" expected="$2" actual
  [[ "$expected" =~ ^[0-9a-fA-F]{64}$ ]] ||
    { agena_setup_error "The official download checksum is missing or malformed."; return 1; }
  actual="$(agena_setup_sha256 "$file")" || return
  expected="$(printf '%s' "$expected" | tr '[:upper:]' '[:lower:]')"
  [[ "$actual" == "$expected" ]] ||
    agena_setup_error "SHA256 verification failed for $(basename "$file"); the downloaded artifact was not executed."
}

agena_setup_rust() {
  local mode="$1" source_root="$2" os="$3" version cargo_bin target temp base expected
  version="${AGENA_STABLE_TOOLCHAIN:-}"
  if [[ -z "$version" && -r "$source_root/rust-toolchain.toml" ]]; then
    version="$(sed -nE 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' "$source_root/rust-toolchain.toml" | head -1)"
  fi
  version="${version:-1.97.0}"
  [[ "$version" =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ ]] ||
    { agena_setup_error "Invalid Rust toolchain pin: $version"; return 1; }
  target="$(agena_setup_target_triple)" || return
  if [[ "$target" == *-linux-musl ]]; then
    case "$version" in
      *-unknown-linux-*|*-apple-darwin)
        [[ "$version" == *-"$target" ]] ||
          { agena_setup_error "Rust toolchain $version does not match native musl host $target."; return 1; } ;;
      *) version="$version-$target" ;;
    esac
  fi
  cargo_bin="${CARGO_HOME:-$HOME/.cargo}/bin"
  if [[ -x "$cargo_bin/rustup" ]]; then export PATH="$cargo_bin:$PATH"; fi
  if agena_setup_has rustup && rustup which --toolchain "$version" rustc >/dev/null 2>&1 &&
     rustup which --toolchain "$version" cargo >/dev/null 2>&1; then
    export RUSTUP_TOOLCHAIN="$version"
    return 0
  fi
  [[ "$mode" == auto ]] ||
    { agena_setup_error "Rust toolchain $version is missing. Install it with rustup or rerun with --setup-mode auto."; return 1; }
  if ! agena_setup_has rustup; then
    temp="$(mktemp -d "${TMPDIR:-/tmp}/agena-rustup.XXXXXX")" || return
    base="https://static.rust-lang.org/rustup/dist/$target/rustup-init"
    if ! agena_setup_download "$base" "$temp/rustup-init" ||
       ! agena_setup_download "$base.sha256" "$temp/rustup-init.sha256"; then
      rm -rf "$temp"; return 1
    fi
    expected="$(awk 'NR==1 {print $1}' "$temp/rustup-init.sha256")"
    if ! agena_setup_verify_download "$temp/rustup-init" "$expected" ||
       ! chmod +x "$temp/rustup-init" ||
       ! "$temp/rustup-init" -y --no-modify-path --profile minimal --default-toolchain none; then
      rm -rf "$temp"; return 1
    fi
    rm -rf "$temp"
    export PATH="$cargo_bin:$PATH"
  fi
  rustup toolchain install "$version" --profile minimal --component rustfmt --component clippy || return
  rustup which --toolchain "$version" cargo >/dev/null || return
  rustup which --toolchain "$version" rustc >/dev/null || return
  export RUSTUP_TOOLCHAIN="$version"
}

agena_setup_bun() {
  local mode="$1" source_root="$2" os="$3" version directory current target temp base expected libc=gnu
  [[ "${AGENA_WEB_DIST_PREBUILT:-0}" != 1 ]] || return 0
  [[ -r "$source_root/.bun-version" ]] ||
    { agena_setup_error "Source checkout is missing its .bun-version pin."; return 1; }
  version="$(tr -d '[:space:]' < "$source_root/.bun-version")"
  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
    { agena_setup_error "Invalid Bun pin in $source_root/.bun-version"; return 1; }
  if agena_setup_has bun && [[ "$(bun --version)" == "$version" ]]; then return 0; fi
  directory="${XDG_CACHE_HOME:-$HOME/.cache}/agena/toolchains/bun-$version/bin"
  if [[ -x "$directory/bun" && "$("$directory/bun" --version)" == "$version" ]]; then
    export PATH="$directory:$PATH"; return 0
  fi
  [[ "$mode" == auto ]] ||
    { agena_setup_error "Bun $version is required by .bun-version; rerun with --setup-mode auto."; return 1; }
  case "$os:$(uname -m)" in
    Linux:x86_64|Linux:amd64) target=bun-linux-x64-baseline ;;
    Linux:aarch64|Linux:arm64) target=bun-linux-aarch64 ;;
    Darwin:x86_64|Darwin:amd64) target=bun-darwin-x64 ;;
    Darwin:aarch64|Darwin:arm64) target=bun-darwin-aarch64 ;;
    *) agena_setup_error "No supported Bun artifact for $os/$(uname -m)."; return 1 ;;
  esac
  if [[ "$os" == Linux ]]; then
    libc="$(agena_setup_libc)" || return
    if [[ "$libc" == musl ]]; then
      # These are the actual pinned Bun release asset names, not GNU builds.
      case "$target" in
        bun-linux-x64-baseline) target=bun-linux-x64-musl-baseline ;;
        bun-linux-aarch64) target=bun-linux-aarch64-musl ;;
      esac
    fi
  fi
  temp="$(mktemp -d "${TMPDIR:-/tmp}/agena-bun.XXXXXX")" || return
  base="https://github.com/oven-sh/bun/releases/download/bun-v$version"
  if ! agena_setup_download "$base/$target.zip" "$temp/$target.zip" ||
     ! agena_setup_download "$base/SHASUMS256.txt" "$temp/SHASUMS256.txt"; then
    rm -rf "$temp"; return 1
  fi
  expected="$(awk -v name="$target.zip" '$2 == name || $2 == "*" name {print $1}' "$temp/SHASUMS256.txt")"
  if ! agena_setup_verify_download "$temp/$target.zip" "$expected" ||
     ! unzip -p "$temp/$target.zip" "$target/bun" > "$temp/bun" ||
     ! chmod +x "$temp/bun"; then
    rm -rf "$temp"; return 1
  fi
  if [[ "$("$temp/bun" --version)" != "$version" ]]; then
    agena_setup_error "The downloaded Bun executable does not match pin $version."
    rm -rf "$temp"; return 1
  fi
  if ! mkdir -p "$directory" || ! install -m 0755 "$temp/bun" "$directory/bun"; then
    rm -rf "$temp"; return 1
  fi
  rm -rf "$temp"
  export PATH="$directory:$PATH"
}

agena_setup_absolute_path() {
  local path="$1" parent name
  [[ "$path" == /* ]] || { agena_setup_error "An absolute installation path is required."; return 1; }
  if [[ -d "$path" ]]; then (cd "$path" && pwd -P); return; fi
  parent="$(agena_setup_absolute_path "$(dirname "$path")")" || return
  name="$(basename "$path")"
  case "$name" in .) printf '%s' "$parent" ;; ..) dirname "$parent" ;; *) printf '%s/%s' "${parent%/}" "$name" ;; esac
}

agena_setup_managed_file() {
  local target="$1" body="$2" mode="$3" existing="" temp
  [[ ! -L "$target" ]] || { agena_setup_error "Refusing a symlink at system configuration $target"; return 1; }
  if [[ -e "$target" ]]; then
    [[ -f "$target" && -r "$target" ]] ||
      { agena_setup_error "Cannot inspect system configuration $target"; return 1; }
    existing="$(cat "$target")"
    [[ "$existing" == "$body" ]] && return 0
    [[ "$existing" == '# Managed by Agena installer.'$'\n'* ]] ||
      { agena_setup_error "Preserving existing administrator configuration: $target"; return 1; }
  fi
  [[ "$mode" == auto ]] ||
    { agena_setup_error "Required Agena host configuration is missing or outdated: $target"; return 1; }
  temp="$(mktemp "${TMPDIR:-/tmp}/agena-host-config.XXXXXX")" || return
  if ! printf '%s\n' "$body" > "$temp"; then
    rm -f "$temp"
    agena_setup_error "Could not prepare system configuration: $target"
    return 1
  fi
  if [[ ! -d "$(dirname "$target")" ]] &&
     ! agena_setup_as_root install -d -m 0755 "$(dirname "$target")"; then
    rm -f "$temp"; return 1
  fi
  if ! agena_setup_as_root install -o root -g root -m 0644 "$temp" "$target"; then
    rm -f "$temp"; return 1
  fi
  rm -f "$temp"
}

agena_setup_apparmor() {
  local mode="$1" install_dir="$2" restriction executable digest name profile body active
  restriction="$(agena_setup_system_path /proc/sys/kernel/apparmor_restrict_unprivileged_userns)"
  [[ -r "$restriction" && "$(cat "$restriction")" == 1 ]] || return 0
  if [[ ! -r "$(agena_setup_system_path /etc/apparmor.d/abi/4.0)" ||
        ! -r "$(agena_setup_system_path /etc/apparmor.d/tunables/global)" ]]; then
    agena_setup_packages "$mode" Linux apparmor-policy || return
    if [[ ! -r "$(agena_setup_system_path /etc/apparmor.d/abi/4.0)" ||
          ! -r "$(agena_setup_system_path /etc/apparmor.d/tunables/global)" ]]; then
      agena_setup_error "AppArmor ABI 4.0/tunables are unavailable after installing distribution policy files."
      return 1
    fi
  fi
  executable="$(agena_setup_absolute_path "$install_dir")/bin/agena" || return
  # Attachment paths are exact. Never turn an install directory into a glob.
  case "$executable" in
    *'*'*|*'?'*|*'['*|*']'*|*'{'*|*'}'*|*'"'*|*'\'*|*$'\n'*|*$'\r'*|*$'\t'*)
      agena_setup_error "The AppArmor attachment path contains unsupported pattern/control characters: $executable"; return 1 ;;
  esac
  digest="$(printf '%s' "$executable" | agena_setup_sha256)" || return
  [[ "$digest" =~ ^[0-9a-fA-F]{64}$ ]] ||
    { agena_setup_error "Cannot compute the AppArmor attachment identity."; return 1; }
  name="agena-userns-${digest:0:24}"
  profile="$(agena_setup_system_path "/etc/apparmor.d/$name")"
  body="$(printf '# Managed by Agena installer.\nabi <abi/4.0>,\ninclude <tunables/global>\n\nprofile %s "%s" flags=(unconfined) {\n  userns,\n}\n' "$name" "$executable")"
  agena_setup_managed_file "$profile" "$body" "$mode" || return
  if [[ "$mode" == auto ]]; then
    agena_setup_as_root apparmor_parser -r "$profile" || {
      agena_setup_error "Loading the Agena-specific AppArmor profile failed: $profile"
      return 1
    }
  else
    active="$(agena_setup_system_path /sys/kernel/security/apparmor/profiles)"
    if [[ -r "$active" ]] && ! grep -Fq "$name (" "$active"; then
      agena_setup_error "The Agena AppArmor profile exists but is not loaded: $profile"; return 1
    fi
  fi
  agena_setup_note "AppArmor user-namespace profile: $profile (attachment: $executable)"
}

agena_setup_module_boot() {
  local mode="$1" service runlevel enabled=0
  service="$(agena_setup_system_path /etc/init.d/modules)"
  # systemd-modules-load reads modules-load.d directly. Alpine's OpenRC
  # modules service reads the same files, but must be in a boot runlevel.
  if [[ -f "$service" ]]; then
    for runlevel in boot sysinit default; do
      if [[ -e "$(agena_setup_system_path "/etc/runlevels/$runlevel/modules")" ]]; then enabled=1; fi
    done
    if (( ! enabled )); then
      [[ "$mode" == auto ]] ||
        { agena_setup_error "OpenRC modules service is not enabled for boot."; return 1; }
      agena_setup_has rc-update ||
        { agena_setup_error "OpenRC modules service requires rc-update to enable boot-time loading."; return 1; }
      agena_setup_as_root rc-update add modules boot || return
    fi
  fi
}

agena_setup_system_commands_path() {
  local directory
  for directory in /usr/local/sbin /usr/sbin /sbin; do
    directory="$(agena_setup_system_path "$directory")"
    if [[ -d "$directory" && ":$PATH:" != *":$directory:"* ]]; then
      export PATH="$PATH:$directory"
    fi
  done
}

agena_setup_linux_runtime() {
  local mode="$1" install_dir="$2" name version path body
  local required=(bwrap slirp4netns ip unshare nsenter iptables ip6tables iptables-restore ip6tables-restore modprobe)
  path="$(agena_setup_system_path /proc/sys/kernel/apparmor_restrict_unprivileged_userns)"
  if [[ -r "$path" && "$(cat "$path")" == 1 ]]; then required+=(apparmor_parser); fi
  agena_setup_require_commands "$mode" Linux "${required[@]}" || return
  for name in iptables ip6tables iptables-restore ip6tables-restore; do
    version="$("$name" --version)" || return
    if [[ "$version" == *legacy* ]]; then
      agena_setup_error "$name uses the legacy backend ($version). MXC requires an nft-compatible iptables toolchain. Ask your administrator to review the existing firewall and select the appropriate backend; Agena does not change firewall rules or alternatives."
      return 1
    fi
    if [[ "$version" != *nf_tables* ]]; then
      agena_setup_error "$name does not report the required nft backend: $version. Review the system iptables installation; Agena does not change firewall alternatives."
      return 1
    fi
  done
  for path in /proc/sys/kernel/unprivileged_userns_clone /proc/sys/user/max_user_namespaces; do
    path="$(agena_setup_system_path "$path")"
    if [[ -r "$path" && "$(cat "$path")" == 0 ]]; then
      agena_setup_error "User namespaces are disabled by $path. An administrator must review this policy; Agena does not weaken global namespace restrictions."
      return 1
    fi
  done
  path="$(agena_setup_system_path /sys/module/nf_conntrack)"
  if [[ ! -d "$path" && ! -r "$(agena_setup_system_path /proc/sys/net/netfilter/nf_conntrack_max)" ]]; then
    [[ "$mode" == auto ]] ||
      { agena_setup_error "nf_conntrack is not available; rerun with --setup-mode auto to load the required module."; return 1; }
    agena_setup_as_root modprobe nf_conntrack || {
      agena_setup_error "Cannot load nf_conntrack. The kernel/container host must provide it for MXC networking."
      return 1
    }
    if [[ ! -d "$path" && ! -r "$(agena_setup_system_path /proc/sys/net/netfilter/nf_conntrack_max)" ]]; then
      agena_setup_error "nf_conntrack is still unavailable after modprobe."; return 1
    fi
  fi
  # A module already loaded by another process may disappear after reboot.
  # Persist the requirement even when no modprobe was necessary this time.
  body=$'# Managed by Agena installer.\nnf_conntrack'
  agena_setup_managed_file "$(agena_setup_system_path /etc/modules-load.d/agena-sandbox.conf)" "$body" "$mode" || return
  agena_setup_module_boot "$mode" || return
  agena_setup_apparmor "$mode" "$install_dir"
}

agena_setup_host() {
  local phase="${1:-runtime}" mode="${2:-auto}" install_dir="${3:-}" source_root="${4:-}" os
  agena_setup_mode "$mode" || return
  case "$phase" in source|runtime) ;; *) agena_setup_error "Host setup phase must be source or runtime."; return 1 ;; esac
  [[ "$mode" == check ]] || agena_setup_user || return
  [[ "$install_dir" == /* ]] || { agena_setup_error "Host setup requires an absolute installation directory."; return 1; }
  os="$(uname -s)"
  case "$os" in Linux|Darwin) ;; *) agena_setup_error "Unsupported Unix host: $os"; return 1 ;; esac
  if [[ "$os" == Linux ]]; then agena_setup_system_commands_path; fi
  if [[ "$phase" == source ]]; then
    [[ -d "$source_root" ]] || { agena_setup_error "Source setup requires an existing checkout."; return 1; }
    if [[ "$os" == Darwin ]]; then agena_setup_macos source "$mode" || return; fi
    local build_tools=(cc c++ make cmake pkg-config python3 git curl tar)
    if [[ "${AGENA_WEB_DIST_PREBUILT:-0}" != 1 ]]; then build_tools+=(unzip); fi
    agena_setup_require_commands "$mode" "$os" "${build_tools[@]}" || return
    agena_setup_openssl "$mode" "$os" || return
    agena_setup_rust "$mode" "$source_root" "$os" || return
    agena_setup_bun "$mode" "$source_root" "$os" || return
  elif [[ "$os" == Linux ]]; then
    agena_setup_linux_runtime "$mode" "$install_dir" || return
  else
    agena_setup_macos runtime "$mode" || return
  fi
  agena_setup_note "$phase host prerequisites are ready; the installed binary will verify actual sandbox execution."
}
# END AGENA HOST SETUP

absolute_directory() {
  local path="$1" parent name
  [[ -n "$path" ]] || { echo "ERROR: directory must not be empty" >&2; return 2; }
  if [[ -d "$path" ]]; then
    (cd "$path" && pwd -P)
    return
  fi
  parent="$(absolute_directory "$(dirname "$path")")" || return
  name="$(basename "$path")"
  case "$name" in
    .) printf '%s' "$parent" ;;
    ..) dirname "$parent" ;;
    *) printf '%s/%s' "${parent%/}" "$name" ;;
  esac
}

validate_storage_paths() {
  INSTALL_DIR="$(absolute_directory "$INSTALL_DIR")"
  local account_home
  account_home="$(absolute_directory "$HOME")"
  case "$INSTALL_DIR" in
    /|"$account_home"|/bin|/usr|/usr/local|/etc|/var|/opt|/tmp)
      echo "ERROR: unsafe application install directory: $INSTALL_DIR" >&2; return 2 ;;
  esac
  # Lifecycle actions select the installed data root from their manifest.
  # Do not interpret an unrelated inherited AGENA_DATA_DIR before loading it.
  [[ "${1:-}" == "install-only" ]] && return 0
  DATA_DIR="$(absolute_directory "$DATA_DIR")"
  case "$DATA_DIR" in
    /|"$account_home"|/bin|/usr|/usr/local|/etc|/var|/opt|/tmp)
      echo "ERROR: unsafe instance data directory: $DATA_DIR" >&2; return 2 ;;
  esac
  if [[ "$INSTALL_DIR/" == "$DATA_DIR/"* || "$DATA_DIR/" == "$INSTALL_DIR/"* ]]; then
    echo "ERROR: application installation and instance data directories must be separate" >&2
    return 2
  fi
}

if [[ "$ACTION" == "install" ]]; then
  validate_storage_paths
else
  validate_storage_paths install-only
fi

STATE_FILE="$INSTALL_DIR/install-state.env"
BIN_DIR="$INSTALL_DIR/bin"
AGENA_BIN="$BIN_DIR/agena"
WEB_DIR="$INSTALL_DIR/web-dist"
CLIENT_ENV_FILE="$INSTALL_DIR/client-env.sh"
PATH_MARKER_BEGIN="# >>> agena installer >>>"
PATH_MARKER_END="# <<< agena installer <<<"

shell_quote() {
  printf '%q' "$1"
}

write_state() {
  mkdir -p "$INSTALL_DIR"
  umask 077
  {
    printf 'INSTALL_STATE_SCHEMA=2\n'
    printf 'REPO=%s\n' "$(shell_quote "$REPO")"
    printf 'VERSION=%s\n' "$(shell_quote "$VERSION")"
    printf 'RELEASE_VERSION=%s\n' "$(shell_quote "$RELEASE_VERSION")"
    printf 'INSTALL_DIR=%s\n' "$(shell_quote "$INSTALL_DIR")"
    printf 'DATA_DIR=%s\n' "$(shell_quote "$DATA_DIR")"
    printf 'HOST=%s\n' "$(shell_quote "$HOST")"
    printf 'PORT=%s\n' "$(shell_quote "$PORT")"
    printf 'WORKSPACE=%s\n' "$(shell_quote "$WORKSPACE")"
    printf 'SERVER_PASSWORD=%s\n' "$(shell_quote "$SERVER_PASSWORD")"
    printf 'SERVICE_MODE=%s\n' "$(shell_quote "$SERVICE_MODE")"
    printf 'UPDATE_PATH=%s\n' "$(shell_quote "$UPDATE_PATH")"
  } > "$STATE_FILE"
  chmod 600 "$STATE_FILE"
}

write_client_env() {
  mkdir -p "$INSTALL_DIR"
  umask 077
  {
    printf '# Generated by the Agena installer. Source this file from your shell profile.\n'
    printf 'export AGENA_DATA_DIR=%s\n' "$(shell_quote "$DATA_DIR")"
    if [[ -n "$SERVER_PASSWORD" ]]; then
      printf 'export AGENA_SERVER_PASSWORD=%s\n' "$(shell_quote "$SERVER_PASSWORD")"
    fi
  } > "$CLIENT_ENV_FILE"
  chmod 600 "$CLIENT_ENV_FILE"
}

load_state() {
  if [[ ! -f "$STATE_FILE" ]]; then
    echo "ERROR: Agena is not installed at $INSTALL_DIR" >&2
    exit 1
  fi
  local selected_install_dir="$INSTALL_DIR" stored_data_dir
  unset INSTALL_STATE_SCHEMA DATA_DIR RELEASE_VERSION SERVER_PASSWORD
  # shellcheck disable=SC1090
  source "$STATE_FILE"
  if (( NO_PATH_UPDATE_SET )); then
    UPDATE_PATH=0
  fi
  if [[ "${INSTALL_STATE_SCHEMA:-}" != "2" || -z "${DATA_DIR:-}" || "${SERVER_PASSWORD+x}" != "x" ]]; then
    echo "ERROR: installer state is incompatible; use a new installation directory and an explicit --data-dir" >&2
    exit 1
  fi
  stored_data_dir="$DATA_DIR"
  validate_storage_paths
  if [[ "$INSTALL_DIR" != "$selected_install_dir" || "$DATA_DIR" != "$stored_data_dir" ]]; then
    echo "ERROR: installer state resolves to a different installation or data root" >&2
    exit 1
  fi
  BIN_DIR="$INSTALL_DIR/bin"
  AGENA_BIN="$BIN_DIR/agena"
  WEB_DIR="$INSTALL_DIR/web-dist"
  CLIENT_ENV_FILE="$INSTALL_DIR/client-env.sh"
}

normalize_version() {
  local raw="$1"
  raw="${raw#agena-v}"
  raw="${raw#v}"
  printf '%s' "$raw"
}

download_file() {
  local source="$1"
  local destination="$2"
  if [[ "$source" =~ ^https?:// ]]; then
    if command -v curl >/dev/null 2>&1; then
      curl -fL --retry 3 --retry-delay 1 "$source" -o "$destination"
    elif command -v wget >/dev/null 2>&1; then
      wget -qO "$destination" "$source"
    else
      echo "ERROR: curl or wget is required" >&2
      exit 1
    fi
  else
    cp "$source" "$destination"
  fi
}

latest_version() {
  local releases tag
  if command -v curl >/dev/null 2>&1; then
    releases="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases?per_page=100")"
  elif command -v wget >/dev/null 2>&1; then
    releases="$(wget -qO- "https://api.github.com/repos/${REPO}/releases?per_page=100")"
  else
    echo "ERROR: curl or wget is required" >&2
    exit 1
  fi
  tag="$(printf '%s\n' "$releases" \
    | sed -En 's/.*"tag_name"[[:space:]]*:[[:space:]]*"(agena-v[0-9]+\.[0-9]+\.[0-9]+-beta\.[1-9][0-9]*)".*/\1/p' \
    | head -1)"
  if [[ -z "$tag" ]]; then
    echo "ERROR: no Agena beta release was found for ${REPO}" >&2
    exit 1
  fi
  normalize_version "$tag"
}

target_triple() {
  agena_setup_target_triple
}

is_source_root() {
  local path="$1"
  [[ -f "$path/Cargo.toml" &&
     -f "$path/packages/agena-web/package.json" &&
     -f "$path/scripts/agena/package-backend.sh" ]]
}

find_source_root() {
  local current
  current="$(cd "$1" 2>/dev/null && pwd -P)" || return 1
  while :; do
    if is_source_root "$current"; then
      printf '%s' "$current"
      return 0
    fi
    [[ "$current" == "/" ]] && break
    current="$(dirname "$current")"
  done
  return 1
}

installer_source_root() {
  local script="${BASH_SOURCE[0]:-}" directory
  [[ -n "$script" && -f "$script" ]] || return 1
  directory="$(cd "$(dirname "$script")" && pwd -P)" || return 1
  find_source_root "$(cd "$directory/../.." && pwd -P)"
}

is_git_source() {
  case "$1" in
    http://*|https://*|ssh://*|git://*|file://*|git@*:*) return 0 ;;
    *) return 1 ;;
  esac
}

clone_source() {
  local repository="$1" destination="$2"
  if ! command -v git >/dev/null 2>&1; then
    echo "ERROR: git is required to install from a remote source repository" >&2
    return 1
  fi
  local -a clone_args=(clone --depth 1)
  if [[ -n "$SOURCE_REF" ]]; then
    clone_args+=(--branch "$SOURCE_REF")
  fi
  echo "Cloning Agena source from $repository" >&2
  git "${clone_args[@]}" "$repository" "$destination"
}

resolve_source_root() {
  SOURCE_ROOT=""
  local temp="$1" requested="$SOURCE" root
  if [[ -n "$requested" ]]; then
    if [[ -d "$requested" ]]; then
      root="$(find_source_root "$requested")" || {
        echo "ERROR: --source directory is not an Agena checkout: $requested" >&2
        return 1
      }
      SOURCE_ROOT="$root"
      return 0
    fi
    if is_git_source "$requested"; then
      clone_source "$requested" "$temp/source" || return
      root="$(find_source_root "$temp/source")" || {
        echo "ERROR: cloned source is not an Agena checkout: $requested" >&2
        return 1
      }
      SOURCE_ROOT="$root"
      return 0
    fi
    echo "ERROR: --source must name an Agena checkout or a Git URL" >&2
    return 1
  fi
  if root="$(find_source_root "$PWD")"; then
    SOURCE_ROOT="$root"
    return 0
  fi
  if root="$(installer_source_root)"; then
    SOURCE_ROOT="$root"
    return 0
  fi
  clone_source "https://github.com/${REPO}.git" "$temp/source" || return
  root="$(find_source_root "$temp/source")" || {
    echo "ERROR: cloned source is not an Agena checkout" >&2
    return 1
  }
  SOURCE_ROOT="$root"
}

prepare_source_archive() {
  local temp="$1" target="$2" source_root package_script build_log target_flags=""
  resolve_source_root "$temp" || exit 1
  source_root="$SOURCE_ROOT"
  agena_setup_host source "$SETUP_MODE" "$INSTALL_DIR" "$source_root" || return
  if [[ -n "$SOURCE_REF" && "$source_root" != "$temp/source" ]]; then
    echo "WARNING: --source-ref only applies when cloning; using the existing checkout as-is" >&2
  fi
  package_script="$source_root/scripts/agena/package-backend.sh"
  if [[ "$target" == *-linux-musl ]]; then
    # Link the native musl build against the distribution's OpenSSL shared
    # libraries. openssl-dev need not include a separate static-library package.
    target_flags="-C target-feature=-crt-static"
    agena_setup_note "Building native musl with dynamic system OpenSSL; no glibc compatibility layer is installed."
  fi
  echo "Building Agena from source at $source_root"
  build_log="$temp/source-build.log"
  if ! (
    cd "$source_root"
    AGENA_INSTALLER_SETUP_MODE="$SETUP_MODE" AGENA_STABLE_TOOLCHAIN="${RUSTUP_TOOLCHAIN:-1.97.0}" \
      bash "$package_script" "$target" false "$target_flags"
  ) 2>&1 | tee "$build_log"; then
    echo "ERROR: Agena source build failed" >&2
    exit 1
  fi
  SOURCE_ARCHIVE="$(sed -n 's/^Package ready: //p' "$build_log" | tail -1)"
  SOURCE_CHECKSUM="$(sed -n 's/^Checksum ready: //p' "$build_log" | tail -1)"
  [[ -n "$SOURCE_CHECKSUM" ]] || SOURCE_CHECKSUM="${SOURCE_ARCHIVE}.sha256"
  if [[ -z "$SOURCE_ARCHIVE" || ! -f "$SOURCE_ARCHIVE" || ! -f "$SOURCE_CHECKSUM" ]]; then
    echo "ERROR: source build did not produce an Agena archive and checksum" >&2
    exit 1
  fi
  echo "Source package ready: $SOURCE_ARCHIVE"
}

sha256_of() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  else
    shasum -a 256 "$1" | awk '{print $1}'
  fi
}

verify_checksum() {
  local archive_path="$1"
  local checksum_path="$2"
  local expected actual
  expected="$(awk 'NF {print $1; exit}' "$checksum_path" | tr '[:upper:]' '[:lower:]')"
  actual="$(sha256_of "$archive_path" | tr '[:upper:]' '[:lower:]')"
  if [[ ! "$expected" =~ ^[0-9a-f]{64}$ ]]; then
    echo "ERROR: invalid SHA256 file: $checksum_path" >&2
    exit 1
  fi
  if [[ "$expected" != "$actual" ]]; then
    echo "ERROR: SHA256 mismatch for Agena archive" >&2
    echo "expected: $expected" >&2
    echo "actual:   $actual" >&2
    exit 1
  fi
}

managed_profile_files() {
  local zdotdir="${ZDOTDIR:-$HOME}"
  case "${SHELL:-}" in
    */zsh) printf '%s\n' "$zdotdir/.zshenv" "$zdotdir/.zshrc" ;;
    *) printf '%s\n' "$HOME/.profile" ;;
  esac
}

profile_file() {
  managed_profile_files | tail -n 1
}

profile_files() {
  local zdotdir="${ZDOTDIR:-$HOME}"
  printf '%s\n' "$HOME/.profile" "$HOME/.zprofile" "$zdotdir/.zshenv" "$zdotdir/.zshrc" |
    awk '!seen[$0]++'
}

is_managed_profile_file() {
  local candidate="$1" managed
  while IFS= read -r managed; do
    [[ "$candidate" == "$managed" ]] && return 0
  done < <(managed_profile_files)
  return 1
}

remove_managed_block() {
  local profile="$1" temporary
  [[ -f "$profile" ]] || return 0
  grep -Fq "$PATH_MARKER_BEGIN" "$profile" || return 0
  temporary="${profile}.agena.$$"
  awk -v begin="$PATH_MARKER_BEGIN" -v end="$PATH_MARKER_END" '
    $0 == begin { skip=1; next }
    $0 == end { skip=0; next }
    !skip { print }
  ' "$profile" > "$temporary"
  # Write through the original path so dotfile symlinks are preserved.
  cat "$temporary" > "$profile"
  rm -f "$temporary"
}

update_shell_path() {
  (( UPDATE_PATH == 1 )) || return 0
  local profile quoted client_env_quoted
  quoted="$(shell_quote "$BIN_DIR")"
  client_env_quoted="$(shell_quote "$CLIENT_ENV_FILE")"
  while IFS= read -r profile; do
    is_managed_profile_file "$profile" || remove_managed_block "$profile"
  done < <(profile_files)
  while IFS= read -r profile; do
    touch "$profile"
    remove_managed_block "$profile"
    {
      printf '\n%s\n' "$PATH_MARKER_BEGIN"
      printf 'export PATH=%s:"$PATH"\n' "$quoted"
      printf 'if [ -r %s ]; then . %s; fi\n' "$client_env_quoted" "$client_env_quoted"
      printf '%s\n' "$PATH_MARKER_END"
    } >> "$profile"
    echo "Added Agena PATH/client environment setup to $profile"
  done < <(managed_profile_files)
}

remove_shell_path() {
  local profile
  while IFS= read -r profile; do
    remove_managed_block "$profile"
  done < <(profile_files)
}

native_service_available() {
  case "$(uname -s)" in
    Darwin) command -v launchctl >/dev/null 2>&1 ;;
    Linux) command -v systemctl >/dev/null 2>&1 && systemctl --user show-environment >/dev/null 2>&1 ;;
    *) return 1 ;;
  esac
}

resolve_service_mode() {
  if [[ "$SERVICE_MODE" == "auto" ]]; then
    if native_service_available; then
      SERVICE_MODE="native"
    else
      SERVICE_MODE="detached"
      echo "Native user service manager is unavailable; using detached Agena server lifecycle."
    fi
  fi
  if [[ "$SERVICE_MODE" == "native" ]] && ! native_service_available; then
    echo "ERROR: native user service manager is unavailable on this host" >&2
    exit 1
  fi
}

server_config_args() {
  SERVER_ARGS=(
    --host "$HOST"
    --port "$PORT"
    --workspace "$WORKSPACE"
    --ui-dir "$WEB_DIR"
  )
  if [[ -n "$SERVER_PASSWORD" ]]; then
    SERVER_ARGS+=(--password "$SERVER_PASSWORD")
  fi
}

# Every lifecycle call names the data root explicitly so an inherited
# AGENA_DATA_DIR cannot redirect it to a different instance.
agena_lifecycle_at() {
  local data_dir="$1"
  shift
  env \
    -u AGENA_DATA_DIR \
    -u AGENA_SERVER_HOST \
    -u AGENA_SERVER_PORT \
    -u AGENA_SERVER_PASSWORD \
    -u AGENA_MCP_ENABLED \
    -u AGENA_MCP_PUBLIC_URL \
    -u AGENA_MCP_SERVER_URL \
    -u AGENA_MCP_OAUTH_ISSUER_URL \
    -u AGENA_MCP_AUTH_MODE \
    -u AGENA_MCP_ANONYMOUS_ACCESS \
    -u AGENA_MCP_CLIENT_REGISTRATION \
    -u AGENA_WORKSPACE_ROOT \
    -u AGENA_SERVER_UI_DIR \
    "$AGENA_BIN" --data-dir "$data_dir" "$@"
}

agena_lifecycle() {
  agena_lifecycle_at "$DATA_DIR" "$@"
}

server_is_running_at() {
  [[ -x "$AGENA_BIN" ]] || return 1
  local output
  # This is a local lifecycle probe, including when the installed executable
  # predates the client's loopback proxy bypass. Do not pass this override to
  # server start: its child still needs the user's normal internet routing.
  if ! output="$(NO_PROXY='*' no_proxy='*' agena_lifecycle_at "$1" server status 2>&1)"; then
    printf 'ERROR: failed to determine the installed Agena running state:\n%s\n' "$output" >&2
    return 2
  fi
  case "$output" in
    *" is running at "*) return 0 ;;
    *"Agena server is not running"*|*"Agena server user service is installed but not running"*) return 1 ;;
    *) printf 'ERROR: unrecognized Agena server status:\n%s\n' "$output" >&2; return 2 ;;
  esac
}

server_is_running() {
  server_is_running_at "$DATA_DIR"
}

# Stop or deregister the instance described by the given settings. During a
# replacement the global settings already describe the candidate, so the
# current installation passes its own saved values here.
stop_installation() {
  local data_dir="$1" service_mode="$2"
  [[ -x "$AGENA_BIN" ]] || return 0
  if [[ "$service_mode" == "native" ]]; then
    agena_lifecycle_at "$data_dir" server uninstall >/dev/null
  else
    if server_is_running_at "$data_dir"; then
      agena_lifecycle_at "$data_dir" server stop >/dev/null
    else
      local status=$?
      (( status == 1 )) || return "$status"
    fi
  fi
}

start_installed() {
  server_config_args
  if [[ "$SERVICE_MODE" == "native" ]]; then
    agena_lifecycle server install "${SERVER_ARGS[@]}"
  else
    agena_lifecycle server start "${SERVER_ARGS[@]}"
  fi
}

release_package_version() {
  if [[ ! "$1" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-beta\.[1-9][0-9]*$ ]]; then
    echo "ERROR: --version must name a beta release, for example 0.1.0-beta.1" >&2
    exit 2
  fi
  printf '%s' "${BASH_REMATCH[1]}"
}

# Replacement pipeline shared by install, reinstall, and upgrade.
#
#   1. resolve_replacement   read saved state, overlay explicit options
#   2. prepare_candidate     download or build, verify, unpack, probe version
#   3. prepare_host          dependencies and permissions while the old service runs
#   4. activate_candidate    stop the current instance, swap program files
#   5. finalize_replacement  verify sandbox, start, restore run state, save state
#
# A failure in 4 or 5 runs rollback_replacement, which restores the previous
# program files, installer state, settings, and running state. The stages only
# differ in their preconditions: install requires no existing installation,
# while reinstall and upgrade require one.

TEMP_DIR=""
BACKUP_DIR=""
CANDIDATE_DIR=""
CANDIDATE_VERSION=""
CANDIDATE_INSTALLED=0
ACTIVATION_STARTED=0
HAD_STATE=0
OLD_DATA_DIR=""
OLD_SERVICE_MODE=""
OLD_WAS_RUNNING=0

resolve_replacement() {
  local requested_repo="$REPO"
  local requested_version="$VERSION"
  local requested_data_dir="$DATA_DIR"
  local requested_host="$HOST"
  local requested_port="$PORT"
  local requested_workspace="$WORKSPACE"
  local requested_server_password="$SERVER_PASSWORD"
  local requested_service_mode="$SERVICE_MODE"

  case "$ACTION" in
    install)
      if [[ -f "$STATE_FILE" ]]; then
        echo "ERROR: Agena is already installed at $INSTALL_DIR" >&2
        echo "  reinstall  replace it with the installed version" >&2
        echo "  upgrade    install the newest or requested version" >&2
        exit 1
      fi
      if (( SOURCE_MODE )); then
        RELEASE_VERSION=""
      elif (( ARCHIVE_SET )); then
        RELEASE_VERSION="$requested_version"
      else
        RELEASE_VERSION="$requested_version"
      fi
      ;;
    reinstall|upgrade)
      load_state
      OLD_DATA_DIR="$DATA_DIR"
      OLD_SERVICE_MODE="$SERVICE_MODE"
      if server_is_running_at "$OLD_DATA_DIR"; then
        OLD_WAS_RUNNING=1
      else
        local status=$?
        (( status == 1 )) || return "$status"
      fi
      (( REPO_SET )) && REPO="$requested_repo"
      # An upgrade selects a new release; a reinstall repeats the recorded
      # release unless another source or version was requested explicitly.
      RELEASE_VERSION="${RELEASE_VERSION:-}"
      if (( SOURCE_MODE )); then
        RELEASE_VERSION=""
      elif (( ARCHIVE_SET )); then
        RELEASE_VERSION="$requested_version"
        (( VERSION_SET )) || RELEASE_VERSION=""
      elif (( VERSION_SET )); then
        RELEASE_VERSION="$requested_version"
      elif [[ "$ACTION" == "upgrade" ]]; then
        RELEASE_VERSION=""
      fi
      (( DATA_DIR_SET )) && DATA_DIR="$requested_data_dir"
      (( HOST_SET )) && HOST="$requested_host"
      (( PORT_SET )) && PORT="$requested_port"
      (( WORKSPACE_SET )) && WORKSPACE="$requested_workspace"
      (( SERVER_PASSWORD_SET )) && SERVER_PASSWORD="$requested_server_password"
      (( SERVICE_MODE_SET )) && SERVICE_MODE="$requested_service_mode"
      validate_storage_paths
      if [[ "$ACTION" == "reinstall" ]] && (( ! SOURCE_MODE )) &&
         [[ -z "$ARCHIVE" && -z "$RELEASE_VERSION" ]]; then
        echo "ERROR: this installation did not record a release version" >&2
        echo "  reinstall with --archive PATH, --source, or --version VERSION" >&2
        exit 1
      fi
      ;;
  esac

  resolve_service_mode
  mkdir -p "$WORKSPACE"
}

prepare_candidate() {
  local temp="$1"
  local target package_version="" archive_source checksum_source archive_path checksum_path
  target="$(target_triple)"
  if (( SOURCE_MODE )); then
    RELEASE_VERSION=""
    prepare_source_archive "$temp" "$target"
    archive_source="$SOURCE_ARCHIVE"
    checksum_source="$SOURCE_CHECKSUM"
  elif [[ -z "$ARCHIVE" ]]; then
    if [[ "$target" == *-linux-musl ]]; then
      echo "ERROR: Published Agena releases do not include $target. Rerun with --source for a native musl build, or provide a native package with --archive." >&2
      return 1
    fi
    if [[ -z "$RELEASE_VERSION" ]]; then
      RELEASE_VERSION="$(latest_version)"
    else
      RELEASE_VERSION="$(normalize_version "$RELEASE_VERSION")"
    fi
    package_version="$(release_package_version "$RELEASE_VERSION")"
    archive_source="https://github.com/${REPO}/releases/download/agena-v${RELEASE_VERSION}/agena-backend-${target}-v${package_version}.tar.gz"
    checksum_source="${archive_source}.sha256"
  else
    archive_source="$ARCHIVE"
    checksum_source="${CHECKSUM:-${ARCHIVE}.sha256}"
    if [[ -n "$RELEASE_VERSION" ]]; then
      RELEASE_VERSION="$(normalize_version "$RELEASE_VERSION")"
      package_version="$(release_package_version "$RELEASE_VERSION")"
    fi
  fi

  archive_path="$temp/agena.tar.gz"
  checksum_path="$temp/agena.sha256"
  download_file "$archive_source" "$archive_path"
  download_file "$checksum_source" "$checksum_path"
  verify_checksum "$archive_path" "$checksum_path"

  CANDIDATE_DIR="$temp/stage"
  mkdir -p "$CANDIDATE_DIR"
  tar -C "$CANDIDATE_DIR" -xzf "$archive_path"
  if [[ ! -f "$CANDIDATE_DIR/bin/agena" || ! -f "$CANDIDATE_DIR/web-dist/index.html" ]]; then
    echo "ERROR: release archive is missing bin/agena or web-dist/index.html" >&2
    exit 1
  fi
  chmod +x "$CANDIDATE_DIR/bin/agena"
  CANDIDATE_VERSION="$("$CANDIDATE_DIR/bin/agena" --version | awk '{print $NF}' | head -1)"
  if [[ -z "$CANDIDATE_VERSION" ]]; then
    echo "ERROR: installed Agena binary did not report a version" >&2
    exit 1
  fi
  if [[ -n "$package_version" && "$CANDIDATE_VERSION" != "$package_version" ]]; then
    echo "ERROR: archive contains Agena $CANDIDATE_VERSION but release $RELEASE_VERSION contains $package_version" >&2
    exit 1
  fi
  VERSION="$CANDIDATE_VERSION"
}

activate_candidate() {
  local temp="$1"
  BACKUP_DIR="$temp/backup"
  mkdir -p "$BACKUP_DIR"
  if [[ -f "$STATE_FILE" ]]; then
    HAD_STATE=1
    cp "$STATE_FILE" "$BACKUP_DIR/install-state.env"
    if [[ -f "$INSTALL_DIR/sandbox-report.json" ]]; then
      cp -p "$INSTALL_DIR/sandbox-report.json" "$BACKUP_DIR/sandbox-report.json"
    fi
  fi

  if [[ -x "$AGENA_BIN" ]]; then
    echo "Stopping the installed Agena instance"
    stop_installation "$OLD_DATA_DIR" "$OLD_SERVICE_MODE"
  fi

  ACTIVATION_STARTED=1
  mkdir -p "$INSTALL_DIR"
  [[ -d "$BIN_DIR" ]] && mv "$BIN_DIR" "$BACKUP_DIR/bin"
  [[ -d "$WEB_DIR" ]] && mv "$WEB_DIR" "$BACKUP_DIR/web-dist"
  mv "$CANDIDATE_DIR/bin" "$BIN_DIR"
  mv "$CANDIDATE_DIR/web-dist" "$WEB_DIR"
  CANDIDATE_INSTALLED=1
}

verify_installed_sandbox() {
  local report
  echo "Verifying native sandbox filesystem and network access"
  if ! report="$("$AGENA_BIN" --agena-sandbox-check)"; then
    printf 'ERROR: native sandbox verification failed:\n%s\n' "$report" >&2
    echo "Resolve the reported host requirement and rerun this installation command." >&2
    return 1
  fi
  if ! printf '%s\n' "$report" | python3 -c '
import json, sys
try:
    raw = sys.stdin.read(65537)
    if len(raw) > 65536:
        raise ValueError("sandbox report exceeds the size limit")
    report = json.loads(raw)
    if type(report.get("schema_version")) is not int or report["schema_version"] != 1 or report.get("ok") is not True:
        raise ValueError("unsupported or unsuccessful sandbox report")
    checks = report.get("checks")
    if not isinstance(checks, list) or not checks or not all(isinstance(item, dict) for item in checks):
        raise ValueError("sandbox report has no checks")
    by_name = {item.get("name"): item for item in checks}
    if len(by_name) != len(checks) or report.get("platform") not in ("linux", "macos"):
        raise ValueError("sandbox report has invalid platform or duplicate checks")
    for name in ("offline", "http_proxy"):
        check = by_name.get(name, {})
        if check.get("required") is not True or check.get("status") != "passed":
            raise ValueError("required sandbox check did not pass: " + name)
    for check in checks:
        if type(check.get("required")) is not bool or check.get("status") not in ("passed", "failed", "unsupported"):
            raise ValueError("invalid sandbox check result")
        if check["required"] and check["status"] != "passed":
            raise ValueError("required sandbox check failed")
        print("  " + str(check.get("name")) + ": " + str(check["status"]) + " - " + str(check.get("detail", "")))
except (ValueError, TypeError, AttributeError) as error:
    print("ERROR: invalid sandbox verification report: " + str(error), file=sys.stderr)
    sys.exit(1)
'; then
    return 1
  fi
  SANDBOX_REPORT="$report"
}

write_sandbox_report() {
  local temporary
  [[ -n "$SANDBOX_REPORT" ]] || return 1
  temporary="$(umask 077; mktemp "$INSTALL_DIR/.sandbox-report.XXXXXX")" || return
  if ! printf '%s\n' "$SANDBOX_REPORT" > "$temporary" ||
     ! mv -f "$temporary" "$INSTALL_DIR/sandbox-report.json"; then
    rm -f "$temporary"
    return 1
  fi
}

start_candidate() {
  if [[ "$SETUP_MODE" == check ]]; then
    AGENA_MACOS_OPEN_PERMISSIONS=0 start_installed
  else
    start_installed
  fi
}

prepare_macos_privacy() {
  [[ "$(uname -s)" == Darwin ]] || return 0
  if [[ "$SERVICE_MODE" != native ]]; then
    echo "macOS privacy permissions follow the terminal for detached mode; use native service mode for a persistent Agena.app identity."
    return 0
  fi
  local request="${AGENA_MACOS_OPEN_PERMISSIONS:-1}"
  [[ "$SETUP_MODE" != check ]] || request=0
  AGENA_MACOS_PERMISSION_INSTALL=1 AGENA_MACOS_OPEN_PERMISSIONS="$request" \
    agena_lifecycle server permissions
}

finalize_replacement() {
  verify_installed_sandbox || return
  prepare_macos_privacy || return
  if ! start_candidate; then
    echo "ERROR: the replacement Agena version failed to start" >&2
    return 1
  fi
  # A replacement preserves whether the instance was running before.
  if [[ "$ACTION" != "install" ]] && (( OLD_WAS_RUNNING == 0 )); then
    if ! agena_lifecycle server stop >/dev/null; then
      echo "ERROR: the replacement could not return the instance to its stopped state" >&2
      return 1
    fi
  fi
  if ! write_state || ! write_client_env || ! write_sandbox_report; then
    echo "ERROR: the replacement started but its installer state could not be saved" >&2
    return 1
  fi
  update_shell_path || echo "WARNING: Agena is installed, but shell environment update failed" >&2
}

restore_program_dir() {
  local current="$INSTALL_DIR/$1" saved="$BACKUP_DIR/$1"
  [[ -d "$saved" ]] || return 0
  rm -rf "$current"
  mv "$saved" "$current"
}

rollback_replacement() {
  echo "ERROR: restoring the previous Agena installation" >&2
  if [[ -x "$AGENA_BIN" ]]; then
    stop_installation "$DATA_DIR" "$SERVICE_MODE" || true
  fi
  restore_program_dir bin
  restore_program_dir web-dist

  if (( HAD_STATE == 1 )); then
    cp "$BACKUP_DIR/install-state.env" "$STATE_FILE"
    load_state
    if [[ -f "$BACKUP_DIR/sandbox-report.json" ]]; then
      cp -p "$BACKUP_DIR/sandbox-report.json" "$INSTALL_DIR/sandbox-report.json"
    else
      rm -f "$INSTALL_DIR/sandbox-report.json"
    fi
    write_client_env || echo "ERROR: rollback restored state but could not restore the client environment" >&2
    if [[ "$SERVICE_MODE" == "native" ]]; then
      if start_installed; then
        if (( OLD_WAS_RUNNING == 0 )); then
          agena_lifecycle server stop >/dev/null 2>&1 || true
        fi
      else
        echo "ERROR: rollback restored the old files but could not restore the native service" >&2
      fi
    elif (( OLD_WAS_RUNNING == 1 )); then
      start_installed || echo "ERROR: rollback restored the old files but could not restart the old server" >&2
    fi
  else
    if (( CANDIDATE_INSTALLED == 1 )); then
      rm -rf "$BIN_DIR" "$WEB_DIR"
    fi
    rm -f "$STATE_FILE" "$CLIENT_ENV_FILE" "$INSTALL_DIR/sandbox-report.json"
  fi
}

cleanup_replacement() {
  if (( ACTIVATION_STARTED == 1 )); then
    rollback_replacement || true
    ACTIVATION_STARTED=0
  fi
  if [[ -n "$TEMP_DIR" ]]; then
    rm -rf "$TEMP_DIR"
  fi
}

report_replacement() {
  local verb="installed"
  case "$ACTION" in
    reinstall) verb="reinstalled" ;;
    upgrade) verb="upgraded" ;;
  esac
  echo "Agena $VERSION $verb at $INSTALL_DIR"
  echo "Instance data: $DATA_DIR"
  echo "Sandbox verification: $INSTALL_DIR/sandbox-report.json"
  echo "Web UI: http://${HOST}:${PORT}"
  if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo "Open a new shell to load the Agena PATH and client environment."
  elif [[ -n "$SERVER_PASSWORD" && "${AGENA_SERVER_PASSWORD:-}" != "$SERVER_PASSWORD" ]]; then
    echo "Open a new shell (or source $(profile_file)) to load AGENA_SERVER_PASSWORD for the TUI."
  fi
}

run_replacement() {
  resolve_replacement
  TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/agena-install.XXXXXX")"
  trap cleanup_replacement EXIT
  agena_setup_bootstrap "$SETUP_MODE" "$SOURCE_MODE"
  prepare_candidate "$TEMP_DIR"
  agena_setup_host runtime "$SETUP_MODE" "$INSTALL_DIR" "${SOURCE_ROOT:-}"
  activate_candidate "$TEMP_DIR"

  local status=0
  finalize_replacement || status=$?
  if (( status != 0 )); then
    ACTIVATION_STARTED=0
    rollback_replacement
    trap - EXIT
    rm -rf "$TEMP_DIR"
    return "$status"
  fi

  report_replacement
  ACTIVATION_STARTED=0
  trap - EXIT
  rm -rf "$TEMP_DIR"
}

do_start() {
  load_state
  server_config_args
  if [[ "$SERVICE_MODE" == "native" ]]; then
    agena_lifecycle server start
  else
    agena_lifecycle server start "${SERVER_ARGS[@]}"
  fi
}

do_stop() {
  load_state
  agena_lifecycle server stop
}

do_status() {
  load_state
  agena_lifecycle --version
  agena_lifecycle server status
}

do_permissions() {
  load_state
  agena_setup_bootstrap "$SETUP_MODE" 0 || return
  agena_setup_host runtime "$SETUP_MODE" "$INSTALL_DIR" "" || return
  verify_installed_sandbox || return
  write_sandbox_report || return
  if [[ "$(uname -s)" == Darwin ]]; then
    local request="${AGENA_MACOS_OPEN_PERMISSIONS:-1}"
    [[ "$SETUP_MODE" != check ]] || request=0
    AGENA_MACOS_OPEN_PERMISSIONS="$request" agena_lifecycle server permissions
  fi
}

do_uninstall() {
  load_state
  if (( PURGE_DATA == 1 )) && [[ -e "$DATA_DIR" && ! -f "$DATA_DIR/agena.db" ]]; then
    echo "ERROR: refusing to purge a directory without an Agena instance database: $DATA_DIR" >&2
    return 2
  fi
  if [[ -x "$AGENA_BIN" ]]; then
    if [[ "$SERVICE_MODE" == "native" ]]; then
      agena_lifecycle server uninstall >/dev/null
    else
      if server_is_running; then
        agena_lifecycle server stop >/dev/null
      else
        local status=$?
        (( status == 1 )) || return "$status"
      fi
    fi
  fi
  remove_shell_path
  rm -rf "$INSTALL_DIR"
  if (( PURGE_DATA == 1 )); then
    rm -rf "$DATA_DIR"
    echo "Removed Agena runtime data at $DATA_DIR"
  fi
  echo "Agena uninstalled. Configuration/session data was preserved unless --purge-data was used."
}

case "$ACTION" in
  install|reinstall|upgrade) run_replacement ;;
  uninstall) do_uninstall ;;
  start) do_start ;;
  stop) do_stop ;;
  restart) do_stop || true; do_start ;;
  status) do_status ;;
  permissions) do_permissions ;;
esac
