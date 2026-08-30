Param(
  [ValidateSet("Install", "Upgrade", "Uninstall", "Start", "Stop", "Restart", "Status")]
  [string]$Action = "Install",
  [string]$Repo = "canxin121/agena",
  [string]$Version = "",
  [string]$Archive = "",
  [string]$Checksum = "",
  [string]$InstallDir = "",
  [string]$ListenHost = "127.0.0.1",
  [int]$Port = 3210,
  [string]$Workspace = "",
  [string]$UiPassword = "",
  [ValidateSet("auto", "native", "detached")]
  [string]$ServiceMode = "auto",
  [switch]$NoPathUpdate,
  [switch]$PurgeData
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if (-not $InstallDir) {
  $InstallDir = Join-Path $env:LOCALAPPDATA "Agena"
}
if (-not $Workspace) {
  $Workspace = $HOME
}

$StateFile = Join-Path $InstallDir "install-state.json"
$BinDir = Join-Path $InstallDir "bin"
$AgenaBin = Join-Path $BinDir "agena.exe"
$WebDir = Join-Path $InstallDir "web-dist"

function Normalize-Version([string]$Raw) {
  $value = $Raw.Trim()
  if ($value.StartsWith("agena-v")) { return $value.Substring(7) }
  if ($value.StartsWith("v")) { return $value.Substring(1) }
  return $value
}

function Get-TargetTriple {
  $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
  switch ($arch) {
    "X64" { return "x86_64-pc-windows-msvc" }
    "Arm64" { return "aarch64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $arch" }
  }
}

function Get-LatestVersion {
  $temp = Join-Path $env:TEMP ("agena-release-latest-" + [Guid]::NewGuid().ToString("N") + ".json")
  try {
    Get-FileFromSource "https://api.github.com/repos/$Repo/releases/latest" $temp
    $release = Get-Content -LiteralPath $temp -Raw | ConvertFrom-Json
  }
  finally {
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
  }
  if (-not $release.tag_name) { throw "Latest GitHub release has no tag_name" }
  return Normalize-Version ([string]$release.tag_name)
}

function Get-FileFromSource([string]$Source, [string]$Destination) {
  if ($Source -notmatch '^https?://') {
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
    return
  }

  Write-Host "Downloading $Source"
  $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
  if ($curl) {
    & $curl.Source `
      --fail `
      --location `
      --silent `
      --show-error `
      --retry 3 `
      --retry-delay 1 `
      --retry-connrefused `
      --connect-timeout 15 `
      --max-time 600 `
      --user-agent "agena-installer" `
      --output $Destination `
      $Source
    if ($LASTEXITCODE -ne 0) {
      throw "curl download failed with exit code ${LASTEXITCODE}: $Source"
    }
    $bytes = (Get-Item -LiteralPath $Destination).Length
    Write-Host "Downloaded $bytes bytes"
    return
  }

  # curl.exe ships with supported Windows releases. Keep a bounded
  # Invoke-WebRequest fallback for older/minimal PowerShell environments.
  $command = Get-Command Invoke-WebRequest
  for ($attempt = 1; $attempt -le 3; $attempt++) {
    try {
      $params = @{
        Uri = $Source
        OutFile = $Destination
        Headers = @{ "User-Agent" = "agena-installer" }
      }
      if ($command.Parameters.ContainsKey("ConnectionTimeoutSeconds")) {
        $params.ConnectionTimeoutSeconds = 15
        if ($command.Parameters.ContainsKey("OperationTimeoutSeconds")) {
          $params.OperationTimeoutSeconds = 600
        }
      }
      elseif ($command.Parameters.ContainsKey("TimeoutSec")) {
        $params.TimeoutSec = 600
      }
      Invoke-WebRequest @params
      $bytes = (Get-Item -LiteralPath $Destination).Length
      Write-Host "Downloaded $bytes bytes"
      return
    }
    catch {
      Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
      if ($attempt -eq 3) { throw }
      Start-Sleep -Seconds $attempt
    }
  }
}

function Assert-Checksum([string]$ArchivePath, [string]$ChecksumPath) {
  $line = (Get-Content -LiteralPath $ChecksumPath | Where-Object { $_.Trim() } | Select-Object -First 1)
  if (-not $line -or $line -notmatch '^\s*([0-9A-Fa-f]{64})\b') {
    throw "Invalid SHA256 file: $ChecksumPath"
  }
  $expected = $Matches[1].ToLowerInvariant()
  $actual = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expected) {
    throw "SHA256 mismatch: expected $expected, got $actual"
  }
}

