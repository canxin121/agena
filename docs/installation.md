# Install Agena

Agena releases contain one native executable plus the matching Web frontend.
The public installers detect the host architecture, download the matching
GitHub Release asset, verify its SHA-256 file, install both pieces, and start
the server.

## One-click install

### macOS and Linux

```bash
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash
```

Supported release architectures are x86_64 and arm64/aarch64 on both macOS and
Linux.

### Windows PowerShell

```powershell
irm https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.ps1 | iex
```

Agena publishes beta releases. The repository and binary versions are `0.1.0`;
release tags add a beta sequence, such as `agena-v0.1.0-beta.1`. With no
explicit `--version` / `-Version`, the installers select the newest beta
release whose tag matches `agena-vX.Y.Z-beta.N`.

The Windows installer supports x86_64 and ARM64 release packages.

By default Agena is installed under the current user's application-data area,
the Web/server listens on `127.0.0.1:3210`, and the workspace is the current
user's home directory. macOS uses a launchd user agent, Windows uses a Task
Scheduler logon task, and Linux uses a systemd user service when one is
available. Linux environments without a user systemd session automatically use
Agena's detached server lifecycle instead.

The executable and Web assets have a separate installation directory from the
instance's data. Unix defaults to `~/.local/share/agena` for application files;
Windows defaults to `%LOCALAPPDATA%\Agena`. Instance data defaults to `~/agena`
on every platform. Set `AGENA_DATA_DIR`, or pass `--data-dir` to the Unix
installer / `-DataDir` to the PowerShell installer, to choose another root:

```bash
bash scripts/agena/install.sh install --data-dir /path/to/agena-data
```

```powershell
& $installer -Action Install -DataDir 'D:\AgenaData'
```

The chosen root contains `agena.db` for sessions, Scheduler and server state,
`agena.json` for user configuration, `secrets/` for the application-managed
encrypted SecretStore, `workspaces/` for workspace data, `logs/` for logs, and
`run/server.json` for the live endpoint record. A single instance has one root;
database and server-state paths have no separate environment overrides.

Installers save the selected root in their installation state. Start, stop,
upgrade, rollback and uninstall continue to address that installed root even
when the invoking shell has a different `AGENA_DATA_DIR`. Native service
definitions explicitly pass `--data-dir` to the executable. Keep the
installation and data directories separate; neither may contain the other.
Native service labels, systemd unit names and Windows task names include the
full hash of the selected data root, so installing or stopping one instance
does not address another instance's service.
With the normal shell/environment update enabled, Unix exports the root from
its private client environment file and Windows sets the user
`AGENA_DATA_DIR`. On Unix the managed block goes to `~/.zshenv` for zsh
(honoring `ZDOTDIR`) and `~/.profile` for other shells; an older Agena block
is removed from the other startup files. Open a new shell or source that file
to load the command.
With `--no-path-update` / `-NoPathUpdate`, set that variable yourself or run
clients with `agena --data-dir /path/to/agena-data ...`.

When `--ui-password` (or `AGENA_SERVER_UI_PASSWORD`) is used on macOS/Linux,
the installer also writes a private client environment file under the install
directory and sources it from the shell profile. The same operator credential
is exposed to thin clients as `AGENA_SERVER_PASSWORD`, so a newly opened shell
can run `agena` / `agena tui` against the protected local server without a
second manual password export. The password itself is not written directly into
the shell profile.

The TUI workspace is independent of the long-lived server process working
directory: `cd /path/to/project && agena` uses `/path/to/project` for new
sessions, while `agena tui --workspace /other/path` remains the explicit
override.

## Install from source

Build and install from a checkout without downloading a release archive:

```bash
cd /path/to/agena
bash scripts/agena/install.sh install --source
```

The installer uses the checkout that contains the current directory, then the
checkout that contains the installer script. It does not clone another copy
when either checkout is available. To build another checkout, pass its path;
to clone a remote repository, pass a Git URL:

```bash
bash scripts/agena/install.sh install --source /path/to/agena
bash scripts/agena/install.sh install --source https://github.com/canxin121/agena.git
bash scripts/agena/install.sh install --source https://github.com/canxin121/agena.git --source-ref master
```

`--source` cannot be combined with `--archive`, `--checksum`, or `--version`.
Source mode builds the native binary and Web frontend, creates the normal
archive and SHA-256 file under `artifacts/agena/`, verifies that archive, and
then uses the same install, startup, and rollback path as a release install.
Use `upgrade --source` to rebuild and replace an existing installation.

The source build requires Rust 1.97.0 through rustup (or a compatible
`AGENA_STABLE_TOOLCHAIN`), Bun, Python 3, `tar` on macOS/Linux, Git when
cloning, and the platform C/C++ build tools required by the Rust dependencies.
If the Web frontend is already built, set `AGENA_WEB_DIST_PREBUILT=1` to skip
Bun and the frontend build. Build output stays in the checkout's `target/` and
`packages/agena-web/dist/`; for a local checkout, the generated package stays
in `artifacts/agena/`. A checkout cloned from a Git URL is temporary and is
removed after installation.

Windows PowerShell:

```powershell
.\scripts\agena\install.ps1 -Action Install -Source
```

Use `-SourcePath D:\src\agena` to select another checkout or a Git URL, and
`-SourceRef master` when cloning a branch or tag. Source mode does not use
`-Version`.

## Lifecycle operations

On macOS/Linux the installer can be re-run with an action:

```bash
# Upgrade to the latest beta release.
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- upgrade

# Inspect, stop, start, or restart the installed server.
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- status
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- stop
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- start
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- restart

# Remove the application while keeping configuration, sessions, and logs.
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh | bash -s -- uninstall
```

