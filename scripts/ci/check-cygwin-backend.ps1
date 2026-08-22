Param(
  [string]$TargetTriple = "x86_64-pc-cygwin"
)

$ErrorActionPreference = "Stop"
$StableToolchain = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }
$NightlyToolchain = if ($env:AGENA_NIGHTLY_TOOLCHAIN) { $env:AGENA_NIGHTLY_TOOLCHAIN } else { "nightly-2026-08-18" }

if ($TargetTriple -ne "x86_64-pc-cygwin") {
  throw "This check script only supports the manifest target x86_64-pc-cygwin, not $TargetTriple"
}

$Linker = $env:CARGO_TARGET_X86_64_PC_CYGWIN_LINKER
$CygwinRoot = $env:AGENA_CYGWIN_ROOT
if (-not $Linker -or -not (Test-Path -LiteralPath $Linker -PathType Leaf)) {
  throw "The official Cygwin linker is required; run setup-cygwin-toolchain.ps1 before this check"
}
if (-not $CygwinRoot) {
  throw "AGENA_CYGWIN_ROOT is required for the official Cygwin toolchain"
}
$CygwinRuntime = Join-Path $CygwinRoot "bin/cygwin1.dll"
if (-not (Test-Path -LiteralPath $CygwinRuntime -PathType Leaf)) {
  throw "The official Cygwin runtime is missing: $CygwinRuntime"
}
$Machine = (& $Linker -dumpmachine).Trim()
if ($LASTEXITCODE -ne 0 -or $Machine -ne "x86_64-pc-cygwin") {
  throw "Cygwin linker reports unexpected target '$Machine'"
}

$env:RUSTUP_TOOLCHAIN = $StableToolchain
$StableRustc = (& rustup which --toolchain $StableToolchain rustc).Trim()
$StableRustcExit = $LASTEXITCODE
$StableRustdoc = (& rustup which --toolchain $StableToolchain rustdoc).Trim()
$StableRustdocExit = $LASTEXITCODE
$NightlyCargo = (& rustup which --toolchain $NightlyToolchain cargo).Trim()
$NightlyCargoExit = $LASTEXITCODE
if ($StableRustcExit -ne 0 -or -not $StableRustc) {
  throw "Failed to locate Rust $StableToolchain rustc"
}
if ($StableRustdocExit -ne 0 -or -not $StableRustdoc) {
  throw "Failed to locate Rust $StableToolchain rustdoc"
}
if ($NightlyCargoExit -ne 0 -or -not $NightlyCargo) {
  throw "Failed to locate nightly Cargo $NightlyToolchain"
}

$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "../..")
$RunnerWorkspace = if ($env:GITHUB_WORKSPACE) {
  $env:GITHUB_WORKSPACE
} else {
  Split-Path -Parent $env:RUNNER_TEMP
}
$BuildTargetDir = Join-Path $RunnerWorkspace ".agena-target\$TargetTriple"
$BuildStdProfile = Join-Path $BuildTargetDir "$TargetTriple\debug"
$RustcWrapperDir = Join-Path $env:RUNNER_TEMP "agena-rustc-build-std\$TargetTriple"
$RustcWrapperSource = Join-Path $RepoRoot "scripts\ci\rustc-build-std-wrapper.rs"
$RustcWrapper = Join-Path $RustcWrapperDir "rustc-build-std-wrapper.exe"
if (-not (Test-Path -LiteralPath $RustcWrapperSource -PathType Leaf)) {
  throw "shared build-std rustc wrapper missing at $RustcWrapperSource"
}
New-Item -ItemType Directory -Force -Path $RustcWrapperDir | Out-Null
& $StableRustc $RustcWrapperSource -o $RustcWrapper
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $RustcWrapper -PathType Leaf)) {
  throw "failed to compile the build-std rustc wrapper"
}

# zstd-sys builds its C sources with paths relative to the package directory.
# That is reliable with a native Windows compiler, but the official Cygwin GCC
# receives those paths through a Windows process boundary and does not resolve
# them relative to zstd-sys. Supply the real package include directories in
# Cygwin form; this keeps the complete zstd implementation enabled and makes
# the compiler consume the headers belonging to the locked zstd-sys release.
$CargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE ".cargo" }
& $NightlyCargo fetch --manifest-path (Join-Path $RepoRoot "Cargo.toml") --locked
if ($LASTEXITCODE -ne 0) {
  throw "failed to fetch the locked dependency sources before locating zstd-sys"
}
$ZstdRoot = Get-ChildItem -LiteralPath (Join-Path $CargoHome "registry\src") -Directory -Recurse -ErrorAction SilentlyContinue |
  Where-Object { $_.Name -eq "zstd-sys-2.0.16+zstd.1.5.7" } |
  Select-Object -First 1
