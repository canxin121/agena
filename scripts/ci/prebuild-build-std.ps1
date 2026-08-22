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
$ProbeRoot = Join-Path $env:RUNNER_TEMP "agena-build-std-probe\$TargetTriple"
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
  "--target-dir", $TargetDir
)
if ($Release) {
  $Args += "--release"
}
$Args += @("-Z", "build-std=std,panic_abort,proc_macro")

Write-Host "Prebuilding real build-std sysroot for $TargetTriple..."
& $NightlyCargo @Args
if ($LASTEXITCODE -ne 0) {
  throw "build-std sysroot prebuild failed for $TargetTriple"
}

$Profile = if ($Release) { "release" } else { "debug" }
$ProfileDir = Join-Path $TargetDir "$TargetTriple\$Profile"
$StdArtifacts = @(
  Get-ChildItem -LiteralPath $ProfileDir -Recurse -File -ErrorAction SilentlyContinue |
    Where-Object {
      $_.Name -match "^(lib)?std-[^/]+\.(rlib|rmeta)$"
    }
)
if ($StdArtifacts.Count -eq 0) {
  throw "real build-std sysroot prebuild produced no std artifact below $ProfileDir"
}

# Direct build-script probes need the same private standard-library dependency
# graph that Cargo passes to ordinary target crates.  Capture it before the
# Agena dependency graph is built, while this target directory contains only
# the real build-std probe and its target sysroot.  The wrapper consumes this
# manifest instead of guessing between same-named application dependencies
# (notably multiple hashbrown versions) later in the build.
$BuildStdCrates = @(
  "addr2line",
  "alloc",
  "cfg_if",
  "compiler_builtins",
  "core",
  "hashbrown",
  "libc",
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
$ManifestLines = @(
  foreach ($CrateName in $BuildStdCrates) {
    $Candidates = @(
      Get-ChildItem -LiteralPath $ProfileDir -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object {
          $_.Name -match "^(lib)?$([regex]::Escape($CrateName))-[^/]+\.(rlib|rmeta)$"
        }
    )
    if ($Candidates.Count -eq 0) {
      continue
    }
    $Artifact = $Candidates |
      Where-Object { $_.Extension -eq ".rlib" } |
      Select-Object -First 1
    if ($null -eq $Artifact) {
      $Artifact = $Candidates | Select-Object -First 1
    }
    "$CrateName`t$($Artifact.FullName)"
  }
)
$ManifestPath = Join-Path $ProfileDir "agena-build-std-artifacts.txt"
Set-Content -LiteralPath $ManifestPath -Value $ManifestLines -Encoding utf8
foreach ($RequiredCrate in @("compiler_builtins", "core", "std")) {
  if (-not ($ManifestLines | Where-Object { $_ -like "$RequiredCrate`t*" })) {
    throw "real build-std sysroot prebuild produced no $RequiredCrate artifact below $ProfileDir"
  }
}
Write-Host "Real target build-std artifact manifest ready: $ManifestPath"
Write-Host "Real target std artifact ready: $($StdArtifacts[0].FullName)"
