# Install, upgrade, and remove Agena

Agena is distributed as a native executable plus a built Web frontend. The
release installers detect the host architecture, download the matching GitHub
Release archive, verify its SHA-256 checksum, install the executable and Web
assets, and start the server through a native per-user service when one is
available, or through Agena's detached server lifecycle otherwise. A release
install does not need Rust, Bun, or a source checkout.

This guide covers both release installs and installs built from source. The two
paths share the same archive layout, checksum verification, installation state,
startup check, and rollback behavior.

## Supported release targets

Release archives are published for these Rust targets:

| Operating system | Architectures |
| --- | --- |
| Linux | `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu` |
| macOS | `x86_64-apple-darwin`, `aarch64-apple-darwin` |
| Windows | `x86_64-pc-windows-msvc`, `aarch64-pc-windows-msvc` |

The source of truth is
[`scripts/agena/universal-targets.json`](../scripts/agena/universal-targets.json).
The Unix installer accepts x86_64 and arm64/aarch64; the Windows installer
accepts x64 and ARM64.

## Release install prerequisites

The macOS/Linux release path needs Bash, `curl` or `wget`, `tar`, and either
`sha256sum` or `shasum`. The one-line command below uses `curl` to download
the installer itself.

The Windows installer needs PowerShell. It prefers `curl.exe` and `tar.exe`
when present and falls back to `Invoke-WebRequest` and `Expand-Archive`.
No Rust, Bun, Git, or source checkout is required for a release install.

Both paths use the current user's profile and do not require administrator or
root privileges for the default directories and user service.

## Quick install

### macOS and Linux

```bash
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash
```

The installer writes a managed block to the shell startup files so a newly
opened terminal can run `agena`. For zsh it updates `~/.zshenv` and `~/.zshrc`
(honoring `ZDOTDIR`); for other shells it updates `~/.profile`. Open a new
terminal after installation. In zsh, update the current terminal with:

```bash
source "${ZDOTDIR:-$HOME}/.zshrc"
```

For other shells, source `~/.profile` instead.

If `agena` is still not found, the PATH block may not have been loaded, the
shell may not read the updated file, or installation may have used
`--no-path-update`. You can always call the installed binary directly:

```bash
~/.local/share/agena/bin/agena --data-dir ~/agena
```

### Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.ps1 | iex
```

The installer updates the user `Path` and user-level `AGENA_DATA_DIR` unless
`-NoPathUpdate` is supplied. Open a new PowerShell window after installation.
Until then, call `"$env:LOCALAPPDATA\Agena\bin\agena.exe"` directly.

## Defaults

| Setting | macOS/Linux default | Windows default |
| --- | --- | --- |
| Installation root | `~/.local/share/agena` | `%LOCALAPPDATA%\Agena` |
| Instance data root | `~/agena` | `~/agena` |
| Listen address | `127.0.0.1:3210` | `127.0.0.1:3210` |
| Workspace root | `$HOME` | `$HOME` |
| Service mode | `auto` | `auto` (resolved to `native`) |

On macOS/Linux, `auto` selects the native user service when launchd or a
working per-user systemd session is available. Otherwise it uses Agena's
detached server lifecycle. On Windows, `auto` always resolves to the native
Task Scheduler logon task.

Reinstall and upgrade preserve whether the existing instance was running.
On Unix, the installer probes the local server directly, including with older
installed clients, so shell proxy settings cannot make a running service look
stopped. A failed or unrecognized status check stops the replacement before
program files or services are changed; inspect `agena server status` and
start or stop the service explicitly before retrying.

The installation root contains program files. The data root contains the
instance's database, configuration, secrets, workspace data, logs, and client
state. They must be different directories, and neither may contain the other.

## Unix installer options

```text
install.sh [install|reinstall|upgrade|uninstall|start|stop|restart|status|permissions] [options]
```

The `permissions` action is macOS-specific; other Unix hosts report that it is
only needed on macOS.

| Option | Meaning |
| --- | --- |
| `--repo OWNER/REPO` | GitHub repository to download releases from |
| `--version VERSION` | Select a beta release instead of the newest one |
| `--archive PATH_OR_URL` | Install a specific backend archive |
| `--checksum PATH_OR_URL` | Checksum file for `--archive`; defaults to `ARCHIVE.sha256` |
| `--source [PATH_OR_URL]` | Build from a local checkout or clone a Git URL |
| `--source-ref REF` | Branch or tag used when cloning a remote source |
| `--install-dir DIR` | Override the installation root |
| `--data-dir DIR` | Override the instance data root |
| `--host HOST` | Server listen host |
| `--port PORT` | Server listen port |
| `--workspace PATH` | Default workspace root for the server |
| `--password PASSWORD` | Server password shared by Web/TUI login and MCP OAuth |
| `--service-mode MODE` | `auto`, `native`, or `detached` |
| `--no-path-update` | Do not update shell startup files or `AGENA_DATA_DIR` |
| `--purge-data` | With `uninstall`, also remove the saved data root |

The matching environment variables are `AGENA_INSTALL_REPO`,
`AGENA_INSTALL_VERSION`, `AGENA_INSTALL_DIR`, `AGENA_DATA_DIR`,
`AGENA_SERVER_HOST`, `AGENA_SERVER_PORT`, `AGENA_SERVER_PASSWORD`,
`AGENA_WORKSPACE_ROOT`, and `AGENA_INSTALL_SERVICE_MODE`. Command-line options
take precedence over the environment.

## Windows installer options

```powershell
.\scripts\agena\install.ps1 -Action Install
```

The supported actions are `Install`, `Reinstall`, `Upgrade`, `Uninstall`,
`Start`, `Stop`, `Restart`, and `Status`. There is no `Permissions` action on
Windows.

| Parameter | Meaning |
| --- | --- |
| `-Repo OWNER/REPO` | GitHub repository to download releases from |
| `-Version VERSION` | Select a beta release instead of the newest one |
| `-Archive PATH_OR_URL` | Install a specific backend archive |
| `-Checksum PATH_OR_URL` | Checksum file for `-Archive`; defaults to `ARCHIVE.sha256` |
| `-Source` | Build from source |
| `-SourcePath PATH_OR_URL` | Select another checkout or clone a Git URL |
| `-SourceRef REF` | Branch or tag used when cloning a remote source |
| `-InstallDir DIR` | Override the installation root |
| `-DataDir DIR` | Override the instance data root |
| `-ListenHost HOST` | Server listen host |
| `-Port PORT` | Server listen port |
| `-Workspace PATH` | Default workspace root for the server |
| `-ServerPassword PASSWORD` | Server password shared by Web/TUI login and MCP OAuth |
| `-ServiceMode MODE` | `auto`, `native`, or `detached` |
| `-NoPathUpdate` | Do not update the user `Path` or `AGENA_DATA_DIR` |
| `-PurgeData` | With `Uninstall`, also remove the saved data root |

## Installer actions

Each installer action has one job. `install`, `reinstall`, and `upgrade` share
the same preparation, activation, startup check, and rollback pipeline and
differ only in their preconditions and version selection.

| Action | Precondition | Version selection | Runtime settings |
| --- | --- | --- | --- |
| `install` | no installation at the selected root | newest beta; or `--version`, `--archive`, `--source` | flags, otherwise defaults |
| `reinstall` | an installation exists | recorded release; or `--version`, `--archive`, `--source` | saved values; explicit flags override |
| `upgrade` | an installation exists | newest beta; or `--version`, `--archive`, `--source` | saved values; explicit flags override |
| `start`, `stop`, `restart`, `status`, `permissions` | an installation exists | not applicable | saved values |
| `uninstall` | an installation exists | not applicable | saved values |

`install` refuses to modify an existing installation and names `reinstall` and
`upgrade` instead. `reinstall` replaces the program in place: the data root,
workspace, bind address, port, service mode, and password come from the
installation state unless the corresponding option is supplied. A running
instance stays running, and a stopped instance stays stopped.

A release install records the release tag, so `reinstall` can download and
verify the same release again. A source build or a custom archive has no
recorded release tag; reinstall those with `--source`, `--archive`, or
`--version`.

All replacement actions are transactional. The installer prepares and verifies
the package, stops the current instance, swaps `bin/` and `web-dist/`, starts
the replacement, restores the previous running or stopped state, and only then
writes the installation state. A failure while starting or while saving state
restores the previous program files, installer state, data root, and service.

Options are validated against the selected action, so `status --host` or
`stop --archive` fails instead of ignoring the option.

## Installation and data layout

A Unix installation root contains:

```text
<install-dir>/
  bin/agena
  web-dist/
  install-state.env
  client-env.sh
