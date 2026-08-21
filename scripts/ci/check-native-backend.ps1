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
  $BuildTargetDir = Join-Path $env:RUNNER_TEMP "agena-check-target\$TargetTriple"
  try {
    $env:RUSTC = $StableRustc
    $env:RUSTDOC = $StableRustdoc
    $env:RUSTC_BOOTSTRAP = "1"
    $env:CARGO_TARGET_DIR = $BuildTargetDir
    $RustFlags = @($OldRustFlags, $TargetRustFlags) | Where-Object { $_ }
    if ($TargetTriple -match "-win7-windows-(msvc|gnu)$") {
      # Build-std places the target's real libcore/libstd artifacts under the
      # target-specific debug deps directory.  Cargo passes that path to
      # normal target rustc invocations, but direct rustc probes launched by
      # build scripts (notably autocfg) only see CARGO_ENCODED_RUSTFLAGS.  Add
      # the actual build-std search path so those probes use the same target
      # standard library instead of falling back to the host sysroot.
      $BuildStdDeps = Join-Path $BuildTargetDir "$TargetTriple\debug\deps"
      $RustFlags += @("-L", "dependency=$BuildStdDeps")
    }
    $env:RUSTFLAGS = ($RustFlags | Join-String -Separator " ")
    Write-Host "Using build-std driver: cargo=$NightlyCargo rustc=$StableRustc rustdoc=$StableRustdoc target-dir=$BuildTargetDir"
    $CargoExtraArgs = @()
    if ($TargetTriple -match "-win7-windows-(msvc|gnu)$") {
      # Win7 custom target builds have historically failed only after the
      # dependency graph is built. Keep the target real, but expose the exact
      # rustc --extern/search-path command so a dependency-sysroot regression
      # cannot be diagnosed from E0463 names alone.
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
