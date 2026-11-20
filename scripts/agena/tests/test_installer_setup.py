"""Offline host preparation tests; all privileged commands and system paths are stubs."""

import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
import zipfile


SCRIPTS = Path(__file__).resolve().parents[1]
BEGIN = "# BEGIN AGENA HOST SETUP"
END = "# END AGENA HOST SETUP"

STUB = r'''
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

name = Path(sys.argv[0]).name
args = sys.argv[1:]
root = Path(os.environ["TEST_ROOT"])
system = root / "system"
bin_dir = root / "bin"
config = json.loads((root / "config.json").read_text())
with (root / "calls.jsonl").open("a") as log:
    log.write(json.dumps([name, *args]) + "\n")

def provide(command):
    destination = bin_dir / command
    if not destination.exists():
        destination.symlink_to(root / "dispatch")

if name == "uname":
    print(config.get("os", "Linux") if args == ["-s"] else config.get("arch", "x86_64"))
elif name == "id":
    print(config.get("uid", 1000))
elif name in ("sudo", "doas"):
    if args == ["-n", "true"]:
        sys.exit(0 if config.get(name + "_cached", True) else 1)
    while args and args[0] in ("-n", "--"):
        args.pop(0)
    sys.exit(subprocess.run(args).returncode)
elif name == "su":
    assert args[:3] == ["-s", "/bin/sh", "-c"] and args[-1] == "root"
    sys.exit(subprocess.run(["/bin/sh", "-c", args[3]]).returncode)
elif name in ("apt-get", "apt", "dnf", "dnf5", "yum", "pacman", "zypper", "apk", "brew"):
    verb = next((item for item in ("install", "add", "-S") if item in args), None)
    if verb:
        mapping = {
            "bubblewrap": ["bwrap"], "slirp4netns": ["slirp4netns"],
            "iproute2": ["ip"], "iproute": ["ip"],
            "iptables": ["iptables", "ip6tables", "iptables-restore", "ip6tables-restore"],
            "iptables-nft": ["iptables", "ip6tables", "iptables-restore", "ip6tables-restore"],
            "util-linux": ["unshare", "nsenter"], "util-linux-misc": ["unshare", "nsenter"], "kmod": ["modprobe"],
            "apparmor": ["apparmor_parser"], "apparmor-parser": ["apparmor_parser"],
            "build-essential": ["cc", "c++", "make"], "build-base": ["cc", "c++", "make"],
            "gcc": ["cc", "c++"], "gcc-c++": ["c++"],
            "python": ["python3"], "coreutils": ["sha256sum"], "pkgconf": ["pkg-config"],
        }
        for package in args[args.index(verb) + 1:]:
            if package.startswith("-"):
                continue
            if package == "ca-certificates":
                ca = system / "etc/ssl/certs/ca-certificates.crt"
                ca.parent.mkdir(parents=True, exist_ok=True)
                ca.write_text("fixture CA\n")
            elif package in ("libssl-dev", "openssl-devel", "libopenssl-devel", "openssl-dev", "openssl"):
                if not config.get("openssl_install_failure"):
                    (root / "openssl-installed").touch()
            else:
                for command in mapping.get(package, [package]):
                    provide(command)
            if package in ("util-linux", "util-linux-misc"):
                (root / "util-linux-installed").touch()
            if package in ("iproute", "iproute2"):
                (root / "iproute-installed").touch()
            if package in ("python", "python3"):
                (root / "python-installed").touch()
            if package in ("apparmor", "apparmor-abstractions", "apparmor-profiles-tunables"):
                for suffix in ("abi/4.0", "tunables/global"):
                    target = system / "etc/apparmor.d" / suffix
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_text("fixture policy\n")
elif name == "python3":
    sys.exit(0 if config.get("python_works", True) or (root / "python-installed").exists() else 1)
elif name == "unshare":
    assert args == ["--help"]
    print("BusyBox unshare" if config.get("busybox_util_linux") and not (root / "util-linux-installed").exists()
          else "unshare from util-linux: --map-current-user --keep-caps")
elif name == "nsenter":
    assert args == ["--version"]
    print("BusyBox nsenter" if config.get("busybox_util_linux") and not (root / "util-linux-installed").exists()
          else "nsenter from util-linux 2.40")
elif name == "ip":
    assert args == ["-Version"]
    print("BusyBox ip" if config.get("busybox_ip") and not (root / "iproute-installed").exists()
          else "ip utility, iproute2-6.17.0")
elif name == "getconf":
    if config.get("libc", "gnu") == "gnu": print("glibc 2.40")
    else: sys.exit(1)
elif name == "ldd":
    print("musl libc 1.2.5" if config.get("libc") == "musl" else "ldd (GNU libc) 2.40")
elif name == "rc-update":
    assert args == ["add", "modules", "boot"]
    target = system / "etc/runlevels/boot/modules"
    target.parent.mkdir(parents=True, exist_ok=True)
    target.symlink_to(system / "etc/init.d/modules")
elif name in ("iptables", "ip6tables", "iptables-restore", "ip6tables-restore"):
    if args != ["--version"]:
        raise AssertionError("The installer attempted to alter firewall rules")
    print(f"{name} v1.8.10 ({config.get('iptables', 'nf_tables')})")
elif name == "modprobe":
    assert args == ["nf_conntrack"]
    if config.get("module_failure"):
        sys.exit(1)
    (system / "sys/module/nf_conntrack").mkdir(parents=True, exist_ok=True)
elif name == "install":
    directories = "-d" in args
    positional = []
    index = 0
    while index < len(args):
        if args[index] in ("-o", "-g", "-m"):
            index += 2
        elif args[index] == "-d":
            index += 1
        else:
            positional.append(args[index])
            index += 1
    if directories:
        for path in positional:
            Path(path).mkdir(parents=True, exist_ok=True)
    else:
        shutil.copyfile(*positional)
        Path(positional[-1]).chmod(0o755 if "0755" in args else 0o644)
elif name == "apparmor_parser":
    if config.get("parser_failure"):
        sys.exit(17)
    assert args[0] == "-r"
    active = system / "sys/kernel/security/apparmor/profiles"
    active.parent.mkdir(parents=True, exist_ok=True)
    active.write_text(Path(args[-1]).name + " (unconfined)\n")
elif name == "sw_vers":
    print(config.get("macos", "14.6"))
elif name == "xcode-select":
    if args == ["-p"]:
        ready = config.get("clt", True) or (root / "clt-installed").exists()
        if not ready and (root / "clt-requested").exists() and config.get("clt_install_completes", True):
            count_file = root / "clt-polls"
            polls = int(count_file.read_text()) + 1 if count_file.exists() else 1
            count_file.write_text(str(polls))
            if polls >= config.get("clt_ready_after_polls", 1):
                (root / "clt-installed").touch()
                ready = True
        if not ready:
            sys.exit(2)
        print("/Library/Developer/CommandLineTools")
    else:
        assert args == ["--install"]
        if config.get("clt_install_failure"):
            sys.exit(1)
        (root / "clt-requested").touch()
elif name == "pkg-config":
    assert args == ["--exists", "openssl"]
    sys.exit(0 if config.get("openssl", True) or (root / "openssl-installed").exists() else 1)
elif name == "rustup":
    installed = root / "rust-installed"
    if args and args[0] == "which":
        if not installed.exists() or installed.read_text() != args[2]:
            sys.exit(1)
        print(str(bin_dir / args[-1]))
    elif args[:2] == ["toolchain", "install"]:
        installed.write_text(args[2])
    else:
        raise AssertionError(f"unexpected rustup invocation {args}")
elif name == "bun":
    print(config.get("bun", "1.3.14"))
elif name == "curl":
    assert "--proto" in args and args[args.index("--proto") + 1] == "=https"
    assert "--proto-redir" in args and args[args.index("--proto-redir") + 1] == "=https"
    url = args[-1]
    assert url.startswith(("https://github.com/oven-sh/bun/", "https://static.rust-lang.org/rustup/",
                           "https://raw.githubusercontent.com/Homebrew/install/"))
    asset = root / "downloads" / url.rsplit("/", 1)[-1]
    if not asset.exists():
        sys.exit(22)
    shutil.copyfile(asset, args[args.index("--output") + 1])
else:
    pass
'''


