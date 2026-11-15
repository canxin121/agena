"""Execute the PowerShell installer's state and argument contracts when available."""

import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
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




FUNCTION_HARNESS = r'''
Param([string]$Installer, [string]$Functions, [string]$Body)
$ErrorActionPreference = "Stop"
$tokens = $null
$parseErrors = $null
$ast = [System.Management.Automation.Language.Parser]::ParseFile(
  $Installer, [ref]$tokens, [ref]$parseErrors
)
if ($parseErrors.Count) { throw ($parseErrors | Out-String) }
$names = $Functions.Split(',')
$helpers = $ast.FindAll({
  param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst] -and
    $names -contains $node.Name
}, $true)
if ($helpers.Count -ne $names.Count) { throw "requested installer helpers were not found" }
foreach ($helper in $helpers) { Invoke-Expression $helper.Extent.Text }
Invoke-Expression (Get-Content -LiteralPath $Body -Raw)
'''


def powershell_literal(value):
    return "'" + str(value).replace("'", "''") + "'"


@unittest.skipUnless(POWERSHELL, "PowerShell runtime is not installed")
class PowerShellInstallerSetupTests(unittest.TestCase):
    setUp = PowerShellInstallerStateTests.setUp
    run_powershell = PowerShellInstallerStateTests.run_powershell

    def run_helpers(self, names, body, source=None):
        harness = self.root / "functions.ps1"
        harness.write_text(FUNCTION_HARNESS, encoding="utf-8")
        script = self.root / "scenario.ps1"
        prefix = "$fixture = " + powershell_literal(self.root) + "\n"
        script.write_text(prefix + body, encoding="utf-8")
        return self.run_powershell(
            harness, "-Installer", source or SCRIPTS / "install.ps1",
            "-Functions", ",".join(names), "-Body", script,
        )

    def result_json(self, result):
        self.assertEqual(result.returncode, 0, result.stdout)
        marker = next(
            (line[len("RESULT:"):] for line in reversed(result.stdout.splitlines())
             if line.startswith("RESULT:")), None,
        )
        self.assertIsNotNone(marker, result.stdout)
        return json.loads(marker)

    def source_versions(self):
        source = self.root / "source"
        source.mkdir(exist_ok=True)
        (source / "rust-toolchain.toml").write_text(
            '[toolchain]\nchannel = "1.97.0"\n', encoding="utf-8",
        )
        (source / ".bun-version").write_text("1.3.14\n", encoding="utf-8")
        return source

    def test_setup_mode_is_rejected_for_unrelated_actions(self):
        for action in ("Status", "Start", "Stop", "Restart", "Uninstall"):
            with self.subTest(action=action):
                result = self.run_powershell(
                    SCRIPTS / "install.ps1", "-Action", action,
                    "-InstallDir", self.install_dir, "-SetupMode", "check",
                )
                self.assertNotEqual(result.returncode, 0, result.stdout)
                self.assertIn("-SetupMode only applies", result.stdout)
        self.assertEqual(self.database.read_bytes(), b"preserved fixture database")

    def test_missing_bun_is_prepared_once_with_the_repository_version(self):
        result = self.run_helpers(
            ["Ensure-WindowsCommand", "Install-WindowsDependency", "Test-WindowsCommand"],
            r'''
$script:SetupMode = "auto"
$script:installed = $false
$script:requests = @()
function Get-DependencyCommand([string]$Name) {
  if ($Name -eq "winget" -or $script:installed) { return @{ Source = $Name } }
  return $null
}
function Refresh-DependencyPath {}
function Invoke-DependencyCommand([string]$Name, [string[]]$Arguments) {
  if ($Name -eq "winget") {
    $script:requests += ,$Arguments
    $script:installed = $true
    return @{ ExitCode = 0; Output = "fixture installed" }
  }
  return @{ ExitCode = 0; Output = "1.3.14" }
}
Ensure-WindowsCommand "bun" "Oven-sh.Bun" "1.3.14" "1.3.14"
Ensure-WindowsCommand "bun" "Oven-sh.Bun" "1.3.14" "1.3.14"
Write-Output ("RESULT:" + (@{ requests = @($script:requests) } | ConvertTo-Json -Depth 5 -Compress))
''',
        )
        requests = self.result_json(result)["requests"]
        self.assertEqual(len(requests), 1)
        self.assertEqual(requests[0][:5], ["install", "--exact", "--id", "Oven-sh.Bun", "--source"])
        self.assertEqual(requests[0][-2:], ["--version", "1.3.14"])

    def test_check_mode_does_not_invoke_a_package_manager_or_uac(self):
        result = self.run_helpers(
            ["Ensure-WindowsCommand", "Install-WindowsDependency", "Test-WindowsCommand",
             "Ensure-WindowsBuildTools"],
            r'''
$script:SetupMode = "check"
$script:errors = @()
function Get-DependencyCommand { return $null }
function Get-WindowsBuildToolsState { return @{ Ready = $false; InstallPath = "" } }
function Invoke-DependencyCommand { throw "PACKAGE MANAGER WAS CALLED" }
function Start-Process { throw "UAC WAS CALLED" }
foreach ($dependency in @(@("bun", "Oven-sh.Bun"), @("python", "Python.Python.3.13"), @("rustup", "Rustlang.Rustup"))) {
  try { Ensure-WindowsCommand $dependency[0] $dependency[1] } catch { $script:errors += $_.Exception.Message }
}
try { Ensure-WindowsBuildTools "aarch64-pc-windows-msvc" } catch { $script:errors += $_.Exception.Message }
Write-Output ("RESULT:" + (@{ errors = $script:errors } | ConvertTo-Json -Compress))
''',
        )
        errors = self.result_json(result)["errors"]
        self.assertEqual(len(errors), 4)
        self.assertTrue(all("check never" in error for error in errors), errors)

    def test_ready_source_dependencies_do_not_install_or_upgrade(self):
        source = self.source_versions()
        result = self.run_helpers(
            ["Get-SourceToolVersions", "Ensure-WindowsSourceDependencies"],
            "$source = " + powershell_literal(source) + "\n" + r'''
$script:SetupMode = "auto"
$env:AGENA_WEB_DIST_PREBUILT = $null
$script:commands = @()
$script:requirements = @()
function Refresh-DependencyPath {}
function Ensure-WindowsBuildTools([string]$Target) {}
function Ensure-WindowsCommand([string]$Name, [string]$Package, [string]$MinimumVersion = "", [string]$PackageVersion = "") {
  $script:requirements += @{ name = $Name; package = $Package; minimum = $MinimumVersion; version = $PackageVersion }
}
function Invoke-DependencyCommand([string]$Name, [string[]]$Arguments) {
  $script:commands += ($Name + " " + ($Arguments -join " "))
  switch ($Arguments[0]) {
    "toolchain" { return @{ ExitCode = 0; Output = "1.97.0-x86_64-pc-windows-msvc (default)" } }
    "component" { return @{ ExitCode = 0; Output = ("rustfmt-x86_64-pc-windows-msvc" + [Environment]::NewLine + "clippy-x86_64-pc-windows-msvc") } }
    "target" { return @{ ExitCode = 0; Output = "x86_64-pc-windows-msvc" } }
  }
  throw "unexpected dependency mutation"
}
Ensure-WindowsSourceDependencies $source "x86_64-pc-windows-msvc"
Write-Output ("RESULT:" + (@{ commands = $script:commands; requirements = $script:requirements } | ConvertTo-Json -Depth 5 -Compress))
''',
        )
        data = self.result_json(result)
        self.assertEqual(len(data["commands"]), 3)
        self.assertTrue(all(" list " in command for command in data["commands"]))
        bun = next(item for item in data["requirements"] if item["name"] == "bun")
        self.assertEqual((bun["minimum"], bun["version"]), ("1.3.14", "1.3.14"))

    def test_missing_rust_toolchain_in_check_mode_stops_before_install(self):
        source = self.source_versions()
        result = self.run_helpers(
            ["Get-SourceToolVersions", "Ensure-WindowsSourceDependencies"],
            "$source = " + powershell_literal(source) + "\n" + r'''
$script:SetupMode = "check"
$script:commands = @()
function Refresh-DependencyPath {}
function Ensure-WindowsBuildTools {}
function Ensure-WindowsCommand {}
function Invoke-DependencyCommand([string]$Name, [string[]]$Arguments) {
  $script:commands += ($Arguments -join " ")
  return @{ ExitCode = 0; Output = "1.90.0-x86_64-pc-windows-msvc" }
}
$errorText = ""
try { Ensure-WindowsSourceDependencies $source "x86_64-pc-windows-msvc" } catch { $errorText = $_.Exception.Message }
Write-Output ("RESULT:" + (@{ error = $errorText; commands = $script:commands } | ConvertTo-Json -Compress))
''',
        )
        data = self.result_json(result)
        self.assertIn("Rust 1.97.0 is missing", data["error"])
        self.assertEqual(data["commands"], ["toolchain list --quiet"])

    def test_source_build_restores_temporary_environment_when_package_fails(self):
        source = self.source_versions()
        package = source / "scripts" / "agena" / "package-backend.ps1"
        package.parent.mkdir(parents=True)
        toolchain = source / "scripts" / "ci" / "setup-windows-toolchain.ps1"
        toolchain.parent.mkdir()
        shutil.copyfile(SCRIPTS.parent / "ci" / "setup-windows-toolchain.ps1", toolchain)
        observed = self.root / "observed-build.json"
        package.write_text(
            'Param([string]$TargetTriple)\n'
            '$value = @{ mode=$env:AGENA_INSTALLER_SETUP_MODE; toolchain=$env:RUSTUP_TOOLCHAIN; target=$TargetTriple }\n'
            '$value | ConvertTo-Json | Set-Content -LiteralPath ' + powershell_literal(observed) + '\n'
            'throw "fixture build failure"\n', encoding="utf-8",
        )
        cargo = self.root / "fake-cargo.exe"
        cargo.write_bytes(b"not executed")
        result = self.run_helpers(
            ["Get-SourceToolVersions", "Invoke-SourcePackage"],
            "$source = " + powershell_literal(source) + "\n$cargo = " + powershell_literal(cargo) + "\n" + r'''
$script:SetupMode = "check"
$env:AGENA_INSTALLER_SETUP_MODE = "caller-mode"
$env:RUSTUP_TOOLCHAIN = "caller-toolchain"
$previousPath = $env:PATH
function Invoke-DependencyCommand { return @{ ExitCode = 0; Output = $cargo } }
$errorText = ""
try { Invoke-SourcePackage $source "aarch64-pc-windows-msvc" } catch { $errorText = $_.Exception.Message }
Write-Output ("RESULT:" + (@{
  error = $errorText; mode = $env:AGENA_INSTALLER_SETUP_MODE
  toolchain = $env:RUSTUP_TOOLCHAIN; path_restored = ($previousPath -eq $env:PATH)
} | ConvertTo-Json -Compress))
''',
        )
        data = self.result_json(result)
        self.assertIn("fixture build failure", data["error"])
        self.assertEqual(data["mode"], "caller-mode")
        self.assertEqual(data["toolchain"], "caller-toolchain")
        self.assertTrue(data["path_restored"])
        observed_data = json.loads(observed.read_text(encoding="utf-8-sig"))
        self.assertEqual(observed_data, {
            "mode": "check", "toolchain": "1.97.0", "target": "aarch64-pc-windows-msvc",
        })

    def test_toolchain_check_mode_blocks_native_installer_and_downloads(self):
        result = self.run_helpers(
            ["Install-VsComponents", "Install-LlvmMingw"],
            r'''
$env:AGENA_INSTALLER_SETUP_MODE = "check"
$env:RUNNER_TEMP = $fixture
$errors = @()
try { Install-VsComponents -Installer "never-run.exe" -InstallPath "not installed" -Components @("VC") } catch { $errors += $_.Exception.Message }
try { Install-LlvmMingw -Target "x86_64-pc-windows-gnu" } catch { $errors += $_.Exception.Message }
Write-Output ("RESULT:" + (@{ errors = $errors } | ConvertTo-Json -Compress))
''',
            source=SCRIPTS.parent / "ci" / "setup-windows-toolchain.ps1",
        )
        errors = self.result_json(result)["errors"]
        self.assertEqual(len(errors), 2)
        self.assertTrue(all("check never" in error for error in errors), errors)
        self.assertFalse(any(self.root.glob("agena-llvm-mingw-*")))



    @staticmethod
    def sandbox_report(ok=True):
        return {
            "schema_version": 1, "ok": ok, "platform": "windows", "duration_ms": 10,
            "checks": [
                {"name": "offline", "required": True, "status": "passed" if ok else "failed",
                 "detail": "fixture offline isolation" if ok else "fixture AppContainer failure"},
                {"name": "http_proxy", "required": False, "status": "unsupported",
                 "detail": "AppContainer identity-bound HTTP proxy is not implemented"},
            ],
        }

    def test_windows_optional_proxy_failure_is_reported_after_required_probe(self):
        report = json.dumps(self.sandbox_report())
        result = self.run_helpers(
            ["Test-InstalledSandbox", "Save-SandboxReport", "Complete-Replacement"],
            "$report = " + powershell_literal(report) + "\n" + r'''
$script:SetupMode = "check"
$script:InstallDir = $fixture
$script:AgenaBin = Join-Path $fixture "application files/bin/agena.exe"
$script:ServiceMode = "native"
$script:ReplacementAction = "Install"
$script:events = @()
function Invoke-AgenaLifecycle([string[]]$Arguments, [switch]$Capture) {
  $script:events += ($Arguments -join " ")
  return @{ ExitCode = 0; Output = $report }
}
function Start-Installed { $script:events += "start" }
function Save-State { $script:events += "save" }
function Update-UserPath { $script:events += "path" }
Complete-Replacement
Write-Output ("RESULT:" + (@{ events = $script:events } | ConvertTo-Json -Compress))
''',
        )
        self.assertEqual(self.result_json(result)["events"], ["--agena-sandbox-check", "start", "save", "path"])
        self.assertIn("identity-bound HTTP proxy", result.stdout)
        self.assertEqual(json.loads((self.root / "sandbox-report.json").read_text()), self.sandbox_report())
        self.assertFalse(any(self.root.glob(".sandbox-report-*.tmp")))

    def test_invalid_or_failed_probe_prevents_start_and_state_save(self):
        good = self.sandbox_report()
        cases = [
            ("not JSON", 0),
            (json.dumps(self.sandbox_report(False)), 1),
            (json.dumps({**good, "ok": "true"}), 0),
            (json.dumps({**good, "schema_version": 2}), 0),
            (json.dumps({**good, "schema_version": "1"}), 0),
            (json.dumps({**good, "duration_ms": "10"}), 0),
            (json.dumps(good), 1),
            (json.dumps({**good, "checks": []}), 0),
            (json.dumps({**good, "checks": good["checks"][:1]}), 0),
            (json.dumps({**good, "checks": good["checks"] + good["checks"][:1]}), 0),
            (json.dumps({**good, "checks": [{**good["checks"][0], "required": "true"}, good["checks"][1]]}), 0),
            (json.dumps({**good, "checks": [{**good["checks"][0], "status": "PASSED"}, good["checks"][1]]}), 0),
        ]
        for report, code in cases:
            with self.subTest(report=report, code=code):
                result = self.run_helpers(
                    ["Test-InstalledSandbox", "Complete-Replacement"],
                    "$report = " + powershell_literal(report) + "\n$code = " + str(code) + "\n" + r'''
$script:AgenaBin = "final-installed-agena.exe"
$script:events = @()
function Invoke-AgenaLifecycle { $script:events += "probe"; return @{ ExitCode = $code; Output = $report } }
function Start-Installed { $script:events += "STARTED" }
function Save-State { $script:events += "SAVED" }
function Update-UserPath { $script:events += "PATH" }
$errorText = ""
try { Complete-Replacement } catch { $errorText = $_.Exception.Message }
Write-Output ("RESULT:" + (@{ events = $script:events; error = $errorText } | ConvertTo-Json -Compress))
''',
                )
                data = self.result_json(result)
                self.assertTrue(data["error"], result.stdout)
                self.assertEqual(data["events"], ["probe"])

    def test_permissions_action_writes_report_without_restarting_or_changing_settings(self):
        report = json.dumps(self.sandbox_report())
        result = self.run_helpers(
            ["Repair-Permissions", "Test-InstalledSandbox", "Save-SandboxReport"],
            "$report = " + powershell_literal(report) + "\n" + r'''
$script:InstallDir = $fixture
$script:AgenaBin = "final-installed-agena.exe"
$script:events = @()
function Load-State { $script:events += "load" }
function Invoke-AgenaLifecycle { $script:events += "probe"; return @{ ExitCode = 0; Output = $report } }
function Start-Installed { throw "must not start a service" }
function Save-State { throw "must not rewrite settings" }
Repair-Permissions
Write-Output ("RESULT:" + (@{ events = $script:events } | ConvertTo-Json -Compress))
''',
        )
        self.assertEqual(self.result_json(result)["events"], ["load", "probe"])
        self.assertEqual(json.loads((self.root / "sandbox-report.json").read_text()), self.sandbox_report())

    def test_lifecycle_wrapper_restores_the_callers_instance_environment(self):
        result = self.run_helpers(
            ["Invoke-AgenaLifecycleAt"],
            r'''
$script:AgenaBin = "final-installed-agena.exe"
$env:AGENA_DATA_DIR = "caller-instance"
$script:seen = @()
function Invoke-InstallerExecutable {
  $script:seen += $env:AGENA_DATA_DIR
  return @{ ExitCode = 9; Output = "fixture output" }
}
$result = Invoke-AgenaLifecycleAt "selected-instance" @("--agena-sandbox-check") -Capture
$afterSuccess = $env:AGENA_DATA_DIR
function Invoke-InstallerExecutable { throw "fixture spawn failure" }
try { Invoke-AgenaLifecycleAt "another-instance" @("server", "status") } catch {}
Write-Output ("RESULT:" + (@{
  seen = $script:seen; after_success = $afterSuccess; after_failure = $env:AGENA_DATA_DIR
  code = $result.ExitCode; output = $result.Output
} | ConvertTo-Json -Compress))
''',
        )
        data = self.result_json(result)
        self.assertEqual(data["seen"], ["selected-instance"])
        self.assertEqual(data["after_success"], "caller-instance")
        self.assertEqual(data["after_failure"], "caller-instance")
        self.assertEqual(data["code"], 9)

    def test_probe_failure_restores_old_files_state_and_running_instance(self):
        for directory in ("bin", "web-dist"):
            (self.install_dir / directory).mkdir()
        binary = self.install_dir / "bin" / "agena.exe"
        web = self.install_dir / "web-dist" / "index.html"
        binary.write_text("old binary", encoding="utf-8")
        web.write_text("old web", encoding="utf-8")
        self.state_path.write_text(json.dumps(self.state), encoding="utf-8")
        saved_state = self.state_path.read_bytes()
        report_path = self.install_dir / "sandbox-report.json"
        old_report = b'{"previous":"successful report"}\n'
        report_path.write_bytes(old_report)
        report = json.dumps(self.sandbox_report(False))
        result = self.run_helpers(
            ["Resolve-Directory", "Assert-StoragePaths", "Load-State", "Install-Candidate",
             "Test-InstalledSandbox", "Complete-Replacement", "Undo-Replacement", "Invoke-Replacement"],
            "\n".join([
                "$script:InstallDir = " + powershell_literal(self.install_dir),
                "$script:DataDir = " + powershell_literal(self.data_dir),
                "$script:StateFile = " + powershell_literal(self.state_path),
                "$script:AgenaBin = " + powershell_literal(binary),
                "$report = " + powershell_literal(report),
            ]) + "\n" + r'''
$env:TEMP = $fixture
$script:ServiceMode = "detached"
$script:OldServiceMode = "detached"
$script:OldDataDir = $script:DataDir
$script:OldWasRunning = $true
$script:HadState = $false
$script:ActivationStarted = $false
$script:CandidateInstalled = $false
$script:ReplacementAction = "Upgrade"
$script:events = @()
function Resolve-Replacement {}
function New-Candidate([string]$Temp) {
  $script:CandidateStage = Join-Path $Temp "stage"
  foreach ($name in @("bin", "web-dist")) { New-Item -ItemType Directory -Path (Join-Path $script:CandidateStage $name) -Force | Out-Null }
  Set-Content -LiteralPath (Join-Path $script:CandidateStage "bin/agena.exe") -Value "new binary"
  Set-Content -LiteralPath (Join-Path $script:CandidateStage "web-dist/index.html") -Value "new web"
}
function Stop-Installation { $script:events += "stop-old" }
function Test-ServerRunning { throw "fixture candidate status failure" }
function Invoke-AgenaLifecycle {
  $current = (Get-Content -LiteralPath $script:AgenaBin -Raw).Trim()
  $script:events += "probe:$current"
  return @{ ExitCode = 1; Output = $report }
}
function Start-Installed {
  $current = (Get-Content -LiteralPath $script:AgenaBin -Raw).Trim()
  $script:events += "start:$current"
}
function Save-State { throw "failed candidate must never save state" }
function Update-UserPath { throw "failed candidate must never change PATH" }
$errorText = ""
try { Invoke-Replacement } catch { $errorText = $_.Exception.Message }
Write-Output ("RESULT:" + (@{ events = $script:events; error = $errorText } | ConvertTo-Json -Compress))
''',
        )
        data = self.result_json(result)
        self.assertIn("Required sandbox verification failed", data["error"])
        self.assertEqual(data["events"], ["stop-old", "probe:new binary", "start:old binary"])
        self.assertEqual(binary.read_text(encoding="utf-8"), "old binary")
        self.assertEqual(web.read_text(encoding="utf-8"), "old web")
        self.assertEqual(self.state_path.read_bytes(), saved_state)
        self.assertEqual(report_path.read_bytes(), old_report)
        self.assertEqual(self.database.read_bytes(), b"preserved fixture database")
        self.assertFalse(any(self.root.glob("agena-install-*")))

    def test_release_candidate_does_not_prepare_build_dependencies(self):
        result = self.run_helpers(
            ["New-Candidate"],
            r'''
$script:Source = $false
$script:Archive = "fixture.zip"
$script:Checksum = "fixture.zip.sha256"
$script:ReleaseVersion = ""
$script:events = @()
function Get-TargetTriple { return "x86_64-pc-windows-msvc" }
function Ensure-WindowsSourceDependencies { throw "release must not install build tools" }
function New-SourceArchive { throw "release must not build from source" }
function Get-FileFromSource { $script:events += "download" }
function Assert-Checksum { $script:events += "checksum" }
function Get-Command { return $null }
function Expand-Archive([string]$LiteralPath, [string]$DestinationPath, [switch]$Force) {
  foreach ($name in @("bin", "web-dist")) { New-Item -ItemType Directory -Force -Path (Join-Path $DestinationPath $name) | Out-Null }
  Set-Content -LiteralPath (Join-Path $DestinationPath "bin/agena.exe") -Value "fixture binary"
  Set-Content -LiteralPath (Join-Path $DestinationPath "web-dist/index.html") -Value "fixture web"
}
function Invoke-InstallerExecutable([string]$Executable, [string[]]$Arguments) {
  $script:events += ($Arguments -join " ")
  return @{ ExitCode = 0; Output = "agena 0.1.0" }
}
New-Candidate $fixture
Write-Output ("RESULT:" + (@{ events = $script:events; version = $script:Version } | ConvertTo-Json -Compress))
''',
        )
        data = self.result_json(result)
        self.assertEqual(data["version"], "0.1.0")
        self.assertEqual(data["events"], ["download", "download", "checksum", "--version"])



    def test_auto_mode_installs_only_missing_rust_components_and_target(self):
        source = self.source_versions()
        result = self.run_helpers(
            ["Get-SourceToolVersions", "Ensure-WindowsSourceDependencies"],
            "$source = " + powershell_literal(source) + "\n" + r'''
$script:SetupMode = "auto"
$env:AGENA_WEB_DIST_PREBUILT = "1"
$script:commands = @()
function Refresh-DependencyPath {}
function Ensure-WindowsBuildTools {}
function Ensure-WindowsCommand {}
function Invoke-DependencyCommand([string]$Name, [string[]]$Arguments) {
  $script:commands += ,$Arguments
  if ($Arguments[0] -eq "toolchain" -and $Arguments[1] -eq "list") {
    return @{ ExitCode = 0; Output = "1.97.0-x86_64-pc-windows-msvc" }
  }
  if ($Arguments[0] -eq "component" -and $Arguments[1] -eq "list") {
    return @{ ExitCode = 0; Output = "rustfmt-x86_64-pc-windows-msvc" }
  }
  if ($Arguments[0] -eq "target" -and $Arguments[1] -eq "list") {
    return @{ ExitCode = 0; Output = "x86_64-pc-windows-msvc" }
  }
  return @{ ExitCode = 0; Output = "" }
}
Ensure-WindowsSourceDependencies $source "aarch64-pc-windows-msvc"
Write-Output ("RESULT:" + (@{ commands = $script:commands } | ConvertTo-Json -Depth 5 -Compress))
''',
        )
        commands = self.result_json(result)["commands"]
        mutations = [command for command in commands if command[1] in ("add", "install")]
        self.assertEqual(mutations, [
            ["component", "add", "clippy", "--toolchain", "1.97.0"],
            ["target", "add", "aarch64-pc-windows-msvc", "--toolchain", "1.97.0"],
        ])

    def test_visual_studio_repair_elevates_only_component_installer_with_quoted_path(self):
        program_files = self.root / "Program Files (x86)"
        setup = program_files / "Microsoft Visual Studio" / "Installer" / "setup.exe"
        setup.parent.mkdir(parents=True)
        setup.write_bytes(b"never executed")
        installation = self.root / "Visual Studio With Spaces"
        result = self.run_helpers(
            ["Ensure-WindowsBuildTools"],
            "$programFiles = " + powershell_literal(program_files) + "\n$installation = "
            + powershell_literal(installation) + "\n" + r'''
[Environment]::SetEnvironmentVariable("ProgramFiles(x86)", $programFiles, "Process")
$script:SetupMode = "auto"
$script:queries = 0
$script:starts = @()
function Get-WindowsBuildToolsState {
  $script:queries += 1
  return @{ Ready = ($script:queries -gt 1); InstallPath = $installation }
}
function Start-Process([string]$FilePath, [string[]]$ArgumentList, [string]$Verb, [switch]$Wait, [switch]$PassThru) {
  $script:starts += @{ program = $FilePath; arguments = $ArgumentList; verb = $Verb; wait = $Wait.IsPresent }
  return @{ ExitCode = 0 }
}
function Install-WindowsDependency { throw "existing VS must not install another product" }
Ensure-WindowsBuildTools "aarch64-pc-windows-msvc"
Write-Output ("RESULT:" + (@{ starts = $script:starts } | ConvertTo-Json -Depth 5 -Compress))
''',
        )
        starts = self.result_json(result)["starts"]
        self.assertEqual(len(starts), 1)
        self.assertEqual(Path(starts[0]["program"]), setup)
        self.assertEqual(starts[0]["verb"], "RunAs")
        self.assertTrue(starts[0]["wait"])
        self.assertIn('"' + str(installation) + '"', starts[0]["arguments"])
        self.assertIn("Microsoft.VisualStudio.Component.VC.Tools.ARM64", starts[0]["arguments"])
        self.assertIn("--norestart", starts[0]["arguments"])

    def test_msvc_readiness_requires_real_headers_and_libraries(self):
        program_files = self.root / "Program Files (x86)"
        installation = self.root / "Visual Studio"
        files = [
            program_files / "Microsoft Visual Studio/Installer/vswhere.exe",
            installation / "Common7/Tools/VsDevCmd.bat",
            installation / "VC/Tools/MSVC/14.44.35207/bin/HostX64/x64/cl.exe",
            installation / "VC/Tools/MSVC/14.44.35207/include/vcruntime.h",
            installation / "VC/Tools/MSVC/14.44.35207/lib/x64/vcruntime.lib",
            program_files / "Windows Kits/10/Include/10.0.26100.0/ucrt/stddef.h",
            program_files / "Windows Kits/10/Include/10.0.26100.0/um/Windows.h",
            program_files / "Windows Kits/10/Lib/10.0.26100.0/um/x64/kernel32.lib",
        ]
        for file in files:
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_bytes(b"fixture tool")
        last_library = program_files / "Windows Kits/10/Lib/10.0.26100.0/ucrt/x64/ucrt.lib"
        result = self.run_helpers(
            ["Get-WindowsBuildToolsState"],
            "\n".join([
                "$programFiles = " + powershell_literal(program_files),
                "$installation = " + powershell_literal(installation),
                "$lastLibrary = " + powershell_literal(last_library),
            ]) + "\n" + r'''
[Environment]::SetEnvironmentVariable("ProgramFiles(x86)", $programFiles, "Process")
$env:PROCESSOR_ARCHITECTURE = "AMD64"
function Invoke-InstallerExecutable { return @{ ExitCode = 0; Output = $installation } }
$before = Get-WindowsBuildToolsState "x86_64-pc-windows-msvc"
New-Item -ItemType Directory -Path (Split-Path -Parent $lastLibrary) -Force | Out-Null
Set-Content -LiteralPath $lastLibrary -Value "fixture library"
$after = Get-WindowsBuildToolsState "x86_64-pc-windows-msvc"
Write-Output ("RESULT:" + (@{ before = $before.Ready; after = $after.Ready } | ConvertTo-Json -Compress))
''',
        )
        self.assertEqual(self.result_json(result), {"before": False, "after": True})



    def test_private_probe_uses_exact_native_argv_and_restores_environment(self):
        captured = self.root / "captured-argv.jsonl"
        program = self.root / "native-probe.py"
        program.write_text(
            "import json, os, sys\n"
            "from pathlib import Path\n"
            "with Path(" + repr(str(captured)) + ").open('a', encoding='utf-8') as stream:\n"
            "    stream.write(json.dumps({'argv':sys.argv[1:], 'data':os.environ.get('AGENA_DATA_DIR')})+'\\n')\n"
            "if sys.argv[1:] == ['--agena-sandbox-check']:\n"
            "    print(json.dumps(" + repr(self.sandbox_report()) + "))\n"
            "elif sys.argv[1:] == ['--fixture-error']:\n"
            "    print('native fixture error', file=sys.stderr)\n"
            "    sys.exit(17)\n"
            "else:\n"
            "    print('unexpected private CLI arguments', file=sys.stderr)\n"
            "    sys.exit(19)\n",
            encoding="utf-8",
        )
        binary = self.root / ("final installed agena.cmd" if os.name == "nt" else "final-installed-agena")
        if os.name == "nt":
            binary.write_text(
                '@echo off\n"' + sys.executable + '" "' + str(program) + '" %*\nexit /b %errorlevel%\n',
                encoding="utf-8",
            )
        else:
            binary.write_text(
                "#!" + sys.executable + "\n" + program.read_text(), encoding="utf-8",
            )
            binary.chmod(0o700)
        result = self.run_helpers(
            ["Invoke-InstallerExecutable", "Invoke-AgenaLifecycleAt", "Invoke-AgenaLifecycle",
             "Test-InstalledSandbox"],
            "$script:AgenaBin = " + powershell_literal(binary) + "\n" + r'''
$script:DataDir = "selected-instance"
$env:AGENA_DATA_DIR = "caller-instance"
Test-InstalledSandbox
$failed = Invoke-AgenaLifecycleAt "failure-instance" @("--fixture-error") -Capture
Write-Output ("RESULT:" + (@{
  ok = $script:SandboxReport.ok; restored = $env:AGENA_DATA_DIR
  failure_code = $failed.ExitCode; failure_output = $failed.Output
} | ConvertTo-Json -Compress))
''',
        )
        data = self.result_json(result)
        self.assertTrue(data["ok"])
        self.assertEqual(data["restored"], "caller-instance")
        self.assertEqual(data["failure_code"], 17)
        self.assertIn("native fixture error", data["failure_output"])
        invocations = [json.loads(line) for line in captured.read_text().splitlines()]
        self.assertEqual(invocations, [
            {"argv": ["--agena-sandbox-check"], "data": "selected-instance"},
            {"argv": ["--fixture-error"], "data": "failure-instance"},
        ])

    def test_failed_start_or_permissions_probe_preserves_last_successful_report(self):
        path = self.root / "sandbox-report.json"
        original = b'{"previous":"successful report"}\n'
        path.write_bytes(original)
        success = json.dumps(self.sandbox_report())
        failure = json.dumps(self.sandbox_report(False))
        result = self.run_helpers(
            ["Test-InstalledSandbox", "Save-SandboxReport", "Complete-Replacement", "Repair-Permissions"],
            "$successReport = " + powershell_literal(success) + "\n$failureReport = "
            + powershell_literal(failure) + "\n" + r'''
$script:InstallDir = $fixture
$script:AgenaBin = "final-installed-agena.exe"
$script:ServiceMode = "native"
$script:ReplacementAction = "Install"
$script:errors = @()
function Invoke-AgenaLifecycle { return @{ ExitCode = 0; Output = $successReport } }
function Start-Installed { throw "fixture start failure" }
function Save-State { throw "must not save after start failure" }
function Update-UserPath { throw "must not update PATH after start failure" }
try { Complete-Replacement } catch { $script:errors += $_.Exception.Message }
function Load-State {}
function Invoke-AgenaLifecycle { return @{ ExitCode = 1; Output = $failureReport } }
try { Repair-Permissions } catch { $script:errors += $_.Exception.Message }
Write-Output ("RESULT:" + (@{ errors = $script:errors } | ConvertTo-Json -Compress))
''',
        )
        errors = self.result_json(result)["errors"]
        self.assertIn("fixture start failure", errors[0])
        self.assertIn("Required sandbox verification failed", errors[1])
        self.assertEqual(path.read_bytes(), original)



    def test_python_store_alias_does_not_hide_an_existing_interpreter(self):
        alias = self.root / "Microsoft" / "WindowsApps" / "python.exe"
        interpreter = self.root / "real-python" / "python.exe"
        result = self.run_helpers(
            ["Get-DependencyCommand", "Invoke-DependencyCommand", "Test-WindowsCommand",
             "Ensure-WindowsCommand"],
            "$aliasPath = " + powershell_literal(alias) + "\n$pythonPath = "
            + powershell_literal(interpreter) + "\n" + r'''
$script:SetupMode = "check"
function Get-Command {
  return @(@{ Source = $aliasPath }, @{ Source = $pythonPath })
}
function Invoke-InstallerExecutable { return @{ ExitCode = 0; Output = "Python 3.13.2" } }
function Install-WindowsDependency { throw "must use the existing interpreter" }
Ensure-WindowsCommand "python" "Python.Python.3.13" "3.0"
Write-Output ("RESULT:" + (@{
  selected = (Get-DependencyCommand "python").Source
  selected_for_children = $env:PATH.StartsWith((Split-Path -Parent $pythonPath) + ";")
} | ConvertTo-Json -Compress))
''',
        )
        data = self.result_json(result)
        self.assertEqual(Path(data["selected"]), interpreter)
        self.assertTrue(data["selected_for_children"])



    def test_check_mode_reuses_the_official_nested_llvm_archive_layout(self):
        cache = self.root / "agena-llvm-mingw-20260616"
        binary_dir = cache / "toolchain" / "llvm-mingw-20260616-ucrt-x86_64" / "bin"
        binary_dir.mkdir(parents=True)
        for name in ("clang.exe", "x86_64-w64-mingw32-clang.exe",
                     "x86_64-w64-mingw32-clang++.exe", "llvm-ar.exe"):
            (binary_dir / name).write_bytes(b"existing fixture tool")
        archive = cache / "llvm-mingw-20260616-ucrt-x86_64.zip"
        archive.write_bytes(b"fixture archive")
        original_files = sorted(path.relative_to(cache) for path in cache.rglob("*"))
        result = self.run_helpers(
            ["Find-LlvmMingwBin", "Install-LlvmMingw"],
            r'''
$env:RUNNER_TEMP = $fixture
$env:AGENA_INSTALLER_SETUP_MODE = "check"
function Get-FileHash { return @{ Hash = "b9b68a4d276e16fa25802aaba458e4638f64b3884c290aaccdc2d87083b6ca35" } }
function Invoke-WebRequest { throw "must not download an existing verified toolchain" }
function Expand-Archive { throw "must not re-extract an existing toolchain" }
Install-LlvmMingw -Target "x86_64-pc-windows-gnu"
Write-Output ("RESULT:" + (@{
  compiler = [Environment]::GetEnvironmentVariable("CC_x86_64_pc_windows_gnu")
} | ConvertTo-Json -Compress))
''',
            source=SCRIPTS.parent / "ci" / "setup-windows-toolchain.ps1",
        )
        self.assertEqual(Path(self.result_json(result)["compiler"]), binary_dir / "x86_64-w64-mingw32-clang.exe")
        self.assertEqual(archive.read_bytes(), b"fixture archive")
        self.assertEqual(sorted(path.relative_to(cache) for path in cache.rglob("*")), original_files)

    def test_status_probe_accepts_only_explicit_running_and_stopped_results(self):
        binary = self.root / "agena.exe"
        binary.write_bytes(b"not executed")
        result = self.run_helpers(
            ["Test-ServerRunningAt"],
            "$script:AgenaBin = " + powershell_literal(binary) + "\n" + r'''
$script:statusText = ""
function Invoke-AgenaLifecycleAt { return @{ ExitCode = 0; Output = $script:statusText } }
$values = @()
foreach ($status in @(
  "Agena server is running at http://127.0.0.1:3210 (pid 1).",
  "Agena server is not running (no record at fixture).",
  "Agena server user service is installed but not running (no record at fixture)."
)) {
  $script:statusText = $status
  $values += Test-ServerRunningAt "fixture"
}
Write-Output ("RESULT:" + (@{ values = $values } | ConvertTo-Json -Compress))
''',
        )
        self.assertEqual(self.result_json(result)["values"], [True, False, False])

    def test_failed_or_unknown_status_aborts_before_preparation_or_service_changes(self):
        binary = self.root / "agena.exe"
        binary.write_bytes(b"preserved old binary")
        for code, output, expected in [
            (7, "Agena server is running at http://127.0.0.1:3210 (pid 1).", "Unable to inspect"),
            (0, "unrecognized fixture status", "unrecognized response"),
        ]:
            with self.subTest(code=code, output=output):
                result = self.run_helpers(
                    ["Test-ServerRunningAt", "Test-ServerRunning", "Resolve-Replacement", "Invoke-Replacement"],
                    "$script:AgenaBin = " + powershell_literal(binary) + "\n$statusCode = " + str(code)
                    + "\n$statusText = " + powershell_literal(output) + "\n" + r'''
$script:Action = "Upgrade"
$script:InstallDir = $fixture
$script:events = @()
$env:TEMP = $fixture
function Load-State { $script:DataDir = $fixture; $script:ServiceMode = "detached"; $script:Workspace = $fixture }
function Invoke-AgenaLifecycleAt { $script:events += "status"; return @{ ExitCode = $statusCode; Output = $statusText } }
function Assert-StoragePaths {}
function Resolve-ServiceMode {}
function New-Item { $script:events += "MUTATED" }
function New-Candidate { $script:events += "PREPARED"; throw "must not prepare a candidate" }
function Stop-Installation { $script:events += "STOPPED" }
function Install-Candidate { $script:events += "REPLACED" }
function Complete-Replacement { $script:events += "STARTED" }
$errorText = ""
try { Invoke-Replacement } catch { $errorText = $_.Exception.Message }
Write-Output ("RESULT:" + (@{ error = $errorText; events = $script:events } | ConvertTo-Json -Compress))
''',
                )
                data = self.result_json(result)
                self.assertIn(expected, data["error"])
                self.assertEqual(data["events"], ["status"])
        self.assertEqual(binary.read_bytes(), b"preserved old binary")
        self.assertEqual(self.database.read_bytes(), b"preserved fixture database")


if __name__ == "__main__":
    unittest.main(verbosity=2)
