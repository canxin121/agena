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
printf '%s\t%s\n' "$data_root" "$action" >> "$TEST_CALLS_PATH"
case "$action" in
  start|install)
    mkdir -p "$data_root/run"
    printf 'fixture database\n' > "$data_root/agena.db"
    : > "$data_root/run/stub-running"
    echo 'Started stub Agena server.' ;;
  stop|uninstall)
    rm -f "$data_root/run/stub-running"
    echo 'Stopped stub Agena server.' ;;
  status)
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

    def invoke(self, action, *arguments, environment=None, success=True, cwd=None):
        env = dict(self.env)
        env.update(environment or {})
        result = subprocess.run(
            ["bash", str(SCRIPTS / "install.sh"), action,
             "--install-dir", str(self.install_dir), "--no-path-update", *map(str, arguments)],
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

    def test_service_helpers_pin_saved_root_without_invoking_a_system_service_manager(self):
        result = subprocess.run(
            ["bash", str(SCRIPTS / "install-service.sh"), "--archive", str(self.archive),
             "--install-dir", str(self.install_dir), "--data-dir", str(self.data_dir)],
            env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        env = dict(self.env, AGENA_DATA_DIR="/")
        result = subprocess.run(
            ["bash", str(SCRIPTS / "uninstall-service.sh"), "--install-dir", str(self.install_dir), "--remove-install-dir"],
            env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertFalse(self.install_dir.exists())
        self.assertTrue((self.data_dir / "agena.db").is_file())
        self.assertTrue(self.default_sentinel.is_file())
        self.assert_only_selected_root_was_called()

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