def setup_source():
    text = (SCRIPTS / "install.sh").read_text()
    if BEGIN in text and END in text:
        embedded = text.split(BEGIN, 1)[1].split(END, 1)[0]
        if "agena_setup_host()" in embedded:
            return embedded
    # Development-only fallback; the delivered installer has one embedded implementation.
    return (SCRIPTS / "installer-setup.sh").read_text()


class InstallerSetupTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="agena-offline-host-setup-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name).resolve()
        self.bin = self.root / "bin"
        self.system = self.root / "system"
        self.home = self.root / "home"
        self.checkout = self.root / "source"
        self.install = self.root / "application files"
        for path in [self.bin, self.system, self.home, self.checkout, self.root / "tmp", self.root / "downloads"]:
            path.mkdir()
        (self.root / "dispatch").write_text(f"#!{sys.executable}\n{STUB}")
        (self.root / "dispatch").chmod(0o755)
        self.configuration = {}
        self.configure()
        self.real_tools("awk", "basename", "cat", "chmod", "dirname", "grep", "head", "mktemp",
                        "mkdir", "rm", "sed", "sha256sum", "tar", "tr", "unzip", "sleep", "ln",
                        "tee", "tail", "touch", "bash")
        self.provide("uname", "id", "sudo", "install", "apt-get", "curl", "python3", "git",
                     "bwrap", "slirp4netns", "ip", "unshare", "nsenter", "iptables", "ip6tables",
                     "iptables-restore", "ip6tables-restore", "modprobe", "apparmor_parser",
                     "cc", "c++", "make", "cmake", "pkg-config", "rustup", "bun", "sw_vers", "xcode-select")
        self.write_system("/etc/ssl/certs/ca-certificates.crt", "fixture CA\n")
        self.write_system("/etc/os-release", 'ID=ubuntu\nID_LIKE=debian\n')
        self.write_system("/etc/apparmor.d/abi/4.0", "fixture ABI\n")
        self.write_system("/etc/apparmor.d/tunables/global", "fixture tunables\n")
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "0\n")
        self.write_system("/proc/sys/kernel/unprivileged_userns_clone", "1\n")
        self.write_system("/proc/sys/user/max_user_namespaces", "10000\n")
        (self.system / "sys/module/nf_conntrack").mkdir(parents=True)
        self.write_system("/etc/modules-load.d/agena-sandbox.conf",
                          "# Managed by Agena installer.\nnf_conntrack\n")
        (self.root / "rust-installed").write_text("1.97.0")
        (self.checkout / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.97.0"\n')
        (self.checkout / ".bun-version").write_text("1.3.14\n")
        self.env = {
            "PATH": str(self.bin),
            "HOME": str(self.home),
            "TMPDIR": str(self.root / "tmp"),
            "TEST_ROOT": str(self.root),
            "LC_ALL": "C",
        }

    def configure(self, **values):
        self.configuration.update(values)
        (self.root / "config.json").write_text(json.dumps(self.configuration))

    def provide(self, *names):
        for name in names:
            path = self.bin / name
            if path.exists():
                path.unlink()
            path.symlink_to(self.root / "dispatch")

    def real_tools(self, *names):
        for name in names:
            actual = shutil.which(name)
            if not actual:
                self.skipTest(f"{name} is required to run these offline tests")
            (self.bin / name).symlink_to(actual)

    def write_system(self, path, text):
        target = self.system / path.lstrip("/")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text)
        return target

    def calls(self, name=None):
        path = self.root / "calls.jsonl"
        calls = [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        return [call for call in calls if name is None or call[0] == name]

    def run_setup(self, body, expected=0):
        source = setup_source()
        if getattr(self, "homebrew_checksum", None):
            source = source.replace(
                "5f333bbe53bc490e51e7ccb1df8779b3dd6ee73a1a7379efda216edb08ccb148",
                self.homebrew_checksum,
            )
        script = source + '\nagena_setup_system_path() { printf "%s%s" "$TEST_ROOT/system" "$1"; }\n' + body
        result = subprocess.run(
            [os.environ.get("AGENA_TEST_BASH", "/bin/bash"), "-eu", "-o", "pipefail", "-c", script],
            env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            start_new_session=True,
        )
        if expected is not None:
            self.assertEqual(result.returncode, expected, result.stdout + result.stderr)
        return result

    def host(self, phase="runtime", mode="auto"):
        return f"agena_setup_host {phase} {mode} {shlex.quote(str(self.install))} {shlex.quote(str(self.checkout))}"

    def profile_path(self):
        executable = f"{self.install.resolve()}/bin/agena"
        digest = hashlib.sha256(executable.encode()).hexdigest()[:24]
        return self.system / f"etc/apparmor.d/agena-userns-{digest}"

    def test_complete_runtime_check_does_not_modify_host(self):
        persistent = self.system / "etc/modules-load.d/agena-sandbox.conf"
        before = persistent.read_bytes()
        result = self.run_setup(self.host(mode="check"))
        self.assertIn("runtime host prerequisites are ready", result.stderr)
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("apt-get"), [])
        self.assertEqual(self.calls("modprobe"), [])
        self.assertEqual(persistent.read_bytes(), before)
        self.assertEqual(self.calls("install"), [])

    def test_check_missing_commands_reports_without_installing(self):
        (self.bin / "slirp4netns").unlink()
        result = self.run_setup(self.host(mode="check"), expected=1)
        self.assertIn("slirp4netns", result.stderr)
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("apt-get"), [])

    def test_auto_installs_only_missing_packages_and_updates_apt_once(self):
        for name in ["git", "bwrap", "slirp4netns", "ip"]:
            (self.bin / name).unlink()
        self.run_setup("agena_setup_bootstrap auto 1\n" + self.host())
        packages = self.calls("apt-get")
        self.assertEqual(packages[0], ["apt-get", "update"])
        self.assertEqual(sum(call == ["apt-get", "update"] for call in packages), 1)
        self.assertEqual(packages[1], ["apt-get", "install", "-y", "--no-install-recommends", "git"])
        self.assertEqual(packages[2], ["apt-get", "install", "-y", "--no-install-recommends",
                                      "bubblewrap", "slirp4netns", "iproute2"])

    def test_bootstrap_installs_ca_bundle_only_when_missing(self):
        (self.system / "etc/ssl/certs/ca-certificates.crt").unlink()
        self.run_setup("agena_setup_bootstrap auto 0")
        self.assertEqual(self.calls("apt-get")[-1][-1], "ca-certificates")

    def test_legacy_iptables_is_diagnosed_without_changing_firewall(self):
        self.configure(iptables="legacy")
        result = self.run_setup(self.host(), expected=1)
        self.assertIn("legacy backend", result.stderr)
        self.assertIn("does not change firewall rules or alternatives", result.stderr)
        self.assertEqual(self.calls("sudo"), [])

    def test_disabled_user_namespaces_are_not_relaxed(self):
        path = self.write_system("/proc/sys/kernel/unprivileged_userns_clone", "0\n")
        result = self.run_setup(self.host(), expected=1)
        self.assertIn("does not weaken global namespace restrictions", result.stderr)
        self.assertEqual(path.read_text(), "0\n")
        self.assertEqual(self.calls("sudo"), [])

    def test_module_is_loaded_once_and_persisted_when_missing(self):
        (self.system / "sys/module/nf_conntrack").rmdir()
        (self.system / "etc/modules-load.d/agena-sandbox.conf").unlink()
        self.run_setup(self.host() + "\n" + self.host())
        self.assertEqual(self.calls("modprobe"), [["modprobe", "nf_conntrack"]])
        config = self.system / "etc/modules-load.d/agena-sandbox.conf"
        self.assertEqual(config.read_text(), "# Managed by Agena installer.\nnf_conntrack\n")

    def test_missing_module_in_check_mode_is_read_only(self):
        (self.system / "sys/module/nf_conntrack").rmdir()
        self.run_setup(self.host(mode="check"), expected=1)
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("modprobe"), [])

    def test_apparmor_uses_exact_final_path_before_binary_exists(self):
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
        self.assertFalse(self.install.exists())
        self.run_setup(self.host())
        text = self.profile_path().read_text()
        self.assertIn(f'"{self.install}/bin/agena" flags=(unconfined)', text)
        self.assertIn("  userns,", text)
        self.assertNotIn("/**", text)
        self.assertFalse(self.install.exists())
        self.assertEqual(self.calls("apparmor_parser"), [["apparmor_parser", "-r", str(self.profile_path())]])

    def test_apparmor_does_not_follow_a_replaced_bin_directory_symlink(self):
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
        self.install.mkdir()
        prior = self.root / "old-bin-target"
        prior.mkdir()
        (self.install / "bin").symlink_to(prior, target_is_directory=True)
        self.run_setup(self.host())
        self.assertIn(f'"{self.install}/bin/agena"', self.profile_path().read_text())
        self.assertNotIn(str(prior), self.profile_path().read_text())

    def test_apparmor_check_neither_creates_nor_loads_profile(self):
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
        self.run_setup(self.host(mode="check"), expected=1)
        self.assertFalse(self.profile_path().exists())
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("apparmor_parser"), [])

    def test_apparmor_check_defers_to_sandbox_when_profile_listing_cannot_be_read(self):
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
        self.run_setup(self.host())
        active = self.system / "sys/kernel/security/apparmor/profiles"
        active.write_text("unrelated-profile (enforce)\n")
        result = self.run_setup(self.host(mode="check"), expected=1)
        self.assertIn("exists but is not loaded", result.stderr)

        active.unlink()
        active.mkdir()
        result = self.run_setup(self.host(mode="check"))
        self.assertIn("will verify actual sandbox execution", result.stderr)
        self.assertEqual(len(self.calls("apparmor_parser")), 1)

    def test_preserves_unmanaged_apparmor_file(self):
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
        path = self.profile_path()
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text("# administrator owns this file\n")
        result = self.run_setup(self.host(), expected=1)
        self.assertIn("Preserving existing administrator configuration", result.stderr)
        self.assertEqual(path.read_text(), "# administrator owns this file\n")
        self.assertEqual(self.calls("sudo"), [])

    def test_profile_path_metacharacters_are_rejected(self):
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
        self.install = self.root / "unsafe*attachment"
        result = self.run_setup(self.host(), expected=1)
        self.assertIn("unsupported pattern/control characters", result.stderr)
        self.assertEqual(self.calls("sudo"), [])

    def test_profile_load_error_prevents_success(self):
        self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
        self.configure(parser_failure=True)
        result = self.run_setup(self.host(), expected=1)
        self.assertIn("Loading the Agena-specific AppArmor profile failed", result.stderr)

    def test_noninteractive_sudo_failure_has_actionable_message(self):
        (self.bin / "slirp4netns").unlink()
        self.configure(sudo_cached=False)
        result = self.run_setup(self.host(), expected=1)
        self.assertIn("interactive terminal", result.stderr)
        self.assertIn("sudo -v", result.stderr)
        self.assertEqual(self.calls("apt-get"), [])

    def test_root_is_not_used_to_install_user_toolchains(self):
        self.configure(uid=0)
        result = self.run_setup(self.host(phase="source"), expected=1)
        self.assertIn("normal user, without sudo", result.stderr)
        self.assertEqual(self.calls("rustup"), [])

    def test_source_check_does_not_download_a_missing_toolchain(self):
        (self.root / "rust-installed").unlink()
        result = self.run_setup(self.host(phase="source", mode="check"), expected=1)
        self.assertIn("Rust toolchain 1.97.0 is missing", result.stderr)
        self.assertEqual(self.calls("curl"), [])
        self.assertFalse(any(call[1:3] == ["toolchain", "install"] for call in self.calls("rustup")))

    def test_source_installs_the_checkout_rust_pin_without_sudo(self):
        (self.checkout / "rust-toolchain.toml").write_text('[toolchain]\nchannel = "1.98.2"\n')
        self.run_setup(self.host(phase="source"))
        self.assertIn(["rustup", "toolchain", "install", "1.98.2", "--profile", "minimal",
                       "--component", "rustfmt", "--component", "clippy"], self.calls("rustup"))
        self.assertEqual(self.calls("sudo"), [])

    def test_bun_download_is_pinned_verified_and_user_owned(self):
        self.configure(bun="1.0.0")
        downloads = self.root / "downloads"
        archive = downloads / "bun-linux-x64-baseline.zip"
        with zipfile.ZipFile(archive, "w") as package:
            package.writestr("bun-linux-x64-baseline/bun", "#!/bin/bash\nprintf '1.3.14\\n'\n")
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        (downloads / "SHASUMS256.txt").write_text(f"{checksum}  {archive.name}\n")
        self.run_setup(self.host(phase="source"))
        binary = self.home / ".cache/agena/toolchains/bun-1.3.14/bin/bun"
        self.assertTrue(binary.is_file())
        self.assertTrue(all("bun-v1.3.14/" in call[-1] for call in self.calls("curl")))
        self.assertEqual(self.calls("sudo"), [])

    def test_bun_checksum_failure_does_not_execute_or_install_download(self):
        self.configure(bun="1.0.0")
        downloads = self.root / "downloads"
        archive = downloads / "bun-linux-x64-baseline.zip"
        archive.write_bytes(b"unverified bytes must not run")
        (downloads / "SHASUMS256.txt").write_text(f"{'0' * 64}  {archive.name}\n")
        result = self.run_setup(self.host(phase="source"), expected=1)
        self.assertIn("SHA256 verification failed", result.stderr)
        self.assertFalse((self.home / ".cache/agena/toolchains").exists())
        self.assertEqual(self.calls("sudo"), [])

    def test_macos_clt_wait_has_a_bounded_deadline(self):
        self.configure(os="Darwin", clt=False, clt_install_completes=False)
        self.env["AGENA_SETUP_CLT_TIMEOUT_SECONDS"] = "1"
        result = self.run_setup(self.host(phase="source"), expected=1)
        self.assertIn("did not finish within 1 seconds", result.stderr)
        self.assertIn(["xcode-select", "--install"], self.calls("xcode-select"))
        self.assertEqual(self.calls("rustup"), [])

    def test_macos_clt_check_does_not_open_installer(self):
        self.configure(os="Darwin", clt=False)
        self.run_setup(self.host(phase="source", mode="check"), expected=1)
        self.assertNotIn(["xcode-select", "--install"], self.calls("xcode-select"))

    def test_macos_brew_only_installs_missing_build_tools_without_sudo(self):
        self.configure(os="Darwin")
        self.provide("brew")
        (self.bin / "cmake").unlink()
        self.run_setup(self.host(phase="source"))
        self.assertEqual(self.calls("brew"), [["brew", "install", "cmake"]])
        self.assertEqual(self.calls("sudo"), [])

    def test_macos_runtime_reports_real_privacy_settings_without_grant_commands(self):
        self.configure(os="Darwin")
        result = self.run_setup(self.host(mode="check"))
        self.assertIn("System Settings > Privacy & Security", result.stderr)
        self.assertIn("Accessibility", result.stderr)
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("xcode-select"), [])

    def test_source_installs_missing_openssl_development_files(self):
        self.configure(openssl=False)
        self.run_setup(self.host(phase="source"))
        self.assertIn(["apt-get", "install", "-y", "--no-install-recommends", "libssl-dev"],
                      self.calls("apt-get"))
        self.assertEqual(self.calls("pkg-config"),
                         [["pkg-config", "--exists", "openssl"]] * 2)

    def test_source_check_reports_missing_openssl_without_installing(self):
        self.configure(openssl=False)
        result = self.run_setup(self.host(phase="source", mode="check"), expected=1)
        self.assertIn("openssl-dev", result.stderr)
        self.assertEqual(self.calls("apt-get"), [])
        self.assertEqual(self.calls("sudo"), [])

    def test_openssl_install_is_rechecked_before_building(self):
        self.configure(openssl=False, openssl_install_failure=True)
        result = self.run_setup(self.host(phase="source"), expected=1)
        self.assertIn("still unavailable to pkg-config", result.stderr)
        self.assertEqual(self.calls("rustup"), [])

    def test_openssl_development_package_names_match_each_distribution(self):
        expected = {
            "apt-get": "libssl-dev", "dnf": "openssl-devel", "yum": "openssl-devel",
            "zypper": "libopenssl-devel", "pacman": "openssl", "apk": "openssl-dev",
        }
        for manager, package in expected.items():
            with self.subTest(manager=manager):
                result = self.run_setup(f"agena_setup_package_name {manager} openssl-dev")
                self.assertEqual(result.stdout, package)

    def test_loaded_conntrack_is_also_persisted_for_reboot(self):
        persistent = self.system / "etc/modules-load.d/agena-sandbox.conf"
        persistent.unlink()
        self.run_setup(self.host())
        self.assertEqual(persistent.read_text(), "# Managed by Agena installer.\nnf_conntrack\n")
        self.assertEqual(self.calls("modprobe"), [])

    def test_conntrack_persistence_check_is_read_only(self):
        persistent = self.system / "etc/modules-load.d/agena-sandbox.conf"
        persistent.unlink()
        result = self.run_setup(self.host(mode="check"), expected=1)
        self.assertIn("configuration is missing", result.stderr)
        self.assertFalse(persistent.exists())
        self.assertEqual(self.calls("sudo"), [])

    def test_existing_conntrack_entry_without_marker_is_preserved(self):
        persistent = self.system / "etc/modules-load.d/agena-sandbox.conf"
        for content in ("nf_conntrack\n\n", "# administrator configuration\n\tnf_conntrack \t\nother_module\n"):
            with self.subTest(content=content):
                persistent.write_text(content)
                before = persistent.stat()
                for mode in ("auto", "check"):
                    self.run_setup(self.host(mode=mode) + "\n" + self.host(mode=mode))
                self.assertEqual(persistent.read_text(), content)
                self.assertEqual(persistent.stat().st_ino, before.st_ino)
                self.assertEqual(persistent.stat().st_mtime_ns, before.st_mtime_ns)
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("install"), [])

    def test_unmanaged_module_configuration_without_conntrack_is_preserved_and_rejected(self):
        persistent = self.system / "etc/modules-load.d/agena-sandbox.conf"
        for content in ("# nf_conntrack\n", "; nf_conntrack\n", "nf_conntrack_helper\n"):
            with self.subTest(content=content):
                persistent.write_text(content)
                for mode in ("auto", "check"):
                    result = self.run_setup(self.host(mode=mode), expected=1)
                    self.assertIn("Preserving existing administrator configuration", result.stderr)
                    self.assertEqual(persistent.read_text(), content)
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("install"), [])

    def test_conntrack_configuration_symlink_is_rejected(self):
        persistent = self.system / "etc/modules-load.d/agena-sandbox.conf"
        target = self.root / "administrator-modules.conf"
        content = "nf_conntrack\n"
        target.write_text(content)
        persistent.unlink()
        persistent.symlink_to(target)
        for mode in ("auto", "check"):
            with self.subTest(mode=mode):
                result = self.run_setup(self.host(mode=mode), expected=1)
                self.assertIn("Refusing a symlink", result.stderr)
                self.assertTrue(persistent.is_symlink())
                self.assertEqual(target.read_text(), content)
        self.assertEqual(self.calls("sudo"), [])
        self.assertEqual(self.calls("install"), [])

    def test_failed_config_write_cannot_install_a_partial_file(self):
        target = self.system / "etc/agena-test.conf"
        body = r'''
printf() {
  if [[ "${1-}" == '%s\n' && "${2-}" == 'fixture-body' ]]; then return 37; fi
  builtin printf "$@"
}
if agena_setup_managed_file "$TEST_ROOT/system/etc/agena-test.conf" 'fixture-body' auto; then
  exit 99
fi
'''
        result = self.run_setup(body)
        self.assertIn("Could not prepare system configuration", result.stderr)
        self.assertFalse(target.exists())
        self.assertEqual(self.calls("install"), [])
        self.assertEqual(list((self.root / "tmp").iterdir()), [])

    def test_macos_clt_install_continues_without_rerunning_command(self):
        self.configure(os="Darwin", clt=False, clt_ready_after_polls=2)
        self.env["AGENA_SETUP_CLT_TIMEOUT_SECONDS"] = "5"
        result = self.run_setup(self.host(phase="source"))
        self.assertIn("continues automatically", result.stderr)
        self.assertEqual(self.calls("xcode-select").count(["xcode-select", "--install"]), 1)
        self.assertTrue((self.root / "clt-installed").exists())
        self.assertNotEqual(self.calls("rustup"), [])

    def test_macos_clt_wait_rejects_invalid_deadline(self):
        self.configure(os="Darwin", clt=False)
        self.env["AGENA_SETUP_CLT_TIMEOUT_SECONDS"] = "99999999999999999999999"
        result = self.run_setup(self.host(phase="source"), expected=1)
        self.assertIn("must be an integer", result.stderr)
        self.assertNotIn(["xcode-select", "--install"], self.calls("xcode-select"))

    def prepare_homebrew_download(self, corrupt=False):
        installer = self.root / "downloads/install.sh"
        payload = r'''#!/bin/bash
mkdir -p "$TEST_ROOT/system/opt/homebrew/bin"
ln -s "$TEST_ROOT/dispatch" "$TEST_ROOT/system/opt/homebrew/bin/brew"
printf '%s\n' "${NONINTERACTIVE:-interactive}" > "$TEST_ROOT/homebrew-executed"
'''
        self.homebrew_checksum = hashlib.sha256(payload.encode()).hexdigest()
        installer.write_text(payload + ("\n# changed artifact\n" if corrupt else ""))

    def test_macos_bootstraps_verified_homebrew_and_continues_package_install(self):
        self.configure(os="Darwin", arch="arm64")
        self.prepare_homebrew_download()
        (self.bin / "cmake").unlink()
        self.run_setup(self.host(phase="source"))
        self.assertEqual((self.root / "homebrew-executed").read_text(), "1\n")
        self.assertEqual(self.calls("brew"), [["brew", "install", "cmake"]])
        self.assertTrue((self.bin / "cmake").exists())
        self.assertEqual(self.calls("sudo"), [])
        download = self.calls("curl")[0][-1]
        self.assertIn("/09c62fc577ec170172b0a184060f141a2c622dc1/install.sh", download)
        self.assertEqual(list((self.root / "tmp").iterdir()), [])

    def test_macos_homebrew_checksum_failure_never_executes_the_artifact(self):
        self.configure(os="Darwin")
        self.prepare_homebrew_download(corrupt=True)
        (self.bin / "cmake").unlink()
        result = self.run_setup(self.host(phase="source"), expected=1)
        self.assertIn("SHA256 verification failed", result.stderr)
        self.assertFalse((self.root / "homebrew-executed").exists())
        self.assertEqual(self.calls("brew"), [])
        self.assertEqual(self.calls("sudo"), [])

    def test_macos_finds_existing_homebrew_outside_path(self):
        self.configure(os="Darwin")
        brew = self.system / "opt/homebrew/bin/brew"
        brew.parent.mkdir(parents=True)
        brew.symlink_to(self.root / "dispatch")
        (self.bin / "cmake").unlink()
        self.run_setup(self.host(phase="source"))
        self.assertEqual(self.calls("curl"), [])
        self.assertEqual(self.calls("brew"), [["brew", "install", "cmake"]])

    def test_macos_homebrew_check_does_not_download_or_install(self):
        self.configure(os="Darwin")
        self.run_setup("agena_setup_homebrew check", expected=1)
        self.assertEqual(self.calls("curl"), [])
        self.assertEqual(self.calls("brew"), [])
        self.assertEqual(self.calls("sudo"), [])

    def test_os_release_selects_the_native_package_manager(self):
        cases = [
            ("debian", "", "apt-get"), ("ubuntu", "debian", "apt"),
            ("fedora", "", "dnf"), ("fedora", "", "dnf5"), ("centos", "rhel fedora", "yum"),
            ("arch", "", "pacman"), ("opensuse-tumbleweed", "suse opensuse", "zypper"),
            ("alpine", "", "apk"),
        ]
        managers = ("apt-get", "apt", "dnf", "dnf5", "yum", "pacman", "zypper", "apk")
        for identity, like, manager in cases:
            with self.subTest(manager=manager):
                for name in managers:
                    (self.bin / name).unlink(missing_ok=True)
                self.provide(manager)
                if manager not in ("apt-get", "apt"):
                    self.provide("apt-get")  # A foreign tool on PATH must not win.
                self.write_system("/etc/os-release", f'ID={identity}\nID_LIKE="{like}"\n')
                (self.bin / "slirp4netns").unlink()
                self.run_setup(self.host())
                self.assertIn("slirp4netns", self.calls(manager)[-1])

    def test_distribution_specific_package_names_cover_native_tools(self):
        rows = {
            "apt-get": ("iproute2", "iptables", "util-linux", "apparmor", "apparmor", "pkg-config", "build-essential"),
            "dnf": ("iproute", "iptables-nft", "util-linux", "apparmor", "apparmor", "pkgconf-pkg-config", "gcc-c++"),
            "pacman": ("iproute2", "iptables", "util-linux", "apparmor", "apparmor", "pkgconf", "gcc"),
            "zypper": ("iproute2", "iptables", "util-linux", "apparmor-parser", "apparmor-abstractions", "pkg-config", "gcc-c++"),
            "apk": ("iproute2", "iptables", "util-linux-misc", "apparmor", "apparmor-profiles-tunables", "pkgconf", "build-base"),
        }
        tools = ("ip", "iptables", "unshare", "apparmor_parser", "apparmor-policy", "pkg-config", "c++")
        for manager, packages in rows.items():
            with self.subTest(manager=manager):
                script = "\n".join(f"agena_setup_package_name {manager} {tool}; printf '\\n'" for tool in tools)
                self.assertEqual(self.run_setup(script).stdout.splitlines(), list(packages))

    def test_unknown_or_immutable_systems_do_not_use_foreign_package_managers(self):
        for identity in ("nixos", "opensuse-microos", "unsupported-system"):
            with self.subTest(identity=identity):
                self.write_system("/etc/os-release", f"ID={identity}\n")
                result = self.run_setup("agena_setup_packages auto Linux slirp4netns", expected=1)
                self.assertIn("configuration" if identity != "unsupported-system" else "Unsupported distribution",
                              result.stderr)
        self.write_system("/etc/os-release", "ID=fedora\n")
        self.write_system("/run/ostree-booted", "")
        self.provide("dnf")
        self.run_setup("agena_setup_packages auto Linux slirp4netns", expected=1)
        self.assertEqual(self.calls("apt-get"), [])
        self.assertEqual(self.calls("dnf"), [])

    def test_os_release_is_parsed_as_data_instead_of_sourced(self):
        self.write_system("/etc/os-release", 'ID="$(/bin/touch ' + str(self.root / "injected") + ')"\n')
        self.run_setup("agena_setup_packages auto Linux slirp4netns", expected=1)
        self.assertFalse((self.root / "injected").exists())
        self.assertEqual(self.calls("sudo"), [])

    def test_alpine_replaces_busybox_namespace_and_ip_tools(self):
        self.write_system("/etc/os-release", "ID=alpine\n")
        self.provide("apk")
        self.configure(busybox_util_linux=True, busybox_ip=True)
        self.run_setup(self.host())
        packages = self.calls("apk")[-1]
        self.assertIn("util-linux-misc", packages)
        self.assertIn("iproute2", packages)
        self.assertEqual(packages.count("util-linux-misc"), 1)
        self.assertTrue((self.root / "util-linux-installed").exists())
        self.assertEqual(self.calls("apt-get"), [])

    def test_busybox_namespace_tools_fail_readonly_check(self):
        self.configure(busybox_util_linux=True)
        result = self.run_setup(self.host(mode="check"), expected=1)
        self.assertIn("unshare", result.stderr)
        self.assertIn("nsenter", result.stderr)
        self.assertEqual(self.calls("apt-get"), [])

    def test_apparmor_policy_files_are_installed_from_the_native_package(self):
        for identity, manager, expected in [
            ("alpine", "apk", "apparmor-profiles-tunables"),
            ("opensuse-tumbleweed", "zypper", "apparmor-abstractions"),
        ]:
            with self.subTest(identity=identity):
                self.write_system("/etc/os-release", f"ID={identity}\n")
                self.provide(manager)
                self.write_system("/proc/sys/kernel/apparmor_restrict_unprivileged_userns", "1\n")
                (self.system / "etc/apparmor.d/abi/4.0").unlink()
                self.run_setup(self.host())
                self.assertIn(expected, self.calls(manager)[-1])

    def test_openrc_module_service_is_enabled_once_without_starting_it(self):
        self.write_system("/etc/init.d/modules", "fixture OpenRC service\n")
        self.provide("rc-update")
        self.run_setup(self.host() + "\n" + self.host())
        self.assertEqual(self.calls("rc-update"), [["rc-update", "add", "modules", "boot"]])

    def test_openrc_module_boot_check_does_not_enable_service(self):
        self.write_system("/etc/init.d/modules", "fixture OpenRC service\n")
        self.provide("rc-update")
        self.run_setup(self.host(mode="check"), expected=1)
        self.assertEqual(self.calls("rc-update"), [])
        self.assertEqual(self.calls("sudo"), [])

    def test_doas_preserves_argv_without_a_shell(self):
        (self.bin / "sudo").unlink()
        self.provide("doas", "capture")
        values = ["argument with spaces", "'quoted';$(not-a-command)", "line\nbreak"]
        self.run_setup("agena_setup_as_root capture " + " ".join(map(shlex.quote, values)))
        self.assertEqual(self.calls("capture"), [["capture", *values]])
        self.assertEqual(self.calls("doas")[0], ["doas", "-n", "true"])

    def test_su_command_quotes_spaces_metacharacters_and_newlines(self):
        (self.bin / "sudo").unlink()
        self.provide("su", "capture")
        values = ["argument with spaces", "'; /bin/touch " + str(self.root / "injected") + "; '",
                  "$(echo substituted)", "line\nbreak", ""]
        script = "agena_setup_has_tty() { return 0; }\nagena_setup_from_tty() { \"$@\"; }\n"
        self.run_setup(script + "agena_setup_as_root capture " + " ".join(map(shlex.quote, values)))
        self.assertEqual(self.calls("capture"), [["capture", *values]])
        self.assertFalse((self.root / "injected").exists())
        self.assertEqual(len(self.calls("su")), 1)

    def test_su_without_a_terminal_never_requests_a_password(self):
        (self.bin / "sudo").unlink()
        self.provide("su")
        result = self.run_setup("agena_setup_as_root install fixture", expected=1)
        self.assertIn("root password", result.stderr)
        self.assertEqual(self.calls("su"), [])

    def test_broken_python_command_does_not_count_as_a_ready_runtime(self):
        self.configure(python_works=False)
        self.run_setup("agena_setup_bootstrap auto 0")
        self.assertIn("python3", self.calls("apt-get")[-1])
        self.assertTrue((self.root / "python-installed").exists())

    def test_native_musl_target_and_rustup_host_are_selected(self):
        self.write_system("/etc/os-release", "ID=alpine\n")
        result = self.run_setup("agena_setup_target_triple\n" + self.host(phase="source"))
        self.assertTrue(result.stdout.startswith("x86_64-unknown-linux-musl"))
        self.assertIn(["rustup", "toolchain", "install", "1.97.0-x86_64-unknown-linux-musl",
                       "--profile", "minimal", "--component", "rustfmt", "--component", "clippy"],
                      self.calls("rustup"))

    def test_native_musl_bun_uses_the_pinned_release_asset(self):
        self.write_system("/etc/os-release", "ID=alpine\n")
        self.configure(bun="1.0.0")
        archive = self.root / "downloads/bun-linux-x64-musl-baseline.zip"
        with zipfile.ZipFile(archive, "w") as package:
            package.writestr("bun-linux-x64-musl-baseline/bun", "#!/bin/bash\nprintf '1.3.14\\n'\n")
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        (self.root / "downloads/SHASUMS256.txt").write_text(f"{checksum}  {archive.name}\n")
        self.run_setup(self.host(phase="source"))
        self.assertTrue(any(call[-1].endswith("/" + archive.name) for call in self.calls("curl")))

    def installer_function(self, name):
        source = (SCRIPTS / "install.sh").read_text()
        return name + "() {" + source.split(name + "() {", 1)[1].split("\n}\n", 1)[0] + "\n}\n"

    def test_musl_package_subprocess_receives_toolchain_and_dynamic_link_flags(self):
        self.write_system("/etc/os-release", "ID=alpine\n")
        package = self.checkout / "scripts/agena/package-backend.sh"
        package.parent.mkdir(parents=True)
        package.write_text(r'''#!/bin/bash
printf '%s\n' "$AGENA_STABLE_TOOLCHAIN" > "$TEST_ROOT/package-toolchain"
printf '%s\n' "$@" > "$TEST_ROOT/package-arguments"
touch "$TEST_ROOT/package.tar.gz" "$TEST_ROOT/package.tar.gz.sha256"
printf 'Package ready: %s\nChecksum ready: %s\n' "$TEST_ROOT/package.tar.gz" "$TEST_ROOT/package.tar.gz.sha256"
''')
        script = self.installer_function("prepare_source_archive")
        script += 'resolve_source_root() { return 0; }\nSOURCE_REF=""\nSETUP_MODE=auto\n'
        script += f"INSTALL_DIR={shlex.quote(str(self.install))}\n"
        script += f"SOURCE_ROOT={shlex.quote(str(self.checkout))}\n"
        script += 'prepare_source_archive "$TEST_ROOT/tmp" x86_64-unknown-linux-musl\n'
        self.run_setup(script)
        self.assertEqual((self.root / "package-toolchain").read_text().strip(),
                         "1.97.0-x86_64-unknown-linux-musl")
        self.assertEqual((self.root / "package-arguments").read_text().splitlines(),
                         ["x86_64-unknown-linux-musl", "false", "-C target-feature=-crt-static"])

    def test_musl_official_release_has_a_source_hint_and_custom_archive_is_allowed(self):
        self.write_system("/etc/os-release", "ID=alpine\n")
        functions = self.installer_function("target_triple") + self.installer_function("prepare_candidate")
        common = functions + '\nSOURCE_MODE=0\nRELEASE_VERSION=""\nCHECKSUM=""\n'
        result = self.run_setup(common + 'ARCHIVE=""\nprepare_candidate "$TEST_ROOT/tmp"', expected=1)
        self.assertIn("--source", result.stderr)
        self.assertEqual(self.calls("curl"), [])
        script = common + 'ARCHIVE="own-musl-archive"\ndownload_file() { printf "custom-archive-reached\\n"; return 37; }\n'
        result = self.run_setup(script + 'prepare_candidate "$TEST_ROOT/tmp"', expected=37)
        self.assertIn("custom-archive-reached", result.stdout)
        self.assertNotIn("Published Agena releases", result.stderr)


if __name__ == "__main__":
    unittest.main()
