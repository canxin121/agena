"""Execute the PowerShell installer's state and argument contracts when available."""

import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
POWERSHELL = shutil.which("pwsh") or shutil.which("powershell")
LOAD_STATE_HARNESS = r'''
Param(
  [string]$Installer,
  [string]$StatePath,
  [string]$SelectedInstallDir
)
$ErrorActionPreference = "Stop"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
  $Installer, [ref]$tokens, [ref]$parseErrors
)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
$names = @("Resolve-Directory", "Assert-StoragePaths", "Load-State")
$helpers = $ast.FindAll({
  param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
    $names -contains $node.Name
}, $true)
if ($helpers.Count -ne $names.Count) { throw "installer state helpers were not found" }
foreach ($helper in $helpers) {
  # Execute the actual installer functions without running its lifecycle dispatch.
  Invoke-Expression $helper.Extent.Text
}
$script:InstallDir = $SelectedInstallDir
$script:StateFile = $StatePath
$script:ServerPassword = "ambient-fixture-value"
try {
  Load-State
  [ordered]@{
    serverPassword = $script:ServerPassword
    installDir = $script:InstallDir
    dataDir = $script:DataDir
  } | ConvertTo-Json -Compress
} catch {
  [Console]::Error.WriteLine($_.Exception.Message)
  exit 1
}
'''


@unittest.skipUnless(POWERSHELL, "PowerShell runtime is not installed")
class PowerShellInstallerStateTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="agena-powershell-installer-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.install_dir = self.root / "application files"
        self.install_dir.mkdir()
        self.data_dir = self.root / "selected instance data"
        self.data_dir.mkdir()
        self.database = self.data_dir / "agena.db"
        self.database.write_bytes(b"preserved fixture database")
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()
        self.state_path = self.install_dir / "install-state.json"
        self.harness = self.root / "load-state.ps1"
        self.harness.write_text(LOAD_STATE_HARNESS, encoding="utf-8")
        self.state = {
            "schema": 2,
            "repo": "fixture/agena",
            "version": "0.1.0",
            "releaseVersion": "0.1.0",
            "installDir": str(self.install_dir),
            "dataDir": str(self.data_dir),
            "host": "127.0.0.1",
            "port": 3210,
            "workspace": str(self.workspace),
            "serverPassword": "",
            "serviceMode": "detached",
        }

    def run_powershell(self, script, *arguments):
        return subprocess.run(
            [
                POWERSHELL, "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
                "-File", str(script), *map(str, arguments),
            ],
            cwd=self.root, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, timeout=30,
        )

    def load_state(self):
        self.state_path.write_text(json.dumps(self.state), encoding="utf-8")
        before = self.state_path.read_bytes()
        result = self.run_powershell(
            self.harness, "-Installer", SCRIPTS / "install.ps1",
            "-StatePath", self.state_path, "-SelectedInstallDir", self.install_dir,
        )
        self.assertEqual(self.state_path.read_bytes(), before)
        self.assertEqual(self.database.read_bytes(), b"preserved fixture database")
        return result

    def test_missing_current_password_is_rejected_without_importing_retired_state(self):
        del self.state["serverPassword"]
        for retired in (False, True):
            with self.subTest(retired_password_present=retired):
                if retired:
                    self.state["uiPassword"] = "retired-private-fixture"
                result = self.load_state()
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("installer state is incompatible", result.stdout)
                self.assertNotIn("retired-private-fixture", result.stdout)
                self.assertNotIn("ambient-fixture-value", result.stdout)

    def test_explicit_empty_password_is_preserved(self):
        self.state["uiPassword"] = "retired-private-fixture"
        result = self.load_state()
        self.assertEqual(result.returncode, 0, result.stdout)
        loaded = json.loads(result.stdout)
        self.assertEqual(loaded["serverPassword"], "")
        self.assertEqual(Path(loaded["installDir"]), self.install_dir)
        self.assertEqual(Path(loaded["dataDir"]), self.data_dir)

    def test_current_password_is_preserved(self):
        self.state["serverPassword"] = "current-private-fixture"
        self.state["uiPassword"] = "retired-private-fixture"
        result = self.load_state()
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertEqual(json.loads(result.stdout)["serverPassword"], "current-private-fixture")

    def test_retired_password_parameter_is_rejected_before_installation(self):
        install_dir = self.root / "new application"
        data_dir = self.root / "new instance data"
        # A missing local archive also prevents network access if binding regresses.
        result = self.run_powershell(
            SCRIPTS / "install.ps1", "-Action", "Install",
            "-InstallDir", install_dir, "-DataDir", data_dir,
            "-Workspace", self.workspace, "-Archive", self.root / "missing-fixture.zip",
            "-ServiceMode", "detached", "-NoPathUpdate",
            "-UiPassword", "retired-private-fixture",
        )
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("UiPassword", result.stdout)
        self.assertNotIn("retired-private-fixture", result.stdout)
        self.assertFalse(install_dir.exists())
        self.assertFalse(data_dir.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