function Save-State {
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  $state = [ordered]@{
    repo = $Repo
    version = $Version
    installDir = $InstallDir
    host = $ListenHost
    port = $Port
    workspace = $Workspace
    uiPassword = $UiPassword
    serviceMode = $ServiceMode
  }
  $state | ConvertTo-Json | Set-Content -LiteralPath $StateFile -Encoding UTF8
}

function Load-State {
  if (-not (Test-Path -LiteralPath $StateFile)) {
    throw "Agena is not installed at $InstallDir"
  }
  $state = Get-Content -LiteralPath $StateFile -Raw | ConvertFrom-Json
  $script:Repo = [string]$state.repo
  $script:Version = [string]$state.version
  $script:InstallDir = [string]$state.installDir
  $script:ListenHost = [string]$state.host
  $script:Port = [int]$state.port
  $script:Workspace = [string]$state.workspace
  $script:UiPassword = [string]$state.uiPassword
  $script:ServiceMode = [string]$state.serviceMode
  $script:StateFile = Join-Path $script:InstallDir "install-state.json"
  $script:BinDir = Join-Path $script:InstallDir "bin"
  $script:AgenaBin = Join-Path $script:BinDir "agena.exe"
  $script:WebDir = Join-Path $script:InstallDir "web-dist"
}

function Update-UserPath {
  if ($NoPathUpdate) { return }
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $parts = @($userPath -split ';' | Where-Object { $_ })
  if ($parts -notcontains $BinDir) {
    $next = (@($BinDir) + $parts) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $next, "User")
  }
  if (($env:Path -split ';') -notcontains $BinDir) {
    $env:Path = "$BinDir;$env:Path"
  }
}

function Remove-UserPath {
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  if ($null -eq $userPath) { return }
  $parts = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $BinDir })
  [Environment]::SetEnvironmentVariable("Path", ($parts -join ';'), "User")
}

function Resolve-ServiceMode {
  if ($ServiceMode -eq "auto") {
    $script:ServiceMode = "native"
  }
}

function Get-ServerArgs {
  $serverArgs = @(
    "--host", $ListenHost,
    "--port", "$Port",
    "--workspace", $Workspace,
    "--ui-dir", $WebDir
  )
  if ($UiPassword) { $serverArgs += @("--ui-password", $UiPassword) }
  return $serverArgs
}

