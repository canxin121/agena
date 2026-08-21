Param(
  [Parameter(Mandatory = $true)]
  [string]$TargetTriple
)

$ErrorActionPreference = "Stop"

function Install-VsComponents {
  param(
    [Parameter(Mandatory = $true)][string]$Installer,
    [Parameter(Mandatory = $true)][string]$InstallPath,
    [Parameter(Mandatory = $true)][string[]]$Components
  )

  # Invoke the native installer directly so PowerShell passes the installation
  # path as one argv element.  Start-Process flattens an ArgumentList array
  # into a command line before launching setup.exe; on hosted runners that
  # turns `C:\Program Files\...` into the truncated `C:\Program` path.
  $InstallerArgs = @("modify", "--installPath", $InstallPath)
  foreach ($Component in $Components) {
    $InstallerArgs += @("--add", $Component)
  }
  # A direct native invocation is synchronous, so no unsupported --wait flag
  # or lossy Start-Process quoting is involved.
  # Recommended dependencies include the official C/C++ headers and runtime
  # support used by the selected MSVC ABI.  A component can be present in the
  # image manifest while its payload is absent on a fresh hosted image, so
  # request the recommended payload explicitly during the real installer
  # modify operation.
  $InstallerArgs += @("--includeRecommended", "--quiet", "--norestart", "--noUpdateInstaller")
  & $Installer @InstallerArgs
  $ExitCode = $LASTEXITCODE
  if ($ExitCode -notin @(0, 3010)) {
    throw "Visual Studio component installation failed with exit code $ExitCode`: $($Components -join ', ')"
  }
}

