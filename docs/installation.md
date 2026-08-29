# Install Agena

Agena releases contain one native executable plus the matching Web frontend.
The public installers detect the host architecture, download the matching
GitHub Release asset, verify its SHA-256 file, install both pieces, and start
the server.

## One-click install

### macOS and Linux

```bash
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash
```

Supported release architectures are x86_64 and arm64/aarch64 on both macOS and
Linux.

### Windows PowerShell

```powershell
irm https://github.com/canxin121/agena/releases/latest/download/install.ps1 | iex
```

The Windows installer supports x86_64 and ARM64 release packages.

By default Agena is installed under the current user's application-data area,
the Web/server listens on `127.0.0.1:3210`, and the workspace is the current
user's home directory. macOS uses a launchd user agent, Windows uses a Task
Scheduler logon task, and Linux uses a systemd user service when one is
available. Linux environments without a user systemd session automatically use
Agena's detached server lifecycle instead.

## Lifecycle operations

On macOS/Linux the installer can be re-run with an action:

```bash
# Upgrade to the latest stable release.
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- upgrade

# Inspect, stop, start, or restart the installed server.
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- status
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- stop
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- start
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- restart

# Remove the application while keeping configuration, sessions, and logs.
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh | bash -s -- uninstall
```

For Windows, invoke the downloaded script block with the desired action:

```powershell
$installer = [scriptblock]::Create((irm https://github.com/canxin121/agena/releases/latest/download/install.ps1))
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

## Install a specific version

macOS/Linux:

```bash
curl -fsSL https://github.com/canxin121/agena/releases/latest/download/install.sh \
  | bash -s -- install --version 0.1.0
```

Windows:

```powershell
$installer = [scriptblock]::Create((irm https://github.com/canxin121/agena/releases/latest/download/install.ps1))
& $installer -Action Install -Version 0.1.0
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
service, while preserving Agena's runtime data. `--purge-data` on Unix or
`-PurgeData` on Windows additionally removes `~/agena`.

## Installer CI

`.github/workflows/installer-ci.yml` builds the real release archive format on
GitHub-hosted Linux, macOS, and Windows runners. Each runner then executes the
full lifecycle against that archive:

1. install and start;
2. status and health check;
3. stop and verify the endpoint is gone;
4. start and restart;
5. upgrade through the installer again;
6. uninstall and verify both the process/service and install directory are gone.

The release workflow publishes `install.sh` and `install.ps1` next to every
versioned Agena archive, together with `install.sh.sha256` and
`install.ps1.sha256`, so the one-click URLs always resolve to a tested installer
from the release itself. After publication, the release workflow downloads
those public installer assets again on Linux, macOS, and Windows and runs a
second install/status/stop/start/upgrade/uninstall smoke against the published
backend archives.