```

A Windows installation root contains `bin\agena.exe`, `web-dist\`, and
`install-state.json`. The state file records the selected install and data
roots, the installed version, the release tag when the package came from a
release, host, port, workspace, service mode, and operator password when one
was supplied. Treat it as private.

The data root uses one storage policy for the server and its local clients:

| Path | Contents |
| --- | --- |
| `agena.db` | SQLite database for sessions, Scheduler state, and server state |
| `agena.json` | User configuration |
| `secrets/` | Application-managed encrypted SecretStore |
| `workspaces/<workspace-key>/` | Per-workspace uploads, memory, and artifacts |
| `cache/` | Derived indexes and caches, including memory indexes |
| `tmp/` | Temporary runtime files |
| `logs/` | Server and tool logs |
| `run/server.json` | Live endpoint record for the selected instance |
| `marketplace/` | Plugin marketplace state |
| `plugin-data/` | Plugin-owned data |
| `clients/<client-key>/` | Local client state |
| `terminal-ui.json` | TUI presentation state |
| `macos/Agena.app` | macOS permission host, when installed |

Program files, Web assets, and client environment files belong to the
installation root. Sessions, configuration, credentials, workspace content,
and logs belong to the data root. Use a separate data root for each independent
server instance.

The installer saves the selected roots. The installer's `start`, `stop`,
`status`, `reinstall`, `upgrade`, and `uninstall` actions read that saved state
and address the same instance even when the invoking shell exports a different
`AGENA_DATA_DIR`. The automatic rollback after a failed replacement uses the
same saved state. Native service definitions pass the data root explicitly.
Service names include a hash of the data root, so stopping or upgrading one
instance does not address another instance's service.

## Beta release selection

The repository and binary versions are `0.1.0`. Releases use tags such as
`agena-v0.1.0-beta.1`; archives contain the package version without the
`agena-v` prefix. With no explicit version, the installer selects the newest
release tag matching `agena-vX.Y.Z-beta.N`.

`--version` / `-Version` accepts the full tag or the version portion:

```bash
bash scripts/agena/install.sh install --version 0.1.0-beta.1
```

```powershell
.\scripts\agena\install.ps1 -Action Install -Version 0.1.0-beta.1
```

Release archives are named
`agena-backend-<target>-v<package-version>.tar.gz` on Unix and `.zip` on
Windows. Each archive has a `.sha256` file next to it.

## Install from source

Source mode builds the native executable and Web frontend, creates the same
archive and checksum used by release installs, and then runs the ordinary
install, startup check, and rollback path.

### Unix

From an Agena checkout:

```bash
cd /path/to/agena
bash scripts/agena/install.sh install --source
```

`--source` without a value resolves the checkout in this order:

1. The nearest checkout containing the current working directory.
2. The checkout containing the installer script.
3. A fresh shallow clone of `https://github.com/<repo>.git`.

When either existing checkout is found, the installer builds it in place and
does not clone another copy. To build a different checkout or clone a remote
repository:

```bash
bash scripts/agena/install.sh install --source /path/to/agena
bash scripts/agena/install.sh install --source https://github.com/canxin121/agena.git
bash scripts/agena/install.sh install --source https://github.com/canxin121/agena.git --source-ref master
```

`--source-ref` only applies when the installer clones a remote source. With an
existing checkout it prints a warning and builds the checkout as it is.

Source mode cannot be combined with `--version`, `--archive`, or `--checksum`.
Use `reinstall --source` to rebuild the current checkout and replace the
existing program, or `upgrade --source` to do the same through the upgrade
action:

```bash
bash scripts/agena/install.sh reinstall --source
bash scripts/agena/install.sh upgrade --source
```

Both actions apply any explicitly supplied runtime settings. Omitted settings
keep their saved values:

```bash
bash scripts/agena/install.sh reinstall --source \
  --host 0.0.0.0 \
  --port 8080 \
  --workspace /path/to/projects
```

