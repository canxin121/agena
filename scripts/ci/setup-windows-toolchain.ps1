Param(
  [Parameter(Mandatory = $true)]
  [string]$TargetTriple
)

$ErrorActionPreference = "Stop"

function Set-EnvFromVsDevCmd {
  param(
    [Parameter(Mandatory = $true)][string]$Arch,
    [Parameter(Mandatory = $true)][string]$HostArch,
    [Parameter(Mandatory = $true)][string]$TargetTriple,
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

  # VsDevCmd normally puts the target-specific MSVC compiler first on PATH.
  # Keep that selection explicit: on hosted images the command can leave the
  # host compiler (for example HostX64\x64) first even though
  # VSCMD_ARG_TGT_ARCH reports the requested ARM target.  Using that compiler
  # would silently compile with the wrong ABI and, in the ARM case, commonly
  # leaves the target C headers unavailable to cc-rs.
  $ToolsRoot = Join-Path $Install "VC\Tools\MSVC"
  $ToolsVersion = $env:VCToolsVersion
  if (-not $ToolsVersion) {
    $ToolsVersion = (Get-ChildItem -Path $ToolsRoot -Directory |
      Sort-Object Name -Descending | Select-Object -First 1).Name
  }
  if (-not $ToolsVersion) {
    throw "MSVC tools version not found under $ToolsRoot"
  }
  $VCToolsRoot = Join-Path $ToolsRoot $ToolsVersion
  $HostToolArch = if ($HostArch -eq "arm64") { "HostARM64" } else { "HostX64" }
  $TargetBin = Join-Path $VCToolsRoot "bin\$HostToolArch\$Arch"
  $Compiler = Join-Path $TargetBin "cl.exe"
  $UsingClangCl = $false
  if (-not (Test-Path $Compiler)) {
    if ($Arch -ne "arm") {
      throw "MSVC target compiler missing for host=$HostArch target=${Arch}: $Compiler"
    }

    # Recent hosted VS images can ship the ARM MSVC libraries and Windows SDK
    # without the legacy ARM cl.exe.  Clang-cl is the supported MSVC-compatible
    # compiler for this case: force the ARMv7 Windows MSVC target so an x64
    # host compiler can never silently produce x64 objects for this target.
    $ClangCandidates = @(
      (Join-Path $Install "VC\Tools\Llvm\x64\bin\clang-cl.exe"),
      (Join-Path $Install "VC\Tools\Llvm\bin\clang-cl.exe"),
      (Join-Path ${env:ProgramFiles} "LLVM\bin\clang-cl.exe")
    )
    $ClangCl = $null
    foreach ($Candidate in $ClangCandidates) {
      if (Test-Path $Candidate) {
        $ClangCl = $Candidate
        break
      }
    }
    if (-not $ClangCl) {
      $PathClang = Get-Command clang-cl.exe -ErrorAction SilentlyContinue
      if ($PathClang) {
        $ClangCl = $PathClang.Source
      }
    }
    if (-not $ClangCl) {
      throw "MSVC ARM target compiler missing ($Compiler) and no clang-cl.exe was found in VS/LLVM paths"
    }

    $WrapperRoot = Join-Path $env:RUNNER_TEMP "agena-msvc-clang\$TargetTriple"
    New-Item -ItemType Directory -Force -Path $WrapperRoot | Out-Null
    $Compiler = Join-Path $WrapperRoot "clang-cl-arm.cmd"
    $WrapperText = "@echo off`r`n`"$ClangCl`" --target=thumbv7a-pc-windows-msvc %*`r`nexit /b %ERRORLEVEL%`r`n"
    [IO.File]::WriteAllText($Compiler, $WrapperText, [Text.Encoding]::ASCII)
    $UsingClangCl = $true
    Write-Host "Using clang-cl ARMv7 MSVC fallback: $ClangCl --target=thumbv7a-pc-windows-msvc"
  }
  if (Test-Path $TargetBin) {
    $env:PATH = "$TargetBin;$env:PATH"
  }
  # link.exe is a host tool even when it links ARM COFF.  Keep it available
  # explicitly when the ARM compiler directory is absent from the VS image.
  $HostLinkBin = Join-Path $VCToolsRoot "bin\$HostToolArch\x64"
  if (Test-Path $HostLinkBin) {
    $env:PATH = "$HostLinkBin;$env:PATH"
  }
  $env:VCToolsInstallDir = "$VCToolsRoot\"

  # Preserve the SDK choices made by VsDevCmd while making the target MSVC
  # headers and libraries unambiguous for build scripts that invoke cl.exe
  # through cc-rs rather than through devenv/msbuild.
  $VCToolsInclude = Join-Path $VCToolsRoot "include"
  $VCToolsLib = Join-Path $VCToolsRoot "lib\$Arch"
  if ($env:INCLUDE) {
    $env:INCLUDE = "$VCToolsInclude;$env:INCLUDE"
  } else {
    $env:INCLUDE = $VCToolsInclude
  }

  $TargetLibDirs = @()
  $PreserveExistingLib = $true
  if (Test-Path $VCToolsLib) {
    $TargetLibDirs += $VCToolsLib
  } elseif (-not ($UsingClangCl -and $Arch -eq "arm")) {
    throw "MSVC target library directory missing for host=$HostArch target=${Arch}: $VCToolsLib"
  } else {
    # VS 2026 hosted images may provide clang-cl for ARMv7 while omitting the
    # legacy MSVC ARM compiler and VC\Tools\MSVC\...\lib\arm directory.  In
    # that configuration use the actual ARM Windows SDK import libraries.  Do
    # not substitute x64 or ARM64 libraries: clang-cl still has to link a
    # genuine ARM32 MSVC target.
    $PreserveExistingLib = $false
    $WindowsSdkDir = $env:WindowsSdkDir
    if (-not $WindowsSdkDir) {
      throw "WindowsSdkDir is not set; cannot locate ARM Windows SDK libraries for $TargetTriple"
    }
    $WindowsSdkVersion = if ($env:WindowsSDKVersion) {
      $env:WindowsSDKVersion.TrimEnd('\')
    } else {
      $null
    }
    $WindowsSdkLibRoot = Join-Path $WindowsSdkDir "Lib"
    if (-not $WindowsSdkVersion -or -not (Test-Path (Join-Path $WindowsSdkLibRoot $WindowsSdkVersion))) {
      $WindowsSdkVersion = (Get-ChildItem -Path $WindowsSdkLibRoot -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending | Select-Object -First 1).Name
    }
    if (-not $WindowsSdkVersion) {
      throw "Windows SDK library version not found under $WindowsSdkLibRoot"
    }

    $UniversalCrtDir = if ($env:UniversalCRTSdkDir) {
      $env:UniversalCRTSdkDir
    } else {
      $WindowsSdkDir
    }
    $UniversalCrtVersion = if ($env:UCRTVersion) {
      $env:UCRTVersion.TrimEnd('\')
    } else {
      $WindowsSdkVersion
    }
    $SdkUmArm = Join-Path (Join-Path $WindowsSdkLibRoot $WindowsSdkVersion) "um\arm"
    $SdkUcrtArm = Join-Path (Join-Path (Join-Path $UniversalCrtDir "Lib") $UniversalCrtVersion) "ucrt\arm"
    $MissingSdkArmDirs = @($SdkUmArm, $SdkUcrtArm) | Where-Object { -not (Test-Path $_) }
    if ($MissingSdkArmDirs.Count -gt 0) {
      throw "ARM Windows SDK library directories missing for $TargetTriple: $($MissingSdkArmDirs -join ', ') (WindowsSdkDir=$WindowsSdkDir, WindowsSDKVersion=$WindowsSdkVersion, UniversalCRTSdkDir=$UniversalCrtDir, UCRTVersion=$UniversalCrtVersion)"
    }
    $TargetLibDirs += $SdkUmArm
    $TargetLibDirs += $SdkUcrtArm
    Write-Host "Using ARM Windows SDK libraries: $SdkUmArm; $SdkUcrtArm"
  }
  if ($PreserveExistingLib -and $env:LIB) {
    $env:LIB = (($TargetLibDirs + @($env:LIB)) -join ";")
  } else {
    $env:LIB = ($TargetLibDirs -join ";")
  }

  $Key = $TargetTriple.Replace("-", "_")
  [Environment]::SetEnvironmentVariable("CC_$Key", $Compiler, "Process")
  [Environment]::SetEnvironmentVariable("CXX_$Key", $Compiler, "Process")
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
Set-EnvFromVsDevCmd -Arch $TargetArch -HostArch $HostArch -TargetTriple $TargetTriple -Uwp:$IsUwp
