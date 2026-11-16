"""Offline PowerShell packaging contracts; cargo/rustc are in-process fakes."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
import zipfile

SCRIPTS = Path(__file__).resolve().parents[1]
POWERSHELL = shutil.which("pwsh") or shutil.which("powershell")
HOST = "x86_64-pc-windows-msvc"
CROSS = "aarch64-pc-windows-msvc"

HARNESS = r"""
$ErrorActionPreference = "Stop"
$EnvironmentNames = @("RUSTUP_TOOLCHAIN", "RUSTC", "RUSTDOC", "RUSTC_BOOTSTRAP",
  "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_TARGET_DIR",
  "CARGO_UNSTABLE_BUILD_DIR_NEW_LAYOUT", "AGENA_REAL_RUSTC", "AGENA_BUILD_STD_ROOT",
  "PATH", "CFLAGS_x86_64_pc_windows_msvc", "TEST_SETUP_TARGET_FLAG")
function Get-TestState {
  $state = @{ cwd = (Get-Location).Path; environment = @{} }
  foreach ($name in $EnvironmentNames) {
    $state.environment[$name] = [Environment]::GetEnvironmentVariable($name)
  }
  return $state
}
function rustc {
  $global:LASTEXITCODE = 0
  return $env:TEST_HOST
}
function rustup {
  $global:LASTEXITCODE = 0
  return (Join-Path $env:TEST_CHECKOUT "fake-rustup-tool")
}
function cargo {
  $CargoArgs = @($args)
  if ($CargoArgs[0] -like "+*") { $CargoArgs = $CargoArgs[1..($CargoArgs.Count - 1)] }
  $targetDir = $env:TEST_TARGET_DIR
  if ($env:CARGO_TARGET_DIR) { $targetDir = $env:CARGO_TARGET_DIR }
  $call = @{
    args = $CargoArgs
    cwd = (Get-Location).Path
    rustflags = [Environment]::GetEnvironmentVariable("RUSTFLAGS")
    encoded = [Environment]::GetEnvironmentVariable("CARGO_ENCODED_RUSTFLAGS")
    toolchain = $env:RUSTUP_TOOLCHAIN
    setup_cflags = $env:CFLAGS_x86_64_pc_windows_msvc
    setup_flag = $env:TEST_SETUP_TARGET_FLAG
  }
  [IO.File]::AppendAllText($env:TEST_CARGO_CALLS, ($call | ConvertTo-Json -Depth 10 -Compress) + [Environment]::NewLine)
  $global:LASTEXITCODE = 0
  if ($CargoArgs[0] -eq "metadata") {
    return (@{packages = @(@{name = "agena"; version = "0.1.0"}); target_directory = $targetDir} | ConvertTo-Json -Depth 10 -Compress)
  }
  if ($CargoArgs[0] -ne "build") { throw "Unexpected Cargo call: $CargoArgs" }
  if ($env:TEST_BUILD_FAIL) { $global:LASTEXITCODE = 42; return }
  $targetIndex = [Array]::IndexOf($CargoArgs, "--target")
  $triple = if ($targetIndex -ge 0) { $CargoArgs[$targetIndex + 1] } else { $env:TEST_CONFIG_TARGET }
  if ($env:TEST_ARTIFACT_TARGET) { $triple = $env:TEST_ARTIFACT_TARGET }
  if ($env:TEST_ARTIFACT_NATIVE) { $triple = "" }
  $directoryIndex = [Array]::IndexOf($CargoArgs, "--target-dir")
  if ($directoryIndex -ge 0) { $targetDir = $CargoArgs[$directoryIndex + 1] }
  $directory = if ($triple) { Join-Path $targetDir $triple } else { $targetDir }
  $directory = Join-Path $directory "release"
  New-Item -ItemType Directory -Force -Path $directory | Out-Null
  $artifactTarget = if ($triple) { $triple } else { $env:TEST_HOST }
  $binaryName = if ($artifactTarget -match "windows") { "agena.exe" } else { "agena" }
  $binary = Join-Path $directory $binaryName
  [IO.File]::WriteAllText($binary, "fixture binary")
  if (-not $env:TEST_NO_ARTIFACT) {
    return (@{reason = "compiler-artifact"; target = @{name = "agena"; kind = @("bin")}; executable = $binary; fresh = $true} | ConvertTo-Json -Depth 10 -Compress)
  }
}
$before = Get-TestState
try {
  $PackageArguments = @{}
  if ($env:TEST_TARGET) { $PackageArguments.TargetTriple = $env:TEST_TARGET }
  if ($env:TEST_TARGET_FLAGS) { $PackageArguments.TargetRustFlags = $env:TEST_TARGET_FLAGS }
  if ($env:TEST_BUILD_STD) { $PackageArguments.BuildStd = $true }
  & $env:TEST_PACKAGE_SCRIPT @PackageArguments
}
finally {
  [IO.File]::WriteAllText($env:TEST_RESTORED_STATE, (@{before = $before; after = (Get-TestState)} | ConvertTo-Json -Depth 10))
}
"""


@unittest.skipUnless(POWERSHELL, "PowerShell is unavailable")
class PowerShellPackageBuildTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="agena-powershell-package-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.checkout = self.root / "checkout with spaces"
        self.scripts = self.checkout / "scripts/agena"
        self.scripts.mkdir(parents=True)
        shutil.copy2(SCRIPTS / "package-backend.ps1", self.scripts)
        ci = self.checkout / "scripts/ci"
        ci.mkdir()
        (ci / "setup-windows-toolchain.ps1").write_text(
            'Param([string]$TargetTriple)\n'
            '$env:PATH = $env:TEST_SETUP_PATH + [IO.Path]::PathSeparator + $env:PATH\n'
            '$env:CFLAGS_x86_64_pc_windows_msvc = "-DSETUP " + $env:CFLAGS_x86_64_pc_windows_msvc\n'
            '$env:TEST_SETUP_TARGET_FLAG = "fixture target flag"\n'
            'if ($env:TEST_SETUP_FAIL) { throw "fixture setup failed" }\n'
            '$global:LASTEXITCODE = 0\n'
        )
        (self.checkout / "Cargo.toml").write_text("[workspace]\nmembers=[]\n")
        self.web = self.checkout / "packages/agena-web/dist"
        self.web.mkdir(parents=True)
        (self.web / "index.html").write_text("fixture web")
        self.target = self.root / "workspace configured target"
        self.calls = self.root / "cargo calls.jsonl"
        self.restored = self.root / "restored state.json"
        self.harness = self.root / "harness.ps1"
        self.harness.write_text(HARNESS)
        self.env = dict(os.environ)
        for name in (
            "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET",
            "CARGO_TARGET_DIR", "AGENA_FORCE_EXPLICIT_TARGET", "AGENA_STABLE_TOOLCHAIN",
            "TEST_SETUP_TARGET_FLAG",
        ):
            self.env.pop(name, None)
        self.env.update(
            AGENA_WEB_DIST_PREBUILT="1",
            TEST_CHECKOUT=str(self.checkout),
            TEST_HOST=HOST,
            TEST_TARGET_DIR=str(self.target),
            TEST_CARGO_CALLS=str(self.calls),
            TEST_RESTORED_STATE=str(self.restored),
            TEST_PACKAGE_SCRIPT=str(self.scripts / "package-backend.ps1"),
            TEST_SETUP_PATH=str(self.root / "fixture toolchain bin"),
        )

    def package(self, target=None, *, env=None, success=True):
        environment = self.env | (env or {})
        if target:
            environment["TEST_TARGET"] = target
        result = subprocess.run(
            [POWERSHELL, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
             "-File", str(self.harness)],
            cwd=self.root, env=environment, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, timeout=30,
        )
        self.assertEqual(result.returncode == 0, success, result.stdout)
        state = json.loads(self.restored.read_text(encoding="utf-8-sig"))
        self.assertEqual(state["after"], state["before"], result.stdout)
        return result

    def build_calls(self):
        return [
            call for call in (
                json.loads(line)
                for line in self.calls.read_text(encoding="utf-8-sig").splitlines()
            )
            if call["args"][0] == "build"
        ]

    def archive(self, target=HOST):
        return self.checkout / f"artifacts/agena/agena-backend-{target}-v0.1.0.zip"

    def test_native_uses_workspace_cache_and_reported_binary(self):
        stale = self.target / HOST / "release/agena.exe"
        stale.parent.mkdir(parents=True)
        stale.write_text("stale triple binary")
        self.package()
        self.package(HOST)
        for call in self.build_calls():
            self.assertNotIn("--target", call["args"])
            self.assertEqual(Path(call["cwd"]), self.checkout)
            self.assertIsNone(call["rustflags"])
            self.assertEqual(
                Path(call["args"][call["args"].index("--target-dir") + 1]), self.target
            )
            self.assertIn("--message-format=json-render-diagnostics", call["args"])
            self.assertIn("--bin", call["args"])
        with zipfile.ZipFile(self.archive()) as archive:
            name = next(name for name in archive.namelist() if name.replace("\\", "/") == "bin/agena.exe")
            self.assertEqual(archive.read(name), b"fixture binary")
        self.assertFalse((self.checkout / ".agena-target").exists())

    def test_caller_target_directory_and_flags_are_preserved(self):
        target = self.root / "caller target"
        self.package(HOST, env={
            "CARGO_TARGET_DIR": str(target), "RUSTFLAGS": "--cfg caller_flag",
            "RUSTUP_TOOLCHAIN": "caller-toolchain",
            "CFLAGS_x86_64_pc_windows_msvc": "-DORIGINAL",
        })
        call = self.build_calls()[-1]
        self.assertEqual(Path(call["args"][call["args"].index("--target-dir") + 1]), target)
        self.assertEqual(call["rustflags"], "--cfg caller_flag")
        self.assertEqual(call["toolchain"], "1.97.0")
        self.assertEqual(call["setup_cflags"], "-DSETUP -DORIGINAL")
        self.assertEqual(call["setup_flag"], "fixture target flag")
        self.assertNotIn("--target", call["args"])

    def test_cross_builds_and_target_flags_keep_explicit_target(self):
        self.package(CROSS)
        call = self.build_calls()[-1]
        self.assertEqual(call["args"][call["args"].index("--target") + 1], CROSS)
        self.package(HOST, env={
            "TEST_TARGET_FLAGS": "-C target-cpu=generic", "RUSTFLAGS": "--cfg original",
        })
        call = self.build_calls()[-1]
        self.assertIn("--target", call["args"])
        self.assertEqual(call["rustflags"], "--cfg original -C target-cpu=generic")

    def test_encoded_flags_preserve_argument_boundaries_and_are_extended(self):
        encoded = "--cfg\x1fname=\"with spaces\""
        self.package(HOST, env={"CARGO_ENCODED_RUSTFLAGS": encoded})
        call = self.build_calls()[-1]
        self.assertEqual(call["encoded"], encoded)
        self.assertIsNone(call["rustflags"])
        self.package(HOST, env={
            "CARGO_ENCODED_RUSTFLAGS": encoded,
            "TEST_TARGET_FLAGS": "-C target-cpu=generic",
        })
        call = self.build_calls()[-1]
        self.assertEqual(call["encoded"], encoded + "\x1f-C\x1ftarget-cpu=generic")
        self.assertIsNone(call["rustflags"])
        self.package(HOST, env={
            "CARGO_ENCODED_RUSTFLAGS": "", "RUSTFLAGS": "",
            "TEST_TARGET_FLAGS": "-C target-cpu=generic",
        })
        call = self.build_calls()[-1]
        self.assertEqual(call["encoded"], "-C\x1ftarget-cpu=generic")
        self.assertEqual(call["rustflags"], "")

    def test_configured_native_target_artifact_is_accepted(self):
        self.package(HOST, env={"TEST_CONFIG_TARGET": HOST})
        self.assertNotIn("--target", self.build_calls()[-1]["args"])
        self.assertTrue(self.archive().is_file())

    def test_explicit_target_environment_is_respected(self):
        for name, value in (("AGENA_FORCE_EXPLICIT_TARGET", "1"), ("CARGO_BUILD_TARGET", HOST)):
            with self.subTest(name=name):
                self.package(HOST, env={name: value})
                self.assertIn("--target", self.build_calls()[-1]["args"])

    def test_failures_and_wrong_artifacts_never_package_a_stale_binary(self):
        stale = self.target / "release/agena.exe"
        stale.parent.mkdir(parents=True)
        stale.write_text("stale binary")
        for environment in (
            {"TEST_SETUP_FAIL": "1"},
            {"TEST_BUILD_FAIL": "1"},
            {"TEST_NO_ARTIFACT": "1"},
            {"TEST_ARTIFACT_TARGET": CROSS},
        ):
            with self.subTest(environment=environment):
                self.package(HOST, env=environment, success=False)
                self.assertFalse(self.archive().exists())
        self.package(CROSS, env={"TEST_ARTIFACT_NATIVE": "1"}, success=False)
        self.assertFalse(self.archive(CROSS).exists())

    @unittest.skipUnless(shutil.which("tar"), "tar is unavailable")
    def test_build_std_keeps_explicit_target_and_restores_environment(self):
        host = "x86_64-unknown-linux-gnu"
        self.package(host, env={"TEST_HOST": host, "TEST_BUILD_STD": "1"})
        call = self.build_calls()[-1]
        self.assertIn("--target", call["args"])
        self.assertIn("build-std=std,panic_abort,proc_macro", call["args"])


if __name__ == "__main__":
    unittest.main()
