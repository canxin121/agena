Param(
  [string]$TargetTriple = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../../..")
$ScriptDir = Split-Path -Parent $PSCommandPath

& (Join-Path $RepoRoot "ops/agena-studio/scripts/build-frontend-dist.ps1")
if ($LASTEXITCODE -ne 0) {
  throw "build-frontend-dist.ps1 failed"
}

if ($TargetTriple) {
  & (Join-Path $ScriptDir "prepare-sidecar.ps1") -TargetTriple $TargetTriple
}
else {
  & (Join-Path $ScriptDir "prepare-sidecar.ps1")
}
if ($LASTEXITCODE -ne 0) {
  throw "prepare-sidecar.ps1 failed"
}

. (Join-Path $ScriptDir "import-vs-dev-environment.ps1") -TargetTriple $TargetTriple

$OriginalCargoTargetDir = $env:CARGO_TARGET_DIR
$env:CARGO_TARGET_DIR = Join-Path $RepoRoot "artifacts/t/desktop"
New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR | Out-Null

Push-Location (Join-Path $RepoRoot "apps/agena-studio-desktop/src-tauri")
try {
  if ($TargetTriple) {
    & cargo tauri build --config tauri.conf.full.json --target $TargetTriple
  }
  else {
    & cargo tauri build --config tauri.conf.full.json
  }
  if ($LASTEXITCODE -ne 0) {
    throw "cargo tauri build failed"
  }

  if ($TargetTriple) {
    $BundleSourceDir = Join-Path $env:CARGO_TARGET_DIR "$TargetTriple/release/bundle"
    $BundleExportDir = Join-Path $RepoRoot "artifacts/agena-studio/desktop/$TargetTriple/standard"
  }
  else {
    $BundleSourceDir = Join-Path $env:CARGO_TARGET_DIR "release/bundle"
    $BundleExportDir = Join-Path $RepoRoot "artifacts/agena-studio/desktop/host/standard"
  }

  if (-not (Test-Path -LiteralPath $BundleSourceDir)) {
    throw "bundle output not found: $BundleSourceDir"
  }

  if (Test-Path -LiteralPath $BundleExportDir) {
    Remove-Item -LiteralPath $BundleExportDir -Recurse -Force
  }
  New-Item -ItemType Directory -Force -Path (Split-Path -Parent $BundleExportDir) | Out-Null
  Copy-Item -LiteralPath $BundleSourceDir -Destination $BundleExportDir -Recurse
  Write-Host "Desktop bundle exported: $BundleExportDir"
}
finally {
  Pop-Location
  if ($null -eq $OriginalCargoTargetDir) {
    Remove-Item Env:CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  }
  else {
    $env:CARGO_TARGET_DIR = $OriginalCargoTargetDir
  }
}