For Windows, invoke the downloaded script block with the desired action:

```powershell
$installer = [scriptblock]::Create((irm https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.ps1))
& $installer -Action Upgrade
& $installer -Action Status
& $installer -Action Stop
& $installer -Action Start
& $installer -Action Restart
& $installer -Action Uninstall
```

The installed binary exposes the same server lifecycle directly:

```text
agena server status
agena server stop
agena server start
```

`agena server install` and `agena server uninstall` manage the native per-user
service on macOS, Linux, and Windows.

## macOS file permissions (one-time setup)

For workspaces that span protected folders, other apps' data, external disks,
or network volumes, grant **Agena** **Full Disk Access** once in **System
Settings → Privacy & Security → Full Disk Access**. macOS requires the user
to enable this switch; Agena cannot grant it to itself or preapprove every
future disk/app by opening directories at startup.

Run `agena server permissions` after installing the native user service. It
reveals the exact app in Finder and opens the settings page. Use **+** and
**Cmd+Shift+G** to select the displayed app path (normally
`~/agena/macos/Agena.app`), enable it, then run:

```bash
agena server stop
agena server start
```

The native service runs through a small signed supervisor in this app. Its
executable, bundle ID, Info.plist, and signature are preserved byte-for-byte
across normal upgrades, rollbacks, and reinstalls that preserve runtime data.
The server and the tools it spawns inherit the responsible app's file-access
identity. The large server executable can therefore be upgraded independently.
Do not add the changing `bin/agena` executable to Full Disk Access instead.
An existing grant to an older standalone executable must be granted once to
the new app. The installer verifies an existing app and reports damage instead
of silently replacing/re-signing it and losing the remembered identity.

Interactive first installation opens this setup once. Set
`AGENA_MACOS_OPEN_PERMISSIONS=0` for headless installers. Upgrades and restarts
neither reopen settings nor probe private data to guess authorization status.
Normal uninstall preserves the app with runtime data; `--purge-data`, manually
deleting/replacing the app, revoking/resetting privacy settings, or a future
supervisor migration can require authorization again. macOS controls the final
decision; no app can guarantee a permanent grant after a system reset.

The TUI loads its recursive file index only when file mentions are used.
Recursive Web searches and workspace indexing from a broad home/root scope
skip macOS privacy domains before opening them, including `~/Library`, standard
protected user folders, and unselected mounted disks. Direct directory listing
is still available; select a protected folder/project as the workspace/search
root to index it deliberately. This reduces incidental prompts even without
Full Disk Access. File access does not grant Accessibility, Screen Recording,
microphone, camera, or Automation permissions. Direct `agena server` and
detached/development runs remain under the launching terminal's privacy identity;
use the native user service for the persistent Agena app identity.

Apple describes the code identity constraint in
[TN3127: Inside Code Signing: Requirements](https://developer.apple.com/documentation/technotes/tn3127-inside-code-signing-requirements).

## Install a specific version

macOS/Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.sh \
  | bash -s -- install --version 0.1.0-beta.1
```

Windows:

```powershell
$installer = [scriptblock]::Create((irm https://raw.githubusercontent.com/canxin121/agena/master/scripts/agena/install.ps1))
& $installer -Action Install -Version 0.1.0-beta.1
```

Both installers also accept a local or remote `--archive`/`-Archive` plus its
`.sha256` file. CI uses that path to test the exact release-package layout
without depending on an already-published GitHub release.

Upgrades are transactional at the package level. The existing executable, Web
frontend, and installer state are retained until the replacement has started
successfully. If a replacement passes download/checksum/version validation but
fails during startup, the installer restores the prior files and service. CI
also exercises this rollback path with an intentionally broken replacement
binary.

## Uninstalling data

Normal uninstall removes the installed executable, Web frontend, and user
service, while preserving the selected instance's data. `--purge-data` on Unix
or `-PurgeData` on Windows additionally removes the actual data root saved in
the installation state. A custom root is purged directly; the default
`~/agena` is left alone when another root was selected.

Purge refuses filesystem roots, the account home, shared system locations,
overlapping installation/data directories, and directories without an Agena
instance database. It removes configuration and credentials as well as
sessions. Installer state and databases from older storage layouts are not
migrated by this version.

## Installer CI

`python3 scripts/agena/tests/test_installer_storage.py` runs offline installer
contracts with temporary homes, local archives and a stub executable. It covers
saved-root precedence, paths with spaces, upgrade rollback, stopped instances,
purge boundaries, source-checkout selection, source builds and service-helper
arguments without installing a system service.

`.github/workflows/installer-ci.yml` builds the real release archive format on
GitHub-hosted Linux, macOS, and Windows runners. Each runner then executes the
full lifecycle against that archive:

1. install and start;
2. status and health check;
3. stop and verify the endpoint is gone;
4. start and restart;
5. upgrade through the installer again;
6. uninstall and verify both the process/service and install directory are gone.

The current-package lifecycle fixtures select a custom data root. They verify
that a normal uninstall preserves it, then reinstall and verify that explicit
purge removes that root while preserving a sentinel in the unselected default
directory.

The release workflow publishes `install.sh` and `install.ps1` next to every
versioned Agena archive, together with `install.sh.sha256` and
`install.ps1.sha256`, so the one-click URLs always resolve to a tested installer
from the release itself. After publication, the release workflow downloads
those public installer assets again on Linux, macOS, and Windows and runs a
second install/status/stop/start/upgrade/uninstall smoke against the published
backend archives.
