Param(
  [Parameter(Mandatory = $true)]
  [string]$TargetTriple,
  [Parameter(Mandatory = $true)]
  [string]$NightlyCargo,
  [Parameter(Mandatory = $true)]
  [string]$TargetDir,
  [switch]$Release
)

$ErrorActionPreference = "Stop"

# Build the real target standard library in an isolated dependency-free crate
# before Cargo starts Agena's dependency graph.  Build scripts can invoke
# rustc directly; prepublishing the target artifacts prevents those probes
# from holding Cargo's jobserver while waiting for std.  This uses only
# Rust's build-std output for the requested target, never the host sysroot or
# a synthetic artifact.
$TempRoot = if ($env:RUNNER_TEMP) { $env:RUNNER_TEMP } else { [System.IO.Path]::GetTempPath() }
$ProbeRoot = Join-Path $TempRoot "agena-build-std-probe\$TargetTriple"
$ProbeSourceDir = Join-Path $ProbeRoot "src"
$Manifest = Join-Path $ProbeRoot "Cargo.toml"
$Source = Join-Path $ProbeSourceDir "lib.rs"
New-Item -ItemType Directory -Force -Path $ProbeSourceDir | Out-Null

$ManifestText = @"
[package]
name = "agena-build-std-probe"
version = "0.0.0"
edition = "2024"

[lib]
path = "src/lib.rs"
"@
$SourceText = @"
#![allow(dead_code)]

pub fn build_std_probe() -> String {
    std::env::consts::OS.to_owned()
}
"@
Set-Content -LiteralPath $Manifest -Value $ManifestText -Encoding utf8
Set-Content -LiteralPath $Source -Value $SourceText -Encoding utf8

$Args = @(
  "check",
  "--manifest-path", $Manifest,
  "--target", $TargetTriple,
  "--target-dir", $TargetDir,
  "--message-format=json-render-diagnostics"
)
if ($Release) {
  $Args += "--release"
}
$Args += @("-Z", "build-std=std,panic_abort,proc_macro")

Write-Host "Prebuilding real build-std sysroot for $TargetTriple..."
$BuildMessages = & $NightlyCargo @Args
if ($LASTEXITCODE -ne 0) {
  throw "build-std sysroot prebuild failed for $TargetTriple"
}

$Profile = if ($Release) { "release" } else { "debug" }
$ProfileDir = [System.IO.Path]::GetFullPath((Join-Path $TargetDir "$TargetTriple\$Profile"))
$ProfilePrefix = $ProfileDir.TrimEnd([char[]]"\/") + [System.IO.Path]::DirectorySeparatorChar

# The target cache also contains application dependencies and older sysroots.
# Use only the artifacts reported by this probe's Cargo graph, including fresh
# cached artifacts, so a prior build cannot supply an unrelated core/std pair.
$BuildStdCrates = @(
  "addr2line",
  "adler2",
  "alloc",
  "cfg_if",
  "compiler_builtins",
  "core",
  "gimli",
  "hashbrown",
  "libc",
  "memchr",
  "miniz_oxide",
  "object",
  "panic_abort",
  "panic_unwind",
  "proc_macro",
  "rustc_demangle",
  "rustc_std_workspace_alloc",
  "rustc_std_workspace_core",
  "std",
  "std_detect",
  "unwind",
  "windows_link"
)
$ReportedArtifacts = @{}
# Cargo reports explicit crate types in target.kind; std uses ["dylib", "rlib"].
$LibraryKinds = @("lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro")
foreach ($Line in $BuildMessages) {
  try { $Item = $Line | ConvertFrom-Json -ErrorAction Stop } catch { continue }
  $IsLibrary = @($Item.target.kind | Where-Object { $LibraryKinds -contains $_ }).Count -gt 0
  if ($Item.reason -ne "compiler-artifact" -or
      $BuildStdCrates -notcontains $Item.target.name -or -not $IsLibrary) {
    continue
  }
  $Candidates = @(
    foreach ($Filename in $Item.filenames) {
      if (@(".rlib", ".rmeta") -notcontains [System.IO.Path]::GetExtension([string]$Filename)) {
        continue
      }
      $ArtifactPath = [System.IO.Path]::GetFullPath([string]$Filename)
      # Host build dependencies are outside the requested target's profile.
      if (-not $ArtifactPath.StartsWith($ProfilePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        continue
      }
      if (-not (Test-Path -LiteralPath $ArtifactPath -PathType Leaf)) {
        throw "Cargo reported a missing build-std artifact: $ArtifactPath"
      }
      $ArtifactPath
    }
  )
  if ($Candidates.Count -eq 0) { continue }
  $Artifact = $Candidates |
    Where-Object { [System.IO.Path]::GetExtension($_) -eq ".rlib" } |
    Select-Object -First 1
  if ($null -eq $Artifact) { $Artifact = $Candidates | Select-Object -First 1 }
  $CrateName = $Item.target.name
  if ($ReportedArtifacts.ContainsKey($CrateName) -and $ReportedArtifacts[$CrateName] -ne $Artifact) {
    throw "Cargo reported multiple build-std artifacts for $CrateName"
  }
  $ReportedArtifacts[$CrateName] = $Artifact
}
foreach ($RequiredCrate in @("compiler_builtins", "core", "std")) {
  if (-not $ReportedArtifacts.ContainsKey($RequiredCrate)) {
    throw "real build-std sysroot prebuild produced no $RequiredCrate artifact below $ProfileDir"
  }
}
$ManifestLines = @(
  foreach ($CrateName in $BuildStdCrates) {
    if ($ReportedArtifacts.ContainsKey($CrateName)) {
      "$CrateName`t$($ReportedArtifacts[$CrateName])"
    }
  }
)
$ManifestPath = Join-Path $ProfileDir "agena-build-std-artifacts.txt"
# The Rust wrapper reads the first crate name verbatim; Windows PowerShell's
# Set-Content -Encoding utf8 would prepend a BOM to that name.
[System.IO.File]::WriteAllLines($ManifestPath, [string[]]$ManifestLines, [System.Text.UTF8Encoding]::new($false))
Write-Host "Real target build-std artifact manifest ready: $ManifestPath"
Write-Host "Real target std artifact ready: $($ReportedArtifacts['std'])"
