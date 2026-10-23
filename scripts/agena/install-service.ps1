Param(
  [string]$Archive = "",
  [string]$Repo = "canxin121/agena",
  [string]$Version = "",
  [string]$InstallDir = "",
  [string]$DataDir = "",
  [string]$ListenHost = "127.0.0.1",
  [int]$Port = 3210,
  [string]$UiPassword = "",
  [string]$WorkspaceRoot = "",
  [string[]]$Set = @()
)

# Install the released Agena backend as a user scheduled task.

$ErrorActionPreference = "Stop"

if (-not $InstallDir) {
  $InstallDir = Join-Path $env:LOCALAPPDATA "Agena"
}
if (-not $DataDir) {
  if ($env:AGENA_DATA_DIR) { $DataDir = $env:AGENA_DATA_DIR }
  else { $DataDir = Join-Path $HOME "agena" }
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

Assert-StoragePaths


function Normalize-Version([string]$Raw) {
  if ($Raw.StartsWith("agena-v")) {
    return $Raw.Substring(7)
  }
  if ($Raw.StartsWith("v")) {
    return $Raw.Substring(1)
  }
  return $Raw
}

function Get-TargetTriple {
  $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
  switch ($arch) {
    "X64" { return "x86_64-pc-windows-msvc" }
    "Arm64" { return "aarch64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $arch" }
  }
}

if (-not $Archive) {
  if (-not $Version) {
    throw "Provide -Archive or -Version"
  }
  $NormalizedVersion = Normalize-Version $Version
  if ($NormalizedVersion -notmatch '^\d+\.\d+\.\d+-beta\.[1-9]\d*$') {
    throw "-Version must name a beta release, for example 0.1.0-beta.1"
  }
  $PackageVersion = $NormalizedVersion -replace '-beta\.[1-9]\d*$', ''
  $TargetTriple = Get-TargetTriple
  $Archive = "https://github.com/$Repo/releases/download/agena-v$NormalizedVersion/agena-backend-$TargetTriple-v$PackageVersion.zip"
}

$TempDir = Join-Path $env:TEMP ("agena-install-" + [Guid]::NewGuid().ToString("N"))
New-Item -ItemType Directory -Force -Path $TempDir | Out-Null

try {
  $ArchivePath = Join-Path $TempDir "backend.zip"
  if ($Archive -match '^https?://') {
    Invoke-WebRequest -Uri $Archive -OutFile $ArchivePath
  }
  else {
    Copy-Item -LiteralPath $Archive -Destination $ArchivePath -Force
  }

  $StageDir = Join-Path $TempDir "extract"
  Expand-Archive -LiteralPath $ArchivePath -DestinationPath $StageDir -Force

  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  foreach ($name in @("bin", "web-dist")) {
    $path = Join-Path $InstallDir $name
    if (Test-Path -LiteralPath $path) {
      Remove-Item -LiteralPath $path -Recurse -Force
    }
  }

  Copy-Item -LiteralPath (Join-Path $StageDir "bin") -Destination (Join-Path $InstallDir "bin") -Recurse -Force
  Copy-Item -LiteralPath (Join-Path $StageDir "web-dist") -Destination (Join-Path $InstallDir "web-dist") -Recurse -Force

  $BinaryPath = Join-Path $InstallDir "bin\agena.exe"
  if (-not (Test-Path -LiteralPath $BinaryPath)) {
    throw "Installed binary not found: $BinaryPath"
  }

  $ArgList = @(
    "--data-dir", $DataDir,
    "server", "install",
    "--host", $ListenHost,
    "--port", "$Port",
    "--ui-dir", (Join-Path $InstallDir "web-dist")
  )
  if ($UiPassword) { $ArgList += @("--ui-password", $UiPassword) }
  if ($WorkspaceRoot) { $ArgList += @("--workspace", $WorkspaceRoot) }
  foreach ($item in $Set) {
    $ArgList += @("--set", $item)
  }

  & $BinaryPath @ArgList
  if ($LASTEXITCODE -ne 0) { throw "Agena native user service installation failed" }
  @{ schema = 2; dataDir = $DataDir } | ConvertTo-Json |
    Set-Content -LiteralPath (Join-Path $InstallDir "service-state.json") -Encoding UTF8
  Write-Host "Installed Agena user service. Instance data: $DataDir"

}
finally {
  if (Test-Path -LiteralPath $TempDir) {
    Remove-Item -LiteralPath $TempDir -Recurse -Force
  }
}
