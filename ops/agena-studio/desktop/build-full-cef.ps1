Param(
  [string]$TargetTriple = ""
)

$ErrorActionPreference = "Stop"

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../../..")
$ScriptDir = Split-Path -Parent $PSCommandPath

if (-not (Get-Command cargo-tauri -ErrorAction SilentlyContinue)) {
  throw "cargo-tauri is required. Install with: cargo install tauri-cli --locked --git https://github.com/tauri-apps/tauri --rev 3b2823b918d5ea88fca10b472daf349c67c22d51"
}

& (Join-Path $RepoRoot "ops/agena-studio/scripts/build-frontend-dist.ps1")
if ($LASTEXITCODE -ne 0) {
  throw "build-frontend-dist.ps1 failed"
}

if ($TargetTriple) {
  & (Join-Path $ScriptDir "prepare-sidecar.ps1") -Cef -TargetTriple $TargetTriple
}
else {
  & (Join-Path $ScriptDir "prepare-sidecar.ps1") -Cef
}
if ($LASTEXITCODE -ne 0) {
  throw "prepare-sidecar.ps1 failed"
}

. (Join-Path $ScriptDir "import-vs-dev-environment.ps1") -TargetTriple $TargetTriple

$OriginalCargoTargetDir = $env:CARGO_TARGET_DIR
$env:CARGO_TARGET_DIR = Join-Path $RepoRoot "artifacts/t/cef"
New-Item -ItemType Directory -Force -Path $env:CARGO_TARGET_DIR | Out-Null

Push-Location (Join-Path $RepoRoot "apps/agena-studio-desktop/src-tauri-cef")
try {
  if ($TargetTriple) {
    & cargo tauri build --config tauri.conf.full.json --features cef --target $TargetTriple
  }
  else {
    & cargo tauri build --config tauri.conf.full.json --features cef
  }
  if ($LASTEXITCODE -ne 0) {
    throw "cargo tauri build failed"
  }

  if ($TargetTriple) {
    $BundleSourceDir = Join-Path $env:CARGO_TARGET_DIR "$TargetTriple/release/bundle"
    $BundleExportDir = Join-Path $RepoRoot "artifacts/agena-studio/desktop/$TargetTriple/cef"
  }
  else {
    $BundleSourceDir = Join-Path $env:CARGO_TARGET_DIR "release/bundle"
    $BundleExportDir = Join-Path $RepoRoot "artifacts/agena-studio/desktop/host/cef"
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
