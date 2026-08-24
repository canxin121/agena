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
$RunnerWorkspace = if ($env:GITHUB_WORKSPACE) {
  $env:GITHUB_WORKSPACE
} else {
  Split-Path -Parent $env:RUNNER_TEMP
}
$ReleaseDir = Join-Path $RepoRoot "artifacts/agena"
$WebDistDir = Join-Path $RepoRoot "packages/agena-web/dist"

if (-not $TargetTriple) {
  $TargetTriple = Get-HostTriple
}

$IsWindowsTarget = $TargetTriple -match "windows"
if ($IsWindowsTarget) {
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
if ($IsWindowsTarget) {
  $Ext = ".exe"
  $ArchiveExt = ".zip"
}

$RuntimeReadmeLine = ""
# Cargo creates the target-specific subdirectory below this short root. Keep
# the triple out of the root so release rustc commands do not repeat it in
# every dependency path.
$BuildTargetDir = Join-Path $RunnerWorkspace ".agena-target"

Write-Host "Building agena for $TargetTriple..."
$BuildArgs = @(
  "build",
  "--manifest-path", "$ServerManifest",
  "-p", "agena",
  "--release",
  "--target", "$TargetTriple",
  "--locked",
  "--target-dir", $BuildTargetDir
)
if ($BuildStd) {
  $StableToolchain = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }
  $NightlyToolchain = if ($env:AGENA_NIGHTLY_TOOLCHAIN) { $env:AGENA_NIGHTLY_TOOLCHAIN } else { "nightly-2026-08-18" }
  $StableRustc = (& rustup which --toolchain $StableToolchain rustc).Trim()
  $StableRustdoc = (& rustup which --toolchain $StableToolchain rustdoc).Trim()
  $NightlyCargo = (& rustup which --toolchain $NightlyToolchain cargo).Trim()
  $NightlyCargoExit = $LASTEXITCODE
  if ($LASTEXITCODE -ne 0 -or -not $StableRustc) {
    throw "Failed to locate rustc for $StableToolchain"
  }
  if (-not $StableRustdoc) {
    throw "Failed to locate rustdoc for $StableToolchain"
  }
  if ($NightlyCargoExit -ne 0 -or -not $NightlyCargo) {
    throw "Failed to locate Cargo for $NightlyToolchain"
  }
  $OldRustc = $env:RUSTC
  $OldRustdoc = $env:RUSTDOC
  $OldBootstrap = $env:RUSTC_BOOTSTRAP
  $OldRustFlags = $env:RUSTFLAGS
  $OldTargetDir = $env:CARGO_TARGET_DIR
  $OldCargoUnstableBuildDirNewLayout = $env:CARGO_UNSTABLE_BUILD_DIR_NEW_LAYOUT
  $OldAgenaRealRustc = $env:AGENA_REAL_RUSTC
  $OldAgenaBuildStdRoot = $env:AGENA_BUILD_STD_ROOT
  try {
    $env:RUSTC = $StableRustc
    $env:RUSTDOC = $StableRustdoc
    $env:RUSTC_BOOTSTRAP = "1"
    $env:CARGO_TARGET_DIR = $BuildTargetDir
    if ($TargetTriple -match "-windows-(msvc|gnu)$") {
      # Keep release target dependency paths below Windows' CreateProcess
      # command-line limit. The new build-dir layout expands every --extern
      # path into build/<crate>/<hash>/out and can make the real Agena graph
      # appear as E0463 even though no dependency is missing.
      $env:CARGO_UNSTABLE_BUILD_DIR_NEW_LAYOUT = "false"
    }
    $RustFlags = @($OldRustFlags, $TargetRustFlags) | Where-Object { $_ }
    if ($TargetTriple -match "-windows-(msvc|gnu)$") {
      # Build scripts such as autocfg invoke rustc directly with --target. The
      # shared wrapper below supplies the real target standard-library artifacts
      # for those probes. Keep target-only search paths out of global RUSTFLAGS
      # so host build scripts use the host sysroot.
      $BuildStdProfile = Join-Path $BuildTargetDir "$TargetTriple\release"
      $RustcWrapperDir = Join-Path $env:RUNNER_TEMP "agena-rustc-build-std\$TargetTriple"
      New-Item -ItemType Directory -Force -Path $RustcWrapperDir | Out-Null
      $RustcWrapperSource = Join-Path $RepoRoot "scripts\ci\rustc-build-std-wrapper.rs"
      $RustcWrapper = Join-Path $RustcWrapperDir "rustc-build-std-wrapper.exe"
      if (-not (Test-Path -LiteralPath $RustcWrapperSource)) {
        throw "shared build-std rustc wrapper missing at $RustcWrapperSource"
      }
      & $StableRustc $RustcWrapperSource -o $RustcWrapper
      if ($LASTEXITCODE -ne 0 -or -not (Test-Path $RustcWrapper)) {
        throw "failed to compile the build-std rustc wrapper"
      }
      $PrebuildScript = Join-Path $RepoRoot "scripts\ci\prebuild-build-std.ps1"
      & $PrebuildScript -TargetTriple $TargetTriple -NightlyCargo $NightlyCargo -TargetDir $BuildTargetDir -Release
      if ($LASTEXITCODE -ne 0) {
        throw "failed to prebuild the real build-std sysroot for $TargetTriple"
      }
      $env:AGENA_REAL_RUSTC = $StableRustc
      $env:AGENA_BUILD_STD_ROOT = $BuildStdProfile
      $env:RUSTC = $RustcWrapper
    }
    $env:RUSTFLAGS = ($RustFlags | Join-String -Separator " ")
    & cargo "+$NightlyToolchain" @BuildArgs -Z "build-std=std,panic_abort,proc_macro"
  }
  finally {
    $env:RUSTC = $OldRustc
    $env:RUSTDOC = $OldRustdoc
    $env:RUSTC_BOOTSTRAP = $OldBootstrap
    $env:RUSTFLAGS = $OldRustFlags
    $env:CARGO_TARGET_DIR = $OldTargetDir
    if ($null -eq $OldCargoUnstableBuildDirNewLayout) {
      Remove-Item Env:CARGO_UNSTABLE_BUILD_DIR_NEW_LAYOUT -ErrorAction SilentlyContinue
    } else {
      $env:CARGO_UNSTABLE_BUILD_DIR_NEW_LAYOUT = $OldCargoUnstableBuildDirNewLayout
    }
    if ($null -eq $OldAgenaRealRustc) {
      Remove-Item Env:AGENA_REAL_RUSTC -ErrorAction SilentlyContinue
    } else {
      $env:AGENA_REAL_RUSTC = $OldAgenaRealRustc
    }
    if ($null -eq $OldAgenaBuildStdRoot) {
      Remove-Item Env:AGENA_BUILD_STD_ROOT -ErrorAction SilentlyContinue
    } else {
      $env:AGENA_BUILD_STD_ROOT = $OldAgenaBuildStdRoot
    }
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
$RuntimeReadmeLine
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
