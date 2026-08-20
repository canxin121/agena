Param(
  [Parameter(Mandatory = $true)]
  [string]$TargetTriple
)

$ErrorActionPreference = "Stop"

function Set-EnvFromVsDevCmd {
  param(
    [Parameter(Mandatory = $true)][string]$Arch,
    [Parameter(Mandatory = $true)][string]$HostArch,
    [switch]$Uwp
  )

  $VsWhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
  if (-not (Test-Path $VsWhere)) {
    throw "vswhere.exe not found at $VsWhere"
  }
  $Install = (& $VsWhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath).Trim()
  if (-not $Install) {
    throw "Visual Studio installation with VC tools not found"
  }
  $VsDevCmd = Join-Path $Install "Common7\Tools\VsDevCmd.bat"
  if (-not (Test-Path $VsDevCmd)) {
    throw "VsDevCmd.bat not found at $VsDevCmd"
  }

  $Args = @("-no_logo", "-arch=$Arch", "-host_arch=$HostArch")
  if ($Uwp) {
    $Args += "-app_platform=UWP"
  }
  $ArgLine = ($Args -join " ")
  $Output = & cmd.exe /s /c "`"$VsDevCmd`" $ArgLine >nul && set"
  if ($LASTEXITCODE -ne 0) {
    throw "VsDevCmd failed for target arch=$Arch host=$HostArch UWP=$Uwp"
  }
  foreach ($Line in $Output) {
    $Index = $Line.IndexOf("=")
    if ($Index -le 0) { continue }
    $Name = $Line.Substring(0, $Index)
    $Value = $Line.Substring($Index + 1)
    [Environment]::SetEnvironmentVariable($Name, $Value, "Process")
  }
}

function Install-LlvmMingw {
  param([Parameter(Mandatory = $true)][string]$Target)

  $Version = "20260616"
  $ArchiveName = "llvm-mingw-$Version-ucrt-x86_64.zip"
  $ExpectedSha256 = "b9b68a4d276e16fa25802aaba458e4638f64b3884c290aaccdc2d87083b6ca35"
  $Root = Join-Path $env:RUNNER_TEMP "agena-llvm-mingw-$Version"
  $Archive = Join-Path $Root $ArchiveName
  $Extracted = Join-Path $Root "toolchain"
  New-Item -ItemType Directory -Force -Path $Root | Out-Null

  $NeedsDownload = $true
  if (Test-Path $Archive) {
    $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    $NeedsDownload = $Actual -ne $ExpectedSha256
  }
  if ($NeedsDownload) {
    Remove-Item -Force -ErrorAction SilentlyContinue $Archive
    $Url = "https://github.com/mstorsjo/llvm-mingw/releases/download/$Version/$ArchiveName"
    Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $Archive
    $Actual = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
    if ($Actual -ne $ExpectedSha256) {
      throw "llvm-mingw SHA256 mismatch: expected $ExpectedSha256 got $Actual"
    }
  }

  if (-not (Test-Path (Join-Path $Extracted "bin\clang.exe"))) {
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $Extracted
    New-Item -ItemType Directory -Force -Path $Extracted | Out-Null
    Expand-Archive -Path $Archive -DestinationPath $Extracted
  }

  $Top = Get-ChildItem -Path $Extracted -Directory | Select-Object -First 1
  if ($Top -and (Test-Path (Join-Path $Top.FullName "bin\clang.exe"))) {
    $Bin = Join-Path $Top.FullName "bin"
  } elseif (Test-Path (Join-Path $Extracted "bin\clang.exe")) {
    $Bin = Join-Path $Extracted "bin"
  } else {
    throw "llvm-mingw clang.exe not found after extraction"
  }
  $env:PATH = "$Bin;$env:PATH"

  switch -Wildcard ($Target) {
    "aarch64-*" { $Prefix = "aarch64-w64-mingw32" }
    "i686-*" { $Prefix = "i686-w64-mingw32" }
    "x86_64-*" { $Prefix = "x86_64-w64-mingw32" }
    default { throw "No llvm-mingw target prefix mapping for $Target" }
  }

  $CC = Join-Path $Bin "$Prefix-clang.exe"
  $CXX = Join-Path $Bin "$Prefix-clang++.exe"
  $AR = Join-Path $Bin "llvm-ar.exe"
  if (-not (Test-Path $CC)) { throw "llvm-mingw compiler missing: $CC" }
  if (-not (Test-Path $CXX)) { throw "llvm-mingw compiler missing: $CXX" }
  if (-not (Test-Path $AR)) { throw "llvm-mingw archiver missing: $AR" }

  $Key = $Target.Replace("-", "_")
  $Upper = $Key.ToUpperInvariant()
  [Environment]::SetEnvironmentVariable("CC_$Key", $CC, "Process")
  [Environment]::SetEnvironmentVariable("CXX_$Key", $CXX, "Process")
  [Environment]::SetEnvironmentVariable("AR_$Key", $AR, "Process")
  [Environment]::SetEnvironmentVariable("CARGO_TARGET_${Upper}_LINKER", $CC, "Process")
}

if ($TargetTriple -match "-windows-(gnu|gnullvm)$" -or $TargetTriple -match "-(uwp|win7)-windows-gnu$") {
  Install-LlvmMingw -Target $TargetTriple
  return
}

if ($TargetTriple -notmatch "windows-msvc$") {
  return
}

$TargetArch = if ($TargetTriple.StartsWith("thumbv7a-")) {
  "arm"
} elseif ($TargetTriple.StartsWith("aarch64-") -or $TargetTriple.StartsWith("arm64ec-")) {
  "arm64"
} elseif ($TargetTriple.StartsWith("i686-")) {
  "x86"
} else {
  "x64"
}

$HostArch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
$IsUwp = $TargetTriple -match "-uwp-windows-msvc$"
Set-EnvFromVsDevCmd -Arch $TargetArch -HostArch $HostArch -Uwp:$IsUwp