The same rule applies to `--data-dir`, `--password`, and `--service-mode`.
The installer stops the saved service, starts the replacement with the new
settings, and writes them to the installation state. If startup fails, it
restores the previous binary, settings, active data root, and service.

On Windows, use `-Action Reinstall` or `-Action Upgrade` with `-ListenHost`,
`-Port`, `-Workspace`, `-DataDir`, `-ServerPassword`, or `-ServiceMode`.

Environment variables are used for a fresh install, but do not override saved
values during `reinstall` or `upgrade`; pass the corresponding option
explicitly. `--data-dir` changes which data root subsequent runs use. Agena
does not copy or migrate the old database and workspace data into the new
root. Changing the program installation root is outside the replacement path;
uninstall while preserving data, then run `install` with the new
`--install-dir`.

A remote clone is temporary and is removed after installation. A local
checkout keeps its build output: Cargo output stays in `target/`, the Web build
stays in `packages/agena-web/dist/`, and the package archive stays in
`artifacts/agena/`.

On Unix, native source installs reuse the same Cargo cache as
`cargo build --release`, including a configured `CARGO_TARGET_DIR`; no extra
host `--target` is added. Repeated `reinstall --source` commands also reuse a
successful Web build when its inputs and complete output match, and reuse a
verified package archive when its contents have not changed. Cargo still runs
to detect Rust changes, and the normal install, verification and activation
steps still run. The build output identifies which stages were reused.

Use `AGENA_WEB_FORCE_BUILD=1 bash scripts/agena/install.sh reinstall --source`
to force the Web checks and build. Cross compilation and build-std retain
their target-specific caches. For an explicitly selected host-target graph,
set `AGENA_FORCE_EXPLICIT_TARGET=1`.

### Windows

```powershell
.\scripts\agena\install.ps1 -Action Install -Source
```

Use `-SourcePath D:\src\agena` for another checkout, or a Git URL such as
`-SourcePath https://github.com/canxin121/agena.git`. Use `-SourceRef master`
when cloning a branch or tag. Source mode cannot be combined with `-Version`,
`-Archive`, or `-Checksum`.

The same checkout resolution order applies: current directory, installer
checkout, then a shallow remote clone. A local checkout keeps its package
archive under `artifacts/agena/`; Windows packaging normally builds in
`.agena-target` under the checkout (or under the CI workspace when one is set).
A temporary remote clone is removed after installation.

### Source build requirements

Source installs require:

- Rust 1.97.0 through `rustup`, or a compatible override through
  `AGENA_STABLE_TOOLCHAIN`.
- Bun 1.3.14 for the Web frontend, unless a prebuilt frontend is supplied.
- Python 3 for the Cargo metadata step in the Unix packaging script.
- Git when cloning a remote repository.
- `tar` and `sha256sum` or `shasum` on macOS/Linux.
- PowerShell archive creation on Windows.
- The platform C/C++ build tools required by Rust dependencies. Windows
  packaging runs `scripts/ci/setup-windows-toolchain.ps1` for Windows targets.

If the Web frontend is already built at
`packages/agena-web/dist/index.html`, set `AGENA_WEB_DIST_PREBUILT=1` to skip
Bun and the frontend build. The build fails if that file is missing.

The source build uses the version from Cargo metadata. To install a specific
branch, set `--source-ref` / `-SourceRef` while cloning.

## Lifecycle operations

### Unix

```bash
# Install the newest beta.
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash

# Reinstall the recorded release, or upgrade to the newest beta.
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- reinstall
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- upgrade

# Inspect, stop, start, or restart the installed server.
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- status
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- stop
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- start
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- restart

# Remove the application while keeping data.
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- uninstall
```

Use the downloaded `install.sh` as a local script for repeated operations. A
piped install works, but it downloads the installer again each time.

### Windows

```powershell
$installer = Join-Path $PWD scripts\agena\install.ps1
& $installer -Action Reinstall
& $installer -Action Upgrade
& $installer -Action Status
& $installer -Action Stop
& $installer -Action Start
& $installer -Action Restart
& $installer -Action Uninstall
```

### Installed binary

The installed executable exposes server lifecycle commands directly:

