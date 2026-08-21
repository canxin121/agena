Param(
  [Parameter(Mandatory = $true)]
  [string]$TargetTriple,
  [bool]$BuildStd = $false,
  [string]$TargetRustFlags = ""
)

$ErrorActionPreference = "Stop"
$env:RUSTUP_TOOLCHAIN = if ($env:AGENA_STABLE_TOOLCHAIN) { $env:AGENA_STABLE_TOOLCHAIN } else { "1.97.0" }

if ($TargetTriple -match "windows") {
  & "$PSScriptRoot/setup-windows-toolchain.ps1" -TargetTriple $TargetTriple
}

$Args = @(
  "check",
  "--manifest-path", "Cargo.toml",
  "-p", "agena",
  "--target", $TargetTriple,
  "--target-dir", (Join-Path $env:RUNNER_TEMP "agena-check-target\$TargetTriple"),
  "--locked"
)

if ($BuildStd) {
  $StableRustc = (& rustup which --toolchain 1.97.0 rustc).Trim()
  $StableRustcExit = $LASTEXITCODE
  $StableRustdoc = (& rustup which --toolchain 1.97.0 rustdoc).Trim()
  $StableRustdocExit = $LASTEXITCODE
  $NightlyCargo = (& rustup which --toolchain nightly-2026-08-18 cargo).Trim()
  $NightlyCargoExit = $LASTEXITCODE
  if ($StableRustcExit -ne 0 -or -not $StableRustc) {
    throw "Failed to locate Rust 1.97.0 rustc"
  }
  if ($StableRustdocExit -ne 0 -or -not $StableRustdoc) {
    throw "Failed to locate Rust 1.97.0 rustdoc"
  }
  if ($NightlyCargoExit -ne 0 -or -not $NightlyCargo) {
    throw "Failed to locate nightly-2026-08-18 Cargo"
  }
  $OldRustc = $env:RUSTC
  $OldRustdoc = $env:RUSTDOC
  $OldBootstrap = $env:RUSTC_BOOTSTRAP
  $OldRustFlags = $env:RUSTFLAGS
  $OldTargetDir = $env:CARGO_TARGET_DIR
  $BuildTargetDir = Join-Path $env:RUNNER_TEMP "agena-check-target\$TargetTriple"
  try {
    $env:RUSTC = $StableRustc
    $env:RUSTDOC = $StableRustdoc
    $env:RUSTC_BOOTSTRAP = "1"
    $env:CARGO_TARGET_DIR = $BuildTargetDir
    $RustFlags = @($OldRustFlags, $TargetRustFlags) | Where-Object { $_ }
    if ($TargetTriple -match "-win7-windows-(msvc|gnu)$") {
      # Build-std places the target's real libcore/libstd artifacts under the
      # target-specific debug deps directory.  Cargo passes that path to
      # normal target rustc invocations, but direct rustc probes launched by
      # build scripts (notably autocfg) only see CARGO_ENCODED_RUSTFLAGS.  Add
      # the actual build-std search path so those probes use the same target
      # standard library instead of falling back to the host sysroot.
      $BuildStdDeps = Join-Path $BuildTargetDir "$TargetTriple\debug\deps"
      $RustFlags += @("-L", "dependency=$BuildStdDeps")

      # Build-std emits core/std into hashed build-script output directories,
      # not only into debug\deps.  Cargo knows those paths for ordinary
      # target rustc invocations, but build scripts such as autocfg invoke the
      # RUSTC executable directly and only inherit CARGO_ENCODED_RUSTFLAGS.
      # Compile a tiny native wrapper with the already-installed official
      # Rust toolchain. It adds the real core/std output directories at the
      # moment each target probe runs; it never supplies host or synthetic
      # standard-library artifacts.
      $RustcWrapperDir = Join-Path $env:RUNNER_TEMP "agena-rustc-build-std\$TargetTriple"
      New-Item -ItemType Directory -Force -Path $RustcWrapperDir | Out-Null
      $RustcWrapperSource = Join-Path $RustcWrapperDir "rustc-build-std-wrapper.rs"
      $RustcWrapper = Join-Path $RustcWrapperDir "rustc-build-std-wrapper.exe"
      $OldAgenaRealRustc = $env:AGENA_REAL_RUSTC
      $OldAgenaBuildStdRoot = $env:AGENA_BUILD_STD_ROOT
      $RustcWrapperText = @'
use std::env;
use std::path::PathBuf;
use std::process::Command;

fn target_argument(args: &[String]) -> Option<&str> {
    for (index, arg) in args.iter().enumerate() {
        if arg == "--target" {
            return args.get(index + 1).map(String::as_str);
        }
        if let Some(target) = arg.strip_prefix("--target=") {
            return Some(target);
        }
    }
    None
}

fn main() {
    let real_rustc = env::var_os("AGENA_REAL_RUSTC")
        .expect("AGENA_REAL_RUSTC is required by the build-std rustc wrapper");
    let build_root = PathBuf::from(
        env::var_os("AGENA_BUILD_STD_ROOT")
            .expect("AGENA_BUILD_STD_ROOT is required by the build-std rustc wrapper"),
    );
    let args: Vec<String> = env::args().skip(1).collect();
    let target = target_argument(&args);
    let mut command = Command::new(real_rustc);
    command.args(&args);

    let is_win7 = target.is_some_and(|value| {
        value.ends_with("-win7-windows-msvc") || value.ends_with("-win7-windows-gnu")
    });
    if is_win7 {
        for crate_name in ["core", "std"] {
            let crate_root = build_root.join(crate_name);
            if let Ok(entries) = std::fs::read_dir(crate_root) {
                for entry in entries.flatten() {
                    let output = entry.path().join("out");
                    if output.is_dir() {
                        command.arg("-L").arg(format!("dependency={}", output.display()));
                    }
                }
            }
        }
    }

    let status = command
        .status()
        .expect("failed to execute the real Rust compiler");
    std::process::exit(status.code().unwrap_or(1));
}
'@
      [IO.File]::WriteAllText($RustcWrapperSource, $RustcWrapperText, [Text.Encoding]::UTF8)
      & $StableRustc $RustcWrapperSource -o $RustcWrapper
      if ($LASTEXITCODE -ne 0 -or -not (Test-Path $RustcWrapper)) {
        throw "failed to compile the build-std rustc wrapper"
      }
      $env:AGENA_REAL_RUSTC = $StableRustc
      $env:AGENA_BUILD_STD_ROOT = Join-Path $BuildTargetDir "$TargetTriple\debug\build"
      $env:RUSTC = $RustcWrapper
    }
    $env:RUSTFLAGS = ($RustFlags | Join-String -Separator " ")
    Write-Host "Using build-std driver: cargo=$NightlyCargo rustc=$StableRustc rustdoc=$StableRustdoc target-dir=$BuildTargetDir"
    $CargoExtraArgs = @()
    if ($TargetTriple -match "-win7-windows-(msvc|gnu)$") {
      # Win7 custom target builds have historically failed only after the
      # dependency graph is built. Keep the target real, but expose the exact
      # rustc --extern/search-path command so a dependency-sysroot regression
      # cannot be diagnosed from E0463 names alone.
      $CargoExtraArgs += "-vv"
    }
    & $NightlyCargo @Args @CargoExtraArgs -Z "build-std=std,panic_abort,proc_macro"
  }
  finally {
    $env:RUSTC = $OldRustc
    $env:RUSTDOC = $OldRustdoc
    $env:RUSTC_BOOTSTRAP = $OldBootstrap
    $env:RUSTFLAGS = $OldRustFlags
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
}
else {
  $OldRustFlags = $env:RUSTFLAGS
  try {
    $env:RUSTFLAGS = (($OldRustFlags, $TargetRustFlags) | Where-Object { $_ } | Join-String -Separator " ")
    & cargo @Args
  }
  finally {
    $env:RUSTFLAGS = $OldRustFlags
  }
}

if ($LASTEXITCODE -ne 0) {
  throw "cargo check failed for $TargetTriple"
}
