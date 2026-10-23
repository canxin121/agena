Param(
  [string]$InstallDir = "",
  [string]$DataDir = "",
  [switch]$RemoveInstallDir
)

# Remove the native Agena user service; instance data is preserved.
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

Assert-StoragePaths -InstallOnly

$serviceState = Join-Path $InstallDir "service-state.json"
$installState = Join-Path $InstallDir "install-state.json"
if (Test-Path -LiteralPath $serviceState) {
  $state = Get-Content -LiteralPath $serviceState -Raw | ConvertFrom-Json
} elseif (Test-Path -LiteralPath $installState) {
  $state = Get-Content -LiteralPath $installState -Raw | ConvertFrom-Json
} else {
  throw "no Agena installer state found at $InstallDir"
}
if ($state.schema -ne 2 -or -not $state.dataDir) { throw "incompatible Agena installer state" }
if ($state.installDir -and (Resolve-Directory ([string]$state.installDir)) -ne $InstallDir) {
  throw "installer state belongs to a different installation"
}
$DataDir = [string]$state.dataDir
Assert-StoragePaths
$binary = Join-Path $InstallDir "bin\agena.exe"
if (-not (Test-Path -LiteralPath $binary)) { throw "native service uninstall requires $binary" }
& $binary --data-dir $DataDir server uninstall
if ($LASTEXITCODE -ne 0) { throw "Agena native user service uninstall failed" }
if ($RemoveInstallDir) {
  Remove-Item -LiteralPath $InstallDir -Recurse -Force
}
Write-Host "Agena background service removed. Instance data preserved at $DataDir"
