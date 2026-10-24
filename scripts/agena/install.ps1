Param(
  [ValidateSet("Install", "Upgrade", "Uninstall", "Start", "Stop", "Restart", "Status")]
  [string]$Action = "Install",
  [string]$Repo = "canxin121/agena",
  [string]$Version = "",
  [string]$Archive = "",
  [string]$Checksum = "",
  [switch]$Source,
  [string]$SourcePath = "",
  [string]$SourceRef = "",
  [string]$InstallDir = "",
  [string]$DataDir = "",
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

$DataDirSpecified = $PSBoundParameters.ContainsKey("DataDir")
$ListenHostSpecified = $PSBoundParameters.ContainsKey("ListenHost")
$PortSpecified = $PSBoundParameters.ContainsKey("Port")
$WorkspaceSpecified = $PSBoundParameters.ContainsKey("Workspace")
$UiPasswordSpecified = $PSBoundParameters.ContainsKey("UiPassword")
$ServiceModeSpecified = $PSBoundParameters.ContainsKey("ServiceMode")

if (-not $InstallDir) {
  $InstallDir = Join-Path $env:LOCALAPPDATA "Agena"
}
if (-not $DataDir) {
  if ($env:AGENA_DATA_DIR) { $DataDir = $env:AGENA_DATA_DIR }
  else { $DataDir = Join-Path $HOME "agena" }
}
if (-not $Workspace) {
  $Workspace = $HOME
}

if ($Source) {
  if ($Action -notin @("Install", "Upgrade")) {
    throw "-Source is only supported for Install and Upgrade"
  }
  if ($Archive) { throw "-Source cannot be combined with -Archive" }
  if ($Checksum) { throw "-Source cannot be combined with -Checksum" }
  if ($Version) { throw "-Version applies to release installs; use -SourceRef to select a branch or tag" }
}
elseif ($SourcePath -or $SourceRef) {
  throw "-SourcePath and -SourceRef require -Source"
}

function Resolve-Directory([string]$Path) {
  if ([string]::IsNullOrWhiteSpace($Path)) { throw "directory must not be empty" }
  return [IO.Path]::GetFullPath($Path).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
}

function Assert-StoragePaths([switch]$InstallOnly) {
  $script:InstallDir = Resolve-Directory $InstallDir
  $paths = @($InstallDir)
  if (-not $InstallOnly) {
    $script:DataDir = Resolve-Directory $DataDir
    $paths += $DataDir
  }
  $blocked = @($HOME, $env:SystemRoot, $env:ProgramFiles, $env:LOCALAPPDATA, $env:TEMP) |
    Where-Object { $_ } | ForEach-Object { Resolve-Directory $_ }
  foreach ($path in $paths) {
    if ($path -eq [IO.Path]::GetPathRoot($path).TrimEnd('\', '/') -or $blocked -contains $path) {
      throw "unsafe Agena storage directory: $path"
    }
    $ancestor = $path
    while ($ancestor) {
      if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Agena storage directories must not traverse links or junctions: $ancestor"
      }
      $ancestor = [IO.Path]::GetDirectoryName($ancestor)
    }
  }
  if ($InstallOnly) { return }
  $separator = [IO.Path]::DirectorySeparatorChar
  $comparison = [StringComparison]::OrdinalIgnoreCase
  if (($InstallDir + $separator).StartsWith($DataDir + $separator, $comparison) -or
      ($DataDir + $separator).StartsWith($InstallDir + $separator, $comparison)) {
    throw "application installation and instance data directories must be separate"
  }
}

if ($Action -eq "Install") { Assert-StoragePaths }
else { Assert-StoragePaths -InstallOnly }

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

function Test-SourceRoot([string]$Path) {
  return ((Test-Path -LiteralPath (Join-Path $Path "Cargo.toml") -PathType Leaf) -and
    (Test-Path -LiteralPath (Join-Path $Path "packages/agena-web/package.json") -PathType Leaf) -and
    (Test-Path -LiteralPath (Join-Path $Path "scripts/agena/package-backend.ps1") -PathType Leaf))
}

function Find-SourceRoot([string]$Start) {
  if ([string]::IsNullOrWhiteSpace($Start)) { return $null }
  $current = Resolve-Directory $Start
  while ($current) {
    if (Test-SourceRoot $current) { return $current }
    $parent = [IO.Path]::GetDirectoryName($current)
    if (-not $parent -or $parent -eq $current) { break }
    $current = $parent
  }
  return $null
}

function Get-InstallerSourceRoot {
  if ([string]::IsNullOrWhiteSpace($PSScriptRoot)) { return $null }
  $scriptsDir = Split-Path -Parent $PSScriptRoot
  $repositoryRoot = Split-Path -Parent $scriptsDir
  return Find-SourceRoot $repositoryRoot
}

function Test-GitSource([string]$Value) {
  return $Value -match '^(https?://|ssh://|git://|file://|git@.+:)'
}

function Invoke-SourceClone([string]$Repository, [string]$Temp) {
  $git = Get-Command git -ErrorAction SilentlyContinue
  if (-not $git) { throw "git is required to install from a remote source repository" }
  $destination = Join-Path $Temp "source"
  $cloneArgs = @("clone", "--depth", "1")
  if ($SourceRef) { $cloneArgs += @("--branch", $SourceRef) }
  $cloneArgs += @($Repository, $destination)
  Write-Host "Cloning Agena source from $Repository"
  & $git.Source @cloneArgs
  if ($LASTEXITCODE -ne 0) { throw "git clone failed with exit code $LASTEXITCODE" }
  if (-not (Test-Path -LiteralPath $destination -PathType Container)) {
    throw "git clone did not create $destination"
  }
  return $destination
}

function Resolve-SourceRoot([string]$Temp) {
  $requested = $SourcePath.Trim()
  if ($requested) {
    if (Test-Path -LiteralPath $requested -PathType Container) {
      $root = Find-SourceRoot $requested
      if (-not $root) { throw "-SourcePath is not an Agena checkout: $requested" }
      return $root
    }
    if (Test-GitSource $requested) {
      $cloned = Invoke-SourceClone $requested $Temp
      $root = Find-SourceRoot $cloned
      if (-not $root) { throw "cloned source is not an Agena checkout: $requested" }
      return $root
    }
    throw "-SourcePath must name an Agena checkout or a Git URL"
  }

  $root = Find-SourceRoot (Get-Location).Path
  if ($root) { return $root }
  $root = Get-InstallerSourceRoot
  if ($root) { return $root }

  $cloned = Invoke-SourceClone "https://github.com/$Repo.git" $Temp
  $root = Find-SourceRoot $cloned
  if (-not $root) { throw "cloned source is not an Agena checkout" }
  return $root
}

function New-SourceArchive([string]$Temp, [string]$Target) {
  $root = Resolve-SourceRoot $Temp
  if ($SourceRef -and $root -ne (Resolve-Directory (Join-Path $Temp "source"))) {
    Write-Warning "-SourceRef only applies when cloning; using the existing checkout as-is"
  }
  Write-Host "Building Agena from source at $root"
  $packageScript = Join-Path $root "scripts/agena/package-backend.ps1"
  if (-not (Test-Path -LiteralPath $packageScript -PathType Leaf)) {
    throw "Agena source build script is missing: $packageScript"
  }
  & $packageScript -TargetTriple $Target
  if ($LASTEXITCODE -ne 0) { throw "Agena source build failed with exit code $LASTEXITCODE" }

  $artifactDir = Join-Path $root "artifacts/agena"
  $archive = Get-ChildItem -LiteralPath $artifactDir -File -Filter "agena-backend-$Target-v*.zip" |
    Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
  if (-not $archive) { throw "Agena source build did not produce a package archive" }
  $checksum = "$($archive.FullName).sha256"
  if (-not (Test-Path -LiteralPath $checksum -PathType Leaf)) {
    throw "Agena source build did not produce a package checksum"
  }
  Write-Host "Source package ready: $($archive.FullName)"
  return [pscustomobject]@{
    Archive = $archive.FullName
    Checksum = $checksum
  }
}

function Get-LatestVersion {
  $temp = Join-Path $env:TEMP ("agena-release-latest-" + [Guid]::NewGuid().ToString("N") + ".json")
  try {
    Get-FileFromSource "https://api.github.com/repos/$Repo/releases?per_page=100" $temp
    $releases = @(Get-Content -LiteralPath $temp -Raw | ConvertFrom-Json)
  }
  finally {
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
  }
  $release = $releases | Where-Object {
    -not $_.draft -and $_.prerelease -and ([string]$_.tag_name -match '^agena-v\d+\.\d+\.\d+-beta\.[1-9]\d*$')
  } | Select-Object -First 1
  if (-not $release -or -not $release.tag_name) { throw "No Agena beta release was found for $Repo" }
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
    schema = 2
    repo = $Repo
    version = $Version
    installDir = $InstallDir
    dataDir = $DataDir
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
  $selectedInstallDir = $InstallDir
  $state = Get-Content -LiteralPath $StateFile -Raw | ConvertFrom-Json
  if ($state.schema -ne 2 -or -not $state.dataDir) {
    throw "installer state is incompatible; reinstall with an explicit -DataDir"
  }
  $script:Repo = [string]$state.repo
  $script:Version = [string]$state.version
  $script:InstallDir = Resolve-Directory ([string]$state.installDir)
  if ($InstallDir -ne $selectedInstallDir) { throw "installer state belongs to a different installation" }
  $script:DataDir = [string]$state.dataDir
  $script:ListenHost = [string]$state.host
  $script:Port = [int]$state.port
  $script:Workspace = [string]$state.workspace
  $script:UiPassword = [string]$state.uiPassword
  $script:ServiceMode = [string]$state.serviceMode
  Assert-StoragePaths
  $script:StateFile = Join-Path $script:InstallDir "install-state.json"
  $script:BinDir = Join-Path $script:InstallDir "bin"
  $script:AgenaBin = Join-Path $script:BinDir "agena.exe"
  $script:WebDir = Join-Path $script:InstallDir "web-dist"
}

function Update-UserPath {
  if ($NoPathUpdate) { return }
  [Environment]::SetEnvironmentVariable("AGENA_DATA_DIR", $DataDir, "User")
  $env:AGENA_DATA_DIR = $DataDir
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
  if (-not $NoPathUpdate -and [Environment]::GetEnvironmentVariable("AGENA_DATA_DIR", "User") -eq $DataDir) {
    [Environment]::SetEnvironmentVariable("AGENA_DATA_DIR", $null, "User")
  }
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
    "AGENA_DATA_DIR",
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
    $output = (& $AgenaBin --data-dir $DataDir @Arguments 2>&1 | Out-String)
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
  $requestedDataDir = $DataDir
  $requestedListenHost = $ListenHost
  $requestedPort = $Port
  $requestedWorkspace = $Workspace
  $requestedUiPassword = $UiPassword
  $requestedServiceMode = $ServiceMode
  $oldDataDir = $DataDir
  $oldServiceMode = $ServiceMode
  $oldWasRunning = $false
  if ($RequestedAction -eq "Upgrade") {
    Load-State
    $oldDataDir = $DataDir
    $oldServiceMode = $ServiceMode
    $oldWasRunning = Test-ServerRunning
    $script:Version = $requestedVersion
    if ($DataDirSpecified) { $script:DataDir = $requestedDataDir }
    if ($ListenHostSpecified) { $script:ListenHost = $requestedListenHost }
    if ($PortSpecified) { $script:Port = $requestedPort }
    if ($WorkspaceSpecified) { $script:Workspace = $requestedWorkspace }
    if ($UiPasswordSpecified) { $script:UiPassword = $requestedUiPassword }
    if ($ServiceModeSpecified) { $script:ServiceMode = $requestedServiceMode }
    Assert-StoragePaths
  } elseif (Test-Path -LiteralPath $StateFile) {
    throw "Agena is already installed at $InstallDir; use -Action Upgrade to reinstall or change settings"
  }
  Resolve-ServiceMode
  $newDataDir = $DataDir
  $newServiceMode = $ServiceMode
  New-Item -ItemType Directory -Force -Path $Workspace | Out-Null

  $temp = Join-Path $env:TEMP ("agena-install-" + [Guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Force -Path $temp | Out-Null
  try {
    $target = Get-TargetTriple
    $packageVersion = ""
    if ($Source) {
      $sourceArchive = New-SourceArchive $temp $target
      $archiveSource = $sourceArchive.Archive
      $checksumSource = $sourceArchive.Checksum
    }
    elseif (-not $Archive) {
      if (-not $Version) { $releaseVersion = Get-LatestVersion } else { $releaseVersion = Normalize-Version $Version }
      if ($releaseVersion -notmatch '^(\d+\.\d+\.\d+)-beta\.[1-9]\d*$') {
        throw "-Version must name a beta release, for example 0.1.0-beta.1"
      }
      $script:Version = $releaseVersion
      $packageVersion = $Matches[1]
      $archiveSource = "https://github.com/$Repo/releases/download/agena-v$releaseVersion/agena-backend-$target-v$packageVersion.zip"
      $checksumSource = "$archiveSource.sha256"
    }
    else {
      $archiveSource = $Archive
      $checksumSource = if ($Checksum) { $Checksum } else { "$Archive.sha256" }
      if ($Version) {
        $releaseVersion = Normalize-Version $Version
        if ($releaseVersion -notmatch '^(\d+\.\d+\.\d+)-beta\.[1-9]\d*$') {
          throw "-Version must name a beta release, for example 0.1.0-beta.1"
        }
        $packageVersion = $Matches[1]
        $script:Version = $releaseVersion
      }
    }

    $archivePath = Join-Path $temp "agena.zip"
    $checksumPath = Join-Path $temp "agena.sha256"
    Get-FileFromSource $archiveSource $archivePath
    Get-FileFromSource $checksumSource $checksumPath
    Assert-Checksum $archivePath $checksumPath
    Write-Host "Verified Agena release checksum"

    $stage = Join-Path $temp "stage"
    New-Item -ItemType Directory -Force -Path $stage | Out-Null
    Write-Host "Extracting Agena release archive"
    $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
    if ($tar) {
      & $tar.Source -xf $archivePath -C $stage
      if ($LASTEXITCODE -ne 0) {
        throw "tar.exe extraction failed with exit code ${LASTEXITCODE}"
      }
    }
    else {
      Expand-Archive -LiteralPath $archivePath -DestinationPath $stage -Force
    }
    Write-Host "Extracted Agena release archive"
    $stageBin = Join-Path $stage "bin\agena.exe"
    $stageWeb = Join-Path $stage "web-dist\index.html"
    if (-not (Test-Path -LiteralPath $stageBin) -or -not (Test-Path -LiteralPath $stageWeb)) {
      throw "Release archive is missing bin/agena.exe or web-dist/index.html"
    }
    Write-Host "Validating Agena release binary"
    $binaryVersionText = (& $stageBin --version | Out-String).Trim()
    if ($LASTEXITCODE -ne 0 -or $binaryVersionText -notmatch '(\S+)$') {
      throw "Installed Agena binary did not report a version"
    }
    $binaryVersion = $Matches[1]
    if ($packageVersion -and $binaryVersion -ne $packageVersion) {
      throw "Archive contains Agena $binaryVersion but release $Version contains $packageVersion"
    }
    $script:Version = $binaryVersion
    Write-Host "Validated Agena $binaryVersion"

    $backup = Join-Path $temp "backup"
    New-Item -ItemType Directory -Force -Path $backup | Out-Null
    if (Test-Path -LiteralPath $StateFile) {
      Copy-Item -LiteralPath $StateFile -Destination (Join-Path $backup "install-state.json") -Force
    }

    Write-Host "Stopping existing Agena installation if needed"
    if ($RequestedAction -eq "Upgrade") {
      $script:DataDir = $oldDataDir
      $script:ServiceMode = $oldServiceMode
    }
    Stop-ForUpgrade
    if ($RequestedAction -eq "Upgrade") {
      $script:DataDir = $newDataDir
      $script:ServiceMode = $newServiceMode
    }
    Write-Host "Installing Agena files"
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    foreach ($name in @("bin", "web-dist")) {
      $existing = Join-Path $InstallDir $name
      if (Test-Path -LiteralPath $existing) {
        Move-Item -LiteralPath $existing -Destination (Join-Path $backup $name)
      }
      Move-Item -LiteralPath (Join-Path $stage $name) -Destination $existing
    }
    Write-Host "Installed Agena files"

    try {
      Write-Host "Starting Agena server using $ServiceMode mode"
      Start-Installed
      Write-Host "Started Agena server"
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
    Write-Host "Instance data: $DataDir"
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
  if ($PurgeData -and (Test-Path -LiteralPath $DataDir) -and -not (Test-Path -LiteralPath (Join-Path $DataDir "agena.db") -PathType Leaf)) {
    throw "refusing to purge a directory without an Agena instance database: $DataDir"
  }
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
    if (Test-Path -LiteralPath $DataDir) { Remove-Item -LiteralPath $DataDir -Recurse -Force }
    Write-Host "Removed Agena runtime data at $DataDir"
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
