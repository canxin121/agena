[CmdletBinding()]
Param(
  [ValidateSet("Install", "Reinstall", "Upgrade", "Uninstall", "Start", "Stop", "Restart", "Status", "Permissions")]
  [string]$Action = "Install",
  [string]$Repo = "canxin121/agena",
  [string]$Version = "",
  [string]$Archive = "",
  [string]$Checksum = "",
  [switch]$Source,
  [string]$SourcePath = "",
  [string]$SourceRef = "",
  [string]$InstallDir = "",
  [string]$DataDir = "",
  [string]$ListenHost = "127.0.0.1",
  [int]$Port = 3210,
  [string]$Workspace = "",
  [string]$ServerPassword = $env:AGENA_SERVER_PASSWORD,
  [ValidateSet("auto", "native", "detached")]
  [string]$ServiceMode = "auto",
  [ValidateSet("auto", "check")]
  [string]$SetupMode = "auto",
  [switch]$NoPathUpdate,
  [switch]$PurgeData
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$RepoSpecified = $PSBoundParameters.ContainsKey("Repo")
$VersionSpecified = $PSBoundParameters.ContainsKey("Version")
$DataDirSpecified = $PSBoundParameters.ContainsKey("DataDir")
$ListenHostSpecified = $PSBoundParameters.ContainsKey("ListenHost")
$PortSpecified = $PSBoundParameters.ContainsKey("Port")
$WorkspaceSpecified = $PSBoundParameters.ContainsKey("Workspace")
$ServerPasswordSpecified = $PSBoundParameters.ContainsKey("ServerPassword")
$ServiceModeSpecified = $PSBoundParameters.ContainsKey("ServiceMode")

$ReleaseVersion = ""
$script:SandboxReport = $null

if (-not $InstallDir) {
  $InstallDir = Join-Path $env:LOCALAPPDATA "Agena"
}
if (-not $DataDir) {
  if ($env:AGENA_DATA_DIR) { $DataDir = $env:AGENA_DATA_DIR }
  else { $DataDir = Join-Path $HOME "agena" }
}
if (-not $Workspace) {
  $Workspace = $HOME
}

$replacementAction = $Action -in @("Install", "Reinstall", "Upgrade")
$candidateParameters = @(
  foreach ($name in @("Repo", "Version", "Archive", "Checksum", "Source", "SourcePath", "SourceRef")) {
    if ($PSBoundParameters.ContainsKey($name)) { $name }
  }
)
$runtimeParameters = @(
  foreach ($name in @("DataDir", "ListenHost", "Port", "Workspace", "ServerPassword", "ServiceMode")) {
    if ($PSBoundParameters.ContainsKey($name)) { $name }
  }
)
if (-not $replacementAction -and $candidateParameters) {
  throw "$(($candidateParameters | ForEach-Object { "-$_" }) -join ', ') only applies to Install, Reinstall, and Upgrade"
}
if (-not $replacementAction -and $runtimeParameters) {
  throw "$(($runtimeParameters | ForEach-Object { "-$_" }) -join ', ') only applies to Install, Reinstall, and Upgrade"
}
if ($PSBoundParameters.ContainsKey("SetupMode") -and -not $replacementAction -and $Action -ne "Permissions") {
  throw "-SetupMode only applies to Install, Reinstall, Upgrade, and Permissions"
}
if ($PurgeData -and $Action -ne "Uninstall") {
  throw "-PurgeData only applies to Uninstall"
}
if ($NoPathUpdate -and $Action -notin @("Install", "Reinstall", "Upgrade", "Uninstall")) {
  throw "-NoPathUpdate only applies to Install, Reinstall, Upgrade, and Uninstall"
}

if ($Source) {
  if (-not $replacementAction) {
    throw "-Source is only supported for Install, Reinstall, and Upgrade"
  }
  if ($Archive) { throw "-Source cannot be combined with -Archive" }
  if ($Checksum) { throw "-Source cannot be combined with -Checksum" }
  if ($Version) { throw "-Version applies to release installs; use -SourceRef to select a branch or tag" }
}
elseif ($SourcePath -or $SourceRef) {
  throw "-SourcePath and -SourceRef require -Source"
}

function Resolve-Directory([string]$Path) {
  if ([string]::IsNullOrWhiteSpace($Path)) { throw "directory must not be empty" }
  return [IO.Path]::GetFullPath($Path).TrimEnd([IO.Path]::DirectorySeparatorChar, [IO.Path]::AltDirectorySeparatorChar)
}

function Assert-StoragePaths([switch]$InstallOnly) {
  $script:InstallDir = Resolve-Directory $InstallDir
  $paths = @($InstallDir)
  if (-not $InstallOnly) {
    $script:DataDir = Resolve-Directory $DataDir
    $paths += $DataDir
  }
  $blocked = @($HOME, $env:SystemRoot, $env:ProgramFiles, $env:LOCALAPPDATA, $env:TEMP) |
    Where-Object { $_ } | ForEach-Object { Resolve-Directory $_ }
  foreach ($path in $paths) {
    if ($path -eq [IO.Path]::GetPathRoot($path).TrimEnd('\', '/') -or $blocked -contains $path) {
      throw "unsafe Agena storage directory: $path"
    }
    $ancestor = $path
    while ($ancestor) {
      if ((Test-Path -LiteralPath $ancestor) -and ((Get-Item -LiteralPath $ancestor).Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Agena storage directories must not traverse links or junctions: $ancestor"
      }
      $ancestor = [IO.Path]::GetDirectoryName($ancestor)
    }
  }
  if ($InstallOnly) { return }
  $separator = [IO.Path]::DirectorySeparatorChar
  $comparison = [StringComparison]::OrdinalIgnoreCase
  if (($InstallDir + $separator).StartsWith($DataDir + $separator, $comparison) -or
      ($DataDir + $separator).StartsWith($InstallDir + $separator, $comparison)) {
    throw "application installation and instance data directories must be separate"
  }
}

if ($Action -eq "Install") { Assert-StoragePaths }
else { Assert-StoragePaths -InstallOnly }

$StateFile = Join-Path $InstallDir "install-state.json"
$BinDir = Join-Path $InstallDir "bin"
$AgenaBin = Join-Path $BinDir "agena.exe"
$WebDir = Join-Path $InstallDir "web-dist"

function Normalize-Version([string]$Raw) {
  $value = $Raw.Trim()
  if ($value.StartsWith("agena-v")) { return $value.Substring(7) }
  if ($value.StartsWith("v")) { return $value.Substring(1) }
  return $value
}

function Get-TargetTriple {
  $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
  switch ($arch) {
    "X64" { return "x86_64-pc-windows-msvc" }
    "Arm64" { return "aarch64-pc-windows-msvc" }
    default { throw "Unsupported Windows architecture: $arch" }
  }
}

function Invoke-InstallerExecutable([string]$Executable, [string[]]$Arguments) {
  # Capture native failures explicitly, including in Windows PowerShell 5.1.
  $previousErrorAction = $ErrorActionPreference
  try {
    $ErrorActionPreference = "Continue"
    $PSNativeCommandUseErrorActionPreference = $false
    $global:LASTEXITCODE = $null
    $output = (& $Executable @Arguments 2>&1 | Out-String).Trim()
    $code = if ($null -eq $LASTEXITCODE) { 1 } else { $LASTEXITCODE }
  }
  finally { $ErrorActionPreference = $previousErrorAction }
  return [pscustomobject]@{ ExitCode = $code; Output = $output }
}

function Get-DependencyCommand([string]$Name) {
  $commands = @(Get-Command $Name -CommandType Application -All -ErrorAction SilentlyContinue)
  # App Installer's Python alias opens the Store instead of running Python.
  if ($Name -eq "python") {
    $commands = @($commands | Where-Object { $_.Source -notmatch '[\\/]Microsoft[\\/]WindowsApps[\\/]python(?:3)?\.exe$' })
  }
  return $commands | Select-Object -First 1
}

function Invoke-DependencyCommand([string]$Name, [string[]]$Arguments) {
  $command = Get-DependencyCommand $Name
  if (-not $command) { throw "$Name is required to build Agena from source" }
  return Invoke-InstallerExecutable $command.Source $Arguments
}

function Refresh-DependencyPath {
  $cargoRoot = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $HOME ".cargo" }
  $bunRoot = if ($env:BUN_INSTALL) { $env:BUN_INSTALL } else { Join-Path $HOME ".bun" }
  $parts = @(
    $env:PATH,
    [Environment]::GetEnvironmentVariable("Path", "User"),
    [Environment]::GetEnvironmentVariable("Path", "Machine"),
    (Join-Path $cargoRoot "bin"),
    (Join-Path $bunRoot "bin")
  )
  $env:PATH = (($parts | Where-Object { $_ }) -join ";")
}

function Install-WindowsDependency([string]$Package, [string]$PackageVersion = "", [string]$InstallerOptions = "") {
  if ($SetupMode -eq "check") {
    throw "Missing build dependency $Package; -SetupMode check never installs dependencies. Rerun with -SetupMode auto."
  }
  if (-not (Get-DependencyCommand "winget")) {
    throw "winget is required to prepare $Package. Install Microsoft's App Installer (https://aka.ms/getwinget), then rerun this command."
  }
  Write-Host "Preparing missing build dependency $Package. Windows may request UAC approval for its installer; Agena remains a user service."
  $arguments = @("install", "--exact", "--id", $Package, "--source", "winget", "--accept-source-agreements", "--accept-package-agreements", "--disable-interactivity")
  if ($PackageVersion) { $arguments += @("--version", $PackageVersion) }
  if ($InstallerOptions) { $arguments += @("--override", $InstallerOptions) }
  $result = Invoke-DependencyCommand "winget" $arguments
  if ($result.ExitCode -notin @(0, 3010)) {
    throw "winget could not prepare $Package (exit $($result.ExitCode)). Approve its installer if Windows requests it, then retry. $($result.Output)"
  }
  if ($result.ExitCode -eq 3010) { Write-Warning "$Package requests a Windows restart before it can be used." }
  Refresh-DependencyPath
}

function Test-WindowsCommand([string]$Name, [string]$MinimumVersion = "") {
  if (-not (Get-DependencyCommand $Name)) { return $false }
  if (-not $MinimumVersion) { return $true }
  $result = Invoke-DependencyCommand $Name @("--version")
  if ($result.ExitCode -ne 0 -or $result.Output -notmatch '(\d+\.\d+(?:\.\d+)?)') { return $false }
  return ([version]$Matches[1] -ge [version]$MinimumVersion)
}

function Ensure-WindowsCommand([string]$Name, [string]$Package, [string]$MinimumVersion = "", [string]$PackageVersion = "") {
  if (-not (Test-WindowsCommand $Name $MinimumVersion)) {
    Install-WindowsDependency $Package $PackageVersion
    if (-not (Test-WindowsCommand $Name $MinimumVersion)) {
      throw "$Name is still unavailable or too old after installing $Package. Open a new PowerShell window and rerun the installer."
    }
  }
  # Use the validated executable for nested builds as well, especially when
  # WindowsApps' Python Store alias appears earlier in the caller's PATH.
  $selected = Get-DependencyCommand $Name
  $directory = Split-Path -Parent $selected.Source
  if ($directory) { $env:PATH = $directory + ";" + $env:PATH }
}

function Get-SourceToolVersions([string]$Root) {
  $rustFile = Join-Path $Root "rust-toolchain.toml"
  $bunFile = Join-Path $Root ".bun-version"
  if (-not (Test-Path -LiteralPath $rustFile) -or -not (Test-Path -LiteralPath $bunFile)) {
    throw "Source checkout is missing rust-toolchain.toml or .bun-version"
  }
  $rustConfig = Get-Content -LiteralPath $rustFile -Raw
  if ($rustConfig -notmatch '(?m)^\s*channel\s*=\s*"(\d+\.\d+\.\d+)"\s*$') {
    throw "rust-toolchain.toml must select a pinned Rust release"
  }
  $rustVersion = $Matches[1]
  $bunVersion = (Get-Content -LiteralPath $bunFile -Raw).Trim()
  if ($bunVersion -notmatch '^\d+\.\d+\.\d+$') { throw ".bun-version must select a pinned Bun release" }
  return [pscustomobject]@{ Rust = $rustVersion; Bun = $bunVersion }
}

function Get-WindowsBuildToolsState([string]$Target) {
  $programFilesX86 = [Environment]::GetEnvironmentVariable("ProgramFiles(x86)")
  $missing = [pscustomobject]@{ Ready = $false; InstallPath = "" }
  if (-not $programFilesX86) { return $missing }
  $vswhere = Join-Path $programFilesX86 "Microsoft Visual Studio/Installer/vswhere.exe"
  if (-not (Test-Path -LiteralPath $vswhere)) { return $missing }
  $query = @("-latest", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath")
  $result = Invoke-InstallerExecutable $vswhere $query
  $installation = $result.Output.Trim()
  if ($result.ExitCode -ne 0 -or -not $installation) {
    $result = Invoke-InstallerExecutable $vswhere @("-latest", "-products", "*", "-property", "installationPath")
    if ($result.ExitCode -ne 0) { return $missing }
    return [pscustomobject]@{ Ready = $false; InstallPath = $result.Output.Trim() }
  }
  $missing.InstallPath = $installation
  if (-not (Test-Path -LiteralPath (Join-Path $installation "Common7/Tools/VsDevCmd.bat"))) { return $missing }
  $toolsets = Join-Path $installation "VC/Tools/MSVC"
  $defaultTools = Join-Path $installation "VC/Auxiliary/Build/Microsoft.VCToolsVersion.default.txt"
  $toolset = $null
  if (Test-Path -LiteralPath $defaultTools) {
    $defaultVersion = (Get-Content -LiteralPath $defaultTools -Raw).Trim()
    if ($defaultVersion -match '^\d+\.\d+\.\d+$') {
      $toolset = Get-Item -LiteralPath (Join-Path $toolsets $defaultVersion) -ErrorAction SilentlyContinue
    }
  }
  if (-not $toolset) {
    $toolset = Get-ChildItem -LiteralPath $toolsets -Directory -ErrorAction SilentlyContinue | Sort-Object Name -Descending | Select-Object -First 1
  }
  if (-not $toolset) { return $missing }
  $arch = if ($Target.StartsWith("aarch64-")) { "arm64" } else { "x64" }
  $hostArch = if ($env:PROCESSOR_ARCHITECTURE -eq "ARM64") { "HostARM64" } else { "HostX64" }
  foreach ($file in @("bin/$hostArch/$arch/cl.exe", "include/vcruntime.h", "lib/$arch/vcruntime.lib")) {
    if (-not (Test-Path -LiteralPath (Join-Path $toolset.FullName $file))) { return $missing }
  }
  $sdk = Join-Path $programFilesX86 "Windows Kits/10"
  $sdkVersions = Get-ChildItem -LiteralPath (Join-Path $sdk "Include") -Directory -ErrorAction SilentlyContinue
  foreach ($sdkVersion in $sdkVersions) {
    $version = $sdkVersion.Name
    $paths = @("Include/$version/ucrt/stddef.h", "Include/$version/um/Windows.h", "Lib/$version/um/$arch/kernel32.lib", "Lib/$version/ucrt/$arch/ucrt.lib")
    $absent = @($paths | Where-Object { -not (Test-Path -LiteralPath (Join-Path $sdk $_)) })
    if ($absent.Count -eq 0) { return [pscustomobject]@{ Ready = $true; InstallPath = $installation } }
  }
  return $missing
}

function Ensure-WindowsBuildTools([string]$Target) {
  $state = Get-WindowsBuildToolsState $Target
  if ($state.Ready) { return }
  if ($SetupMode -eq "check") {
    throw "MSVC C++ tools or Windows SDK headers/libraries are missing for $Target; -SetupMode check never changes Visual Studio. Rerun with -SetupMode auto."
  }
  $components = @("Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "Microsoft.VisualStudio.Component.Windows11SDK.26100")
  if ($Target.StartsWith("aarch64-")) { $components += "Microsoft.VisualStudio.Component.VC.Tools.ARM64" }
  if ($state.InstallPath) {
    $installer = Join-Path ([Environment]::GetEnvironmentVariable("ProgramFiles(x86)")) "Microsoft Visual Studio/Installer/setup.exe"
    if (-not (Test-Path -LiteralPath $installer)) { throw "Visual Studio Installer is missing; repair Visual Studio Build Tools and rerun." }
    # Only this dependency installer is elevated. Quotes preserve spaces when
    # Start-Process converts its argument list to the native Windows command line.
    $installPath = $state.InstallPath.TrimEnd('\')
    if ($installPath -match '["\r\n]') { throw "Visual Studio returned an invalid installation path" }
    $arguments = @("modify", "--installPath", ('"' + $installPath + '"'), "--passive", "--norestart", "--includeRecommended")
    foreach ($component in $components) { $arguments += @("--add", $component) }
    Write-Host "Preparing missing Microsoft C++/SDK components. Approve the Visual Studio Installer UAC prompt."
    $process = Start-Process -FilePath $installer -ArgumentList $arguments -Verb RunAs -Wait -PassThru
    if ($process.ExitCode -notin @(0, 3010)) { throw "Visual Studio component installation failed (exit $($process.ExitCode))" }
  } else {
    $options = "--wait --passive --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
    foreach ($component in $components) { $options += " --add $component" }
    Install-WindowsDependency "Microsoft.VisualStudio.2022.BuildTools" "" $options
  }
  if (-not (Get-WindowsBuildToolsState $Target).Ready) {
    throw "MSVC/Windows SDK is still incomplete for $Target. Finish the Visual Studio Installer and any requested Windows restart, then rerun."
  }
}

function Ensure-WindowsSourceDependencies([string]$Root, [string]$Target) {
  $versions = Get-SourceToolVersions $Root
  Write-Host "Checking source build dependencies (setup mode: $SetupMode)"
  Refresh-DependencyPath
  Ensure-WindowsBuildTools $Target
  Ensure-WindowsCommand "rustup" "Rustlang.Rustup"
  $installed = Invoke-DependencyCommand "rustup" @("toolchain", "list", "--quiet")
  if ($installed.ExitCode -ne 0) { throw "Unable to inspect installed Rust toolchains: $($installed.Output)" }
  if ($installed.Output -notmatch ("(?m)^" + [regex]::Escape($versions.Rust) + "(?:-|\s|$)")) {
    if ($SetupMode -eq "check") { throw "Rust $($versions.Rust) is missing; -SetupMode check never installs toolchains." }
    $result = Invoke-DependencyCommand "rustup" @("toolchain", "install", $versions.Rust, "--profile", "minimal", "--component", "rustfmt", "--component", "clippy")
    if ($result.ExitCode -ne 0) { throw "Rust toolchain installation failed: $($result.Output)" }
  }
  $components = Invoke-DependencyCommand "rustup" @("component", "list", "--installed", "--toolchain", $versions.Rust)
  if ($components.ExitCode -ne 0) { throw "Unable to inspect Rust components: $($components.Output)" }
  foreach ($component in @("rustfmt", "clippy")) {
    if ($components.Output -match ("(?m)^" + $component + "(?:-|\s|$)")) { continue }
    if ($SetupMode -eq "check") { throw "Rust component $component is missing; -SetupMode check never installs components." }
    $result = Invoke-DependencyCommand "rustup" @("component", "add", $component, "--toolchain", $versions.Rust)
    if ($result.ExitCode -ne 0) { throw "Rust component installation failed: $($result.Output)" }
  }
  $targets = Invoke-DependencyCommand "rustup" @("target", "list", "--installed", "--toolchain", $versions.Rust)
  if ($targets.ExitCode -ne 0) { throw "Unable to inspect Rust targets: $($targets.Output)" }
  if (@($targets.Output -split '\r?\n') -notcontains $Target) {
    if ($SetupMode -eq "check") { throw "Rust target $Target is missing; -SetupMode check never installs targets." }
    $result = Invoke-DependencyCommand "rustup" @("target", "add", $Target, "--toolchain", $versions.Rust)
    if ($result.ExitCode -ne 0) { throw "Rust target installation failed: $($result.Output)" }
  }
  if ($env:AGENA_WEB_DIST_PREBUILT -ne "1") { Ensure-WindowsCommand "bun" "Oven-sh.Bun" $versions.Bun $versions.Bun }
  # Match the repository's Python CI line when Python 3 is not already available.
  Ensure-WindowsCommand "python" "Python.Python.3.13" "3.0"
  Write-Host "Source build dependencies are ready"
}

function Test-SourceRoot([string]$Path) {
  return ((Test-Path -LiteralPath (Join-Path $Path "Cargo.toml") -PathType Leaf) -and
    (Test-Path -LiteralPath (Join-Path $Path "packages/agena-web/package.json") -PathType Leaf) -and
    (Test-Path -LiteralPath (Join-Path $Path "scripts/agena/package-backend.ps1") -PathType Leaf))
}

function Find-SourceRoot([string]$Start) {
  if ([string]::IsNullOrWhiteSpace($Start)) { return $null }
  $current = Resolve-Directory $Start
  while ($current) {
    if (Test-SourceRoot $current) { return $current }
    $parent = [IO.Path]::GetDirectoryName($current)
    if (-not $parent -or $parent -eq $current) { break }
    $current = $parent
  }
  return $null
}

function Get-InstallerSourceRoot {
  if ([string]::IsNullOrWhiteSpace($PSScriptRoot)) { return $null }
  $scriptsDir = Split-Path -Parent $PSScriptRoot
  $repositoryRoot = Split-Path -Parent $scriptsDir
  return Find-SourceRoot $repositoryRoot
}

function Test-GitSource([string]$Value) {
  return $Value -match '^(https?://|ssh://|git://|file://|git@.+:)'
}

function Invoke-SourceClone([string]$Repository, [string]$Temp) {
  Ensure-WindowsCommand "git" "Git.Git"
  $git = Get-Command git -ErrorAction SilentlyContinue
  if (-not $git) { throw "git is required to install from a remote source repository" }
  $destination = Join-Path $Temp "source"
  $cloneArgs = @("clone", "--depth", "1")
  if ($SourceRef) { $cloneArgs += @("--branch", $SourceRef) }
  $cloneArgs += @($Repository, $destination)
  Write-Host "Cloning Agena source from $Repository"
  & $git.Source @cloneArgs
  if ($LASTEXITCODE -ne 0) { throw "git clone failed with exit code $LASTEXITCODE" }
  if (-not (Test-Path -LiteralPath $destination -PathType Container)) {
    throw "git clone did not create $destination"
  }
  return $destination
}

function Resolve-SourceRoot([string]$Temp) {
  $requested = $SourcePath.Trim()
  if ($requested) {
    if (Test-Path -LiteralPath $requested -PathType Container) {
      $root = Find-SourceRoot $requested
      if (-not $root) { throw "-SourcePath is not an Agena checkout: $requested" }
      return $root
    }
    if (Test-GitSource $requested) {
      $cloned = Invoke-SourceClone $requested $Temp
      $root = Find-SourceRoot $cloned
      if (-not $root) { throw "cloned source is not an Agena checkout: $requested" }
      return $root
    }
    throw "-SourcePath must name an Agena checkout or a Git URL"
  }

  $root = Find-SourceRoot (Get-Location).Path
  if ($root) { return $root }
  $root = Get-InstallerSourceRoot
  if ($root) { return $root }

  $cloned = Invoke-SourceClone "https://github.com/$Repo.git" $Temp
  $root = Find-SourceRoot $cloned
  if (-not $root) { throw "cloned source is not an Agena checkout" }
  return $root
}

function Invoke-SourcePackage([string]$Root, [string]$Target) {
  $packageScript = Join-Path $Root "scripts/agena/package-backend.ps1"
  if (-not (Test-Path -LiteralPath $packageScript -PathType Leaf)) {
    throw "Agena source build script is missing: $packageScript"
  }
  if ($SetupMode -eq "check") {
    $setupScript = Join-Path $Root "scripts/ci/setup-windows-toolchain.ps1"
    if (-not (Test-Path -LiteralPath $setupScript) -or
        (Get-Content -LiteralPath $setupScript -Raw) -notmatch 'AGENA_INSTALLER_SETUP_MODE') {
      throw "This source checkout does not support dependency check mode. Update it before using -Source -SetupMode check."
    }
  }
  $versions = Get-SourceToolVersions $Root
  $cargo = Invoke-DependencyCommand "rustup" @("which", "--toolchain", $versions.Rust, "cargo")
  if ($cargo.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $cargo.Output -PathType Leaf)) {
    throw "The requested Rust $($versions.Rust) Cargo executable is unavailable: $($cargo.Output)"
  }
  $previousMode = $env:AGENA_INSTALLER_SETUP_MODE
  $previousToolchain = $env:RUSTUP_TOOLCHAIN
  $previousPath = $env:PATH
  try {
    $env:AGENA_INSTALLER_SETUP_MODE = $SetupMode
    $env:RUSTUP_TOOLCHAIN = $versions.Rust
    $env:PATH = (Split-Path -Parent $cargo.Output) + ";" + $env:PATH
    & $packageScript -TargetTriple $Target
    if ($LASTEXITCODE -ne 0) { throw "Agena source build failed with exit code $LASTEXITCODE" }
  }
  finally {
    $env:AGENA_INSTALLER_SETUP_MODE = $previousMode
    $env:RUSTUP_TOOLCHAIN = $previousToolchain
    $env:PATH = $previousPath
  }
}

function New-SourceArchive([string]$Temp, [string]$Target) {
  $root = Resolve-SourceRoot $Temp
  if ($SourceRef -and $root -ne (Resolve-Directory (Join-Path $Temp "source"))) {
    Write-Warning "-SourceRef only applies when cloning; using the existing checkout as-is"
  }
  Ensure-WindowsSourceDependencies $root $Target
  Write-Host "Building Agena from source at $root"
  Invoke-SourcePackage $root $Target

  $artifactDir = Join-Path $root "artifacts/agena"
  $archive = Get-ChildItem -LiteralPath $artifactDir -File -Filter "agena-backend-$Target-v*.zip" |
    Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
  if (-not $archive) { throw "Agena source build did not produce a package archive" }
  $checksum = "$($archive.FullName).sha256"
  if (-not (Test-Path -LiteralPath $checksum -PathType Leaf)) {
    throw "Agena source build did not produce a package checksum"
  }
  Write-Host "Source package ready: $($archive.FullName)"
  return [pscustomobject]@{
    Archive = $archive.FullName
    Checksum = $checksum
  }
}

function Get-LatestVersion {
  $temp = Join-Path $env:TEMP ("agena-release-latest-" + [Guid]::NewGuid().ToString("N") + ".json")
  try {
    Get-FileFromSource "https://api.github.com/repos/$Repo/releases?per_page=100" $temp
    $releases = @(Get-Content -LiteralPath $temp -Raw | ConvertFrom-Json)
  }
  finally {
    Remove-Item -LiteralPath $temp -Force -ErrorAction SilentlyContinue
  }
  $release = $releases | Where-Object {
    -not $_.draft -and $_.prerelease -and ([string]$_.tag_name -match '^agena-v\d+\.\d+\.\d+-beta\.[1-9]\d*$')
  } | Select-Object -First 1
  if (-not $release -or -not $release.tag_name) { throw "No Agena beta release was found for $Repo" }
  return Normalize-Version ([string]$release.tag_name)
}

function Get-FileFromSource([string]$Source, [string]$Destination) {
  if ($Source -notmatch '^https?://') {
    Copy-Item -LiteralPath $Source -Destination $Destination -Force
    return
  }

  Write-Host "Downloading $Source"
  $curl = Get-Command curl.exe -ErrorAction SilentlyContinue
  if ($curl) {
    & $curl.Source `
      --fail `
      --location `
      --silent `
      --show-error `
      --retry 3 `
      --retry-delay 1 `
      --retry-connrefused `
      --connect-timeout 15 `
      --max-time 600 `
      --user-agent "agena-installer" `
      --output $Destination `
      $Source
    if ($LASTEXITCODE -ne 0) {
      throw "curl download failed with exit code ${LASTEXITCODE}: $Source"
    }
    $bytes = (Get-Item -LiteralPath $Destination).Length
    Write-Host "Downloaded $bytes bytes"
    return
  }

  # curl.exe ships with supported Windows releases. Keep a bounded
  # Invoke-WebRequest fallback for older/minimal PowerShell environments.
  $command = Get-Command Invoke-WebRequest
  for ($attempt = 1; $attempt -le 3; $attempt++) {
    try {
      $params = @{
        Uri = $Source
        OutFile = $Destination
        Headers = @{ "User-Agent" = "agena-installer" }
      }
      if ($command.Parameters.ContainsKey("ConnectionTimeoutSeconds")) {
        $params.ConnectionTimeoutSeconds = 15
        if ($command.Parameters.ContainsKey("OperationTimeoutSeconds")) {
          $params.OperationTimeoutSeconds = 600
        }
      }
      elseif ($command.Parameters.ContainsKey("TimeoutSec")) {
        $params.TimeoutSec = 600
      }
      Invoke-WebRequest @params
      $bytes = (Get-Item -LiteralPath $Destination).Length
      Write-Host "Downloaded $bytes bytes"
      return
    }
    catch {
      Remove-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue
      if ($attempt -eq 3) { throw }
      Start-Sleep -Seconds $attempt
    }
  }
}

function Assert-Checksum([string]$ArchivePath, [string]$ChecksumPath) {
  $line = (Get-Content -LiteralPath $ChecksumPath | Where-Object { $_.Trim() } | Select-Object -First 1)
  if (-not $line -or $line -notmatch '^\s*([0-9A-Fa-f]{64})\b') {
    throw "Invalid SHA256 file: $ChecksumPath"
  }
  $expected = $Matches[1].ToLowerInvariant()
  $actual = (Get-FileHash -LiteralPath $ArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $expected) {
    throw "SHA256 mismatch: expected $expected, got $actual"
  }
}

function Save-State {
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  $state = [ordered]@{
    schema = 2
    repo = $Repo
    version = $Version
    releaseVersion = $ReleaseVersion
    installDir = $InstallDir
    dataDir = $DataDir
    host = $ListenHost
    port = $Port
    workspace = $Workspace
    serverPassword = $ServerPassword
    serviceMode = $ServiceMode
  }
  $state | ConvertTo-Json | Set-Content -LiteralPath $StateFile -Encoding UTF8
}

function Load-State {
  if (-not (Test-Path -LiteralPath $StateFile)) {
    throw "Agena is not installed at $InstallDir"
  }
  $selectedInstallDir = $InstallDir
  $state = Get-Content -LiteralPath $StateFile -Raw | ConvertFrom-Json
  if ($state.schema -ne 2 -or -not $state.dataDir -or
      $state.PSObject.Properties.Name -notcontains "serverPassword") {
    throw "installer state is incompatible; use a new installation directory and an explicit -DataDir"
  }
  $script:Repo = [string]$state.repo
  $script:Version = [string]$state.version
  $script:ReleaseVersion = [string]$state.releaseVersion
  $script:InstallDir = Resolve-Directory ([string]$state.installDir)
  if ($InstallDir -ne $selectedInstallDir) { throw "installer state belongs to a different installation" }
  $script:DataDir = [string]$state.dataDir
  $script:ListenHost = [string]$state.host
  $script:Port = [int]$state.port
  $script:Workspace = [string]$state.workspace
  $script:ServerPassword = [string]$state.serverPassword
  $script:ServiceMode = [string]$state.serviceMode
  Assert-StoragePaths
  $script:StateFile = Join-Path $script:InstallDir "install-state.json"
  $script:BinDir = Join-Path $script:InstallDir "bin"
  $script:AgenaBin = Join-Path $script:BinDir "agena.exe"
  $script:WebDir = Join-Path $script:InstallDir "web-dist"
}

function Update-UserPath {
  if ($NoPathUpdate) { return }
  [Environment]::SetEnvironmentVariable("AGENA_DATA_DIR", $DataDir, "User")
  $env:AGENA_DATA_DIR = $DataDir
  [Environment]::SetEnvironmentVariable("AGENA_SERVER_PASSWORD", $ServerPassword, "User")
  $env:AGENA_SERVER_PASSWORD = $ServerPassword
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  $parts = @($userPath -split ';' | Where-Object { $_ })
  if ($parts -notcontains $BinDir) {
    $next = (@($BinDir) + $parts) -join ';'
    [Environment]::SetEnvironmentVariable("Path", $next, "User")
  }
  if (($env:Path -split ';') -notcontains $BinDir) {
    $env:Path = "$BinDir;$env:Path"
  }
}

function Remove-UserPath {
  if (-not $NoPathUpdate -and [Environment]::GetEnvironmentVariable("AGENA_DATA_DIR", "User") -eq $DataDir) {
    [Environment]::SetEnvironmentVariable("AGENA_DATA_DIR", $null, "User")
  }
  $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
  if ($null -eq $userPath) { return }
  $parts = @($userPath -split ';' | Where-Object { $_ -and $_ -ne $BinDir })
  [Environment]::SetEnvironmentVariable("Path", ($parts -join ';'), "User")
}

function Resolve-ServiceMode {
  if ($ServiceMode -eq "auto") {
    $script:ServiceMode = "native"
  }
}

function Get-ServerArgs {
  $serverArgs = @(
    "--host", $ListenHost,
    "--port", "$Port",
    "--workspace", $Workspace,
    "--ui-dir", $WebDir
  )
  if ($ServerPassword) { $serverArgs += @("--password", $ServerPassword) }
  return $serverArgs
}

function Invoke-AgenaLifecycleAt([string]$DataRoot, [string[]]$Arguments, [switch]$Capture) {
  $previousDataDir = $env:AGENA_DATA_DIR
  try {
    $env:AGENA_DATA_DIR = $DataRoot
    $result = Invoke-InstallerExecutable $AgenaBin $Arguments
    if ($Capture) { return $result }
    if ($result.Output) { Write-Host $result.Output }
    return $result.ExitCode
  }
  finally { $env:AGENA_DATA_DIR = $previousDataDir }
}

function Invoke-AgenaLifecycle([string[]]$Arguments, [switch]$Capture) {
  return Invoke-AgenaLifecycleAt $DataDir $Arguments -Capture:$Capture
}

function Test-InstalledSandbox {
  $script:SandboxReport = $null
  Write-Host "Verifying Agena sandbox at its final installed path: $AgenaBin"
  $result = Invoke-AgenaLifecycle @("--agena-sandbox-check") -Capture
  try { $report = $result.Output | ConvertFrom-Json }
  catch { throw "Sandbox verification did not return a valid JSON report. $($result.Output)" }
  if ($report -isnot [System.Management.Automation.PSCustomObject] -or
      ($report.schema_version -isnot [int] -and $report.schema_version -isnot [long]) -or
      $report.schema_version -ne 1 -or $report.platform -isnot [string] -or $report.platform -cne "windows" -or
      $report.ok -isnot [bool] -or $report.checks -isnot [array] -or
      ($report.duration_ms -isnot [int] -and $report.duration_ms -isnot [long]) -or $report.duration_ms -lt 0) {
    throw "Sandbox verification returned an incompatible report. Install an Agena build that supports --agena-sandbox-check."
  }
  $checks = @($report.checks)
  $names = New-Object 'System.Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
  foreach ($check in $checks) {
    if ($check -isnot [System.Management.Automation.PSCustomObject] -or
        $check.name -isnot [string] -or $check.name -cnotmatch '^[a-z][a-z0-9_]*$' -or
        $check.required -isnot [bool] -or $check.status -isnot [string] -or
        @("passed", "failed", "unsupported") -cnotcontains $check.status -or $check.detail -isnot [string] -or
        -not $names.Add($check.name)) {
      throw "Sandbox verification returned malformed or duplicate checks."
    }
  }
  if (-not $names.Contains("native_network")) { throw "Sandbox verification is missing the native_network capability report." }
  $offline = @($checks | Where-Object { $_.name -eq "offline" -and $_.required -eq $true })
  $nativeNetwork = @($checks | Where-Object { $_.name -eq "native_network" -and $_.required -eq $true -and $_.status -eq "passed" })
  foreach ($check in $checks) {
    if ($check.status -eq "passed") { Write-Host "Sandbox $($check.name): passed" }
    else { Write-Warning "Sandbox $($check.name): $($check.status). $($check.detail)" }
  }
  $failed = @($checks | Where-Object { $_.required -eq $true -and $_.status -ne "passed" })
  if ($result.ExitCode -ne 0 -or -not $report.ok -or $offline.Count -ne 1 -or $nativeNetwork.Count -ne 1 -or $failed.Count -ne 0) {
    $details = ($checks | Where-Object { $_.status -ne "passed" } | ForEach-Object { "$($_.name): $($_.detail)" }) -join "; "
    throw "Required sandbox verification failed (exit $($result.ExitCode)). $details"
  }
  $script:SandboxReport = $report
}

function Save-SandboxReport {
  if (-not $script:SandboxReport) { throw "A successful sandbox verification report is required before saving." }
  $path = Join-Path $InstallDir "sandbox-report.json"
  $temporary = Join-Path $InstallDir (".sandbox-report-" + [Guid]::NewGuid().ToString("N") + ".tmp")
  try {
    $json = ($script:SandboxReport | ConvertTo-Json -Depth 8 -Compress) + [Environment]::NewLine
    [IO.File]::WriteAllText($temporary, $json, (New-Object Text.UTF8Encoding $false))
    if (Test-Path -LiteralPath $path) { [IO.File]::Replace($temporary, $path, $null) }
    else { Move-Item -LiteralPath $temporary -Destination $path }
  }
  finally {
    if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
  }
}

function Repair-Permissions {
  Load-State
  # Windows AppContainer is built in. No global policy or elevated Agena
  # process is needed; this also reports optional network capabilities.
  Test-InstalledSandbox
  Save-SandboxReport
  Write-Host "Agena sandbox permissions verified at $InstallDir"
}

function Test-ServerRunningAt([string]$DataRoot) {
  if (-not (Test-Path -LiteralPath $AgenaBin)) { return $false }
  $result = Invoke-AgenaLifecycleAt $DataRoot @("server", "status") -Capture
  if ($result.ExitCode -ne 0) {
    throw "Unable to inspect the installed Agena server (exit $($result.ExitCode)); installation was not changed. $($result.Output)"
  }
  if ($result.Output -match '(?m)^Agena server is running at ') { return $true }
  if ($result.Output -match '(?m)^Agena server (?:is not running|user service is installed but not running) \(') { return $false }
  throw "Agena server status returned an unrecognized response; installation was not changed. $($result.Output)"
}

function Test-ServerRunning {
  return Test-ServerRunningAt $DataDir
}

# Stop or deregister the instance described by the given saved settings.
function Stop-Installation([string]$DataRoot, [string]$Mode) {
  if (-not (Test-Path -LiteralPath $AgenaBin)) { return }
  if ($Mode -eq "native") {
    $code = Invoke-AgenaLifecycleAt $DataRoot @("server", "uninstall")
    if ($code -ne 0) { throw "Failed to stop/uninstall the existing Agena user service" }
  } elseif (Test-ServerRunningAt $DataRoot) {
    $code = Invoke-AgenaLifecycleAt $DataRoot @("server", "stop")
    if ($code -ne 0) { throw "Agena server stop failed" }
  }
}

function Start-Installed {
  $serverArgs = Get-ServerArgs
  if ($ServiceMode -eq "native") {
    $code = Invoke-AgenaLifecycle (@("server", "install") + $serverArgs)
  } else {
    $code = Invoke-AgenaLifecycle (@("server", "start") + $serverArgs)
  }
  if ($code -ne 0) { throw "Agena server failed to start" }
}

# Replacement pipeline shared by Install, Reinstall, and Upgrade:
#
#   Resolve-Replacement    read saved state, overlay explicit options
#   New-Candidate          download or build, verify, unpack, probe version
#   Install-Candidate      stop the current instance, swap program files
#   Complete-Replacement   verify sandbox, start, restore prior run state, save state
#
# Failures during activation restore the previous program files, state,
# settings, and running state.

$script:TempDir = ""
$script:BackupDir = ""
$script:CandidateStage = ""
$script:CandidateVersion = ""
$script:CandidateInstalled = $false
$script:ActivationStarted = $false
$script:HadState = $false
$script:OldDataDir = ""
$script:OldServiceMode = ""
$script:OldWasRunning = $false
$script:ReplacementAction = ""

function Resolve-Replacement {
  $requestedRepo = $Repo
  $requestedVersion = $Version
  $requestedDataDir = $DataDir
  $requestedListenHost = $ListenHost
  $requestedPort = $Port
  $requestedWorkspace = $Workspace
  $requestedServerPassword = $ServerPassword
  $requestedServiceMode = $ServiceMode

  $script:ReplacementAction = $Action
  $script:OldWasRunning = $false
  if ($Action -eq "Install") {
    if (Test-Path -LiteralPath $StateFile) {
      throw ("Agena is already installed at $InstallDir; " +
        "use -Action Reinstall to replace the installed version or -Action Upgrade to install a newer version")
    }
    if ($Source) { $script:ReleaseVersion = "" }
    elseif ($Archive) { if (-not $VersionSpecified) { $script:ReleaseVersion = "" } }
    else { $script:ReleaseVersion = $requestedVersion }
  }
  else {
    Load-State
    $script:OldDataDir = $DataDir
    $script:OldServiceMode = $ServiceMode
    $script:OldWasRunning = Test-ServerRunning
    $script:ReleaseVersion = [string]$ReleaseVersion
    if ($RepoSpecified) { $script:Repo = $requestedRepo }
    if ($Source) { $script:ReleaseVersion = "" }
    elseif ($Archive) {
      $script:ReleaseVersion = if ($VersionSpecified) { $requestedVersion } else { "" }
    }
    elseif ($VersionSpecified) { $script:ReleaseVersion = $requestedVersion }
    elseif ($Action -eq "Upgrade") { $script:ReleaseVersion = "" }
    if ($DataDirSpecified) { $script:DataDir = $requestedDataDir }
    if ($ListenHostSpecified) { $script:ListenHost = $requestedListenHost }
    if ($PortSpecified) { $script:Port = $requestedPort }
    if ($WorkspaceSpecified) { $script:Workspace = $requestedWorkspace }
    if ($ServerPasswordSpecified) { $script:ServerPassword = $requestedServerPassword }
    if ($ServiceModeSpecified) { $script:ServiceMode = $requestedServiceMode }
    Assert-StoragePaths
    if ($Action -eq "Reinstall" -and -not $Source -and -not $Archive -and -not $ReleaseVersion) {
      throw "This installation has no recorded release version; reinstall with -Archive PATH, -Source, or -Version VERSION"
    }
  }
  Resolve-ServiceMode
  New-Item -ItemType Directory -Force -Path $Workspace | Out-Null
}

function New-Candidate([string]$Temp) {
  $target = Get-TargetTriple
  $packageVersion = ""
  if ($Source) {
    $script:ReleaseVersion = ""
    $sourceArchive = New-SourceArchive $Temp $target
    $archiveSource = $sourceArchive.Archive
    $checksumSource = $sourceArchive.Checksum
  }
  elseif (-not $Archive) {
    if (-not $ReleaseVersion) { $selectedRelease = Get-LatestVersion } else { $selectedRelease = Normalize-Version $ReleaseVersion }
    if ($selectedRelease -notmatch '^(\d+\.\d+\.\d+)-beta\.[1-9]\d*$') {
      throw "-Version must name a beta release, for example 0.1.0-beta.1"
    }
    $script:ReleaseVersion = $selectedRelease
    $packageVersion = $Matches[1]
    $archiveSource = "https://github.com/$Repo/releases/download/agena-v$selectedRelease/agena-backend-$target-v$packageVersion.zip"
    $checksumSource = "$archiveSource.sha256"
  }
  else {
    $archiveSource = $Archive
    $checksumSource = if ($Checksum) { $Checksum } else { "$Archive.sha256" }
    if ($ReleaseVersion) {
      $selectedRelease = Normalize-Version $ReleaseVersion
      if ($selectedRelease -notmatch '^(\d+\.\d+\.\d+)-beta\.[1-9]\d*$') {
        throw "-Version must name a beta release, for example 0.1.0-beta.1"
      }
      $packageVersion = $Matches[1]
      $script:ReleaseVersion = $selectedRelease
    }
  }

  $archivePath = Join-Path $Temp "agena.zip"
  $checksumPath = Join-Path $Temp "agena.sha256"
  Get-FileFromSource $archiveSource $archivePath
  Get-FileFromSource $checksumSource $checksumPath
  Assert-Checksum $archivePath $checksumPath
  Write-Host "Verified Agena package checksum"

  $stage = Join-Path $Temp "stage"
  New-Item -ItemType Directory -Force -Path $stage | Out-Null
  Write-Host "Extracting Agena package archive"
  $tar = Get-Command tar.exe -ErrorAction SilentlyContinue
  if ($tar) {
    & $tar.Source -xf $archivePath -C $stage
    if ($LASTEXITCODE -ne 0) {
      throw "tar.exe extraction failed with exit code ${LASTEXITCODE}"
    }
  }
  else {
    Expand-Archive -LiteralPath $archivePath -DestinationPath $stage -Force
  }
  $stageBin = Join-Path $stage "bin\agena.exe"
  $stageWeb = Join-Path $stage "web-dist\index.html"
  if (-not (Test-Path -LiteralPath $stageBin) -or -not (Test-Path -LiteralPath $stageWeb)) {
    throw "Package archive is missing bin/agena.exe or web-dist/index.html"
  }
  Write-Host "Validating Agena binary"
  $versionResult = Invoke-InstallerExecutable $stageBin @("--version")
  $binaryVersionText = $versionResult.Output
  if ($versionResult.ExitCode -ne 0 -or $binaryVersionText -notmatch '(\S+)$') {
    throw "Installed Agena binary did not report a version"
  }
  $binaryVersion = $Matches[1]
  if ($packageVersion -and $binaryVersion -ne $packageVersion) {
    throw "Archive contains Agena $binaryVersion but release $ReleaseVersion contains $packageVersion"
  }
  $script:Version = $binaryVersion
  $script:CandidateVersion = $binaryVersion
  $script:CandidateStage = $stage
  Write-Host "Validated Agena $binaryVersion"
}

function Install-Candidate([string]$Temp) {
  $backup = Join-Path $Temp "backup"
  New-Item -ItemType Directory -Force -Path $backup | Out-Null
  $script:BackupDir = $backup
  if (Test-Path -LiteralPath $StateFile) {
    $script:HadState = $true
    Copy-Item -LiteralPath $StateFile -Destination (Join-Path $backup "install-state.json") -Force
  }
  $previousReport = Join-Path $InstallDir "sandbox-report.json"
  if (Test-Path -LiteralPath $previousReport) {
    Copy-Item -LiteralPath $previousReport -Destination (Join-Path $backup "sandbox-report.json") -Force
  }

  Write-Host "Stopping the installed Agena instance if needed"
  Stop-Installation $script:OldDataDir $script:OldServiceMode

  $script:ActivationStarted = $true
  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  foreach ($name in @("bin", "web-dist")) {
    $existing = Join-Path $InstallDir $name
    if (Test-Path -LiteralPath $existing) {
      Move-Item -LiteralPath $existing -Destination (Join-Path $backup $name)
    }
    Move-Item -LiteralPath (Join-Path $CandidateStage $name) -Destination $existing
  }
  $script:CandidateInstalled = $true
}

function Complete-Replacement {
  Test-InstalledSandbox
  Write-Host "Starting Agena server using $ServiceMode mode"
  Start-Installed
  if ($script:ReplacementAction -ne "Install" -and -not $script:OldWasRunning) {
    $stopCode = Invoke-AgenaLifecycle @("server", "stop")
    if ($stopCode -ne 0) {
      throw "The replacement could not return the instance to its stopped state"
    }
  }
  Save-State
  Save-SandboxReport
  try { Update-UserPath } catch { Write-Warning "Agena installed, but PATH update failed: $_" }
}

function Undo-Replacement {
  Write-Warning "Restoring the previous Agena installation."
  if (Test-Path -LiteralPath $AgenaBin) {
    if ($ServiceMode -eq "native") {
      try { [void](Invoke-AgenaLifecycle @("server", "uninstall")) } catch {}
    } else {
      # A rejected candidate can also fail its status command. Cleanup must
      # remain best effort so the previous files and saved state are restored.
      try {
        if (Test-ServerRunning) { [void](Invoke-AgenaLifecycle @("server", "stop")) }
      } catch {}
    }
  }

  foreach ($name in @("bin", "web-dist")) {
    $saved = Join-Path $BackupDir $name
    if (-not (Test-Path -LiteralPath $saved)) { continue }
    $current = Join-Path $InstallDir $name
    if (Test-Path -LiteralPath $current) { Remove-Item -LiteralPath $current -Recurse -Force }
    Move-Item -LiteralPath $saved -Destination $current
  }
  $savedReport = Join-Path $BackupDir "sandbox-report.json"
  $currentReport = Join-Path $InstallDir "sandbox-report.json"
  if (Test-Path -LiteralPath $savedReport) {
    Copy-Item -LiteralPath $savedReport -Destination $currentReport -Force
  } else {
    Remove-Item -LiteralPath $currentReport -Force -ErrorAction SilentlyContinue
  }

  if ($HadState) {
    Copy-Item -LiteralPath (Join-Path $BackupDir "install-state.json") -Destination $StateFile -Force
    Load-State
    if ($ServiceMode -eq "native") {
      try {
        Start-Installed
        if (-not $OldWasRunning) {
          [void](Invoke-AgenaLifecycle @("server", "stop"))
        }
      } catch {
        Write-Warning "Rollback restored the old files but could not restore the native service: $_"
      }
    } elseif ($OldWasRunning) {
      try { Start-Installed } catch { Write-Warning "Rollback restored the old files but could not restart the old server: $_" }
    }
  } else {
    if ($CandidateInstalled) {
      foreach ($name in @("bin", "web-dist")) {
        Remove-Item -LiteralPath (Join-Path $InstallDir $name) -Recurse -Force -ErrorAction SilentlyContinue
      }
    }
    Remove-Item -LiteralPath $StateFile -Force -ErrorAction SilentlyContinue
  }
}

function Invoke-Replacement {
  Resolve-Replacement
  $temp = Join-Path $env:TEMP ("agena-install-" + [Guid]::NewGuid().ToString("N"))
  New-Item -ItemType Directory -Force -Path $temp | Out-Null
  $script:TempDir = $temp
  try {
    New-Candidate $temp
    Install-Candidate $temp
    Complete-Replacement
    $script:ActivationStarted = $false
    $verb = switch ($script:ReplacementAction) {
      "Reinstall" { "reinstalled" }
      "Upgrade" { "upgraded" }
      default { "installed" }
    }
    Write-Host "Agena $Version $verb at $InstallDir"
    Write-Host "Instance data: $DataDir"
    Write-Host "Web UI: http://${ListenHost}:$Port"
  }
  catch {
    if ($script:ActivationStarted) {
      $script:ActivationStarted = $false
      try { Undo-Replacement } catch { Write-Warning "Rollback failed: $_" }
    }
    throw
  }
  finally {
    if (Test-Path -LiteralPath $temp) { Remove-Item -LiteralPath $temp -Recurse -Force }
  }
}

function Start-Agena {
  Load-State
  $serverArgs = Get-ServerArgs
  if ($ServiceMode -eq "native") { $code = Invoke-AgenaLifecycle @("server", "start") }
  else { $code = Invoke-AgenaLifecycle (@("server", "start") + $serverArgs) }
  if ($code -ne 0) { throw "Agena server start failed" }
}

function Stop-Agena {
  Load-State
  $code = Invoke-AgenaLifecycle @("server", "stop")
  if ($code -ne 0) { throw "Agena server stop failed" }
}

function Show-Status {
  Load-State
  $versionCode = Invoke-AgenaLifecycle @("--version")
  if ($versionCode -ne 0) { throw "Agena version check failed" }
  $statusCode = Invoke-AgenaLifecycle @("server", "status")
  if ($statusCode -ne 0) { throw "Agena server status failed" }
}

function Uninstall-Agena {
  Load-State
  if ($PurgeData -and (Test-Path -LiteralPath $DataDir) -and -not (Test-Path -LiteralPath (Join-Path $DataDir "agena.db") -PathType Leaf)) {
    throw "refusing to purge a directory without an Agena instance database: $DataDir"
  }
  if (Test-Path -LiteralPath $AgenaBin) {
    if ($ServiceMode -eq "native") {
      $code = Invoke-AgenaLifecycle @("server", "uninstall")
      if ($code -ne 0) { throw "Agena user service uninstall failed" }
    } elseif (Test-ServerRunning) {
      $code = Invoke-AgenaLifecycle @("server", "stop")
      if ($code -ne 0) { throw "Agena server stop failed" }
    }
  }
  Remove-UserPath
  Remove-Item -LiteralPath $InstallDir -Recurse -Force
  if ($PurgeData) {
    if (Test-Path -LiteralPath $DataDir) { Remove-Item -LiteralPath $DataDir -Recurse -Force }
    Write-Host "Removed Agena runtime data at $DataDir"
  }
  Write-Host "Agena uninstalled. Configuration/session data was preserved unless -PurgeData was used."
}

switch ($Action) {
  "Install" { Invoke-Replacement }
  "Reinstall" { Invoke-Replacement }
  "Upgrade" { Invoke-Replacement }
  "Uninstall" { Uninstall-Agena }
  "Start" { Start-Agena }
  "Stop" { Stop-Agena }
  "Restart" { try { Stop-Agena } catch {}; Start-Agena }
  "Status" { Show-Status }
  "Permissions" { Repair-Permissions }
}
