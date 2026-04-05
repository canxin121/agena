Param(
  [string]$TargetTriple = "",
  [switch]$Cef
)

$ErrorActionPreference = "Stop"

function Get-HostTriple {
  try {
    $t = (& rustc --print host-tuple).Trim()
    if ($t) { return $t }
  } catch {}

  $vv = (& rustc -Vv)
  foreach ($line in $vv) {
    if ($line -match '^host:\s+(\S+)') { return $Matches[1] }
  }
  throw "Unable to determine Rust host triple"
}

if (-not $TargetTriple) {
  $TargetTriple = Get-HostTriple
}

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../../..")
$ServerManifest = Join-Path $RepoRoot "apps/agena-studio-server/Cargo.toml"
$ServerTargetDir = Join-Path $RepoRoot "target"

$tauriVariant = "src-tauri"
if ($Cef) { $tauriVariant = "src-tauri-cef" }

$TauriBinDir = Join-Path $RepoRoot "apps/agena-studio-desktop/$tauriVariant/binaries"

$Ext = ""
if ($TargetTriple -match 'windows') { $Ext = ".exe" }

Write-Host "Building backend service binary for $TargetTriple..."
& cargo build --manifest-path "$ServerManifest" --release --target "$TargetTriple" --locked --target-dir "$ServerTargetDir"

$SrcBin = Join-Path $ServerTargetDir "$TargetTriple/release/agena-studio$Ext"
if (-not (Test-Path $SrcBin)) {
  throw "Built binary not found at: $SrcBin"
}

New-Item -ItemType Directory -Force -Path $TauriBinDir | Out-Null
$DestBin = Join-Path $TauriBinDir "agena-studio-$TargetTriple$Ext"
Copy-Item -Force $SrcBin $DestBin

Write-Host "Backend binary ready: $DestBin"