if ($null -eq $ZstdRoot) {
  throw "locked zstd-sys source tree was not found below $(Join-Path $CargoHome 'registry\src')"
}
$Cygpath = Join-Path $CygwinRoot "bin\cygpath.exe"
if (-not (Test-Path -LiteralPath $Cygpath -PathType Leaf)) {
  throw "official Cygwin cygpath is missing: $Cygpath"
}
$ZstdRootUnix = (& $Cygpath -u $ZstdRoot.FullName).Trim()
if ($LASTEXITCODE -ne 0 -or -not $ZstdRootUnix) {
  throw "failed to convert the zstd-sys source path to Cygwin form"
}
$OldCflags = $env:CFLAGS_X86_64_PC_CYGWIN
$env:CFLAGS_X86_64_PC_CYGWIN = @(
  "-I$ZstdRootUnix/zstd/lib"
  "-I$ZstdRootUnix/zstd/lib/common"
  "-I$ZstdRootUnix/zstd/lib/compress"
  "-I$ZstdRootUnix/zstd/lib/decompress"
  "-I$ZstdRootUnix/zstd/lib/dictBuilder"
  "-I$ZstdRootUnix/zstd/lib/legacy"
) -join " "

$Args = @(
  "check",
  "--manifest-path", (Join-Path $RepoRoot "Cargo.toml"),
  "-p", "agena",
  "--target", $TargetTriple,
  "--target-dir", $BuildTargetDir,
  "--locked",
  "-Z", "build-std=std,panic_abort,proc_macro",
  "-vv"
)
$OldRustc = $env:RUSTC
$OldRustdoc = $env:RUSTDOC
$OldBootstrap = $env:RUSTC_BOOTSTRAP
$OldRustFlags = $env:RUSTFLAGS
$OldTargetDir = $env:CARGO_TARGET_DIR
$OldAgenaRealRustc = $env:AGENA_REAL_RUSTC
$OldAgenaBuildStdRoot = $env:AGENA_BUILD_STD_ROOT
$CargoExitCode = 1
try {
  $env:RUSTC = $StableRustc
  $env:RUSTDOC = $StableRustdoc
  $env:RUSTC_BOOTSTRAP = "1"
  $env:CARGO_TARGET_DIR = $BuildTargetDir
  # The shared wrapper supplies the real target standard-library artifacts to
  # direct target probes. Keep target-only search paths out of global RUSTFLAGS
  # so host build scripts use the host sysroot.
  $RustFlags = @($OldRustFlags) | Where-Object { $_ }
  $env:RUSTFLAGS = ($RustFlags | Join-String -Separator " ")
  $PrebuildScript = Join-Path $RepoRoot "scripts\ci\prebuild-build-std.ps1"
  & $PrebuildScript -TargetTriple $TargetTriple -NightlyCargo $NightlyCargo -TargetDir $BuildTargetDir
  if ($LASTEXITCODE -ne 0) {
    throw "failed to prebuild the real build-std sysroot for $TargetTriple"
  }
  $env:AGENA_REAL_RUSTC = $StableRustc
  $env:AGENA_BUILD_STD_ROOT = $BuildStdProfile
  $env:RUSTC = $RustcWrapper
  Write-Host "Using official Cygwin compiler: $Linker"
  Write-Host "Using build-std driver: cargo=$NightlyCargo rustc=$StableRustc target-dir=$BuildTargetDir"
  & $NightlyCargo @Args
  $CargoExitCode = $LASTEXITCODE
}
finally {
  $env:RUSTC = $OldRustc
  $env:RUSTDOC = $OldRustdoc
  $env:RUSTC_BOOTSTRAP = $OldBootstrap
  $env:RUSTFLAGS = $OldRustFlags
  if ($null -eq $OldCflags) {
    Remove-Item Env:CFLAGS_X86_64_PC_CYGWIN -ErrorAction SilentlyContinue
  } else {
    $env:CFLAGS_X86_64_PC_CYGWIN = $OldCflags
  }
  $env:CARGO_TARGET_DIR = $OldTargetDir
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

if ($CargoExitCode -ne 0) {
  throw "cargo check failed for $TargetTriple"
}
Write-Host "Cygwin full backend check passed for $TargetTriple"
