Param(
  [string]$TargetTriple = "",
  [bool]$BuildStd = $false,
  [string]$TargetRustFlags = ""
)

$ErrorActionPreference = "Stop"

function Get-HostTriple {
  try {
    $triple = (& rustc --print host-tuple).Trim()
    if ($LASTEXITCODE -eq 0 -and $triple) { return $triple }
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

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "../..")).Path
$ServerManifest = Join-Path $RepoRoot "Cargo.toml"
$ReleaseDir = Join-Path $RepoRoot "artifacts/agena"
$WebProjectDir = Join-Path $RepoRoot "packages/agena-web"
$WebDistDir = Join-Path $WebProjectDir "dist"

function Invoke-AgenaPackage {
  foreach ($tool in @("cargo", "rustc")) {
    if (-not (Get-Command $tool -ErrorAction SilentlyContinue)) {
      throw "$tool is required to build Agena from source"
    }
  }

  if ($env:AGENA_WEB_DIST_PREBUILT -ne "1") {
    $bun = Get-Command bun -ErrorAction SilentlyContinue
    if (-not $bun) { throw "bun is required to build the Agena Web frontend" }
    & $bun.Source (Join-Path $PSScriptRoot "build-web.mjs") $WebProjectDir
    if ($LASTEXITCODE -ne 0) { throw "Agena Web build failed with exit code $LASTEXITCODE" }
  }

  $HostTriple = Get-HostTriple
  if (-not $TargetTriple) { $TargetTriple = $HostTriple }

  $IsWindowsTarget = $TargetTriple -match "windows"
  if (-not $IsWindowsTarget -and -not (Get-Command tar -ErrorAction SilentlyContinue)) {
    throw "tar is required to build Agena from source"
  }
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
  if (-not $metadata.target_directory) {
    throw "Cargo metadata does not contain a target directory"
  }
  $BuildTargetDir = [System.IO.Path]::GetFullPath([string]$metadata.target_directory)

  if (-not (Test-Path -LiteralPath (Join-Path $WebDistDir "index.html") -PathType Leaf)) {
    throw "Prebuilt Agena Web frontend not found at $WebDistDir"
  }

  $Ext = ""
  $ArchiveExt = ".tar.gz"
  if ($IsWindowsTarget) {
    $Ext = ".exe"
    $ArchiveExt = ".zip"
  }

  Write-Host "Building agena for $TargetTriple..."
  $BuildArgs = @(
    "build",
    "--manifest-path", "$ServerManifest",
    "-p", "agena",
    "--bin", "agena",
    "--release",
    "--locked",
    "--target-dir", $BuildTargetDir,
    "--message-format=json-render-diagnostics"
  )
  # A native --target creates a separate Cargo graph. Share the ordinary release
  # graph unless cross compilation or target-only flags require that separation.
  $ExplicitTarget = ($TargetTriple -ne $HostTriple -or $BuildStd -or
    $TargetRustFlags -or $env:CARGO_BUILD_TARGET -or $env:AGENA_FORCE_EXPLICIT_TARGET -eq "1")
  if ($ExplicitTarget) { $BuildArgs += @("--target", $TargetTriple) }

  $BuildEnvironment = @{}
  foreach ($name in @("RUSTC", "RUSTDOC", "RUSTC_BOOTSTRAP", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS",
      "CARGO_TARGET_DIR", "CARGO_UNSTABLE_BUILD_DIR_NEW_LAYOUT", "AGENA_REAL_RUSTC",
      "AGENA_BUILD_STD_ROOT")) {
    $BuildEnvironment[$name] = [Environment]::GetEnvironmentVariable($name)
  }
  $RustTimer = [System.Diagnostics.Stopwatch]::StartNew()
  Write-Host "Building Rust backend (Cargo cache: $BuildTargetDir)..."
  try {
    # Do not export empty RUSTFLAGS: that would suppress workspace Cargo config.
    if (-not [string]::IsNullOrWhiteSpace($TargetRustFlags)) {
      if ($null -ne $BuildEnvironment["CARGO_ENCODED_RUSTFLAGS"]) {
        # TargetRustFlags has the same whitespace-separated format as RUSTFLAGS.
        # Preserve the caller's encoded arguments, including arguments with spaces.
        $EncodedParts = @()
        if ($env:CARGO_ENCODED_RUSTFLAGS) { $EncodedParts += $env:CARGO_ENCODED_RUSTFLAGS }
        $EncodedParts += $TargetRustFlags.Trim() -split '\s+'
        $env:CARGO_ENCODED_RUSTFLAGS = $EncodedParts -join [char]31
      } else {
        $env:RUSTFLAGS = (@($env:RUSTFLAGS, $TargetRustFlags) | Where-Object { $_ }) -join " "
      }
    }
    if ($BuildStd) {
      $StableToolchain = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }
      $NightlyToolchain = if ($env:AGENA_NIGHTLY_TOOLCHAIN) { $env:AGENA_NIGHTLY_TOOLCHAIN } else { "nightly-2026-08-18" }
      $StableRustc = (& rustup which --toolchain $StableToolchain rustc).Trim()
      $StableRustcExit = $LASTEXITCODE
      $StableRustdoc = (& rustup which --toolchain $StableToolchain rustdoc).Trim()
      $StableRustdocExit = $LASTEXITCODE
      $NightlyCargo = (& rustup which --toolchain $NightlyToolchain cargo).Trim()
      $NightlyCargoExit = $LASTEXITCODE
      if ($StableRustcExit -ne 0 -or -not $StableRustc) {
        throw "Failed to locate rustc for $StableToolchain"
      }
      if ($StableRustdocExit -ne 0 -or -not $StableRustdoc) {
        throw "Failed to locate rustdoc for $StableToolchain"
      }
      if ($NightlyCargoExit -ne 0 -or -not $NightlyCargo) {
        throw "Failed to locate Cargo for $NightlyToolchain"
      }
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
        # Build scripts such as autocfg invoke rustc directly with --target. The
        # shared wrapper supplies the real target standard-library artifacts for
        # those probes without adding target-only search paths to host RUSTFLAGS.
        $BuildStdProfile = Join-Path $BuildTargetDir "$TargetTriple\release"
        $WrapperRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
        $RustcWrapperDir = Join-Path $WrapperRoot "agena-rustc-build-std\$TargetTriple"
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
      $BuildMessages = & cargo "+$NightlyToolchain" @BuildArgs -Z "build-std=std,panic_abort,proc_macro"
    }
    else {
      $BuildMessages = & cargo @BuildArgs
    }
    $CargoExit = $LASTEXITCODE
  }
  finally {
    foreach ($name in $BuildEnvironment.Keys) {
      if ($null -eq $BuildEnvironment[$name]) {
        Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
      } else {
        [Environment]::SetEnvironmentVariable($name, $BuildEnvironment[$name])
      }
    }
  }
  if ($CargoExit -ne 0) {
    throw "cargo build failed with exit code $CargoExit"
  }
  $RustTimer.Stop()
  Write-Host "Rust backend ready in $([math]::Round($RustTimer.Elapsed.TotalSeconds, 1))s."

  # Use the artifact Cargo actually reported, including a configured native
  # build.target, and reject stale or different-target executables.
  $Artifacts = @(
    foreach ($line in $BuildMessages) {
      try { $item = $line | ConvertFrom-Json -ErrorAction Stop } catch { continue }
      if ($item.reason -eq "compiler-artifact" -and $item.target.name -eq "agena" -and
          $item.target.kind -contains "bin" -and $item.executable) {
        [System.IO.Path]::GetFullPath([string]$item.executable)
      }
    }
  ) | Select-Object -Unique
  $Artifacts = @($Artifacts)
  $AllowedArtifacts = @([System.IO.Path]::GetFullPath((Join-Path $BuildTargetDir "$TargetTriple/release/agena$Ext")))
  if (-not $ExplicitTarget) {
    $AllowedArtifacts += [System.IO.Path]::GetFullPath((Join-Path $BuildTargetDir "release/agena$Ext"))
  }
  if ($Artifacts.Count -ne 1 -or $AllowedArtifacts -notcontains $Artifacts[0]) {
    throw "Cargo did not report the requested Agena executable; check build.target or set AGENA_FORCE_EXPLICIT_TARGET=1"
  }
  $BinPath = $Artifacts[0]
  if (-not (Test-Path -LiteralPath $BinPath -PathType Leaf)) {
    throw "Built binary not found at $BinPath"
  }

  $ArchiveName = "agena-backend-$TargetTriple-v$Version$ArchiveExt"
  $ArchivePath = Join-Path $ReleaseDir $ArchiveName
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
  if (Test-Path -LiteralPath $ArchivePath) {
    Remove-Item -LiteralPath $ArchivePath -Force
  }
  Write-Host "Creating package archive..."
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
}

# --manifest-path alone does not load this workspace's .cargo/config.toml.
# Toolchain setup also changes PATH, compiler flags and SDK variables. Restore
# the entire caller environment so repeated installs cannot accumulate flags.
$OriginalEnvironment = [Environment]::GetEnvironmentVariables()
Push-Location $RepoRoot
try {
  $env:RUSTUP_TOOLCHAIN = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }
  Invoke-AgenaPackage
}
finally {
  foreach ($name in [Environment]::GetEnvironmentVariables().Keys) {
    if (-not $OriginalEnvironment.Contains($name)) {
      Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue
    }
  }
  foreach ($name in $OriginalEnvironment.Keys) {
    [Environment]::SetEnvironmentVariable($name, $OriginalEnvironment[$name])
  }
  Pop-Location
}
