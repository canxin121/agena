Param(
  [Parameter(Mandatory = $true)]
  [string]$TargetTriple,
  [bool]$BuildStd = $false,
  [string]$TargetRustFlags = ""
)

$ErrorActionPreference = "Stop"
$env:RUSTUP_TOOLCHAIN = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }

if ($TargetTriple -match "windows") {
  & "$PSScriptRoot/setup-windows-toolchain.ps1" -TargetTriple $TargetTriple
}

$Args = @(
  "check",
  "--manifest-path", "Cargo.toml",
  "-p", "agena",
  "--target", $TargetTriple,
  "--target-dir", (Join-Path $env:RUNNER_TEMP "agena-check-target\$TargetTriple"),
  "--locked"
)

if ($BuildStd) {
  $StableRustc = (& rustup which --toolchain 1.97.0 rustc).Trim()
  $StableRustcExit = $LASTEXITCODE
  $StableRustdoc = (& rustup which --toolchain 1.97.0 rustdoc).Trim()
  $StableRustdocExit = $LASTEXITCODE
  $NightlyCargo = (& rustup which --toolchain nightly-2026-08-18 cargo).Trim()
  $NightlyCargoExit = $LASTEXITCODE
  if ($StableRustcExit -ne 0 -or -not $StableRustc) {
    throw "Failed to locate Rust 1.97.0 rustc"
  }
  if ($StableRustdocExit -ne 0 -or -not $StableRustdoc) {
    throw "Failed to locate Rust 1.97.0 rustdoc"
  }
  if ($NightlyCargoExit -ne 0 -or -not $NightlyCargo) {
    throw "Failed to locate nightly-2026-08-18 Cargo"
  }
  $OldRustc = $env:RUSTC
  $OldRustdoc = $env:RUSTDOC
  $OldBootstrap = $env:RUSTC_BOOTSTRAP
  $OldRustFlags = $env:RUSTFLAGS
  $OldTargetDir = $env:CARGO_TARGET_DIR
  $OldCargoBuildJobs = $env:CARGO_BUILD_JOBS
  $BuildTargetDir = Join-Path $env:RUNNER_TEMP "agena-check-target\$TargetTriple"
  try {
    $env:RUSTC = $StableRustc
    $env:RUSTDOC = $StableRustdoc
    $env:RUSTC_BOOTSTRAP = "1"
    $env:CARGO_TARGET_DIR = $BuildTargetDir
    $RustFlags = @($OldRustFlags, $TargetRustFlags) | Where-Object { $_ }
    if ($TargetTriple -match "-windows-(msvc|gnu)$") {
      # Windows build-std places the target's real libcore/libstd artifacts
      # under the target-specific debug deps directory. Cargo passes that path
      # to normal target rustc invocations, but direct rustc probes launched by
      # build scripts (notably autocfg) only see CARGO_ENCODED_RUSTFLAGS. Add
      # the actual build-std search path so those probes use the same target
      # standard library instead of falling back to the host sysroot.
      $BuildStdProfile = Join-Path $BuildTargetDir "$TargetTriple\debug"
      $BuildStdDeps = Join-Path $BuildStdProfile "deps"
      $RustFlags += @("-L", "dependency=$BuildStdDeps")

      # Build-std emits core/std into hashed build-script output directories,
      # not only into debug\deps. Cargo knows those paths for ordinary target
      # rustc invocations, but build scripts such as autocfg invoke RUSTC
      # directly. Use the shared wrapper so those probes receive the exact
      # target artifacts generated in this target directory. This applies to
      # every Windows target built from source, including thumbv7a.
      $RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../..")
      $RustcWrapperDir = Join-Path $env:RUNNER_TEMP "agena-rustc-build-std\$TargetTriple"
      New-Item -ItemType Directory -Force -Path $RustcWrapperDir | Out-Null
      $RustcWrapperSource = Join-Path $RepoRoot "scripts\ci\rustc-build-std-wrapper.rs"
      $RustcWrapper = Join-Path $RustcWrapperDir "rustc-build-std-wrapper.exe"
      $OldAgenaRealRustc = $env:AGENA_REAL_RUSTC
      $OldAgenaBuildStdRoot = $env:AGENA_BUILD_STD_ROOT
      if (-not (Test-Path -LiteralPath $RustcWrapperSource)) {
        throw "shared build-std rustc wrapper missing at $RustcWrapperSource"
      }
      & $StableRustc $RustcWrapperSource -o $RustcWrapper
      if ($LASTEXITCODE -ne 0 -or -not (Test-Path $RustcWrapper)) {
        throw "failed to compile the build-std rustc wrapper"
      }
      $env:AGENA_REAL_RUSTC = $StableRustc
      $env:AGENA_BUILD_STD_ROOT = $BuildStdProfile
      $env:RUSTC = $RustcWrapper
      # Direct build-script rustc probes wait for the real target std artifacts.
      # With Cargo's normal parallel scheduler those probes can occupy every
      # jobserver slot before build-std finishes compiling std. Serialize this
      # target build so the real core/alloc/std sequence completes first;
      # never substitute the host sysroot or a synthetic artifact.
      $env:CARGO_BUILD_JOBS = "1"
    }
    $env:RUSTFLAGS = ($RustFlags | Join-String -Separator " ")
    Write-Host "Using build-std driver: cargo=$NightlyCargo rustc=$StableRustc rustdoc=$StableRustdoc target-dir=$BuildTargetDir"
    $CargoExtraArgs = @()
    if ($TargetTriple -match "-windows-(msvc|gnu)$") {
      # Windows build-std target builds can fail only after the dependency
      # graph is built. Keep the target real, but expose the exact rustc
      # --extern/search-path command so a dependency-sysroot regression cannot
      # be diagnosed from E0463 names alone.
      $CargoExtraArgs += "-vv"
    }
    & $NightlyCargo @Args @CargoExtraArgs -Z "build-std=std,panic_abort,proc_macro"
  }
  finally {
    $env:RUSTC = $OldRustc
    $env:RUSTDOC = $OldRustdoc
    $env:RUSTC_BOOTSTRAP = $OldBootstrap
    $env:RUSTFLAGS = $OldRustFlags
    $env:CARGO_TARGET_DIR = $OldTargetDir
    if ($null -eq $OldCargoBuildJobs) {
      Remove-Item Env:CARGO_BUILD_JOBS -ErrorAction SilentlyContinue
    } else {
      $env:CARGO_BUILD_JOBS = $OldCargoBuildJobs
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
    & cargo @Args
  }
  finally {
    $env:RUSTFLAGS = $OldRustFlags
  }
}

if ($LASTEXITCODE -ne 0) {
  throw "cargo check failed for $TargetTriple"
}
