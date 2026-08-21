Param()

$ErrorActionPreference = "Stop"

# Cygwin is the target operating system, not a MinGW ABI.  Install the
# official Cygwin compiler and target libraries on a Windows runner so Rust
# links against cygwin1.dll/libcygwin rather than silently using a Windows or
# host compiler.
$RunnerTemp = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [IO.Path]::GetTempPath() }
$Root = if ($env:AGENA_CYGWIN_ROOT) {
  $env:AGENA_CYGWIN_ROOT
} else {
  Join-Path $RunnerTemp "agena-cygwin"
}
$Setup = Join-Path $RunnerTemp "setup-x86_64.exe"
$SetupUrl = "https://cygwin.com/setup-x86_64.exe"
$Mirror = if ($env:AGENA_CYGWIN_MIRROR) {
  $env:AGENA_CYGWIN_MIRROR
} else {
  "https://mirrors.kernel.org/sourceware/cygwin/"
}
$Packages = @(
  "binutils",
  "cmake",
  "cygwin-devel",
  "gcc-core",
  "gcc-g++",
  "make",
  "ninja",
  "patch",
  "perl",
  "pkg-config",
  "python3",
  "git",
  "unzip"
) -join ","

if (-not (Test-Path -LiteralPath $Setup -PathType Leaf)) {
  Invoke-WebRequest -Uri $SetupUrl -OutFile $Setup
}

$SetupArgs = @(
  "--quiet-mode",
  "--no-admin",
  "--no-desktop",
  "--no-shortcuts",
  "--no-startmenu",
  "--no-version-check",
  "--root", $Root,
  "--site", $Mirror,
  "--packages", $Packages
)
& $Setup @SetupArgs
if ($LASTEXITCODE -ne 0) {
  throw "Cygwin setup failed with exit code $LASTEXITCODE"
}

$Bin = Join-Path $Root "bin"
$Required = @(
  (Join-Path $Bin "x86_64-pc-cygwin-gcc.exe"),
  (Join-Path $Bin "x86_64-pc-cygwin-g++.exe"),
  (Join-Path $Bin "ar.exe"),
  (Join-Path $Bin "ranlib.exe"),
  (Join-Path $Bin "ld.exe"),
  (Join-Path $Bin "cygwin1.dll")
)
foreach ($Path in $Required) {
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "Official Cygwin toolchain file is missing: $Path"
  }
}

$env:AGENA_CYGWIN_ROOT = $Root
$env:CARGO_TARGET_X86_64_PC_CYGWIN_LINKER = Join-Path $Bin "x86_64-pc-cygwin-gcc.exe"
$env:CC_x86_64_pc_cygwin = Join-Path $Bin "x86_64-pc-cygwin-gcc.exe"
$env:CXX_x86_64_pc_cygwin = Join-Path $Bin "x86_64-pc-cygwin-g++.exe"
$env:AR_x86_64_pc_cygwin = Join-Path $Bin "ar.exe"
$env:RANLIB_x86_64_pc_cygwin = Join-Path $Bin "ranlib.exe"

if ($env:GITHUB_ENV) {
  @(
    "AGENA_CYGWIN_ROOT=$Root"
    "CARGO_TARGET_X86_64_PC_CYGWIN_LINKER=$env:CARGO_TARGET_X86_64_PC_CYGWIN_LINKER"
    "CC_x86_64_pc_cygwin=$env:CC_x86_64_pc_cygwin"
    "CXX_x86_64_pc_cygwin=$env:CXX_x86_64_pc_cygwin"
    "AR_x86_64_pc_cygwin=$env:AR_x86_64_pc_cygwin"
    "RANLIB_x86_64_pc_cygwin=$env:RANLIB_x86_64_pc_cygwin"
  ) | Add-Content -LiteralPath $env:GITHUB_ENV
  Add-Content -LiteralPath $env:GITHUB_PATH -Value $Bin
}

Write-Host "Cygwin root: $Root"
& (Join-Path $Bin "x86_64-pc-cygwin-gcc.exe") --version | Select-Object -First 1
$Machine = (& (Join-Path $Bin "x86_64-pc-cygwin-gcc.exe") -dumpmachine).Trim()
if ($Machine -ne "x86_64-pc-cygwin") {
  throw "Cygwin compiler reports unexpected target: $Machine"
}
