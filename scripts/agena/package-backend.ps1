Param(
  [string]$TargetTriple = ""
)

$ErrorActionPreference = "Stop"

function Get-HostTriple {
  try {
    $triple = (& rustc --print host-tuple).Trim()
    if ($triple) { return $triple }
  }
  catch {}

  $vv = & rustc -Vv
  foreach ($line in $vv) {
    if ($line -match '^host:\s+(\S+)') {
      return $Matches[1]
    }
  }

  throw "Unable to determine Rust host triple"
}

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../..")
$ScriptsDir = Split-Path -Parent $PSCommandPath
$ServerManifest = Join-Path $RepoRoot "Cargo.toml"
$ServerTargetDir = Join-Path $RepoRoot "target"
$ReleaseDir = Join-Path $RepoRoot "artifacts/agena"
$WebDistDir = Join-Path $RepoRoot "packages/agena-web-ui/dist"

if (-not $TargetTriple) {
  $TargetTriple = Get-HostTriple
}

$metadataJson = & cargo metadata --manifest-path "$ServerManifest" --format-version 1 --no-deps --locked | Out-String
if ($LASTEXITCODE -ne 0) {
  throw "Failed to read Cargo metadata from $ServerManifest"
}
$metadata = $metadataJson | ConvertFrom-Json
$agenaPackage = $metadata.packages | Where-Object { $_.name -eq "agena" } | Select-Object -First 1
if (-not $agenaPackage) {
  throw "Cargo metadata does not contain the agena package"
}
$Version = $agenaPackage.version

$Ext = ""
$ArchiveExt = ".tar.gz"
if ($TargetTriple -match 'windows') {
  $Ext = ".exe"
  $ArchiveExt = ".zip"
}

& (Join-Path $ScriptsDir "build-frontend-dist.ps1")
if ($LASTEXITCODE -ne 0) {
  throw "build-frontend-dist.ps1 failed"
}

Write-Host "Building agena backend for $TargetTriple..."
& cargo build `
  --manifest-path "$ServerManifest" `
  --release `
  --target "$TargetTriple" `
  --locked `
  --target-dir "$ServerTargetDir"

if ($LASTEXITCODE -ne 0) {
  throw "cargo build failed"
}

$BinPath = Join-Path $ServerTargetDir "$TargetTriple/release/agena$Ext"
if (-not (Test-Path -LiteralPath $BinPath)) {
  throw "Built backend binary not found at $BinPath"
}

$StageDir = Join-Path $ReleaseDir "backend/$TargetTriple"
if (Test-Path -LiteralPath $StageDir) {
  Remove-Item -LiteralPath $StageDir -Recurse -Force
}

$StageBinDir = Join-Path $StageDir "bin"
$StageWebDir = Join-Path $StageDir "web-dist"
New-Item -ItemType Directory -Force -Path $StageBinDir, $StageWebDir | Out-Null

Copy-Item -LiteralPath $BinPath -Destination (Join-Path $StageBinDir "agena$Ext") -Force
Get-ChildItem -LiteralPath $WebDistDir | ForEach-Object {
  Copy-Item -LiteralPath $_.FullName -Destination $StageWebDir -Recurse -Force
}

$Readme = @"
Agena server package
Version: $Version
Target: $TargetTriple

Contents:
- bin/agena$Ext
- web-dist/

Example:
  agena server --ui-dir .\web-dist
"@
Set-Content -LiteralPath (Join-Path $StageDir "README.txt") -Value $Readme

New-Item -ItemType Directory -Force -Path $ReleaseDir | Out-Null
$ArchiveName = "agena-backend-$TargetTriple-v$Version$ArchiveExt"
$ArchivePath = Join-Path $ReleaseDir $ArchiveName
if (Test-Path -LiteralPath $ArchivePath) {
  Remove-Item -LiteralPath $ArchivePath -Force
}

if ($ArchiveExt -eq ".zip") {
  Push-Location $StageDir
  try {
    Compress-Archive -Path * -DestinationPath $ArchivePath -Force
  }
  finally {
    Pop-Location
  }
}
else {
  & tar -C $StageDir -czf $ArchivePath .
  if ($LASTEXITCODE -ne 0) {
    throw "tar packaging failed"
  }
}

Write-Host "Backend package ready: $ArchivePath"
