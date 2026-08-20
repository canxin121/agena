Param(
  [string]$TargetTriple = "",
  [bool]$BuildStd = $false,
  [string]$TargetRustFlags = ""
)

$ErrorActionPreference = "Stop"
$env:RUSTUP_TOOLCHAIN = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }

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
$ServerManifest = Join-Path $RepoRoot "Cargo.toml"
$ServerTargetDir = Join-Path $RepoRoot "target"
$ReleaseDir = Join-Path $RepoRoot "artifacts/agena"
$WebDistDir = Join-Path $RepoRoot "packages/agena-web/dist"

if (-not $TargetTriple) {
  $TargetTriple = Get-HostTriple
}

if ($TargetTriple -match "windows") {
  & (Join-Path $RepoRoot "scripts/ci/setup-windows-toolchain.ps1") -TargetTriple $TargetTriple
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

if (-not (Test-Path -LiteralPath (Join-Path $WebDistDir "index.html"))) {
  throw "Prebuilt Agena Web frontend not found at $WebDistDir"
}

$Ext = ""
$ArchiveExt = ".tar.gz"
if ($TargetTriple -match 'windows') {
  $Ext = ".exe"
  $ArchiveExt = ".zip"
}

Write-Host "Building agena for $TargetTriple..."
$BuildArgs = @(
  "build",
  "--manifest-path", "$ServerManifest",
  "-p", "agena",
  "--release",
  "--target", "$TargetTriple",
  "--locked",
  "--target-dir", (Join-Path $env:RUNNER_TEMP "agena-release-target\$TargetTriple")
)
$BuildTargetDir = Join-Path $env:RUNNER_TEMP "agena-release-target\$TargetTriple"
if ($BuildStd) {
  $StableToolchain = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }
  $NightlyToolchain = if ($env:AGENA_NIGHTLY_TOOLCHAIN) { $env:AGENA_NIGHTLY_TOOLCHAIN } else { "nightly-2026-08-18" }
  $StableRustc = (& rustup which --toolchain $StableToolchain rustc).Trim()
  $StableRustdoc = (& rustup which --toolchain $StableToolchain rustdoc).Trim()
  if ($LASTEXITCODE -ne 0 -or -not $StableRustc) {
    throw "Failed to locate rustc for $StableToolchain"
  }
  if (-not $StableRustdoc) {
    throw "Failed to locate rustdoc for $StableToolchain"
  }
  $OldRustc = $env:RUSTC
  $OldRustdoc = $env:RUSTDOC
  $OldBootstrap = $env:RUSTC_BOOTSTRAP
  $OldRustFlags = $env:RUSTFLAGS
  $OldTargetDir = $env:CARGO_TARGET_DIR
  try {
    $env:RUSTC = $StableRustc
    $env:RUSTDOC = $StableRustdoc
    $env:RUSTC_BOOTSTRAP = "1"
    $env:CARGO_TARGET_DIR = $BuildTargetDir
    $env:RUSTFLAGS = (($OldRustFlags, $TargetRustFlags) | Where-Object { $_ } | Join-String -Separator " ")
    & cargo "+$NightlyToolchain" @BuildArgs -Z "build-std=std,panic_abort,proc_macro"
  }
  finally {
    $env:RUSTC = $OldRustc
    $env:RUSTDOC = $OldRustdoc
    $env:RUSTC_BOOTSTRAP = $OldBootstrap
    $env:RUSTFLAGS = $OldRustFlags
    $env:CARGO_TARGET_DIR = $OldTargetDir
  }
}
else {
  $OldRustFlags = $env:RUSTFLAGS
  try {
    $env:RUSTFLAGS = (($OldRustFlags, $TargetRustFlags) | Where-Object { $_ } | Join-String -Separator " ")
    & cargo @BuildArgs
  }
  finally {
    $env:RUSTFLAGS = $OldRustFlags
  }
}

if ($LASTEXITCODE -ne 0) {
  throw "cargo build failed"
}

$BinPath = Join-Path $BuildTargetDir "$TargetTriple/release/agena$Ext"
if (-not (Test-Path -LiteralPath $BinPath)) {
  throw "Built binary not found at $BinPath"
}

$StageDir = Join-Path $ReleaseDir "backend/$TargetTriple"
if (Test-Path -LiteralPath $StageDir) {
  Remove-Item -LiteralPath $StageDir -Recurse -Force
}

$StageBinDir = Join-Path $StageDir "bin"
$StageWebDir = Join-Path $StageDir "web-dist"
New-Item -ItemType Directory -Force -Path $StageBinDir | Out-Null
New-Item -ItemType Directory -Force -Path $StageWebDir | Out-Null

Copy-Item -LiteralPath $BinPath -Destination (Join-Path $StageBinDir "agena$Ext") -Force
Copy-Item -Path (Join-Path $WebDistDir "*") -Destination $StageWebDir -Recurse -Force

$Readme = @"
Agena package
Version: $Version
Target: $TargetTriple

Contents:
- bin/agena$Ext
- web-dist/ (served by the Agena server on the same host and port)
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

$Hash = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
$ChecksumPath = "$ArchivePath.sha256"
Set-Content -LiteralPath $ChecksumPath -Value "$Hash  $ArchiveName"

Write-Host "Package ready: $ArchivePath"
Write-Host "Checksum ready: $ChecksumPath"
