"""Offline installer storage contracts. All binaries and homes are temporary stubs."""

import hashlib
import os
from pathlib import Path
import shlex
import subprocess
import tarfile
import tempfile
import unittest


SCRIPTS = Path(__file__).resolve().parents[1]
STUB = r'''#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "--version" ]]; then
  echo 'agena 0.1.0'
  exit 0
fi
[[ "${1:-}" == "--data-dir" ]] || exit 90
data_root="$2"
shift 2
if [[ "${1:-}" == "--version" ]]; then
  echo 'agena 0.1.0'
  exit 0
fi
# A regression in the installer still cannot make this stub write real data.
[[ "$data_root" == "$TEST_SANDBOX_ROOT/"* ]] || exit 91
[[ "${1:-}" == "server" ]] || exit 92
action="${2:-}"
printf '%s\t%s\t%s\n' "$data_root" "$action" "$*" >> "$TEST_CALLS_PATH"
case "$action" in
  start|install)
    if [[ -n "${TEST_ROUTING_PATH:-}" ]]; then
      printf '%s\t%s\t%s\n' "${NO_PROXY:-}" "${no_proxy:-}" "${http_proxy:-}" >> "$TEST_ROUTING_PATH"
    fi
    mkdir -p "$data_root/run"
    printf 'fixture database\n' > "$data_root/agena.db"
    : > "$data_root/run/stub-running"
    echo 'Started stub Agena server.' ;;
  stop|uninstall)
    rm -f "$data_root/run/stub-running"
    echo 'Stopped stub Agena server.' ;;
  status)
    if [[ "${TEST_STATUS_FAILURE:-}" == 1 ]]; then
      echo 'Error: server readiness handshake failed'
      exit 42
    fi
    if [[ "${TEST_STATUS_UNEXPECTED:-}" == 1 ]]; then
      echo 'Unrecognized status fixture'
      exit 0
    fi
    if [[ "${TEST_REQUIRE_DIRECT_STATUS:-}" == 1 && ( "${NO_PROXY:-}" != '*' || "${no_proxy:-}" != '*' ) ]]; then
      echo 'Error: proxy returned an empty response'
      exit 43
    fi
    if [[ -f "$data_root/run/stub-running" ]]; then
      echo 'Agena server is running at http://127.0.0.1:3210'
    else
      echo 'Agena server is not running'
    fi ;;
  permissions) echo 'Stub permissions setup.' ;;
  *) exit 93 ;;
esac
'''
BAD_STUB = r'''#!/usr/bin/env bash
if [[ "${1:-}" == "--version" ]]; then
  echo 'agena 0.1.0'
  exit 0
fi
exit 42
'''


class InstallerStorageTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="agena-offline-installer-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.home = self.root / "home"
        self.home.mkdir()
        self.install_dir = self.root / "application files"
        self.data_dir = self.root / "selected instance data"
        self.workspace = self.root / "workspace"
        self.workspace.mkdir()
        self.calls_path = self.root / "calls.tsv"
        (self.root / "tmp").mkdir()
        self.env = {
            "PATH": os.environ.get("PATH", "/usr/bin:/bin"),
            "HOME": str(self.home),
            "TMPDIR": str(self.root / "tmp"),
            "TEST_SANDBOX_ROOT": str(self.root),
            "TEST_CALLS_PATH": str(self.calls_path),
        }
        self.archive = self.package("good", STUB)
        self.default_data = self.home / "agena"
        self.default_data.mkdir()
        self.default_sentinel = self.default_data / "preserve"
        self.default_sentinel.write_text("unselected default data")

    def package(self, name, source):
        stage = self.root / (name + " stage")
        (stage / "bin").mkdir(parents=True)
        binary = stage / "bin/agena"
        binary.write_text(source)
        binary.chmod(0o755)
        (stage / "web-dist").mkdir()
        (stage / "web-dist/index.html").write_text("<!doctype html><title>fixture</title>")
        archive = self.root / (name + " archive.tar.gz")
        with tarfile.open(archive, "w:gz") as package:
            package.add(stage / "bin", arcname="bin")
            package.add(stage / "web-dist", arcname="web-dist")
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        Path(str(archive) + ".sha256").write_text(digest + "  fixture.tar.gz\n")
        return archive

    def invoke(self, action, *arguments, environment=None, success=True, cwd=None,
               update_path=False):
        env = dict(self.env)
        env.update(environment or {})
        command = [
            "bash", str(SCRIPTS / "install.sh"), action,
            "--install-dir", str(self.install_dir), *map(str, arguments),
        ]
        if not update_path and action in {"install", "reinstall", "upgrade", "uninstall"}:
            command.append("--no-path-update")
        result = subprocess.run(
            command,
            env=env, cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            timeout=30,
        )
        if success:
            self.assertEqual(result.returncode, 0, result.stdout)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout)
        return result

    def install(self):
        return self.invoke(
            "install", "--archive", self.archive, "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            environment={"AGENA_DATA_DIR": str(self.root / "ignored environment data")},
        )

    def source_checkout(self, name="source checkout"):
        root = self.root / name
        (root / "packages/agena-web").mkdir(parents=True)
        (root / "Cargo.toml").write_text("[workspace]\nmembers = []\n")
        (root / "packages/agena-web/package.json").write_text("{}\n")
        scripts = root / "scripts/agena"
        scripts.mkdir(parents=True)
        archive = self.archive
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        package_script = scripts / "package-backend.sh"
        package_script.write_text(
            "#!/usr/bin/env bash\n"
            "set -euo pipefail\n"
            'root="$(cd "$(dirname "$0")/../.." && pwd)"\n'
            'mkdir -p "$root/artifacts/agena"\n'
            f'cp {shlex.quote(str(archive))} "$root/artifacts/agena/agena-backend-${{1:-test}}-v0.1.0.tar.gz"\n'
            'archive="$root/artifacts/agena/agena-backend-${1:-test}-v0.1.0.tar.gz"\n'
            f'printf \'%s  %s\\n\' {shlex.quote(digest)} "$(basename "$archive")" > "$archive.sha256"\n'
            'printf \'Package ready: %s\\n\' "$archive"\n'
            'printf \'Checksum ready: %s.sha256\\n\' "$archive"\n'
        )
        package_script.chmod(0o755)
        return root

    def assert_only_selected_root_was_called(self):
        calls = self.calls_path.read_text().splitlines()
        self.assertTrue(calls)
        self.assertTrue(all(line.split("\t", 1)[0] == str(self.data_dir) for line in calls), calls)

    def state_value(self, name):
        state = self.install_dir / "install-state.env"
        result = subprocess.run(
            ["bash", "-c", f'source "$1"; printf "%s" "${{{name}}}"', "_", str(state)],
            env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout

    def test_lifecycle_uses_saved_root_with_spaces_despite_poisoned_environment(self):
        self.install()
        for poison in [str(self.install_dir), str(self.home), "/"]:
            self.invoke("status", environment={"AGENA_DATA_DIR": poison})
        self.invoke("stop", environment={"AGENA_DATA_DIR": str(self.install_dir)})
        self.assertFalse((self.data_dir / "run/stub-running").exists())
        self.invoke("start", environment={"AGENA_DATA_DIR": "/"})
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.assert_only_selected_root_was_called()
        self.assertFalse((self.home / ".profile").exists())

    def test_normal_uninstall_preserves_data_and_purge_removes_only_saved_root(self):
        self.install()
        self.invoke("uninstall", environment={"AGENA_DATA_DIR": str(self.install_dir)})
        self.assertFalse(self.install_dir.exists())
        self.assertTrue((self.data_dir / "agena.db").is_file())
        self.assertTrue(self.default_sentinel.is_file())
        self.install()
        self.invoke("uninstall", "--purge-data", environment={"AGENA_DATA_DIR": str(self.default_data)})
        self.assertFalse(self.data_dir.exists())
        self.assertFalse(self.install_dir.exists())
        self.assertEqual(self.default_sentinel.read_text(), "unselected default data")
        self.assert_only_selected_root_was_called()

    def test_failed_upgrade_restores_binary_running_state_and_saved_root(self):
        self.install()
        original = (self.install_dir / "bin/agena").read_bytes()
        bad = self.package("broken", BAD_STUB)
        self.invoke("upgrade", "--archive", bad, environment={"AGENA_DATA_DIR": "/"}, success=False)
        self.assertEqual((self.install_dir / "bin/agena").read_bytes(), original)
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.invoke("status", environment={"AGENA_DATA_DIR": str(self.install_dir)})
        self.assert_only_selected_root_was_called()

    def test_successful_upgrade_keeps_previously_stopped_instance_stopped(self):
        self.install()
        self.invoke("stop")
        self.invoke("upgrade", "--archive", self.archive, environment={"AGENA_DATA_DIR": "/"})
        self.assertFalse((self.data_dir / "run/stub-running").exists())
        self.assert_only_selected_root_was_called()

    def test_upgrade_applies_explicit_runtime_overrides_and_persists_them(self):
        self.install()
        new_data = self.root / "replacement instance data"
        new_workspace = self.root / "replacement workspace"
        self.invoke(
            "upgrade", "--archive", self.archive,
            "--data-dir", new_data,
            "--workspace", new_workspace,
            "--host", "0.0.0.0",
            "--port", "4321",
            "--ui-password", "replacement password",
            "--service-mode", "detached",
            environment={
                "AGENA_DATA_DIR": "/",
                "AGENA_SERVER_HOST": "203.0.113.10",
                "AGENA_SERVER_PORT": "9999",
                "AGENA_WORKSPACE_ROOT": "/",
            },
        )
        self.assertEqual(self.state_value("DATA_DIR"), str(new_data))
        self.assertEqual(self.state_value("HOST"), "0.0.0.0")
        self.assertEqual(self.state_value("PORT"), "4321")
        self.assertEqual(self.state_value("WORKSPACE"), str(new_workspace))
        self.assertEqual(self.state_value("UI_PASSWORD"), "replacement password")
        self.assertEqual(self.state_value("SERVICE_MODE"), "detached")
        self.assertTrue(new_workspace.is_dir())
        self.assertTrue((new_data / "agena.db").is_file())
        self.assertTrue((new_data / "run/stub-running").is_file())
        self.assertFalse((self.data_dir / "run/stub-running").exists())

        calls = self.calls_path.read_text().splitlines()
        self.assertTrue(any(
            line.startswith(str(self.data_dir) + "\t") and "\tstop\t" in line
            for line in calls
        ), calls)
        self.assertTrue(any(
            line.startswith(str(new_data) + "\t") and "\tstart\t" in line
            and "--host 0.0.0.0" in line and "--port 4321" in line
            and f"--workspace {new_workspace}" in line
            for line in calls
        ), calls)

        self.invoke("status", environment={"AGENA_DATA_DIR": "/"})
        self.assertEqual(self.calls_path.read_text().splitlines()[-1].split("\t", 1)[0], str(new_data))

    def test_upgrade_rejects_an_unsafe_data_override_before_stopping(self):
        self.install()
        original_state = (self.install_dir / "install-state.env").read_bytes()
        original_calls = self.calls_path.read_bytes()
        self.invoke(
            "upgrade", "--archive", self.archive,
            "--data-dir", self.install_dir / "overlapping data",
            success=False,
        )
        self.assertEqual((self.install_dir / "install-state.env").read_bytes(), original_state)
        extra_calls = self.calls_path.read_bytes()[len(original_calls):].decode().splitlines()
        self.assertTrue(all("\tstatus\t" in line for line in extra_calls), extra_calls)
        self.assertTrue((self.data_dir / "run/stub-running").exists())

    def test_failed_upgrade_with_runtime_overrides_restores_saved_configuration(self):
        self.install()
        original_state = (self.install_dir / "install-state.env").read_bytes()
        bad = self.package("broken overrides", BAD_STUB)
        replacement_data = self.root / "failed replacement data"
        replacement_workspace = self.root / "failed replacement workspace"
        self.invoke(
            "upgrade", "--archive", bad,
            "--data-dir", replacement_data,
            "--workspace", replacement_workspace,
            "--host", "0.0.0.0",
            "--port", "4321",
            success=False,
        )
        self.assertEqual((self.install_dir / "install-state.env").read_bytes(), original_state)
        self.assertEqual(self.state_value("DATA_DIR"), str(self.data_dir))
        self.assertEqual(self.state_value("HOST"), "127.0.0.1")
        self.assertEqual(self.state_value("PORT"), "3210")
        self.assertEqual(self.state_value("WORKSPACE"), str(self.workspace))
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.assert_only_selected_root_was_called()

    def test_source_upgrade_applies_runtime_overrides_from_current_checkout(self):
        self.install()
        checkout = self.source_checkout()
        result = self.invoke(
            "upgrade", "--source", "--host", "0.0.0.0", "--port", "4321",
            cwd=checkout,
        )
        self.assertIn(f"Building Agena from source at {checkout}", result.stdout)
        self.assertEqual(self.state_value("HOST"), "0.0.0.0")
        self.assertEqual(self.state_value("PORT"), "4321")
        self.assertEqual(self.state_value("DATA_DIR"), str(self.data_dir))
        self.assertTrue((self.data_dir / "run/stub-running").exists())

    def test_install_refuses_unsafe_or_overlapping_data_roots_before_calling_binary(self):
        for root in [self.home, "/", self.install_dir, self.install_dir / "nested", self.root]:
            self.invoke("install", "--archive", self.archive, "--data-dir", root, success=False)
        self.assertFalse(self.calls_path.exists())
        self.assertTrue(self.default_sentinel.is_file())

    def test_purge_refuses_a_directory_without_an_instance_database(self):
        self.install()
        (self.data_dir / "agena.db").unlink()
        self.invoke("uninstall", "--purge-data", success=False)
        self.assertTrue((self.install_dir / "bin/agena").exists())
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.assertTrue(self.default_sentinel.is_file())

    def test_retargeted_saved_data_directory_is_rejected_without_deleting_target(self):
        self.install()
        previous = self.root / "previous data"
        self.data_dir.rename(previous)
        replacement = self.root / "unrelated data"
        replacement.mkdir()
        (replacement / "agena.db").write_text("unrelated instance")
        self.data_dir.symlink_to(replacement, target_is_directory=True)
        self.invoke("uninstall", "--purge-data", success=False)
        self.assertEqual((replacement / "agena.db").read_text(), "unrelated instance")
        self.assertTrue((self.install_dir / "bin/agena").exists())
        self.assertTrue((previous / "agena.db").exists())

    def test_copied_install_manifest_cannot_redirect_lifecycle_to_another_install(self):
        self.install()
        copied = self.root / "copied installation"
        copied.mkdir()
        (copied / "install-state.env").write_bytes((self.install_dir / "install-state.env").read_bytes())
        original_calls = self.calls_path.read_bytes()
        result = subprocess.run(
            ["bash", str(SCRIPTS / "install.sh"), "status", "--install-dir", str(copied)],
            env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30,
        )
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertEqual(self.calls_path.read_bytes(), original_calls)

    def test_failed_first_install_leaves_no_partial_installation(self):
        bad = self.package("broken first install", BAD_STUB)
        self.invoke(
            "install", "--archive", bad, "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            success=False,
        )
        self.assertFalse((self.install_dir / "bin").exists())
        self.assertFalse((self.install_dir / "web-dist").exists())
        self.assertFalse((self.install_dir / "install-state.env").exists())

    def test_reinstall_requires_an_existing_installation(self):
        result = self.invoke("reinstall", "--archive", self.archive, success=False)
        self.assertIn("not installed", result.stdout.lower())
        self.assertFalse(self.calls_path.exists())
        self.assertFalse(self.install_dir.exists())

    def test_install_refuses_an_existing_installation_and_names_replacement_actions(self):
        self.install()
        original = (self.install_dir / "bin/agena").read_bytes()
        result = self.invoke("install", "--archive", self.archive, success=False)
        self.assertIn("already installed", result.stdout)
        self.assertIn("reinstall", result.stdout)
        self.assertIn("upgrade", result.stdout)
        self.assertEqual((self.install_dir / "bin/agena").read_bytes(), original)
        self.assertTrue((self.data_dir / "run/stub-running").exists())

    def test_reinstall_replaces_a_running_installation_in_place(self):
        self.install()
        result = self.invoke("reinstall", "--archive", self.archive,
                             environment={"AGENA_DATA_DIR": "/"})
        self.assertIn("reinstalled", result.stdout)
        self.assertEqual(self.state_value("VERSION"), "0.1.0")
        self.assertEqual(self.state_value("DATA_DIR"), str(self.data_dir))
        self.assertEqual(self.state_value("SERVICE_MODE"), "detached")
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.assert_only_selected_root_was_called()

    def test_reinstall_keeps_a_stopped_instance_stopped(self):
        self.install()
        self.invoke("stop")
        self.invoke("reinstall", "--archive", self.archive)
        self.assertFalse((self.data_dir / "run/stub-running").exists())
        self.assert_only_selected_root_was_called()

    def test_reinstall_probes_legacy_client_directly_without_changing_server_proxy(self):
        self.install()
        routing = self.root / "server routing.tsv"
        environment = {
            "http_proxy": "http://127.0.0.1:9",
            "https_proxy": "http://127.0.0.1:9",
            "NO_PROXY": "keep-upper.example",
            "no_proxy": "keep-lower.example",
            "TEST_REQUIRE_DIRECT_STATUS": "1",
            "TEST_ROUTING_PATH": str(routing),
        }
        self.invoke("reinstall", "--archive", self.archive, environment=environment)
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.assertEqual(routing.read_text().strip(),
                         "keep-upper.example\tkeep-lower.example\thttp://127.0.0.1:9")

    def test_reinstall_no_path_update_overrides_the_saved_setting(self):
        self.invoke("install", "--archive", self.archive, "--data-dir", self.data_dir,
                    "--workspace", self.workspace, "--service-mode", "detached",
                    environment={"SHELL": "/bin/zsh"}, update_path=True)
        profiles = [self.home / ".zshenv", self.home / ".zshrc"]
        original = [path.read_bytes() for path in profiles]
        self.assertEqual(self.state_value("UPDATE_PATH"), "1")
        result = self.invoke("reinstall", "--archive", self.archive,
                             environment={"SHELL": "/bin/zsh"})
        self.assertNotIn("Added Agena PATH", result.stdout)
        self.assertEqual([path.read_bytes() for path in profiles], original)
        self.assertEqual(self.state_value("UPDATE_PATH"), "0")

    def test_reinstall_does_not_treat_a_failed_or_unknown_probe_as_stopped(self):
        self.install()
        original = (self.install_dir / "bin/agena").read_bytes()
        state = (self.install_dir / "install-state.env").read_bytes()
        for flag in ["TEST_STATUS_FAILURE", "TEST_STATUS_UNEXPECTED"]:
            with self.subTest(flag=flag):
                before = self.calls_path.read_text().splitlines()
                result = self.invoke("reinstall", "--archive", self.archive,
                                     environment={flag: "1"}, success=False)
                self.assertIn("server", result.stdout)
                self.assertTrue((self.data_dir / "run/stub-running").exists())
                self.assertEqual((self.install_dir / "bin/agena").read_bytes(), original)
                self.assertEqual((self.install_dir / "install-state.env").read_bytes(), state)
                extra_calls = self.calls_path.read_text().splitlines()[len(before):]
                self.assertTrue(extra_calls)
                self.assertTrue(all("\tstatus\t" in line for line in extra_calls), extra_calls)

    def test_reinstall_keeps_saved_settings_and_applies_explicit_overrides(self):
        self.install()
        original_state = (self.install_dir / "install-state.env").read_bytes()
        self.invoke("reinstall", "--archive", self.archive)
        self.assertEqual((self.install_dir / "install-state.env").read_bytes(), original_state)

        new_workspace = self.root / "reinstalled workspace"
        self.invoke(
            "reinstall", "--archive", self.archive,
            "--workspace", new_workspace,
            "--port", "4321",
            environment={"AGENA_SERVER_PORT": "9999", "AGENA_WORKSPACE_ROOT": "/"},
        )
        self.assertEqual(self.state_value("DATA_DIR"), str(self.data_dir))
        self.assertEqual(self.state_value("HOST"), "127.0.0.1")
        self.assertEqual(self.state_value("PORT"), "4321")
        self.assertEqual(self.state_value("WORKSPACE"), str(new_workspace))
        self.assertEqual(self.state_value("SERVICE_MODE"), "detached")
        self.assertTrue(new_workspace.is_dir())
        self.assertTrue((self.data_dir / "run/stub-running").exists())

    def test_reinstall_rejects_an_unsafe_data_override_before_stopping(self):
        self.install()
        original_calls = self.calls_path.read_bytes()
        self.invoke(
            "reinstall", "--archive", self.archive,
            "--data-dir", self.install_dir / "overlapping data",
            success=False,
        )
        extra_calls = self.calls_path.read_bytes()[len(original_calls):].decode().splitlines()
        self.assertTrue(all("\tstatus\t" in line for line in extra_calls), extra_calls)
        self.assertTrue((self.data_dir / "run/stub-running").exists())

    def test_failed_reinstall_restores_binary_state_and_configuration(self):
        self.install()
        original_binary = (self.install_dir / "bin/agena").read_bytes()
        original_state = (self.install_dir / "install-state.env").read_bytes()
        bad = self.package("broken reinstall", BAD_STUB)
        replacement_data = self.root / "failed reinstall data"
        self.invoke(
            "reinstall", "--archive", bad,
            "--data-dir", replacement_data,
            "--host", "0.0.0.0",
            "--port", "4321",
            success=False,
        )
        self.assertEqual((self.install_dir / "bin/agena").read_bytes(), original_binary)
        self.assertEqual((self.install_dir / "install-state.env").read_bytes(), original_state)
        self.assertEqual(self.state_value("DATA_DIR"), str(self.data_dir))
        self.assertEqual(self.state_value("HOST"), "127.0.0.1")
        self.assertEqual(self.state_value("PORT"), "3210")
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.assert_only_selected_root_was_called()

    def test_reinstall_requires_a_package_when_no_release_was_recorded(self):
        self.install()
        original_calls = self.calls_path.read_bytes()
        result = self.invoke("reinstall", success=False)
        self.assertIn("--archive", result.stdout)
        self.assertIn("--source", result.stdout)
        extra_calls = self.calls_path.read_bytes()[len(original_calls):].decode().splitlines()
        self.assertTrue(all("\tstatus\t" in line for line in extra_calls), extra_calls)
        self.assertTrue((self.data_dir / "run/stub-running").exists())

    def test_actions_reject_options_they_would_ignore(self):
        self.install()
        original_calls = self.calls_path.read_bytes()
        cases = [
            (["status", "--host", "0.0.0.0"], "--host only applies"),
            (["stop", "--archive", str(self.archive)], "--archive only applies"),
            (["restart", "--port", "4321"], "--port only applies"),
            (["status", "--source"], "--source only applies"),
            (["install", "--purge-data"], "--purge-data only applies to uninstall"),
        ]
        for arguments, expected in cases:
            result = self.invoke(*arguments, success=False)
            self.assertIn(expected, result.stdout, result.stdout)
        self.assertEqual(self.calls_path.read_bytes(), original_calls)

    def test_source_reinstall_rebuilds_the_current_checkout(self):
        self.install()
        checkout = self.source_checkout("reinstall checkout")
        result = self.invoke("reinstall", "--source", cwd=checkout)
        self.assertIn(f"Building Agena from source at {checkout}", result.stdout)
        self.assertIn("reinstalled", result.stdout)
        self.assertEqual(self.state_value("DATA_DIR"), str(self.data_dir))
        self.assertTrue((self.data_dir / "run/stub-running").exists())

    def fake_release_downloads(self):
        fake_bin = self.root / "release bin"
        fake_bin.mkdir()
        urls = self.root / "release urls.txt"
        urls.write_text("")
        archive = self.archive
        checksum = Path(str(archive) + ".sha256")
        curl = fake_bin / "curl"
        curl.write_text(
            "#!/usr/bin/env bash\n"
            "set -euo pipefail\n"
            'destination=""\n'
            'url=""\n'
            'while [[ $# -gt 0 ]]; do\n'
            '  case "$1" in\n'
            '    -o) destination="$2"; shift 2 ;;\n'
            '    -*) shift ;;\n'
            '    *) url="$1"; shift ;;\n'
            '  esac\n'
            'done\n'
            f'printf \'%s\\n\' "$url" >> {shlex.quote(str(urls))}\n'
            'case "$url" in\n'
            f'  *.sha256) cp {shlex.quote(str(checksum))} "$destination" ;;\n'
            f'  *) cp {shlex.quote(str(archive))} "$destination" ;;\n'
            'esac\n'
        )
        curl.chmod(0o755)
        environment = {"PATH": str(fake_bin) + os.pathsep + self.env["PATH"]}
        return urls, environment

    def test_release_install_records_and_reuses_the_release_for_reinstall(self):
        urls, environment = self.fake_release_downloads()
        self.invoke(
            "install", "--version", "0.1.0-beta.1", "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            environment=environment,
        )
        self.assertEqual(self.state_value("VERSION"), "0.1.0")
        self.assertEqual(self.state_value("RELEASE_VERSION"), "0.1.0-beta.1")
        self.assertIn("agena-v0.1.0-beta.1", urls.read_text())

        urls.write_text("")
        self.invoke("reinstall", environment=environment)
        downloaded = urls.read_text()
        self.assertIn("agena-v0.1.0-beta.1", downloaded)
        self.assertIn("agena-backend-", downloaded)
        self.assertEqual(self.state_value("RELEASE_VERSION"), "0.1.0-beta.1")
        self.assertTrue((self.data_dir / "run/stub-running").exists())
        self.assert_only_selected_root_was_called()

    def test_upgrade_selects_the_newest_release_instead_of_the_recorded_one(self):
        urls, environment = self.fake_release_downloads()
        self.invoke(
            "install", "--version", "0.1.0-beta.1", "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            environment=environment,
        )
        urls.write_text("")
        self.invoke("upgrade", "--version", "0.1.0-beta.2", environment=environment)
        downloaded = urls.read_text()
        self.assertIn("agena-v0.1.0-beta.2", downloaded)
        self.assertEqual(self.state_value("RELEASE_VERSION"), "0.1.0-beta.2")

    def test_source_install_uses_current_checkout_without_cloning(self):
        checkout = self.source_checkout()
        fake_bin = self.root / "fake bin"
        fake_bin.mkdir()
        marker = self.root / "git was called"
        git = fake_bin / "git"
        git.write_text(
            "#!/usr/bin/env bash\n"
            f"touch {shlex.quote(str(marker))}\n"
            "exit 99\n"
        )
        git.chmod(0o755)
        result = self.invoke(
            "install", "--source", "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            cwd=checkout,
            environment={"PATH": str(fake_bin) + os.pathsep + self.env["PATH"]},
        )
        self.assertIn(f"Building Agena from source at {checkout}", result.stdout)
        self.assertFalse(marker.exists())
        self.assertTrue((self.install_dir / "bin/agena").is_file())
        self.assert_only_selected_root_was_called()

    def test_source_install_accepts_an_explicit_checkout_path(self):
        checkout = self.source_checkout("other checkout")
        unrelated = self.root / "unrelated cwd"
        unrelated.mkdir()
        result = self.invoke(
            "install", "--source", checkout, "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            cwd=unrelated,
        )
        self.assertIn(f"Building Agena from source at {checkout}", result.stdout)
        self.assertTrue((self.install_dir / "bin/agena").is_file())
        self.assert_only_selected_root_was_called()

    def test_source_install_clones_an_explicit_git_url(self):
        template = self.source_checkout("source template")
        fake_bin = self.root / "clone bin"
        fake_bin.mkdir()
        calls = self.root / "git calls"
        git = fake_bin / "git"
        git.write_text(
            "#!/usr/bin/env bash\n"
            "set -euo pipefail\n"
            'printf \'%s\\n\' "$*" >> "$TEST_GIT_CALLS"\n'
            '[[ "${1:-}" == "clone" ]] || exit 97\n'
            'destination="${!#}"\n'
            'mkdir -p "$destination"\n'
            f'cp -R {shlex.quote(str(template))}/. "$destination"/\n'
        )
        git.chmod(0o755)
        result = self.invoke(
            "install", "--source", "https://example.invalid/agena.git",
            "--data-dir", self.data_dir, "--workspace", self.workspace,
            "--service-mode", "detached",
            environment={
                "PATH": str(fake_bin) + os.pathsep + self.env["PATH"],
                "TEST_GIT_CALLS": str(calls),
            },
        )
        self.assertIn("https://example.invalid/agena.git", calls.read_text())
        self.assertIn("Building Agena from source", result.stdout)
        self.assertTrue((self.install_dir / "bin/agena").is_file())
        self.assert_only_selected_root_was_called()

    def test_source_install_rejects_release_only_options(self):
        checkout = self.source_checkout()
        result = self.invoke(
            "install", "--source", checkout, "--archive", self.archive,
            "--data-dir", self.data_dir, success=False,
        )
        self.assertIn("--source cannot be combined with --archive", result.stdout)
        result = self.invoke(
            "install", "--source", checkout, "--version", "0.1.0-beta.1",
            "--data-dir", self.data_dir, success=False,
        )
        self.assertIn("--version applies to release installs", result.stdout)
        self.assertFalse(self.calls_path.exists())

    def test_path_update_targets_zsh_startup_files_and_preserves_symlinks(self):
        dotfiles = self.home / "dotfiles"
        dotfiles.mkdir()
        zshrc_target = dotfiles / ".zshrc"
        zshrc_target.write_text("export USER_SETTING=keep\n")
        zshrc = self.home / ".zshrc"
        zshrc.symlink_to(zshrc_target)
        result = self.invoke(
            "install", "--archive", self.archive, "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            update_path=True,
            environment={"SHELL": "/bin/zsh"},
        )
        command_path = str(self.install_dir / "bin").replace(" ", "\\ ")
        zshenv = self.home / ".zshenv"
        self.assertIn(f"Added Agena PATH/client environment setup to {zshenv}", result.stdout)
        self.assertIn(f"Added Agena PATH/client environment setup to {zshrc}", result.stdout)
        self.assertIn(command_path, zshenv.read_text())
        self.assertTrue(zshrc.is_symlink())
        self.assertIn("export USER_SETTING=keep", zshrc_target.read_text())
        self.assertIn(command_path, zshrc_target.read_text())
        self.assertFalse((self.home / ".profile").exists())

    def test_path_update_moves_old_profile_block_and_preserves_user_content(self):
        profile = self.home / ".profile"
        profile.write_text(
            "export USER_SETTING=keep\n"
            "# >>> agena installer >>>\n"
            "export PATH=/old/agena/bin:\"$PATH\"\n"
            "# <<< agena installer <<<\n"
        )
        self.invoke(
            "install", "--archive", self.archive, "--data-dir", self.data_dir,
            "--workspace", self.workspace, "--service-mode", "detached",
            update_path=True,
            environment={"SHELL": "/bin/zsh"},
        )
        command_path = str(self.install_dir / "bin").replace(" ", "\\ ")
        self.assertIn("export USER_SETTING=keep", profile.read_text())
        self.assertNotIn("# >>> agena installer >>>", profile.read_text())
        self.assertIn(command_path, (self.home / ".zshenv").read_text())
        self.assertIn(command_path, (self.home / ".zshrc").read_text())

    def test_source_install_rejects_a_directory_that_is_not_a_checkout(self):
        not_source = self.root / "not source"
        not_source.mkdir()
        result = self.invoke(
            "install", "--source", not_source, "--data-dir", self.data_dir,
            success=False,
        )
        self.assertIn("is not an Agena checkout", result.stdout)
        self.assertFalse(self.calls_path.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