```text
agena server start
agena server status
agena server stop
agena server install
agena server uninstall
agena server permissions
```

`agena server install` and `agena server uninstall` manage the native per-user
service. `agena server permissions` opens the macOS Full Disk Access setup.

When a user service is installed, `agena server start` starts that service using
its saved configuration, or reports that it is already running. Launch options
and server-related environment variables in the calling shell do not replace
the installed definition or prevent it from starting. Use `agena server install`
with the desired options to update the definition and restart the service.
Without an installed user service, `start` launches a detached server using the
supplied options and environment variables.

Running `agena server` without an action runs the server in the foreground for
development. The installer's `restart` action is a stop followed by a start;
`agena server` itself does not define a `restart` action.

The TUI workspace is independent of the long-lived server process working
directory. `cd /path/to/project && agena` uses `/path/to/project` for new
sessions. `agena tui --workspace /other/path` is the explicit override.

## Server password and client environment

Use **one Server Password** for Web login, TUI/CLI clients and MCP OAuth:

```bash
export AGENA_SERVER_PASSWORD='choose-your-server-password'
```

`agena server --password` accepts the same credential; prefer the environment
variable to keep the secret out of process arguments. MCP has no separate
password. Without a server password, OAuth authorization is unavailable.
The old `--ui-password` flag is accepted as a hidden compatibility alias. Rename
`AGENA_SERVER_UI_PASSWORD` in existing shell configuration to
`AGENA_SERVER_PASSWORD`. If a service still has only the old variable, startup
reports the required rename instead of silently disabling login protection.

On macOS/Linux the installer writes `client-env.sh` under the installation
root, and the managed shell block sources it. The file exports `AGENA_DATA_DIR`
and the configured `AGENA_SERVER_PASSWORD`, so a new shell can run `agena` or
`agena tui` against the protected local server. The password is present in
installer state and in the native service definition because the service needs
it at startup; these files are restricted to the current user.

Web, TUI, CLI, and MCP OAuth use this same server password. Both `agena` and
`agena tui` read `AGENA_SERVER_PASSWORD` automatically. For the local installed
instance, they also read the installer's private `client-env.sh` when the current
shell has no credential, including shells or tmux sessions opened before
installation. The saved data directory and the running server's identity must
match, and this fallback only applies to loopback connections. An explicit
password or token takes precedence. When no applicable credential is available,
the TUI prompts for the server password with input hidden. The entered password
stays in memory for that TUI process, including reauthentication after a server
restart. For non-interactive commands, set the environment variable before
launching the client.

A native user service does **not** read `.zshrc`. Exporting a variable in an
interactive shell configures new commands in that shell, but does not update an
already installed service. From a shell where `AGENA_SERVER_PASSWORD` is set,
run `agena server install` with the deployment's existing `--host`, `--port`,
`--workspace`, and `--ui-dir` options. This updates the saved definition and
restarts the service with the shared password. `agena server start` always
uses the saved definition when a service is installed. Changing or removing
the server password invalidates existing MCP OAuth grants, so connectors must
authorize again; restarting with the same password preserves them.

With `--no-path-update`, the installer does not modify startup files. Set
`AGENA_DATA_DIR` yourself or pass `--data-dir` explicitly.

## macOS file access

Workspaces that span protected folders, other applications' data, external
disks, or network volumes need a one-time **Full Disk Access** grant:

1. Install with the native user service.
2. Run `agena server permissions` (or the installer action `permissions`).
3. In **System Settings → Privacy & Security → Full Disk Access**, choose
   **+**, press **Cmd+Shift+G**, and select the displayed app. The normal path
   is `~/agena/macos/Agena.app`.
4. Enable the app, then run `agena server stop` and `agena server start`.

macOS makes the final access decision. Agena cannot grant the permission to
itself, and a future system reset or manual replacement can require the grant
again. The native service runs through a small signed supervisor inside
`Agena.app`; its identity is preserved across normal upgrades and reinstalls
that keep runtime data. The installer reports a damaged existing app instead
of silently replacing or re-signing it and losing the remembered identity.

Interactive first installs open the permission setup once. Set
`AGENA_MACOS_OPEN_PERMISSIONS=0` for headless installers. Upgrades and restarts
do not reopen settings or probe private data to guess authorization state.

