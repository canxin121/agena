Param(
  [string]$TargetTriple = ""
)

$ErrorActionPreference = "Stop"

function Resolve-VsArch {
  Param(
    [string]$RustTargetTriple
  )

  if (-not $RustTargetTriple) {
    return "x64"
  }

  switch -Regex ($RustTargetTriple) {
    "^x86_64-" { return "x64" }
    "^i686-" { return "x86" }
    "^aarch64-" { return "arm64" }
    default { return "x64" }
  }
}

if ($env:INCLUDE -and $env:LIB) {
  return
}

$vswhere = "C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere)) {
  throw "vswhere.exe is required to initialize the Visual Studio build environment"
}

$installationPath = & $vswhere `
  -latest `
  -products * `
  -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
  -property installationPath

if ($LASTEXITCODE -ne 0 -or -not $installationPath) {
  throw "failed to locate a Visual Studio installation with MSVC build tools"
}

$vsDevCmd = Join-Path $installationPath "Common7\Tools\VsDevCmd.bat"
if (-not (Test-Path -LiteralPath $vsDevCmd)) {
  throw "VsDevCmd.bat not found under $installationPath"
}

$arch = Resolve-VsArch -RustTargetTriple $TargetTriple
$envDump = & cmd.exe /s /c "`"$vsDevCmd`" -arch=$arch -host_arch=x64 >nul && set"
if ($LASTEXITCODE -ne 0) {
  throw "failed to import Visual Studio developer environment"
}

foreach ($line in $envDump) {
  $separator = $line.IndexOf("=")
  if ($separator -lt 1) {
    continue
  }

  $name = $line.Substring(0, $separator)
  $value = $line.Substring($separator + 1)
  Set-Item -Path "Env:$name" -Value $value
}