function Set-EnvFromVsDevCmd {
  param(
    [Parameter(Mandatory = $true)][string]$Arch,
    [Parameter(Mandatory = $true)][string]$HostArch,
    [Parameter(Mandatory = $true)][string]$TargetTriple
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

  # VsDevCmd selects the ARM64 environment for ARM64EC; the actual MSVC
  # compiler/library directories remain the distinct arm64ec ABI below.
  $VsDevArch = if ($Arch -eq "arm64ec") { "arm64" } else { $Arch }
  $Args = @("-no_logo", "-arch=$VsDevArch", "-host_arch=$HostArch")
  $ArgLine = ($Args -join " ")
  $Output = & cmd.exe /s /c "`"$VsDevCmd`" $ArgLine >nul && set"
  if ($LASTEXITCODE -ne 0) {
    throw "VsDevCmd failed for target arch=$Arch host=$HostArch"
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
  $ToolsVersion = if ($env:VCToolsVersion) { $env:VCToolsVersion.TrimEnd('\') } else { $null }
  if (-not $ToolsVersion) {
    $ToolsVersion = (Get-ChildItem -Path $ToolsRoot -Directory |
      Sort-Object Name -Descending | Select-Object -First 1).Name
  }
  if (-not $ToolsVersion) {
    throw "MSVC tools version not found under $ToolsRoot"
  }
  $VCToolsRoot = Join-Path $ToolsRoot $ToolsVersion
  if (-not (Test-Path $VCToolsRoot)) {
    $ToolsVersion = (Get-ChildItem -Path $ToolsRoot -Directory |
      Sort-Object Name -Descending | Select-Object -First 1).Name
    $VCToolsRoot = Join-Path $ToolsRoot $ToolsVersion
  }
  $HostToolArch = if ($HostArch -eq "arm64") { "HostARM64" } else { "HostX64" }

  $VsInstaller = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\setup.exe"
  if (-not (Test-Path $VsInstaller)) {
    throw "Visual Studio installer not found at $VsInstaller"
  }

  if ($Arch -eq "arm64ec") {
    $Arm64EcCompilerPath = Join-Path $VCToolsRoot "bin\$HostToolArch\arm64ec\cl.exe"
    $Arm64EcLibraryPath = Join-Path $VCToolsRoot "lib\arm64ec"
    if (-not (Test-Path $Arm64EcCompilerPath) -or -not (Test-Path $Arm64EcLibraryPath)) {
      # ARM64EC is an optional, distinct MSVC ABI component.  Do not point an
      # ARM64EC build at the ARM64 compiler or libraries: install Microsoft's
      # official component when the hosted image omitted it.
      Write-Host "Installing official Visual Studio ARM64EC MSVC component: Microsoft.VisualStudio.Component.VC.Tools.ARM64EC"
      Install-VsComponents -Installer $VsInstaller -InstallPath $Install -Components @(
        "Microsoft.VisualStudio.Workload.NativeDesktop",
        "Microsoft.VisualStudio.Component.VC.Tools.ARM64EC",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "Microsoft.VisualStudio.Component.VC.CoreBuildTools"
      )

      # The installer may add a side-by-side toolset and returns before the
      # files are visible. Wait for a complete compiler/library pair instead
      # of falling through to another architecture.
      $Arm64EcToolsRoot = $null
      $Deadline = (Get-Date).AddMinutes(5)
      do {
        $Arm64EcToolsRoot = Get-ChildItem -Path $ToolsRoot -Directory -ErrorAction SilentlyContinue |
          Sort-Object Name -Descending |
          Where-Object {
            (Test-Path (Join-Path $_.FullName "bin\$HostToolArch\arm64ec\cl.exe")) -and
            (Test-Path (Join-Path $_.FullName "lib\arm64ec"))
          } |
          Select-Object -First 1
        if ($Arm64EcToolsRoot) {
          $ToolsVersion = $Arm64EcToolsRoot.Name
          $VCToolsRoot = $Arm64EcToolsRoot.FullName
          $env:VCToolsVersion = $ToolsVersion
          break
        }
        Start-Sleep -Seconds 2
      } while ((Get-Date) -lt $Deadline)
    }
    if (-not (Test-Path (Join-Path $VCToolsRoot "bin\$HostToolArch\arm64ec\cl.exe")) -or
        -not (Test-Path (Join-Path $VCToolsRoot "lib\arm64ec"))) {
      throw "MSVC ARM64EC component missing after official installation: expected bin\$HostToolArch\arm64ec\cl.exe and lib\arm64ec under $VCToolsRoot"
    }
  }

  if ($Arch -eq "arm") {
    $ArmCompilerPath = Join-Path $VCToolsRoot "bin\$HostToolArch\arm\cl.exe"
    $ArmLibraryPath = Join-Path $VCToolsRoot "lib\arm"
    if (-not (Test-Path $ArmCompilerPath) -or -not (Test-Path $ArmLibraryPath)) {
      # Windows hosted images do not always carry the ARM32 MSVC component in
      # the preinstalled VS instance. Install the official component through
      # the VS installer instead of substituting host or ARM64 libraries.
      Write-Host "Installing official Visual Studio ARM32 MSVC component: Microsoft.VisualStudio.Component.VC.Tools.ARM"
      Install-VsComponents -Installer $VsInstaller -InstallPath $Install -Components @(
        "Microsoft.VisualStudio.Workload.NativeDesktop",
        "Microsoft.VisualStudio.Component.VC.Tools.ARM",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "Microsoft.VisualStudio.Component.VC.CoreBuildTools"
      )

      # The installer can add a side-by-side MSVC tools version. Select the
      # newest installed version that actually contains the ARM32 ABI rather
      # than retaining a stale VCToolsVersion from VsDevCmd.
      $ArmToolsVersion = Get-ChildItem -Path $ToolsRoot -Directory |
        Sort-Object Name -Descending |
        Where-Object {
          (Test-Path (Join-Path $_.FullName "lib\arm")) -and
          (Test-Path (Join-Path $_.FullName "bin\$HostToolArch\arm\cl.exe"))
        } |
        Select-Object -First 1 -ExpandProperty Name
      if ($ArmToolsVersion) {
        $ToolsVersion = $ArmToolsVersion
        $VCToolsRoot = Join-Path $ToolsRoot $ToolsVersion
        $env:VCToolsVersion = $ToolsVersion
      }
    }
  }

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
  if (-not (Test-Path (Join-Path $VCToolsInclude "stddef.h"))) {
    # The legacy ARM compiler can be installed beside a newer MSVC host toolset
    # without carrying its C headers.  Headers are architecture-independent;
    # select the newest installed official MSVC include tree that contains the
    # required standard header while retaining the target-specific compiler
    # and libraries selected above.
    function Find-MsvcHeaderToolsRoot {
      # The compiler and headers can be split across official side-by-side VS
      # installations on hosted images.  Search every installed MSVC toolset,
      # not only the installation selected by `vswhere -latest`; never create
      # or copy a synthetic header tree.
      $ToolRoots = @()
      if (Test-Path $ToolsRoot) {
        $ToolRoots += Get-ChildItem -Path $ToolsRoot -Directory -ErrorAction SilentlyContinue
      }
      $VisualStudioRoots = @(
        (Join-Path ${env:ProgramFiles} "Microsoft Visual Studio"),
        (Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio")
      )
      foreach ($VisualStudioRoot in $VisualStudioRoots) {
        if (-not (Test-Path $VisualStudioRoot)) { continue }
        $ToolRoots += Get-ChildItem -Path (Join-Path $VisualStudioRoot "*\*\VC\Tools\MSVC") -Directory -ErrorAction SilentlyContinue |
          ForEach-Object {
            Get-ChildItem -Path $_.FullName -Directory -ErrorAction SilentlyContinue
          }
      }
      $ToolRoots |
        Where-Object { Test-Path (Join-Path $_.FullName "include\stddef.h") } |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    }

    $HeaderToolsRoot = Find-MsvcHeaderToolsRoot
    if (-not $HeaderToolsRoot) {
      Write-Host "Installing official Visual Studio C++ build tools to obtain MSVC headers"
      Install-VsComponents -Installer $VsInstaller -InstallPath $Install -Components @(
        "Microsoft.VisualStudio.Workload.NativeDesktop",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "Microsoft.VisualStudio.Component.VC.CoreBuildTools"
      )

      # setup.exe delegates modify operations to the Visual Studio installer
      # service and can return before the selected toolset has been unpacked.
      # Do not race that service: wait for a real official MSVC header tree to
      # appear, rather than falling through to a host compiler or synthetic
      # header substitute.
      $Deadline = (Get-Date).AddMinutes(5)
      do {
        $HeaderToolsRoot = Find-MsvcHeaderToolsRoot
        if ($HeaderToolsRoot) { break }
        Start-Sleep -Seconds 2
      } while ((Get-Date) -lt $Deadline)
    }
    if (-not $HeaderToolsRoot) {
      throw "MSVC C headers missing: no installed toolset contains include\stddef.h after installing official C++ build tools"
    }
    $VCToolsInclude = Join-Path $HeaderToolsRoot.FullName "include"
  }
  # cc-rs invokes cl.exe directly and does not print or normalize inherited
  # INCLUDE values in its command diagnostics. Add the verified official
  # header tree as a target-scoped /I flag as well, so C build scripts cannot
  # accidentally lose stddef.h when they replace the environment.
  $TargetEnvKey = $TargetTriple.Replace("-", "_")
  $HeaderFlag = "/I`"$VCToolsInclude`""
  foreach ($FlagName in @("CFLAGS_$TargetEnvKey", "CXXFLAGS_$TargetEnvKey")) {
    $ExistingFlags = [Environment]::GetEnvironmentVariable($FlagName, "Process")
    $CombinedFlags = if ($ExistingFlags) { "$HeaderFlag $ExistingFlags" } else { $HeaderFlag }
    [Environment]::SetEnvironmentVariable($FlagName, $CombinedFlags, "Process")
  }
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
    $RequestedWindowsSdkVersion = if ($env:WindowsSDKVersion) {
      $env:WindowsSDKVersion.TrimEnd('\')
    } else {
      $null
    }
    $WindowsSdkLibRoot = Join-Path $WindowsSdkDir "Lib"
    $InstalledWindowsSdkVersions = @(Get-ChildItem -Path $WindowsSdkLibRoot -Directory -ErrorAction SilentlyContinue |
      Sort-Object Name -Descending | Select-Object -ExpandProperty Name)
    if ($RequestedWindowsSdkVersion -and ($InstalledWindowsSdkVersions -notcontains $RequestedWindowsSdkVersion)) {
      $InstalledWindowsSdkVersions += $RequestedWindowsSdkVersion
    }
    if ($InstalledWindowsSdkVersions.Count -eq 0) {
      throw "Windows SDK library version not found under $WindowsSdkLibRoot"
    }

    $UniversalCrtDir = if ($env:UniversalCRTSdkDir) {
      $env:UniversalCRTSdkDir.TrimEnd('\')
    } else {
      $WindowsSdkDir.TrimEnd('\')
    }
    $RequestedUniversalCrtVersion = if ($env:UCRTVersion) {
      $env:UCRTVersion.TrimEnd('\')
    } else {
      $null
    }
    $InstalledUniversalCrtVersions = @(Get-ChildItem -Path (Join-Path $UniversalCrtDir "Lib") -Directory -ErrorAction SilentlyContinue |
      Sort-Object Name -Descending | Select-Object -ExpandProperty Name)
    if ($RequestedUniversalCrtVersion -and ($InstalledUniversalCrtVersions -notcontains $RequestedUniversalCrtVersion)) {
      $InstalledUniversalCrtVersions += $RequestedUniversalCrtVersion
    }

    # The newest Windows SDK on a hosted image is not guaranteed to retain
    # ARM32 import libraries.  Search the installed SDKs for a matching pair
    # of real ARM32 libraries instead of failing on the first (or newest)
    # version.  The UCRT version is searched independently because VS images
    # can expose it through a different SDK root/version than um\arm.
    $SelectedWindowsSdkVersion = $null
    $SelectedUniversalCrtVersion = $null
    $SdkUmArm = $null
    $SdkUcrtArm = $null
    foreach ($CandidateWindowsSdkVersion in $InstalledWindowsSdkVersions) {
      $CandidateSdkUmArm = Join-Path (Join-Path $WindowsSdkLibRoot $CandidateWindowsSdkVersion) "um\arm"
      if (-not (Test-Path $CandidateSdkUmArm)) {
        continue
      }
      foreach ($CandidateUniversalCrtVersion in $InstalledUniversalCrtVersions) {
        $CandidateSdkUcrtArm = Join-Path (Join-Path (Join-Path $UniversalCrtDir "Lib") $CandidateUniversalCrtVersion) "ucrt\arm"
        if (Test-Path $CandidateSdkUcrtArm) {
          $SelectedWindowsSdkVersion = $CandidateWindowsSdkVersion
          $SelectedUniversalCrtVersion = $CandidateUniversalCrtVersion
          $SdkUmArm = $CandidateSdkUmArm
          $SdkUcrtArm = $CandidateSdkUcrtArm
          break
        }
      }
      if ($SelectedWindowsSdkVersion) {
        break
      }
    }
    if (-not $SelectedWindowsSdkVersion) {
      $KnownUmArm = $InstalledWindowsSdkVersions | ForEach-Object {
        Join-Path (Join-Path $WindowsSdkLibRoot $_) "um\arm"
      }
      $KnownUcrtArm = $InstalledUniversalCrtVersions | ForEach-Object {
        Join-Path (Join-Path (Join-Path $UniversalCrtDir "Lib") $_) "ucrt\arm"
      }
      throw "ARM Windows SDK library directories missing for ${TargetTriple}: no matching um\arm and ucrt\arm pair (um candidates: $($KnownUmArm -join ', '); ucrt candidates: $($KnownUcrtArm -join ', '); WindowsSdkDir=$WindowsSdkDir, UniversalCRTSdkDir=$UniversalCrtDir)"
    }
    $env:WindowsSDKVersion = "$SelectedWindowsSdkVersion\"
    $env:UCRTVersion = "$SelectedUniversalCrtVersion\"
    $env:UniversalCRTSdkDir = "$UniversalCrtDir\"
    $TargetLibDirs += $SdkUmArm
    $TargetLibDirs += $SdkUcrtArm
    Write-Host "Using ARM Windows SDK libraries: $SdkUmArm; $SdkUcrtArm (WindowsSDKVersion=$SelectedWindowsSdkVersion, UCRTVersion=$SelectedUniversalCrtVersion)"
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

if ($TargetTriple -match "-windows-(gnu|gnullvm)$" -or $TargetTriple -match "-win7-windows-gnu$") {
  Install-LlvmMingw -Target $TargetTriple
  return
}

if ($TargetTriple -notmatch "windows-msvc$") {
  return
}

$TargetArch = if ($TargetTriple.StartsWith("thumbv7a-")) {
  "arm"
} elseif ($TargetTriple.StartsWith("arm64ec-")) {
  "arm64ec"
} elseif ($TargetTriple.StartsWith("aarch64-")) {
  "arm64"
} elseif ($TargetTriple.StartsWith("i686-")) {
  "x86"
} else {
  "x64"
}

$HostArch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "arm64" } else { "x64" }
Set-EnvFromVsDevCmd -Arch $TargetArch -HostArch $HostArch -TargetTriple $TargetTriple
