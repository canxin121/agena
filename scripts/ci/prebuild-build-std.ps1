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
Write-Host "Real target std artifact ready: $($StdArtifacts[0].FullName)"