function Invoke-AgenaLifecycle([string[]]$Arguments, [switch]$Capture) {
  $names = @(
    "AGENA_DATABASE_URL",
    "AGENA_DATABASE_PATH",
    "AGENA_SERVER_HOST",
    "AGENA_SERVER_PORT",
    "AGENA_SERVER_UI_PASSWORD",
    "AGENA_MCP_ENABLED",
    "AGENA_MCP_PUBLIC_URL",
    "AGENA_MCP_OAUTH_ISSUER_URL",
    "AGENA_MCP_AUTH_MODE",
    "AGENA_MCP_ANONYMOUS_ACCESS",
    "AGENA_MCP_CLIENT_REGISTRATION",
    "AGENA_WORKSPACE_ROOT",
    "AGENA_SERVER_UI_DIR"
  )
  $saved = @{}
  try {
    foreach ($name in $names) {
      $saved[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
      Remove-Item "Env:$name" -ErrorAction SilentlyContinue
    }
    $output = (& $AgenaBin @Arguments 2>&1 | Out-String)
    $code = $LASTEXITCODE
    if ($Capture) {
      return [pscustomobject]@{ Code = [int]$code; Output = $output }
    }
    if ($output.Trim()) {
      Write-Host $output.TrimEnd()
    }
    return [int]$code
  }
  finally {
    foreach ($name in $names) {
      $value = $saved[$name]
      if ($null -eq $value) {
        Remove-Item "Env:$name" -ErrorAction SilentlyContinue
      } else {
        [Environment]::SetEnvironmentVariable($name, $value, "Process")
      }
    }
  }
}

function Test-ServerRunning {
  if (-not (Test-Path -LiteralPath $AgenaBin)) { return $false }
  $result = Invoke-AgenaLifecycle @("server", "status") -Capture
  return $result.Output.Contains(" is running at ")
}

function Stop-ForUpgrade {
  if (-not (Test-Path -LiteralPath $AgenaBin)) { return }
  if ($ServiceMode -eq "native") {
    $code = Invoke-AgenaLifecycle @("server", "uninstall")
    if ($code -ne 0) { throw "Failed to stop/uninstall the existing Agena user service before upgrade" }
  } elseif (Test-ServerRunning) {
    $code = Invoke-AgenaLifecycle @("server", "stop")
    if ($code -ne 0) { throw "Agena server stop failed" }
  }
}

function Start-Installed {
  $serverArgs = Get-ServerArgs
  if ($ServiceMode -eq "native") {
    $code = Invoke-AgenaLifecycle (@("server", "install") + $serverArgs)
  } else {
    $code = Invoke-AgenaLifecycle (@("server", "start") + $serverArgs)
  }
  if ($code -ne 0) { throw "Agena server failed to start" }
}

function Install-OrUpgrade([string]$RequestedAction) {
  $requestedVersion = $Version
  $oldServiceMode = $ServiceMode
  $oldWasRunning = $false
  if ($RequestedAction -eq "Upgrade") {
    Load-State
    $oldWasRunning = Test-ServerRunning
    $oldServiceMode = $ServiceMode
    $script:Version = $requestedVersion
  } elseif (Test-Path -LiteralPath $StateFile) {
    throw "Agena is already installed at $InstallDir; use -Action Upgrade"
  }
  Resolve-ServiceMode
  New-Item -ItemType Directory -Force -Path $Workspace | Out-Null

  $target = Get-TargetTriple
  if (-not $Archive) {
    if (-not $Version) { $script:Version = Get-LatestVersion } else { $script:Version = Normalize-Version $Version }
    $archiveSource = "https://github.com/$Repo/releases/download/agena-v$Version/agena-backend-$target-v$Version.zip"
    $checksumSource = "$archiveSource.sha256"
  } else {
    $archiveSource = $Archive
    $checksumSource = if ($Checksum) { $Checksum } else { "$Archive.sha256" }
  }

  $temp = Join-Path $env:TEMP ("agena-install-" + [Guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Force -Path $temp | Out-Null
  try {
    $archivePath = Join-Path $temp "agena.zip"
    $checksumPath = Join-Path $temp "agena.sha256"
    Get-FileFromSource $archiveSource $archivePath
    Get-FileFromSource $checksumSource $checksumPath
    Assert-Checksum $archivePath $checksumPath

    $stage = Join-Path $temp "stage"
    Expand-Archive -LiteralPath $archivePath -DestinationPath $stage -Force
    $stageBin = Join-Path $stage "bin\agena.exe"
    $stageWeb = Join-Path $stage "web-dist\index.html"
    if (-not (Test-Path -LiteralPath $stageBin) -or -not (Test-Path -LiteralPath $stageWeb)) {
      throw "Release archive is missing bin/agena.exe or web-dist/index.html"
    }
    $binaryVersionText = (& $stageBin --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $binaryVersionText -notmatch '(\S+)$') {
      throw "Installed Agena binary did not report a version"
    }
    $binaryVersion = $Matches[1]
    if ($Version -and $binaryVersion -ne $Version) {
      throw "Archive contains Agena $binaryVersion but $Version was requested"
    }
    $script:Version = $binaryVersion

    $backup = Join-Path $temp "backup"
    New-Item -ItemType Directory -Force -Path $backup | Out-Null
    if (Test-Path -LiteralPath $StateFile) {
      Copy-Item -LiteralPath $StateFile -Destination (Join-Path $backup "install-state.json") -Force
    }

    Stop-ForUpgrade
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    foreach ($name in @("bin", "web-dist")) {
      $existing = Join-Path $InstallDir $name
      if (Test-Path -LiteralPath $existing) {
        Move-Item -LiteralPath $existing -Destination (Join-Path $backup $name)
      }
      Move-Item -LiteralPath (Join-Path $stage $name) -Destination $existing
    }

    try {
      Start-Installed
      if ($RequestedAction -eq "Upgrade" -and -not $oldWasRunning) {
        $stopCode = Invoke-AgenaLifecycle @("server", "stop")
        if ($stopCode -ne 0) {
          throw "Upgraded Agena could not be returned to its previous stopped state"
        }
      }
      Save-State
      try { Update-UserPath } catch { Write-Warning "Agena installed, but PATH update failed: $_" }
    }
    catch {
      $installError = $_
      Write-Warning "The new Agena version failed to start; restoring the previous installation."
      if (Test-Path -LiteralPath $AgenaBin) {
        if ($ServiceMode -eq "native") {
          try { [void](Invoke-AgenaLifecycle @("server", "uninstall")) } catch {}
        } elseif (Test-ServerRunning) {
          try { [void](Invoke-AgenaLifecycle @("server", "stop")) } catch {}
        }
      }

      foreach ($name in @("bin", "web-dist")) {
        $current = Join-Path $InstallDir $name
        if (Test-Path -LiteralPath $current) { Remove-Item -LiteralPath $current -Recurse -Force }
        $old = Join-Path $backup $name
        if (Test-Path -LiteralPath $old) { Move-Item -LiteralPath $old -Destination $current }
      }

      $oldState = Join-Path $backup "install-state.json"
      if (Test-Path -LiteralPath $oldState) {
        Copy-Item -LiteralPath $oldState -Destination $StateFile -Force
        Load-State
        if ($ServiceMode -eq "native") {
          try {
            Start-Installed
            if (-not $oldWasRunning) {
              [void](Invoke-AgenaLifecycle @("server", "stop"))
            }
          } catch {
            Write-Warning "Rollback restored the old files but could not restore the native service: $_"
          }
        } elseif ($oldWasRunning) {
          try { Start-Installed } catch { Write-Warning "Rollback restored the old files but could not restart the old server: $_" }
        }
      } else {
        Remove-Item -LiteralPath $StateFile -Force -ErrorAction SilentlyContinue
      }
      throw $installError
    }
    Write-Host "Agena $Version installed at $InstallDir"
    Write-Host "Web UI: http://${ListenHost}:$Port"
  }
  finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
  }
}

function Start-Agena {
  Load-State
  $serverArgs = Get-ServerArgs
  if ($ServiceMode -eq "native") { $code = Invoke-AgenaLifecycle @("server", "start") }
  else { $code = Invoke-AgenaLifecycle (@("server", "start") + $serverArgs) }
  if ($code -ne 0) { throw "Agena server start failed" }
}

function Stop-Agena {
  Load-State
  $code = Invoke-AgenaLifecycle @("server", "stop")
  if ($code -ne 0) { throw "Agena server stop failed" }
}

function Show-Status {
  Load-State
  $versionCode = Invoke-AgenaLifecycle @("--version")
  if ($versionCode -ne 0) { throw "Agena version check failed" }
  $statusCode = Invoke-AgenaLifecycle @("server", "status")
  if ($statusCode -ne 0) { throw "Agena server status failed" }
}

function Uninstall-Agena {
  Load-State
  if (Test-Path -LiteralPath $AgenaBin) {
    if ($ServiceMode -eq "native") {
      $code = Invoke-AgenaLifecycle @("server", "uninstall")
      if ($code -ne 0) { throw "Agena user service uninstall failed" }
    } elseif (Test-ServerRunning) {
      $code = Invoke-AgenaLifecycle @("server", "stop")
      if ($code -ne 0) { throw "Agena server stop failed" }
    }
  }
  Remove-UserPath
  Remove-Item -LiteralPath $InstallDir -Recurse -Force
  if ($PurgeData) {
    $dataDir = Join-Path $HOME "agena"
    if (Test-Path -LiteralPath $dataDir) { Remove-Item -LiteralPath $dataDir -Recurse -Force }
    Write-Host "Removed Agena runtime data at $dataDir"
  }
  Write-Host "Agena uninstalled. Configuration/session data was preserved unless -PurgeData was used."
}

switch ($Action) {
  "Install" { Install-OrUpgrade "Install" }
  "Upgrade" { Install-OrUpgrade "Upgrade" }
  "Uninstall" { Uninstall-Agena }
  "Start" { Start-Agena }
  "Stop" { Stop-Agena }
  "Restart" { try { Stop-Agena } catch {}; Start-Agena }
  "Status" { Show-Status }
}
