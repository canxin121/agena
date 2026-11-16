"""Offline contracts for the PowerShell build-std artifact manifest."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[3]
POWERSHELL = shutil.which("pwsh") or shutil.which("powershell")
TARGET = "x86_64-pc-windows-msvc"
REQUIRED = ("compiler_builtins", "core", "std")

HARNESS = r"""
$ErrorActionPreference = "Stop"
function Invoke-TestCargo {
  $call = @{ args = @($args); temp = [System.IO.Path]::GetTempPath() }
  [System.IO.File]::WriteAllText($env:TEST_CALL, ($call | ConvertTo-Json -Depth 10))
  $global:LASTEXITCODE = 0
  Get-Content -LiteralPath $env:TEST_MESSAGES
  if ($env:TEST_CARGO_FAIL) { $global:LASTEXITCODE = 42 }
}
& $env:TEST_SCRIPT -TargetTriple $env:TEST_TARGET -NightlyCargo "Invoke-TestCargo" -TargetDir $env:TEST_TARGET_DIR -Release
"""


@unittest.skipUnless(POWERSHELL, "PowerShell is unavailable")
class BuildStdPowerShellTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="agena-build-std-powershell-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.script = self.root / "prebuild-build-std.ps1"
        shutil.copy2(REPO / "scripts/ci/prebuild-build-std.ps1", self.script)
        self.target_dir = self.root / "shared target"
        self.profile = self.target_dir / TARGET / "release"
        self.manifest = self.profile / "agena-build-std-artifacts.txt"
        self.messages = self.root / "cargo messages.jsonl"
        self.call = self.root / "cargo call.json"
        self.harness = self.root / "harness.ps1"
        self.harness.write_text(HARNESS)
        self.runner_temp = self.root / "runner temp"
        self.system_temp = self.root / "system temp"
        self.system_temp.mkdir()
        self.env = dict(os.environ)
        self.env.update(
            RUNNER_TEMP=str(self.runner_temp),
            TMPDIR=str(self.system_temp),
            TMP=str(self.system_temp),
            TEMP=str(self.system_temp),
            TEST_SCRIPT=str(self.script),
            TEST_TARGET=TARGET,
            TEST_TARGET_DIR=str(self.target_dir),
            TEST_MESSAGES=str(self.messages),
            TEST_CALL=str(self.call),
        )
        self.env.pop("TEST_CARGO_FAIL", None)

    def artifact(self, crate, *, tag="current", fresh=True, extensions=(".rmeta",),
                 directory=None, kind=None, exists=True):
        # Rust 1.97 std declares crate-type = ["dylib", "rlib"]; Cargo reports
        # those types directly in target.kind, with no generic "lib" element.
        if kind is None:
            kinds = ["dylib", "rlib"] if crate == "std" else ["lib"]
        else:
            kinds = [kind] if isinstance(kind, str) else list(kind)
        directory = directory or self.profile / "deps"
        filenames = []
        for extension in extensions:
            path = directory / f"lib{crate}-{tag}{extension}"
            if exists:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(f"{crate} {tag}")
            filenames.append(str(path))
        return {
            "reason": "compiler-artifact",
            "target": {"name": crate, "kind": kinds},
            "filenames": filenames,
            "fresh": fresh,
        }

    def required_artifacts(self):
        return [self.artifact(crate) for crate in REQUIRED]

    def run_prebuild(self, messages, *, success=True, runner_temp=True, cargo_fail=False):
        self.messages.write_text(
            "\n".join(json.dumps(message) if isinstance(message, dict) else message
                      for message in messages) + "\n"
        )
        environment = dict(self.env)
        if not runner_temp:
            environment.pop("RUNNER_TEMP", None)
        if cargo_fail:
            environment["TEST_CARGO_FAIL"] = "1"
        result = subprocess.run(
            [POWERSHELL, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
             "-File", str(self.harness)],
            cwd=self.root, env=environment, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, timeout=30,
        )
        self.assertEqual(result.returncode == 0, success, result.stdout)
        return result

    def read_manifest(self):
        data = self.manifest.read_bytes()
        self.assertFalse(data.startswith(b"\xef\xbb\xbf"), "Rust crate names must not have a BOM")
        return dict(line.split("\t", 1) for line in data.decode("utf-8").splitlines())

    def test_uses_current_graph_including_fresh_artifacts(self):
        # Old .rlib files must not outrank this invocation's cached .rmeta files.
        for crate in REQUIRED:
            self.artifact(crate, tag="000-stale", extensions=(".rlib",))
        messages = self.required_artifacts()
        messages[1]["fresh"] = False
        expected = {
            item["target"]["name"]: item["filenames"][0] for item in messages
        }
        alloc = self.artifact("alloc", extensions=(".rmeta", ".rlib"), kind="rlib")
        expected["alloc"] = alloc["filenames"][1]
        messages += [
            alloc,
            self.artifact("core", tag="host", directory=self.target_dir / "release/deps"),
            self.artifact("core", tag="neighbor",
                          directory=self.profile.with_name("release-neighbor") / "deps"),
            self.artifact("core", tag="build-script", kind="custom-build"),
            self.artifact("core", tag="binary", kind="bin"),
            messages[0],
            "non-JSON diagnostic",
            {"reason": "build-finished", "success": True},
        ]
        self.run_prebuild(messages)
        self.assertEqual(self.read_manifest(), expected)
        call = json.loads(self.call.read_text())
        args = call["args"]
        self.assertEqual(args[0], "check")
        self.assertIn("--message-format=json-render-diagnostics", args)
        self.assertEqual(args[args.index("--target") + 1], TARGET)
        self.assertEqual(Path(args[args.index("--target-dir") + 1]), self.target_dir)
        self.assertIn("--release", args)
        self.assertIn("build-std=std,panic_abort,proc_macro", args)

    def test_rejects_missing_required_artifact_despite_stale_file(self):
        for crate in REQUIRED:
            self.artifact(crate, tag="stale", extensions=(".rlib",))
        messages = [self.artifact(crate) for crate in REQUIRED if crate != "std"]
        result = self.run_prebuild(messages, success=False)
        self.assertIn("produced no std artifact", result.stdout)
        self.assertFalse(self.manifest.exists())

    def test_rejects_missing_reported_file(self):
        messages = [
            self.artifact(crate, exists=crate != "std") for crate in REQUIRED
        ]
        result = self.run_prebuild(messages, success=False)
        self.assertIn("Cargo reported a missing build-std artifact", result.stdout)
        self.assertFalse(self.manifest.exists())

    def test_rejects_ambiguous_current_standard_library(self):
        messages = self.required_artifacts() + [self.artifact("core", tag="other")]
        result = self.run_prebuild(messages, success=False)
        self.assertIn("multiple build-std artifacts for core", result.stdout)
        self.assertFalse(self.manifest.exists())

    def test_cargo_failure_does_not_publish_artifacts(self):
        result = self.run_prebuild(self.required_artifacts(), success=False, cargo_fail=True)
        self.assertIn("build-std sysroot prebuild failed", result.stdout)
        self.assertFalse(self.manifest.exists())

    def test_falls_back_to_system_temp_without_runner_temp(self):
        self.run_prebuild(self.required_artifacts(), runner_temp=False)
        call = json.loads(self.call.read_text())
        args = call["args"]
        probe_manifest = Path(args[args.index("--manifest-path") + 1])
        expected = Path(call["temp"]) / "agena-build-std-probe" / TARGET / "Cargo.toml"
        self.assertEqual(probe_manifest, expected)
        self.assertTrue(probe_manifest.is_file())
        self.assertEqual(set(self.read_manifest()), set(REQUIRED))


if __name__ == "__main__":
    unittest.main()