Recursive Web searches and workspace indexing skip macOS privacy domains when
the search root is broad, including `~/Library`, standard protected user
folders, and unselected mounted disks. Direct listing remains available. File
access does not grant Accessibility, Screen Recording, microphone, camera, or
Automation permissions. A foreground `agena server` or detached development
run remains under the launching terminal's privacy identity; use the native
service for the persistent Agena app identity.

Apple documents the code identity constraint in
[TN3127: Inside Code Signing: Requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).

## Custom archives and checksums

Both installers accept a local or remote archive and its checksum:

```bash
bash scripts/agena/install.sh install \
  --archive ./agena-backend-aarch64-apple-darwin-v0.1.0.tar.gz \
  --checksum ./agena-backend-aarch64-apple-darwin-v0.1.0.tar.gz.sha256
```

```powershell
.\scripts\agena\install.ps1 -Action Install `
  -Archive .\agena-backend-x86_64-pc-windows-msvc-v0.1.0.zip `
  -Checksum .\agena-backend-x86_64-pc-windows-msvc-v0.1.0.zip.sha256
```

The archive must contain `bin/agena` (or `bin/agena.exe`) and
`web-dist/index.html`. If `--version` is also supplied, the binary version must
match the package version in the release tag. Without `--version`, the
archive's binary version becomes the installed version.

## Transactional replacement and rollback

`reinstall` and `upgrade` keep the current executable, Web assets, installer
state, and service until the replacement starts successfully. If a replacement
passes download, checksum, archive, and version validation but fails during
startup or while saving state, the installer restores the previous files,
settings, data root, and running state. The same rollback path runs for source
builds and for local archives.

## Uninstall and purge

A normal uninstall removes the installed executable, Web assets, shell PATH
block, and user service. It preserves the selected data root, including
configuration, sessions, credentials, logs, and workspace data.

`--purge-data` on Unix or `-PurgeData` on Windows removes the data root saved in
the installation state as well. Purge refuses a filesystem root, the account
home, fixed protected platform directories (`/bin`, `/usr`, `/usr/local`,
`/etc`, `/var`, `/opt`, and `/tmp` on Unix; system and profile roots on
Windows), overlapping installation/data directories, and a directory without
an `agena.db`. A custom root is purged directly; the default `~/agena` is left
alone when another root was selected. Purge removes configuration and
credentials as well as sessions.

Installer state uses schema 2. An existing state file with another schema is
rejected instead of migrated, and the message asks for a reinstall with an
explicit data root. Existing SQLite databases must match the current Agena
schema; an incompatible database is rejected without being modified.

## Installer verification

The repository verifies installers in three layers:

1. `python3 scripts/agena/tests/test_installer_storage.py` runs offline
   installer contracts with temporary homes, local archives, and a stub
   executable. It covers saved-root precedence, paths with spaces, action
   preconditions, option validation, reinstall and upgrade rollback, stopped
   instances, purge boundaries, and source-checkout selection without
   installing a system service.
2. `.github/workflows/installer-ci.yml` builds real backend archives and runs
   install, status and health check, stop, start, restart, reinstall, upgrade,
   and uninstall on Linux x86_64 (detached service), macOS arm64 (native
   service), and Windows x86_64 (native service). The same workflow also
   installs the current latest beta through public GitHub release URLs on
   Windows and reinstalls the recorded release.
3. The platform lifecycle checks select a custom data root and verify that a
   normal uninstall preserves it, then reinstall and verify that explicit purge
   removes that root while preserving a sentinel in the unselected default
   directory. They also exercise a deliberately broken replacement binary to
   verify rollback.

The release workflow publishes `install.sh`, `install.ps1`,
`install.sh.sha256`, and `install.ps1.sha256` next to every versioned Agena
archive. After publication it downloads those public installer assets again on
Linux, macOS, and Windows and runs install, status, stop, start, reinstall,
upgrade, and uninstall against the published backend archives.

For local changes, the minimum installer checks are:

```sh
bash -n scripts/agena/install.sh
python3 scripts/agena/tests/test_installer_storage.py
python3 scripts/agena/verify-target-coverage.py
```
